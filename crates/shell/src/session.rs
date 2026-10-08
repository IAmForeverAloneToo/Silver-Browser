//! The window's contents and what input does to them (M15,
//! docs/design/navigation.md, "The toolbar" and "Keys and the
//! pointer"): the toolbar across the top, the page under it, the status
//! bubble at the page's bottom left, and the keys, the pointer, the
//! input method and the clipboard made calls on the engine and the
//! toolbar. Its input comes in the shell's own small types rather than
//! winit's, so that tests drive it with an engine and the test server;
//! the window, the surface and the OS's clipboard stay in `main`.

use std::time::{Duration, Instant};

use engine::address::{self, Address};
use engine::{Engine, FrameTime, Link, LoadState, Preferences, Url};
use ui::{Action, PageState, StatusBubble, TooLong, Toolbar};

use crate::keys::{self, Key, Modifiers, Platform, Scroll, Shortcut};

/// The toolbar's height: the page starts below it.
const TOP: f32 = ui::HEIGHT as f32;
/// One wheel notch, or one arrow key, scrolls this far.
pub(crate) const LINE_PX: f32 = 40.0;
/// Presses of the left button at most this long apart, and this far,
/// are a double click, and three a triple click.
const MULTI_CLICK_TIME: Duration = Duration::from_millis(500);
const MULTI_CLICK_DISTANCE: f32 = 4.0;

/// The OS's clipboard, as the session reaches it: an error is what to
/// tell the user.
pub(crate) trait Clipboard {
    fn get(&mut self) -> Result<String, String>;
    fn set(&mut self, text: &str) -> Result<(), String>;
}

/// What has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Field,
    Page,
}

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Button {
    Left,
    Back,
    Forward,
    Other,
}

/// The pointer's shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    Arrow,
    /// Over a link.
    Hand,
    /// An I-beam, over the address field.
    Text,
}

/// What the left button was pressed on, while it is held.
enum Held {
    Toolbar,
    /// The scrollbar's thumb, grabbed this far below its top.
    Thumb(f32),
    /// A link: released on the same one, the press is a click.
    Link(Box<Link>),
    Page,
}

/// A press of the left button: when and where, and how many presses in
/// a row it made.
struct Press {
    at: Duration,
    x: f32,
    y: f32,
    count: u32,
}

/// The window's contents.
pub(crate) struct Session {
    engine: Engine,
    toolbar: Toolbar,
    bubble: StatusBubble,
    clipboard: Box<dyn Clipboard>,
    platform: Platform,
    mods: Modifiers,
    /// The window's size.
    size: (u32, u32),
    /// Where the pointer was last, in the window, and whether it is in
    /// the window still.
    pointer: (f32, f32),
    inside: bool,
    held: Option<Held>,
    last_press: Option<Press>,
    /// The link under the pointer.
    hover: Option<Link>,
    /// The latest notice, shown until the next input.
    notice: Option<String>,
    /// Whether an input method is composing text.
    composing: bool,
    /// The URL the command line asked for, until the first result of
    /// its load: a failure, or an encoding that cannot be decoded, is
    /// reported on standard error.
    start: Option<Url>,
    reports: Vec<String>,
    title: String,
    /// What the status bubble said in the last frame.
    drawn_bubble: Option<String>,
}

impl Session {
    /// A session showing `start`, or `about:silver` with the address
    /// field focused when the command line asked for nothing.
    pub(crate) fn new(
        mut engine: Engine,
        clipboard: Box<dyn Clipboard>,
        platform: Platform,
        start: Option<Url>,
    ) -> Session {
        let fonts = engine.fonts().clone();
        let about_silver = || Url::parse("about:silver").expect("about:silver parses");
        engine.navigate(start.clone().unwrap_or_else(about_silver));
        let mut session = Session {
            engine,
            toolbar: Toolbar::new(fonts.clone()),
            bubble: StatusBubble::new(fonts),
            clipboard,
            platform,
            mods: Modifiers::default(),
            size: (0, 0),
            pointer: (0.0, 0.0),
            inside: false,
            held: None,
            last_press: None,
            hover: None,
            notice: None,
            composing: false,
            start: start.clone(),
            reports: Vec::new(),
            title: String::new(),
            drawn_bubble: None,
        };
        session.sync();
        if start.is_none() {
            session.toolbar.focus();
        }
        session
    }

