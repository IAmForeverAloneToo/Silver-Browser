//! The window's contents and what input does to them (M15,
//! docs/design/navigation.md, "The toolbar" and "Keys and the
//! pointer"): the toolbar across the top, the page under it, a site's
//! permission prompt over the page's top (M33.4), the status
//! bubble at the page's bottom left, the DevTools panel under the page
//! while it is open (M36), and the keys, the pointer, the
//! input method and the clipboard made calls on the engine and the
//! toolbar. Its input comes in the shell's own small types rather than
//! winit's, so that tests drive it with an engine and the test server;
//! the window, the surface and the OS's clipboard stay in `main`.

use std::time::{Duration, Instant};

use engine::address::{self, Address};
use engine::{Engine, FrameTime, Link, LoadState, PermissionPrompt, Preferences, Url, View};
use ui::{Action, Choice, PageState, PromptBar, StatusBubble, TooLong, Toolbar};

use crate::devtools::Router;
use crate::keys::{self, Key, Modifiers, Platform, Shortcut};

/// The toolbar's height: the page starts below it.
const TOP: f32 = ui::HEIGHT as f32;
/// One wheel notch, or one arrow key, scrolls this far.
pub(crate) const LINE_PX: f32 = 40.0;
/// Presses of the left button at most this long apart, and this far,
/// are a double click, and three a triple click.
const MULTI_CLICK_TIME: Duration = Duration::from_millis(500);
const MULTI_CLICK_DISTANCE: f32 = 4.0;
/// The share of the window under the toolbar the DevTools panel takes
/// while it is open (M36): the page has the rest, above it.
pub(crate) const PANEL_SHARE: f32 = 0.4;

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
    /// The DevTools panel (M36).
    Panel,
}

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Button {
    Left,
    /// The middle button, or the wheel's.
    Middle,
    Right,
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
    /// An I-beam, over the address field or a text control.
    Text,
    /// Another the page's `cursor` names (M26.4).
    Page(engine::Cursor),
}

/// What the left button was pressed on, while it is held.
enum Held {
    Toolbar,
    /// The scrollbar's thumb, grabbed this far below its top.
    Thumb(f32),
    /// The scrollbar's track, which the press scrolled by a page.
    Track,
    /// The page, which was given the press (M26.5).
    Page,
    /// The DevTools panel, which was given the press (M36).
    Panel,
}

/// A press of the left button: when and where, and how many presses in
/// a row it made.
struct Press {
    at: Duration,
    x: f32,
    y: f32,
    count: u32,
}

/// The window's contents: the page an engine shows (M15), or one a
/// renderer shows, through the browser's proxy of it (M34.5).
pub(crate) struct Session<V: View = Engine> {
    engine: V,
    toolbar: Toolbar,
    bubble: StatusBubble,
    /// The permission prompt the engine wants shown (M33.4), its bar, and
    /// which one the last frame showed.
    prompt: Option<PermissionPrompt>,
    prompt_bar: PromptBar,
    drawn_prompt: Option<u64>,
    clipboard: Box<dyn Clipboard>,
    platform: Platform,
    mods: Modifiers,
    /// The window's size.
    size: (u32, u32),
    /// The window's scale factor (M32.4): device pixels to a CSS pixel.
    /// The session's own positions and sizes are CSS pixels; the window's
    /// are device pixels, divided by it on the way in.
    scale: f32,
    /// Where the pointer was last, in the window, and whether it is in
    /// the window still.
    pointer: (f32, f32),
    inside: bool,
    held: Option<Held>,
    last_press: Option<Press>,
    /// The link under the pointer.
    hover: Option<Link>,
    /// The cursor the page gives the point under the pointer (M26.4).
    cursor: engine::Cursor,
    /// The latest notice, shown until the next input.
    notice: Option<String>,
    /// Whether an input method is composing text.
    composing: bool,
    /// Whether the window has the system's focus (M26.6).
    window_focused: bool,
    /// The URL the command line asked for, until the first result of
    /// its load: a failure, or an encoding that cannot be decoded, is
    /// reported on standard error.
    start: Option<Url>,
    reports: Vec<String>,
    title: String,
    /// What the status bubble said in the last frame.
    drawn_bubble: Option<String>,
    /// The DevTools panel while it is open (M36): a view of the same kind
    /// as the page's, showing the panel's page; whether it has the
    /// keyboard; the cursor it gives the point under the pointer; and
    /// what passes messages between it and the page.
    panel: Option<V>,
    panel_keys: bool,
    panel_cursor: engine::Cursor,
    router: Router,
    /// The window's size in device pixels, which the regions are cut of.
    device: (u32, u32),
}

