//! The toolbar (docs/design/navigation.md, "The toolbar"): a strip
//! across the top of the window with back, forward, reload — stop while
//! a load is under way — and the address field. It is one display
//! list, rasterized into a buffer of its own, and drawn again only when
//! something in it changed.

use paint::{DisplayItem, DisplayList, Rgba8};
use render::{RenderBackend as _, SoftwareBackend};
use text::FontCollection;

use crate::field::{Command, TextField, TooLong};

/// The toolbar's height, in pixels.
pub const HEIGHT: u32 = 40;

/// A button's side.
const BUTTON: f32 = 32.0;
/// The space around and between the buttons.
const GAP: f32 = 4.0;
/// The field's left edge: after three buttons and their gaps.
pub(crate) const FIELD_X: f32 = GAP + 3.0 * (BUTTON + GAP) + 2.0;
/// The field's top and height.
pub(crate) const FIELD_Y: f32 = 6.0;
pub(crate) const FIELD_H: f32 = 28.0;
/// The space between the field's edge and its text.
pub(crate) const PADDING: f32 = 8.0;
/// The size of the field's text.
pub(crate) const FONT_SIZE: f32 = 15.0;
/// The narrowest the field is drawn.
const MIN_FIELD_W: f32 = 40.0;
/// How far the pointer must move, with the button held after the click
/// that focused the field, to drag rather than tremble.
const DRAG_THRESHOLD: f32 = 4.0;

const BAR: Rgba8 = Rgba8::rgb(0xED, 0xED, 0xED);
const BAR_EDGE: Rgba8 = Rgba8::rgb(0xC9, 0xC9, 0xC9);
const HOVER: Rgba8 = Rgba8::rgb(0xDA, 0xDA, 0xDA);
const PRESSED: Rgba8 = Rgba8::rgb(0xC8, 0xC8, 0xC8);
const ICON: Rgba8 = Rgba8::rgb(0x3C, 0x3C, 0x3C);
const ICON_DIMMED: Rgba8 = Rgba8::rgb(0xAA, 0xAA, 0xAA);
const FIELD: Rgba8 = Rgba8::rgb(0xFF, 0xFF, 0xFF);
const FIELD_EDGE: Rgba8 = Rgba8::rgb(0xB8, 0xB8, 0xB8);
const FOCUS: Rgba8 = Rgba8::rgb(0x38, 0x74, 0xD8);
const TEXT: Rgba8 = Rgba8::rgb(0x1F, 0x1F, 0x1F);
const SELECTION: Rgba8 = Rgba8::rgb(0xB4, 0xD2, 0xFA);

/// What the toolbar shows of the page: given by the shell from the
/// engine, which the toolbar knows nothing of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageState<'a> {
    /// The current entry's URL, serialized.
    pub url: Option<&'a str>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// Whether a load is under way: reload is stop meanwhile.
    pub loading: bool,
}

/// A part of the toolbar the pointer can be over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Back,
    Forward,
    /// Reload, or stop while a load is under way.
    Reload,
    Field,
}

/// What the toolbar asks the browser to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Back,
    Forward,
    Reload,
    Stop,
    /// Navigate to what the field holds: Enter was pressed.
    Navigate(String),
}

/// The pointer's shape over a point of the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Arrow,
    /// An I-beam, over the field.
    Text,
}

/// What Escape did in the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escape {
    /// It gave up an edit: the field shows the URL again, all of it
    /// selected.
    Reverted,
    /// There was no edit: the focus goes back to the page.
    Unfocused,
}