    #[cfg(test)]
    pub(crate) fn engine(&self) -> &Engine {
        &self.engine
    }

    #[cfg(test)]
    pub(crate) fn toolbar(&self) -> &Toolbar {
        &self.toolbar
    }

    /// What has the keyboard.
    pub(crate) fn focus(&self) -> Focus {
        if self.toolbar.is_focused() {
            Focus::Field
        } else {
            Focus::Page
        }
    }

    /// The window's title.
    pub(crate) fn title(&self) -> &str {
        &self.title
    }

    /// What the status bubble says: where the link under the pointer
    /// goes, or else the latest notice.
    pub(crate) fn bubble_text(&self) -> Option<&str> {
        self.hover
            .as_ref()
            .map(|link| link.url().as_str())
            .or(self.notice.as_deref())
    }

    /// The pointer's shape where it is.
    pub(crate) fn shape(&self) -> Shape {
        let (x, y) = self.pointer;
        if !self.inside {
            Shape::Arrow
        } else if y < TOP {
            match self.toolbar.cursor(x, y) {
                ui::Cursor::Text => Shape::Text,
                ui::Cursor::Arrow => Shape::Arrow,
            }
        } else if self.hover.is_some() {
            Shape::Hand
        } else {
            Shape::Arrow
        }
    }

    /// While the field has the keyboard, where its caret is in the
    /// window, `(x, y, w, h)`, for an input method's window to stand
    /// by; otherwise no input method is wanted.
    pub(crate) fn ime_area(&mut self) -> Option<(f32, f32, f32, f32)> {
        (self.focus() == Focus::Field).then(|| self.toolbar.caret_area())
    }

    /// What is to be reported on standard error since this was last
    /// asked.
    pub(crate) fn take_reports(&mut self) -> Vec<String> {
        std::mem::take(&mut self.reports)
    }

    /// Whether a frame is wanted.
    pub(crate) fn needs_frame(&self) -> bool {
        self.engine.needs_frame()
            || self.toolbar.needs_frame()
            || self.drawn_bubble.as_deref() != self.bubble_text()
    }

    /// When a frame is next wanted, on the clock the frames are drawn
    /// by, when none is now: the end of an animation's delay (M22.3).
    pub(crate) fn next_frame_at(&self) -> Option<Duration> {
        self.engine.next_frame_at()
    }

    /// The window has a new size.
    pub(crate) fn resized(&mut self, width: u32, height: u32) {
        self.size = (width, height);
    }

    /// The user's preferences, as the shell now reads them (M20.3).
    pub(crate) fn set_preferences(&mut self, preferences: Preferences) {
        self.engine.set_preferences(preferences);
    }

    /// The modifier keys held changed.
    pub(crate) fn modifiers(&mut self, mods: Modifiers) {
        self.mods = mods;
    }

    /// A key pressed, with the text it types, if any: a shortcut, or
    /// text typed into the field.
    pub(crate) fn key(&mut self, key: Key, text: Option<&str>) {
        if self.composing {
            return;
        }
        self.notice = None;
        let in_field = self.focus() == Focus::Field;
        match keys::shortcut(&key, self.mods, self.platform, in_field) {
            Some(shortcut) => self.shortcut(shortcut),
            None => {
                if in_field
                    && keys::types_text(self.mods, self.platform)
                    && let Some(text) = text
                {
                    self.insert(text);
                }
            }
        }
        self.sync();
    }

    /// An input method's text being composed: while there is some, the
    /// keys are the input method's.
    pub(crate) fn preedit(&mut self, text: &str) {
        self.composing = !text.is_empty();
    }