impl<V: View> Session<V> {
    /// A session showing `start`, or `about:silver` with the address
    /// field focused when the command line asked for nothing.
    pub(crate) fn new(
        mut engine: V,
        clipboard: Box<dyn Clipboard>,
        platform: Platform,
        start: Option<Url>,
    ) -> Session<V> {
        let fonts = engine.fonts().clone();
        let about_silver = || Url::parse("about:silver").expect("about:silver parses");
        engine.navigate(start.clone().unwrap_or_else(about_silver));
        let mut session = Session {
            engine,
            toolbar: Toolbar::new(fonts.clone()),
            prompt: None,
            prompt_bar: PromptBar::new(fonts.clone()),
            drawn_prompt: None,
            bubble: StatusBubble::new(fonts),
            clipboard,
            platform,
            mods: Modifiers::default(),
            size: (0, 0),
            scale: 1.0,
            pointer: (0.0, 0.0),
            inside: false,
            held: None,
            last_press: None,
            hover: None,
            cursor: engine::Cursor::Default,
            notice: None,
            composing: false,
            window_focused: true,
            start: start.clone(),
            reports: Vec::new(),
            title: String::new(),
            drawn_bubble: None,
            panel: None,
            panel_keys: false,
            panel_cursor: engine::Cursor::Default,
            router: Router::default(),
            device: (0, 0),
        };
        session.sync();
        if start.is_none() {
            session.toolbar.focus();
        }
        session.sync_window_focus();
        session
    }

    /// Whether the frame drawn holds the page's own pixels (M35.6).
    pub(crate) fn has_page_frame(&self) -> bool {
        self.engine.has_page_frame()
    }

    #[cfg(test)]
    pub(crate) fn engine(&self) -> &V {
        &self.engine
    }

    #[cfg(test)]
    pub(crate) fn engine_mut(&mut self) -> &mut V {
        &mut self.engine
    }

    /// Writes to the profile what changed since it was last written
    /// (M28): the window has closed.
    pub(crate) fn flush_profile(&mut self) -> Result<(), String> {
        self.engine.flush_profile().map_err(|e| e.to_string())
    }

    #[cfg(test)]
    pub(crate) fn toolbar(&self) -> &Toolbar {
        &self.toolbar
    }

    /// What has the keyboard.
    pub(crate) fn focus(&self) -> Focus {
        if self.toolbar.is_focused() {
            Focus::Field
        } else if self.panel_keys && self.panel.is_some() {
            Focus::Panel
        } else {
            Focus::Page
        }
    }

    /// The DevTools panel, while it is open (M36).
    #[cfg(test)]
    pub(crate) fn panel_mut(&mut self) -> Option<&mut V> {
        self.panel.as_mut()
    }

    /// The rows of a window `height` device pixels tall, top to bottom:
    /// the toolbar's, the page's, and the DevTools panel's (M36), none
    /// while it is closed.
    fn rows(&self, height: u32) -> (u32, u32, u32) {
        let bar = ui::rows(ui::HEIGHT, self.scale).min(height);
        let rest = height - bar;
        let panel = match self.panel {
            Some(_) => (rest as f32 * PANEL_SHARE).round() as u32,
            None => 0,
        };
        (bar, rest - panel, panel)
    }