/// The toolbar.
pub struct Toolbar {
    fonts: FontCollection,
    backend: SoftwareBackend,
    field: TextField,
    /// Whether the keyboard goes to the field.
    focused: bool,
    /// Whether the user changed the field's text since it last showed
    /// the page's URL: a page's navigation does not overwrite an edit.
    edited: bool,
    /// The URL the field showed last from the page.
    url: String,
    /// The text a navigation from the field was started with, shown
    /// until that navigation commits or ends, with the URL shown before
    /// it.
    pending: Option<(String, String)>,
    can_go_back: bool,
    can_go_forward: bool,
    loading: bool,
    /// The part under the pointer.
    hover: Option<Part>,
    /// The part the left button was pressed on.
    pressed: Option<Part>,
    /// While the button is held after a press that focused the field:
    /// where it was pressed, for a drag to select from.
    focusing: Option<f32>,
    /// How far the field's text is scrolled left, to keep the caret in
    /// view.
    scroll_x: f32,
    /// The x of each character boundary of the field's text, and the
    /// text they were measured for.
    stops: Vec<f32>,
    measured: String,
    width: u32,
    /// Whether something changed since the last frame.
    changed: bool,
}

impl Toolbar {
    /// A toolbar drawing its text with `fonts`.
    pub fn new(fonts: FontCollection) -> Toolbar {
        Toolbar {
            backend: SoftwareBackend::with_fonts(fonts.clone()),
            fonts,
            field: TextField::new(),
            focused: false,
            edited: false,
            url: String::new(),
            pending: None,
            can_go_back: false,
            can_go_forward: false,
            loading: false,
            hover: None,
            pressed: None,
            focusing: None,
            scroll_x: 0.0,
            stops: vec![0.0],
            measured: String::new(),
            width: 0,
            changed: true,
        }
    }

    /// Takes what the toolbar shows of the page. The field shows the
    /// page's URL unless the user is editing it, or a navigation started
    /// from it is under way, which it shows until it commits or ends.
    pub fn update(&mut self, page: PageState) {
        let url = page.url.unwrap_or_default();
        if let Some((_, before)) = &self.pending
            && (!page.loading || url != before)
        {
            self.pending = None;
        }
        let shown = match &self.pending {
            Some((text, _)) => text.clone(),
            None => url.to_owned(),
        };
        if !self.edited && self.field.text() != shown {
            self.field.set_text(&shown);
            if self.focused {
                self.field.select_all();
            }
            self.scroll_x = 0.0;
            self.changed = true;
        }
        self.url = url.to_owned();
        let buttons = (page.can_go_back, page.can_go_forward, page.loading);
        if buttons != (self.can_go_back, self.can_go_forward, self.loading) {
            (self.can_go_back, self.can_go_forward, self.loading) = buttons;
            self.changed = true;
        }
    }

    /// The field's text.
    pub fn text(&self) -> &str {
        self.field.text()
    }