    /// Text an input method committed, inserted as typed text is.
    pub(crate) fn commit(&mut self, text: &str) {
        self.composing = false;
        self.notice = None;
        if self.focus() == Focus::Field {
            self.insert(text);
        }
        self.sync();
    }

    /// The pointer moved to `(x, y)` in the window.
    pub(crate) fn pointer_moved(&mut self, x: f32, y: f32) {
        self.pointer = (x, y);
        self.inside = true;
        if let Some(Held::Thumb(grab)) = self.held {
            self.drag_thumb(grab);
        }
        self.toolbar.pointer_moved(x, y);
        self.refresh_hover();
    }

    /// The pointer left the window.
    pub(crate) fn pointer_left(&mut self) {
        self.inside = false;
        self.toolbar.pointer_left();
        self.hover = None;
    }

    /// A mouse button pressed, at `at` on the shell's clock.
    pub(crate) fn press(&mut self, button: Button, at: Duration) {
        self.notice = None;
        match button {
            Button::Left => self.press_left(at),
            Button::Back => self.engine.go_back(),
            Button::Forward => self.engine.go_forward(),
            Button::Other => {}
        }
        self.sync();
    }

    /// A mouse button released: a press and a release of the left
    /// button on the same button of the toolbar, or the same link, are
    /// a click.
    pub(crate) fn release(&mut self, button: Button) {
        if button != Button::Left {
            return;
        }
        let (x, y) = self.pointer;
        match self.held.take() {
            Some(Held::Toolbar) => {
                if let Some(action) = self.toolbar.release(x, y) {
                    self.act(action);
                }
            }
            Some(Held::Link(link)) => {
                if y >= TOP
                    && self
                        .engine
                        .link_at(x, y - TOP)
                        .is_some_and(|under| under.node() == link.node())
                {
                    self.engine.follow_link(&link);
                }
            }
            Some(Held::Thumb(_) | Held::Page) | None => {}
        }
        self.sync();
        self.refresh_hover();
    }

    /// The wheel turned by `dy` pixels, positive down: the page scrolls
    /// at the pointer, so a scroll container under it takes it first.
    pub(crate) fn wheel(&mut self, dy: f32) {
        self.notice = None;
        let (x, y) = self.pointer;
        if y >= TOP {
            self.engine.wheel(x, y - TOP, dy);
        }
        self.refresh_hover();
    }

    /// A fetch finished: the engine takes what arrived.
    pub(crate) fn network(&mut self) {
        self.engine.poll_network();
        if let Some(start) = self.start.take() {
            if let LoadState::Failed(error) = self.engine.load_state() {
                self.reports.push(format!("{start}: {error}"));
            } else if let Some(encoding) = self.engine.undecodable_encoding() {
                self.reports.push(format!(
                    "{start} is in {encoding}, which this version cannot decode; it is shown as \
                     UTF-8"
                ));
            }
        }
        self.sync();
        self.refresh_hover();
    }

    /// Draws the window, `width` by `height`, for `time`, into `out` as
    /// 0RGB pixels: the toolbar, the page under it, and the status
    /// bubble over the page.
    pub(crate) fn draw(&mut self, width: u32, height: u32, time: FrameTime, out: &mut [u32]) {
        let w = width as usize;
        if out.len() < w * height as usize {
            return;
        }
        self.size = (width, height);
        let bar_rows = (ui::HEIGHT.min(height)) as usize;
        let bar = self.toolbar.frame(width);
        put(&bar[..bar_rows * w * 4], &mut out[..bar_rows * w]);
        let page_height = height - bar_rows as u32;
        if page_height > 0 {
            let page = self.engine.frame_at(width, page_height, time);
            // Past the engine's frame cap, the last frame stays shown.
            if page.len() == w * page_height as usize * 4 {
                put(page, &mut out[bar_rows * w..]);
            }
            // A scroll moves the page under a pointer that stays.
            self.refresh_hover();
        }
        self.drawn_bubble = self.bubble_text().map(str::to_owned);
        if let Some(text) = &self.drawn_bubble
            && page_height >= ui::status::HEIGHT
            && let Some((bubble_w, pixels)) = self.bubble.frame(text, width / 2)
        {
            let (bubble_w, top) = (bubble_w as usize, (height - ui::status::HEIGHT) as usize);
            for (row, line) in pixels.chunks_exact(bubble_w * 4).enumerate() {
                let at = (top + row) * w;
                put(line, &mut out[at..at + bubble_w]);
            }
        }
    }