    /// Where the DevTools panel starts down the window, in CSS pixels.
    fn panel_top(&self) -> f32 {
        let (bar, page, _) = self.rows(self.device.1);
        (bar + page) as f32 / self.scale
    }

    /// Whether `y` down the window is over the DevTools panel.
    fn in_panel(&self, y: f32) -> bool {
        self.panel.is_some() && y >= self.panel_top()
    }

    /// The page's and the panel's viewports, in CSS pixels, as the
    /// window's size and the panel cut them.
    fn place_views(&mut self) {
        let css = |v: u32| (v as f32 / self.scale).round() as u32;
        let (width, height) = self.size;
        let (_, _, panel) = self.rows(self.device.1);
        let page = height - ui::HEIGHT.min(height);
        let panel = css(panel).min(page);
        self.engine.set_viewport(width, page - panel);
        if let Some(view) = &mut self.panel {
            view.set_viewport(width, panel);
        }
    }

    /// Opens the DevTools panel under the page, with the keyboard, or
    /// closes it (M36).
    pub(crate) fn toggle_devtools(&mut self) {
        if self.panel.take().is_some() {
            self.panel_keys = false;
        } else {
            self.panel = self.engine.devtools_panel();
            if let Some(panel) = &mut self.panel {
                panel.set_scale_factor(self.scale);
            }
            self.router.opened();
            self.panel_keys = self.panel.is_some();
            self.toolbar.blur();
        }
        self.place_views();
        self.sync_window_focus();
        self.sync();
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
        } else if self.in_panel(y) {
            shape_of(self.panel_cursor)
        } else {
            shape_of(self.cursor)
        }
    }

    /// While the field has the keyboard, where its caret is in the
    /// window, `(x, y, w, h)`, for an input method's window to stand
    /// by; otherwise no input method is wanted.
    pub(crate) fn ime_area(&mut self) -> Option<(f32, f32, f32, f32)> {
        match self.focus() {
            Focus::Field => return Some(self.toolbar.caret_area()),
            Focus::Panel => {
                let top = self.panel_top();
                let (x, y, w, h) = self.panel.as_mut()?.caret_area()?;
                return Some((x, y + top, w, h));
            }
            Focus::Page => {}
        }
        // The page's text control focused (M26.8), below the toolbar.
        let (x, y, w, h) = self.engine.caret_area()?;
        Some((x, y + TOP, w, h))
    }

    /// What is to be reported on standard error since this was last
    /// asked.
    pub(crate) fn take_reports(&mut self) -> Vec<String> {
        std::mem::take(&mut self.reports)
    }

    /// Whether a frame is wanted.
    pub(crate) fn needs_frame(&self) -> bool {
        self.engine.needs_frame()
            || self.panel.as_ref().is_some_and(View::needs_frame)
            || self.toolbar.needs_frame()
            || self.drawn_bubble.as_deref() != self.bubble_text()
            || self.drawn_prompt != self.prompt.as_ref().map(|prompt| prompt.id)
    }

    /// The permission prompt shown, if any (M33.4).
    #[cfg(test)]
    pub(crate) fn prompt(&self) -> Option<&PermissionPrompt> {
        self.prompt.as_ref()
    }

    /// When a frame is next wanted, on the clock the frames are drawn
    /// by, when none is now: the end of an animation's delay (M22.3).
    pub(crate) fn next_frame_at(&self) -> Option<Duration> {
        earliest(
            self.engine.next_frame_at(),
            self.panel.as_ref().and_then(View::next_frame_at),
        )
    }

    /// When the page's next task is due, on the same clock (M26): its
    /// next timer.
    pub(crate) fn next_task_at(&self) -> Option<Duration> {
        earliest(
            self.engine.next_task_at(),
            self.panel.as_ref().and_then(View::next_task_at),
        )
    }

    /// The page's tasks that are due run (M26): what its scripts wrote
    /// to the console is printed, and what they changed shows.
    pub(crate) fn tick(&mut self) {
        let panel = self
            .panel
            .as_ref()
            .is_some_and(|panel| panel.next_task_at().is_some());
        if self.engine.next_task_at().is_none() && !panel {
            return;
        }
        self.engine.run_tasks();
        self.console();
        if let Some(panel) = &mut self.panel {
            panel.run_tasks();
            print_panel_console(panel);
        }
        self.sync();
    }

    /// The window is hidden, covered or minimized, or shown again (M26):
    /// the page hears it, and what its scripts wrote is printed.
    pub(crate) fn set_hidden(&mut self, hidden: bool) {
        self.engine.set_hidden(hidden);
        if let Some(panel) = &mut self.panel {
            panel.set_hidden(hidden);
        }
        self.console();
        self.sync();
    }

    /// The window has a new size, in device pixels: the page's viewport,
    /// under the toolbar, has too, as a page loading before the next
    /// frame reads it (M25).
    pub(crate) fn resized(&mut self, width: u32, height: u32) {
        let css = |v: u32| (v as f32 / self.scale).round() as u32;
        self.size = (css(width), css(height));
        self.device = (width, height);
        self.place_views();
    }

    /// The window's scale factor changed (M32.4): the page is drawn under
    /// it, its viewport and the pointer taken in CSS pixels by it.
    pub(crate) fn set_scale(&mut self, scale: f32, width: u32, height: u32) {
        if !(scale.is_finite() && scale > 0.0) {
            return;
        }
        self.engine.set_scale_factor(scale);
        self.scale = self.engine.scale_factor();
        if let Some(panel) = &mut self.panel {
            panel.set_scale_factor(scale);
        }
        self.resized(width, height);
    }

    /// The window's scale factor.
    pub(crate) fn scale(&self) -> f32 {
        self.scale
    }

    /// The user's preferences, as the shell now reads them (M20.3).
    pub(crate) fn set_preferences(&mut self, preferences: Preferences) {
        if let Some(panel) = &mut self.panel {
            panel.set_preferences(preferences);
        }
        self.engine.set_preferences(preferences);
    }

    /// The modifier keys held changed.
    pub(crate) fn modifiers(&mut self, mods: Modifiers) {
        self.mods = mods;
        self.engine.set_modifiers(engine::Modifiers {
            shift: mods.shift,
            ctrl: mods.ctrl,
            alt: mods.alt,
            meta: mods.logo,
        });
    }

    /// A key pressed, with the text it types, as the tests press them:
    /// the page hears of it by its name.
    #[cfg(test)]
    pub(crate) fn key(&mut self, key: Key, text: Option<&str>) {
        let name = match &key {
            Key::Enter => "Enter",
            Key::Escape => "Escape",
            Key::Backspace => "Backspace",
            Key::Delete => "Delete",
            Key::Insert => "Insert",
            Key::Left => "ArrowLeft",
            Key::Right => "ArrowRight",
            Key::Up => "ArrowUp",
            Key::Down => "ArrowDown",
            Key::Home => "Home",
            Key::End => "End",
            Key::PageUp => "PageUp",
            Key::PageDown => "PageDown",
            Key::Space => " ",
            Key::Tab => "Tab",
            Key::F5 => "F5",
            Key::F6 => "F6",
            Key::F12 => "F12",
            Key::Char(c) => c.as_str(),
            Key::Other => "Unidentified",
        };
        let mut page = engine::Key::new(name, "");
        page.text = text.map(str::to_owned);
        self.key_down(Some(key), page);
    }

    /// A key pressed: `key` as the browser's keys have it, none for a
    /// modifier key, and `page` as the page hears of it (M26.6). The
    /// browser's shortcuts come first; then the field's keys, or the
    /// page's, which may hand the keyboard back to the field.
    pub(crate) fn key_down(&mut self, key: Option<Key>, page: engine::Key) {
        if self.composing {
            return;
        }
        self.notice = None;
        let in_field = self.focus() == Focus::Field;
        // An open list takes every key (M26.8).
        let popup = match (self.focus(), &self.panel) {
            (Focus::Panel, Some(panel)) => panel.popup_open(),
            _ => self.engine.popup_open(),
        };
        let shortcut = key
            .as_ref()
            .filter(|_| in_field || !popup)
            .and_then(|key| keys::shortcut(key, self.mods, self.platform, in_field));
        match shortcut {
            Some(shortcut) => self.shortcut(shortcut),
            None if in_field => {
                if keys::types_text(self.mods, self.platform)
                    && let Some(text) = page.text.as_deref()
                {
                    self.insert(text);
                }
            }
            None => {
                if self.keyed().key_down(&page) == engine::KeyOutcome::FocusLeft {
                    self.toolbar.focus();
                }
            }
        }
        self.sync();
    }

    /// The view the keys go to when the field has not got them: the
    /// DevTools panel when it has the keyboard, else the page.
    fn keyed(&mut self) -> &mut V {
        match (self.focus(), &mut self.panel) {
            (Focus::Panel, Some(panel)) => panel,
            _ => &mut self.engine,
        }
    }

    /// A key released: the page's, while it has the keyboard (M26.6).
    pub(crate) fn key_up(&mut self, page: engine::Key) {
        if self.composing || self.focus() == Focus::Field {
            return;
        }
        self.keyed().key_up(&page);
        self.sync();
    }

    /// The window gained the system's focus, or lost it (M26.6).
    pub(crate) fn set_window_focus(&mut self, focused: bool) {
        self.window_focused = focused;
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
        } else {
            // Into the page's text control focused, if any (M26.8), or
            // the panel's.
            self.keyed().insert_text(text);
        }
        self.sync();
    }

    /// The pointer moved to `(x, y)` in the window, in device pixels.
    pub(crate) fn pointer_moved(&mut self, x: f32, y: f32) {
        let (x, y) = (x / self.scale, y / self.scale);
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
        self.engine.pointer_leave();
        if let Some(panel) = &mut self.panel {
            panel.pointer_leave();
        }
    }

    /// A mouse button pressed, at `at` on the shell's clock.
    pub(crate) fn press(&mut self, button: Button, at: Duration) {
        self.notice = None;
        match button {
            Button::Left => self.press_left(at),
            Button::Middle => self.press_page(engine::Button::Auxiliary, 1),
            Button::Right => self.press_page(engine::Button::Secondary, 1),
            Button::Back => self.engine.go_back(),
            Button::Forward => self.engine.go_forward(),
            Button::Other => {}
        }
        self.sync();
    }

    /// A mouse button released: a press and a release of the left
    /// button on the same button of the toolbar are a click; one on the
    /// page is the page's (M26.5), which follows a link clicked.
    pub(crate) fn release(&mut self, button: Button) {
        let (x, y) = self.pointer;
        let other = match button {
            Button::Left => None,
            Button::Middle => Some(engine::Button::Auxiliary),
            Button::Right => Some(engine::Button::Secondary),
            Button::Back | Button::Forward | Button::Other => return,
        };
        if let Some(other) = other {
            return match self.in_panel(y) {
                true => self.release_panel(other, 1),
                false => self.release_page(other, 1),
            };
        }
        match self.held.take() {
            Some(Held::Toolbar) => {
                if let Some(action) = self.toolbar.release(x, y) {
                    self.act(action);
                }
            }
            Some(Held::Page) => {
                let count = self.last_press.as_ref().map_or(1, |press| press.count);
                self.release_page(engine::Button::Primary, count);
                return;
            }
            Some(Held::Panel) => {
                let count = self.last_press.as_ref().map_or(1, |press| press.count);
                self.release_panel(engine::Button::Primary, count);
                return;
            }
            Some(Held::Thumb(_) | Held::Track) | None => {}
        }
        self.sync();
        self.refresh_hover();
    }

    /// A button other than the left pressed over the page: the page's.
    fn press_page(&mut self, button: engine::Button, count: u32) {
        let (x, y) = self.pointer;
        if self.inside && self.in_panel(y) {
            let top = self.panel_top();
            if let Some(panel) = &mut self.panel {
                panel.pointer_down(x, y - top, button, count);
            }
        } else if self.inside && y >= TOP {
            self.engine.pointer_down(x, y - TOP, button, count);
        }
    }

    /// A button released over the DevTools panel, or after a press on
    /// it (M36): the panel's.
    fn release_panel(&mut self, button: engine::Button, count: u32) {
        let (x, y) = self.pointer;
        let top = self.panel_top();
        if let Some(panel) = &mut self.panel {
            panel.pointer_up(x, (y - top).max(0.0), button, count);
        }
        self.sync();
        self.refresh_hover();
    }

    /// A button released over the page, or after a press on it: the
    /// page's, its click followed by what it did.
    fn release_page(&mut self, button: engine::Button, count: u32) {
        let (x, y) = self.pointer;
        self.engine.pointer_up(x, (y - TOP).max(0.0), button, count);
        self.sync();
        self.refresh_hover();
    }

    /// The wheel turned by `dy` pixels, positive down: the page scrolls
    /// at the pointer, so a scroll container under it takes it first.
    pub(crate) fn wheel(&mut self, dy: f32) {
        self.notice = None;
        let (x, y) = self.pointer;
        if self.in_panel(y) {
            let top = self.panel_top();
            if let Some(panel) = &mut self.panel {
                panel.wheel(x, y - top, dy);
            }
        } else if y >= TOP {
            self.engine.wheel(x, y - TOP, dy);
        }
        self.refresh_hover();
    }

    /// A fetch finished: the engine takes what arrived, and what the
    /// page's scripts wrote to its console is printed.
    pub(crate) fn network(&mut self) {
        self.engine.poll_network();
        self.console();
        if let Some(panel) = &mut self.panel {
            panel.poll_network();
            print_panel_console(panel);
        }
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
        let css = |v: u32| (v as f32 / self.scale).round() as u32;
        self.size = (css(width), css(height));
        self.device = (width, height);
        let (bar_rows, page_height, panel_rows) = self.rows(height);
        let bar_rows = bar_rows as usize;
        let bar = self.toolbar.frame(width, self.scale);
        put(&bar[..bar_rows * w * 4], &mut out[..bar_rows * w]);
        if page_height > 0 {
            let page = self.engine.frame_at(width, page_height, time);
            // Past the engine's frame cap, the last frame stays shown.
            if page.len() == w * page_height as usize * 4 {
                put(page, &mut out[bar_rows * w..]);
            }
            // A scroll moves the page under a pointer that stays.
            self.refresh_hover();
            // What the frame's animation callbacks wrote (M30), printed
            // as a task's is.
            self.console();
        }
        // The DevTools panel under the page (M36).
        if let Some(panel) = &mut self.panel
            && panel_rows > 0
        {
            let pixels = panel.frame_at(width, panel_rows, time);
            if pixels.len() == w * panel_rows as usize * 4 {
                let at = (bar_rows + page_height as usize) * w;
                put(pixels, &mut out[at..]);
            }
            print_panel_console(panel);
        }
        // The permission prompt over the top of the page (M33.4).
        self.drawn_prompt = self.prompt.as_ref().map(|prompt| prompt.id);
        let prompt_rows = ui::rows(ui::prompt::HEIGHT, self.scale) as usize;
        if let Some(prompt) = &self.prompt
            && page_height as usize >= prompt_rows
        {
            let pixels = self.prompt_bar.frame(&prompt.text(), width, self.scale);
            let rows = bar_rows * w..(bar_rows + prompt_rows) * w;
            put(&pixels[..prompt_rows * w * 4], &mut out[rows]);
        }
        self.drawn_bubble = self.bubble_text().map(str::to_owned);
        let bubble_rows = ui::rows(ui::status::HEIGHT, self.scale);
        if let Some(text) = &self.drawn_bubble
            && page_height >= bubble_rows
            && let Some((bubble_w, pixels)) = self.bubble.frame(text, width / 2, self.scale)
        {
            // At the page's bottom, above the panel.
            let bottom = bar_rows as u32 + page_height;
            let (bubble_w, top) = (bubble_w as usize, (bottom - bubble_rows) as usize);
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
            // Escape dismisses a prompt (M33.4), or else stops a load.
            Shortcut::Escape => {
                if let Some(prompt) = &self.prompt {
                    self.engine.answer_prompt(prompt.id, None);
                } else if *self.engine.load_state() == LoadState::Loading {
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
            // The page's text control focused, if any (M26.8), or the
            // panel's.
            Shortcut::Copy => {
                if let Some(text) = self.keyed().copy() {
                    self.copy(&text);
                }
            }
            Shortcut::Cut => {
                // The text leaves the control only once the clipboard
                // has it.
                if let Some(text) = self.keyed().copy_to_cut()
                    && self.copy(&text)
                {
                    self.keyed().cut();
                }
            }
            Shortcut::Paste => {
                if self.keyed().can_paste() {
                    match self.clipboard.get() {
                        Ok(text) => {
                            self.keyed().paste(&text);
                        }
                        Err(error) => self.notice = Some(clipboard_notice(&error)),
                    }
                }
            }
            Shortcut::Enter => {
                if let Some(action) = self.toolbar.enter() {
                    self.act(action);
                }
            }
            Shortcut::Edit(edit) => self.toolbar.command(edit),
            Shortcut::ToPage => {
                self.toolbar.blur();
                self.panel_keys = false;
                self.sync_window_focus();
                self.engine.focus_next(self.mods.shift);
            }
            Shortcut::DevTools => self.toggle_devtools(),
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

    /// A page's scroll: the page's height less a line, so a line of
    /// context stays.
    fn page_step(&self) -> f32 {
        let bottom = match self.panel {
            Some(_) => self.panel_top(),
            None => self.size.1 as f32,
        };
        let height = bottom - TOP;
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
        // The DevTools panel takes the press, and the keyboard (M36).
        if self.in_panel(y) {
            self.toolbar.blur();
            self.panel_keys = true;
            self.sync_window_focus();
            let top = self.panel_top();
            if let Some(panel) = &mut self.panel {
                panel.pointer_down(x, y - top, engine::Button::Primary, count);
            }
            self.held = Some(Held::Panel);
            return;
        }
        // The prompt's bar, over the page (M33.4): its buttons answer it.
        if let Some(prompt) = &self.prompt
            && y < TOP + ui::prompt::HEIGHT as f32
        {
            let width = self.size.0 as f32;
            if let Some(choice) = PromptBar::choice_at(width, x, y - TOP) {
                self.engine
                    .answer_prompt(prompt.id, Some(choice == Choice::Allow));
            }
            return;
        }
        self.toolbar.blur();
        self.panel_keys = false;
        self.sync_window_focus();
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
                Held::Track
            });
            return;
        }
        // The page takes the press (M26.5).
        self.engine
            .pointer_down(x, y, engine::Button::Primary, count);
        self.held = Some(Held::Page);
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
        self.cursor = engine::Cursor::Default;
        self.panel_cursor = engine::Cursor::Default;
        // The panel has the pointer over it, or while a press on it is
        // held (M36).
        let to_panel = match self.held {
            Some(Held::Panel) => true,
            Some(Held::Page) => false,
            _ => self.in_panel(y),
        };
        if self.inside && free && to_panel {
            let top = self.panel_top();
            if let Some(panel) = &mut self.panel {
                panel.pointer_move(x, y - top);
                self.panel_cursor = panel.cursor_at(x, y - top);
            }
            self.engine.pointer_leave();
            return;
        }
        if let Some(panel) = &mut self.panel {
            panel.pointer_leave();
        }
        if self.inside && free && y >= TOP {
            let y = y - TOP;
            let on_bar = self
                .engine
                .scrollbar()
                .is_some_and(|bar| bar.contains(x, y));
            if !on_bar {
                // The page hovers what is under the pointer and says
                // what cursor it is over (M26.4).
                self.engine.pointer_move(x, y);
                self.hover = self.engine.link_at(x, y);
                self.cursor = self.engine.cursor_at(x, y);
                return;
            }
        }
        self.engine.pointer_leave();
    }

    /// What the page wrote to its console since this was last called:
    /// printed on the standard error, and kept for the DevTools panel,
    /// which is told of it if it is open (M36).
    fn console(&mut self) {
        let entries = self.engine.take_console();
        for entry in &entries {
            if let Some(line) = console_line(entry) {
                eprintln!("{line}");
            }
        }
        self.router.watch(self.engine.url(), self.panel.as_mut());
        self.router.log(entries, self.panel.as_mut());
    }

    /// Brings the engine and what the window shows of it up to date: the
    /// page's focus, the toolbar, the title, and a notice the engine has.
    fn sync(&mut self) {
        // What a renderer answered since (M34.5): the keyboard's focus left
        // the page, or its selection was copied.
        if self.engine.take_focus_left() {
            self.toolbar.focus();
        }
        if let Some((text, cut)) = self.engine.take_copied()
            && self.copy(&text)
            && cut
        {
            self.engine.cut_copied();
        }
        self.sync_window_focus();
        if let Some(notice) = self.engine.take_notice() {
            self.notice = Some(notice);
        }
        self.prompt = self.engine.permission_prompt();
        let url = self.engine.url().map(|url| url.as_str().to_owned());
        self.toolbar.update(PageState {
            url: url.as_deref(),
            can_go_back: self.engine.can_go_back(),
            can_go_forward: self.engine.can_go_forward(),
            loading: *self.engine.load_state() == LoadState::Loading,
        });
        self.title = ui::window_title(&self.engine.title(), url.as_deref());
        // The DevTools' messages passed on (M36).
        if let Some(panel) = &mut self.panel {
            self.router.route(panel, &mut self.engine);
        }
    }

    /// The page has the system's focus while the window has it and the
    /// field does not (M26.6).
    fn sync_window_focus(&mut self) {
        let focus = self.focus();
        self.engine
            .set_window_focus(self.window_focused && focus == Focus::Page);
        if let Some(panel) = &mut self.panel {
            panel.set_window_focus(self.window_focused && focus == Focus::Panel);
        }
    }
}