    /// The field's model, for the shell's copy.
    pub fn field(&self) -> &TextField {
        &self.field
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Whether the user has changed the field's text.
    pub fn is_edited(&self) -> bool {
        self.edited
    }

    /// Gives the field the keyboard, all of its text selected, as
    /// Ctrl+L and a first click do.
    pub fn focus(&mut self) {
        self.focused = true;
        self.field.select_all();
        self.changed = true;
    }

    /// Takes the keyboard from the field. An edit stays, as in the
    /// browsers, until Escape gives it up or a navigation is made.
    pub fn blur(&mut self) {
        if self.focused {
            self.focused = false;
            self.scroll_x = 0.0;
            self.changed = true;
        }
    }

    /// Carries out an editing command in the field.
    pub fn command(&mut self, command: Command) {
        if !self.focused {
            return;
        }
        let before = self.field.text().to_owned();
        self.field.apply(command);
        if self.field.text() != before {
            self.edited = true;
        }
        self.changed = true;
    }

    /// Inserts text at the caret: typed, pasted or committed by an input
    /// method.
    pub fn insert(&mut self, text: &str) -> Result<(), TooLong> {
        if !self.focused {
            return Ok(());
        }
        let before = self.field.text().to_owned();
        self.field.insert(text)?;
        if self.field.text() != before {
            self.edited = true;
        }
        self.changed = true;
        Ok(())
    }

    /// The selected text, for a copy.
    pub fn selected(&self) -> Option<String> {
        let selected = self.field.selected();
        (self.focused && !selected.is_empty()).then(|| selected.to_owned())
    }

    /// Takes the selected text out, for a cut.
    pub fn cut(&mut self) -> Option<String> {
        let cut = self.selected()?;
        self.field.cut();
        self.edited = true;
        self.changed = true;
        Some(cut)
    }

    /// Enter in the field: the text to navigate to. The field shows it
    /// until the navigation commits or ends, and gives the focus back to
    /// the page; the shell makes it a URL.
    pub fn enter(&mut self) -> Option<Action> {
        if !self.focused {
            return None;
        }
        let text = self.field.text().to_owned();
        self.pending = Some((text.clone(), self.url.clone()));
        self.edited = false;
        self.blur();
        Some(Action::Navigate(text))
    }

    /// Gives up the navigation the field shows: it did not start (the
    /// input was not an address), so the field keeps the text, edited,
    /// with the keyboard.
    pub fn keep_editing(&mut self) {
        self.pending = None;
        self.edited = true;
        self.focused = true;
        self.changed = true;
    }

    /// Escape in the field: an edit is given up, or with none the focus
    /// goes back to the page.
    pub fn escape(&mut self) -> Escape {
        if self.edited || self.field.text() != self.url {
            self.edited = false;
            self.pending = None;
            self.field.set_text(&self.url.clone());
            self.field.select_all();
            self.scroll_x = 0.0;
            self.changed = true;
            Escape::Reverted
        } else {
            self.blur();
            Escape::Unfocused
        }
    }

    /// The part of the toolbar at `(x, y)`, in its own pixels.
    pub fn part_at(&self, x: f32, y: f32) -> Option<Part> {
        if !(0.0..HEIGHT as f32).contains(&y) {
            return None;
        }
        [Part::Back, Part::Forward, Part::Reload, Part::Field]
            .into_iter()
            .find(|&part| {
                let (px, py, pw, ph) = self.rect(part);
                x >= px && x < px + pw && y >= py && y < py + ph
            })
    }

    /// The pointer's shape at `(x, y)`.
    pub fn cursor(&self, x: f32, y: f32) -> Cursor {
        match self.part_at(x, y) {
            Some(Part::Field) => Cursor::Text,
            _ => Cursor::Arrow,
        }
    }

    /// The pointer moved to `(x, y)`: the part under it lights, and a
    /// drag in the field selects.
    pub fn pointer_moved(&mut self, x: f32, y: f32) {
        let hover = self.part_at(x, y);
        if hover != self.hover {
            self.hover = hover;
            self.changed = true;
        }
        if self.pressed == Some(Part::Field) {
            if let Some(from) = self.focusing {
                // A drag from the click that focused the field selects
                // from where it began rather than all of it; a pointer
                // that only trembles leaves all of it selected.
                if (x - from).abs() < DRAG_THRESHOLD {
                    return;
                }
                self.focusing = None;
                let start = self.index_at(from);
                self.field.place(start, false);
            }
            let index = self.index_at(x);
            self.field.place(index, true);
            self.changed = true;
        }
    }

    /// The pointer left the toolbar.
    pub fn pointer_left(&mut self) {
        if self.hover.take().is_some() {
            self.changed = true;
        }
    }

    /// The left button pressed at `(x, y)`, the `clicks`th press in a
    /// row; `extend` when Shift is held. A press on a button arms it; one
    /// on the field gives it the focus: a first click selects all of it,
    /// a later one places the caret, a double click selects a word, and
    /// a triple click all of it.
    pub fn press(&mut self, x: f32, y: f32, clicks: u32, extend: bool) {
        self.pressed = self.part_at(x, y);
        match self.pressed {
            Some(Part::Field) if !self.focused => {
                self.focus();
                self.focusing = Some(x);
            }
            Some(Part::Field) => {
                let index = self.index_at(x);
                match clicks {
                    2 => self.field.select_word_at(index),
                    3.. => self.field.select_all(),
                    _ => self.field.place(index, extend),
                }
            }
            _ => {}
        }
        self.changed = true;
    }

    /// The left button released at `(x, y)`: a press and a release on
    /// the same button, which the history or the load lets act, is its
    /// action.
    pub fn release(&mut self, x: f32, y: f32) -> Option<Action> {
        let pressed = self.pressed.take();
        self.focusing = None;
        self.changed = true;
        if pressed.is_none() || pressed != self.part_at(x, y) {
            return None;
        }
        match pressed? {
            Part::Back if self.can_go_back => Some(Action::Back),
            Part::Forward if self.can_go_forward => Some(Action::Forward),
            Part::Reload if self.loading => Some(Action::Stop),
            Part::Reload => Some(Action::Reload),
            _ => None,
        }
    }

    /// Where the caret is, in the toolbar's pixels: `(x, y, w, h)`, for
    /// an input method's window to stand by.
    pub fn caret_area(&mut self) -> (f32, f32, f32, f32) {
        self.measure();
        let x = FIELD_X + PADDING + self.caret_x() - self.scroll_x;
        (x, FIELD_Y, 1.0, FIELD_H)
    }

    /// Whether the toolbar wants drawing again.
    pub fn needs_frame(&self) -> bool {
        self.changed
    }

    /// Draws the toolbar `width` pixels wide and [`HEIGHT`] high: RGBA8
    /// pixels, lent until the next frame.
    pub fn frame(&mut self, width: u32) -> &[u8] {
        self.width = width;
        self.changed = false;
        let list = self.display_list();
        self.backend.rasterize(&list, width, HEIGHT)
    }

    /// The display list of the toolbar as it stands.
    pub(crate) fn display_list(&mut self) -> DisplayList {
        self.measure();
        self.keep_caret_in_view();
        let width = self.width as f32;
        let mut items = vec![
            DisplayItem::Rect {
                x: 0.0,
                y: 0.0,
                w: width,
                h: HEIGHT as f32,
                color: BAR,
            },
            DisplayItem::Rect {
                x: 0.0,
                y: HEIGHT as f32 - 1.0,
                w: width,
                h: 1.0,
                color: BAR_EDGE,
            },
        ];
        for part in [Part::Back, Part::Forward, Part::Reload] {
            self.draw_button(part, &mut items);
        }
        self.draw_field(&mut items);
        DisplayList { items }
    }

    /// The rectangle of `part`: `(x, y, w, h)`.
    fn rect(&self, part: Part) -> (f32, f32, f32, f32) {
        let button = |n: f32| (GAP + n * (BUTTON + GAP), GAP, BUTTON, BUTTON);
        match part {
            Part::Back => button(0.0),
            Part::Forward => button(1.0),
            Part::Reload => button(2.0),
            Part::Field => (FIELD_X, FIELD_Y, self.field_width(), FIELD_H),
        }
    }

    fn field_width(&self) -> f32 {
        (self.width as f32 - FIELD_X - 8.0).max(MIN_FIELD_W)
    }

    /// Whether `part`, a button, can act.
    fn enabled(&self, part: Part) -> bool {
        match part {
            Part::Back => self.can_go_back,
            Part::Forward => self.can_go_forward,
            Part::Reload | Part::Field => true,
        }
    }

    fn draw_button(&self, part: Part, items: &mut Vec<DisplayItem>) {
        let (x, y, w, h) = self.rect(part);
        let enabled = self.enabled(part);
        if enabled && self.pressed == Some(part) && self.hover == Some(part) {
            items.push(DisplayItem::Rect {
                x,
                y,
                w,
                h,
                color: PRESSED,
            });
        } else if enabled && self.hover == Some(part) {
            items.push(DisplayItem::Rect {
                x,
                y,
                w,
                h,
                color: HOVER,
            });
        }
        let color = if enabled { ICON } else { ICON_DIMMED };
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        match part {
            Part::Back => items.push(arrow(cx, cy, -1.0, color)),
            Part::Forward => items.push(arrow(cx, cy, 1.0, color)),
            Part::Reload if self.loading => items.extend(cross(cx, cy, color)),
            Part::Reload => items.extend(circular_arrow(cx, cy, color)),
            Part::Field => {}
        }
    }

    fn draw_field(&self, items: &mut Vec<DisplayItem>) {
        let (x, y, w, h) = self.rect(Part::Field);
        let (edge, thickness) = if self.focused {
            (FOCUS, 2.0)
        } else {
            (FIELD_EDGE, 1.0)
        };
        items.push(DisplayItem::Rect {
            x,
            y,
            w,
            h,
            color: edge,
        });
        items.push(DisplayItem::Rect {
            x: x + thickness,
            y: y + thickness,
            w: w - 2.0 * thickness,
            h: h - 2.0 * thickness,
            color: FIELD,
        });
        let (inner_x, inner_w) = (x + PADDING, w - 2.0 * PADDING);
        items.push(DisplayItem::PushClip {
            x: inner_x,
            y,
            w: inner_w,
            h,
        });
        let origin = inner_x - self.scroll_x;
        // The selection, like the caret, is drawn only while the field
        // has the keyboard.
        let selection = self.field.selection();
        if self.focused && !selection.is_empty() {
            let (start, end) = (self.x_of(selection.start), self.x_of(selection.end));
            items.push(DisplayItem::Rect {
                x: origin + start,
                y: y + 4.0,
                w: end - start,
                h: h - 8.0,
                color: SELECTION,
            });
        }
        // Only the characters in view are drawn.
        let text = self.field.text();
        let boundaries = self.boundaries();
        let first = boundaries
            .iter()
            .rev()
            .find(|&&(_, stop)| stop <= self.scroll_x)
            .map_or(0, |&(i, _)| i);
        let last = boundaries
            .iter()
            .find(|&&(_, stop)| stop >= self.scroll_x + inner_w)
            .map_or(text.len(), |&(i, _)| i);
        let metrics = self.fonts.primary().metrics(FONT_SIZE);
        let baseline = y + (h + metrics.ascent - metrics.descent) / 2.0;
        if first < last {
            items.push(DisplayItem::Text {
                text: text[first..last].to_owned(),
                x: origin + self.x_of(first),
                y: baseline,
                size: FONT_SIZE,
                color: TEXT,
            });
        }
        if self.focused && selection.is_empty() {
            items.push(DisplayItem::Rect {
                x: (origin + self.caret_x()).round(),
                y: y + 5.0,
                w: 1.0,
                h: h - 10.0,
                color: TEXT,
            });
        }
        items.push(DisplayItem::PopClip);
    }

    /// Measures the field's text when it changed.
    fn measure(&mut self) {
        if self.measured != self.field.text() {
            self.measured = self.field.text().to_owned();
            self.stops = self.fonts.caret_stops(&self.measured, FONT_SIZE);
        }
    }

    /// Each character boundary of the field's text, as a byte offset,
    /// with its x.
    fn boundaries(&self) -> Vec<(usize, f32)> {
        let text = self.field.text();
        text.char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .zip(self.stops.iter().copied())
            .collect()
    }

    /// The x of the character boundary at byte offset `index`.
    fn x_of(&self, index: usize) -> f32 {
        let n = self.field.text()[..index].chars().count();
        self.stops.get(n).copied().unwrap_or(0.0)
    }

    fn caret_x(&self) -> f32 {
        self.x_of(self.field.caret())
    }

    /// The byte offset of the character boundary nearest the toolbar's
    /// `x`.
    fn index_at(&mut self, x: f32) -> usize {
        self.measure();
        let x = x - FIELD_X - PADDING + self.scroll_x;
        self.boundaries()
            .into_iter()
            .min_by(|(_, a), (_, b)| (a - x).abs().total_cmp(&(b - x).abs()))
            .map_or(0, |(i, _)| i)
    }

    /// Scrolls the field's text so the caret is in view while the field
    /// has the focus, and shows it from its start otherwise, so that a
    /// URL's scheme and host stay in sight.
    fn keep_caret_in_view(&mut self) {
        let inner_w = self.field_width() - 2.0 * PADDING;
        let whole = self.stops.last().copied().unwrap_or(0.0);
        if !self.focused || whole <= inner_w {
            self.scroll_x = 0.0;
            return;
        }
        let caret = self.caret_x();
        if caret - self.scroll_x > inner_w {
            self.scroll_x = caret - inner_w;
        } else if caret < self.scroll_x {
            self.scroll_x = caret;
        }
        self.scroll_x = self.scroll_x.clamp(0.0, (whole - inner_w).max(0.0));
    }
}

/// An arrow centred on `(cx, cy)`, pointing left when `direction` is -1
/// and right when it is 1.
fn arrow(cx: f32, cy: f32, direction: f32, color: Rgba8) -> DisplayItem {
    let d = direction;
    let points = [
        (-8.0, 0.0),
        (-1.0, -7.0),
        (-1.0, -1.25),
        (8.0, -1.25),
        (8.0, 1.25),
        (-1.0, 1.25),
        (-1.0, 7.0),
    ]
    .iter()
    .map(|&(x, y)| (cx - d * x, cy + y))
    .collect();
    DisplayItem::Polygon { points, color }
}

/// A circular arrow centred on `(cx, cy)`: reload.
fn circular_arrow(cx: f32, cy: f32, color: Rgba8) -> Vec<DisplayItem> {
    let (outer, inner) = (7.5_f32, 5.0_f32);
    let (from, to) = (-50.0_f32.to_radians(), 250.0_f32.to_radians());
    let steps = 24;
    let at = |radius: f32, angle: f32| (cx + radius * angle.cos(), cy + radius * angle.sin());
    let angles: Vec<f32> = (0..=steps)
        .map(|n| from + (to - from) * n as f32 / steps as f32)
        .collect();
    let mut ring: Vec<(f32, f32)> = angles.iter().map(|&a| at(outer, a)).collect();
    ring.extend(angles.iter().rev().map(|&a| at(inner, a)));
    // The head, at the arc's start, pointing along it.
    let head_at = at((outer + inner) / 2.0, from);
    let (tx, ty) = (from.sin(), -from.cos());
    let (nx, ny) = (from.cos(), from.sin());
    let head = vec![
        (head_at.0 - tx * 4.0, head_at.1 - ty * 4.0),
        (
            head_at.0 + nx * 4.5 + tx * 1.5,
            head_at.1 + ny * 4.5 + ty * 1.5,
        ),
        (
            head_at.0 - nx * 4.5 + tx * 1.5,
            head_at.1 - ny * 4.5 + ty * 1.5,
        ),
    ];
    vec![
        DisplayItem::Polygon {
            points: ring,
            color,
        },
        DisplayItem::Polygon {
            points: head,
            color,
        },
    ]
}

/// A cross centred on `(cx, cy)`: stop. Each bar is a diagonal 2.5
/// pixels thick.
fn cross(cx: f32, cy: f32, color: Rgba8) -> Vec<DisplayItem> {
    let half = 6.0_f32;
    // Half the thickness, along each axis of the diagonal's normal.
    let o = 1.25 / std::f32::consts::SQRT_2;
    let bar = |sign: f32| {
        let points = vec![
            (cx - half + o, cy - sign * (half + o)),
            (cx + half + o, cy + sign * (half - o)),
            (cx + half - o, cy + sign * (half + o)),
            (cx - half - o, cy - sign * (half - o)),
        ];
        DisplayItem::Polygon { points, color }
    };
    vec![bar(1.0), bar(-1.0)]
}