    /// Draws ahead until `until`, once a frame is presented: the tiles
    /// the next frames of a scroll will show (M19.4).
    pub(crate) fn draw_ahead(&mut self, until: Instant) {
        self.engine.draw_ahead(until);
    }

    /// Carries out what a key asks for.
    fn shortcut(&mut self, shortcut: Shortcut) {
        let in_field = self.focus() == Focus::Field;
        match shortcut {
            Shortcut::FocusField => self.toolbar.focus(),
            Shortcut::Back => self.engine.go_back(),
            Shortcut::Forward => self.engine.go_forward(),
            Shortcut::Reload => self.engine.reload(),
            Shortcut::Escape if in_field => {
                self.toolbar.escape();
            }
            Shortcut::Escape => {
                if *self.engine.load_state() == LoadState::Loading {
                    self.engine.stop();
                }
            }
            Shortcut::Copy if in_field => {
                if let Some(text) = self.toolbar.selected() {
                    self.copy(&text);
                }
            }
            Shortcut::Cut if in_field => {
                // The text leaves the field only once the clipboard
                // has it.
                if let Some(text) = self.toolbar.selected()
                    && self.copy(&text)
                {
                    self.toolbar.cut();
                }
            }
            Shortcut::Paste if in_field => match self.clipboard.get() {
                Ok(text) => self.insert(&text),
                Err(error) => self.notice = Some(clipboard_notice(&error)),
            },
            // The page has no selection to copy from, nor a field to
            // paste into, yet.
            Shortcut::Copy | Shortcut::Cut | Shortcut::Paste => {}
            Shortcut::Enter => {
                if let Some(action) = self.toolbar.enter() {
                    self.act(action);
                }
            }
            Shortcut::Edit(edit) => self.toolbar.command(edit),
            Shortcut::Scroll(scroll) => self.scroll(scroll),
        }
    }

    /// Carries out what the toolbar asks for.
    fn act(&mut self, action: Action) {
        match action {
            Action::Back => self.engine.go_back(),
            Action::Forward => self.engine.go_forward(),
            Action::Reload => self.engine.reload(),
            Action::Stop => self.engine.stop(),
            Action::Navigate(text) => match address::parse(&text) {
                Address::Url(url) => self.engine.navigate(url),
                refused => {
                    // The field keeps the text, and the keyboard.
                    self.toolbar.keep_editing();
                    self.notice = refused.notice();
                }
            },
        }
    }

    /// Puts `text` on the clipboard: whether it took it.
    fn copy(&mut self, text: &str) -> bool {
        match self.clipboard.set(text) {
            Ok(()) => true,
            Err(error) => {
                self.notice = Some(clipboard_notice(&error));
                false
            }
        }
    }

    /// Inserts `text` into the field, or says why it did not.
    fn insert(&mut self, text: &str) {
        if self.toolbar.insert(text) == Err(TooLong) {
            self.notice = Some(format!(
                "The address field holds at most {} bytes, so the text was not put in",
                ui::field::MAX_LEN
            ));
        }
    }

    fn scroll(&mut self, scroll: Scroll) {
        let page = self.page_step();
        match scroll {
            Scroll::LineUp => self.engine.scroll_by(-LINE_PX),
            Scroll::LineDown => self.engine.scroll_by(LINE_PX),
            Scroll::PageUp => self.engine.scroll_by(-page),
            Scroll::PageDown => self.engine.scroll_by(page),
            Scroll::Top => self.engine.scroll_to(0.0),
            Scroll::Bottom => self.engine.scroll_to(f32::INFINITY),
        }
    }