/// The shape of the pointer over a view that gives `cursor`.
fn shape_of(cursor: engine::Cursor) -> Shape {
    match cursor {
        engine::Cursor::Default => Shape::Arrow,
        engine::Cursor::Pointer => Shape::Hand,
        engine::Cursor::Text => Shape::Text,
        other => Shape::Page(other),
    }
}

/// The earlier of two times, either of which may be none.
fn earliest(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
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

/// Prints what the shown page's scripts wrote to its console, and the
/// errors they reported, to the standard error (M24.7): the console a
/// developer would read until the browser has its own (M36).
pub(crate) fn print_console(engine: &mut impl View) {
    for entry in engine.take_console() {
        if let Some(line) = console_line(&entry) {
            eprintln!("{line}");
        }
    }
}

/// Prints what the DevTools panel's own scripts wrote to their console
/// (M36), which is the browser's, to the standard error.
fn print_panel_console(panel: &mut impl View) {
    for entry in panel.take_console() {
        if let Some(line) = console_line(&entry) {
            eprintln!("devtools {line}");
        }
    }
}

/// A console entry as printed: its level's name, then its text; nothing
/// for a group's end or a clearing, which have none (M36).
fn console_line(entry: &engine::ConsoleEntry) -> Option<String> {
    if matches!(
        entry.kind,
        engine::ConsoleKind::GroupEnd | engine::ConsoleKind::Clear
    ) {
        return None;
    }
    let level = match entry.level {
        engine::ConsoleLevel::Log => "log",
        engine::ConsoleLevel::Info => "info",
        engine::ConsoleLevel::Warn => "warn",
        engine::ConsoleLevel::Error => "error",
        engine::ConsoleLevel::Debug => "debug",
    };
    Some(format!("console: {level} {}", entry.text))
}

#[cfg(test)]
mod tests;