    /// A page's scroll: the page's height less a line, so a line of
    /// context stays.
    fn page_step(&self) -> f32 {
        let height = self.size.1 as f32 - TOP;
        (height - LINE_PX).max(LINE_PX)
    }

    /// The left button pressed: on the toolbar, it is the toolbar's; on
    /// the page, which takes the keyboard, it grabs the scrollbar's
    /// thumb, jumps a page from its track, or notes the link under it.
    fn press_left(&mut self, at: Duration) {
        let (x, y) = self.pointer;
        let count = match &self.last_press {
            Some(last)
                if at.saturating_sub(last.at) <= MULTI_CLICK_TIME
                    && (x - last.x).abs() <= MULTI_CLICK_DISTANCE
                    && (y - last.y).abs() <= MULTI_CLICK_DISTANCE =>
            {
                (last.count + 1).min(3)
            }
            _ => 1,
        };
        self.last_press = Some(Press { at, x, y, count });
        if y < TOP {
            self.toolbar.press(x, y, count, self.mods.shift);
            self.held = Some(Held::Toolbar);
            return;
        }
        self.toolbar.blur();
        let y = y - TOP;
        let page = self.page_step();
        if let Some(bar) = self.engine.scrollbar()
            && bar.contains(x, y)
        {
            self.held = Some(if bar.thumb_contains(x, y) {
                Held::Thumb(y - bar.thumb_y)
            } else {
                self.engine
                    .scroll_by(if y < bar.thumb_y { -page } else { page });
                Held::Page
            });
            return;
        }
        self.held = Some(
            self.engine
                .link_at(x, y)
                .map_or(Held::Page, |link| Held::Link(Box::new(link))),
        );
    }

    /// The pointer moved with the thumb held: the thumb follows it.
    fn drag_thumb(&mut self, grab: f32) {
        let y = self.pointer.1 - TOP;
        if let Some(bar) = self.engine.scrollbar() {
            let travel = bar.h - bar.thumb_h;
            if travel > 0.0 {
                self.engine.scroll_to_fraction((y - grab) / travel);
            }
        }
    }

    /// Finds the link under the pointer, if it is over the page and not
    /// over the scrollbar, nor dragging on the toolbar or the thumb.
    fn refresh_hover(&mut self) {
        let (x, y) = self.pointer;
        let free = !matches!(self.held, Some(Held::Toolbar | Held::Thumb(_)));
        self.hover = None;
        if self.inside && free && y >= TOP {
            let y = y - TOP;
            let on_bar = self
                .engine
                .scrollbar()
                .is_some_and(|bar| bar.contains(x, y));
            if !on_bar {
                self.hover = self.engine.link_at(x, y);
            }
        }
    }

    /// Brings what the window shows of the engine up to date: the
    /// toolbar, the title, and a notice the engine has.
    fn sync(&mut self) {
        if let Some(notice) = self.engine.take_notice() {
            self.notice = Some(notice);
        }
        let url = self.engine.url().map(|url| url.as_str().to_owned());
        self.toolbar.update(PageState {
            url: url.as_deref(),
            can_go_back: self.engine.can_go_back(),
            can_go_forward: self.engine.can_go_forward(),
            loading: *self.engine.load_state() == LoadState::Loading,
        });
        self.title = ui::window_title(&self.engine.title(), url.as_deref());
    }
}

/// What the user is told when the clipboard cannot be reached.
fn clipboard_notice(error: &str) -> String {
    format!("The clipboard could not be reached: {error}")
}

/// Copies RGBA8 pixels into 0RGB ones, as the surface takes them.
fn put(rgba: &[u8], out: &mut [u32]) {
    let (pixels, _) = rgba.as_chunks::<4>();
    for (dst, [r, g, b, _]) in out.iter_mut().zip(pixels) {
        *dst = (*r as u32) << 16 | (*g as u32) << 8 | *b as u32;
    }
}

#[cfg(test)]
mod tests;
