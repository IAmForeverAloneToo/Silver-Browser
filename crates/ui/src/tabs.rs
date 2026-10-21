//! The tab strip (M37, docs/design/chrome.md, "Tabs"): a row above the
//! toolbar with a tab for each page the window holds, the one shown
//! joined to the toolbar, a close button on it and on the one under the
//! pointer, and a button for a new tab after the last. Tabs share the
//! row, `TAB_WIDTH` wide each and narrower as more come, to
//! `MIN_TAB_WIDTH`; past that the row scrolls, the tab shown kept in
//! sight. A tab dragged along the row moves. Like the toolbar, it is
//! one display list rasterized into a buffer of its own.

use paint::{DisplayItem, DisplayList, Rgba8};
use render::{RenderBackend as _, SoftwareBackend};
use text::FontCollection;

/// The strip's height, in CSS pixels.
pub const HEIGHT: u32 = 32;
/// A tab's width while there is room, and the narrowest one is drawn.
pub const TAB_WIDTH: f32 = 220.0;
pub const MIN_TAB_WIDTH: f32 = 48.0;
/// The new-tab button's side, and the space before the first tab.
const NEW_TAB: f32 = 28.0;
const GAP: f32 = 6.0;
/// The tabs' top: the strip's edge above them.
const TAB_TOP: f32 = 4.0;
/// The space between a tab's edge and its title, and the close
/// button's side.
const PADDING: f32 = 10.0;
const CLOSE: f32 = 16.0;
/// The size of a tab's title.
const FONT_SIZE: f32 = 13.0;
/// How far the pointer must move, the button held on a tab, to drag it.
const DRAG_THRESHOLD: f32 = 6.0;
/// What stands for the end of a title cut short.
const ELLIPSIS: &str = "\u{2026}";

/// The strip's colours: light, and dark (M37's settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colors {
    pub strip: Rgba8,
    pub tab: Rgba8,
    pub shown: Rgba8,
    pub hover: Rgba8,
    pub edge: Rgba8,
    pub text: Rgba8,
    pub dimmed: Rgba8,
    pub mark: Rgba8,
}

/// The light strip, joined to the light toolbar.
pub const LIGHT: Colors = Colors {
    strip: Rgba8::rgb(0xD6, 0xD6, 0xD6),
    tab: Rgba8::rgb(0xD6, 0xD6, 0xD6),
    shown: Rgba8::rgb(0xED, 0xED, 0xED),
    hover: Rgba8::rgb(0xE2, 0xE2, 0xE2),
    edge: Rgba8::rgb(0xB4, 0xB4, 0xB4),
    text: Rgba8::rgb(0x1F, 0x1F, 0x1F),
    dimmed: Rgba8::rgb(0x50, 0x50, 0x50),
    mark: Rgba8::rgb(0x38, 0x74, 0xD8),
};

/// The dark strip, joined to the dark toolbar.
pub const DARK: Colors = Colors {
    strip: Rgba8::rgb(0x20, 0x21, 0x24),
    tab: Rgba8::rgb(0x20, 0x21, 0x24),
    shown: Rgba8::rgb(0x35, 0x36, 0x3A),
    hover: Rgba8::rgb(0x2A, 0x2B, 0x2E),
    edge: Rgba8::rgb(0x3C, 0x40, 0x43),
    text: Rgba8::rgb(0xE8, 0xEA, 0xED),
    dimmed: Rgba8::rgb(0xBD, 0xC1, 0xC6),
    mark: Rgba8::rgb(0x8A, 0xB4, 0xF8),
};

/// A tab as the strip shows it: given by the shell from the page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TabItem {
    /// The page's title, or its URL while it has none.
    pub title: String,
    /// Whether a load is under way in it.
    pub loading: bool,
}

/// A part of the strip the pointer can be over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripPart {
    Tab(usize),
    /// A tab's close button.
    Close(usize),
    New,
}

/// What the strip asks the browser to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripAction {
    /// Show the tab of this index.
    Select(usize),
    Close(usize),
    New,
    /// The tab at `from` dragged to `to`.
    Move {
        from: usize,
        to: usize,
    },
}

/// A press on the strip, while the button is held.
#[derive(Debug, Clone, Copy)]
struct Press {
    part: StripPart,
    x: f32,
    /// Whether the middle button made it.
    middle: bool,
    /// While a tab is dragged: its index now, and how far into it the
    /// pointer took it.
    dragging: Option<(usize, f32)>,
}

/// The tab strip.
pub struct TabStrip {
    fonts: FontCollection,
    backend: SoftwareBackend,
    tabs: Vec<TabItem>,
    shown: usize,
    /// The strip's width in CSS pixels, as last drawn.
    width: u32,
    /// How far the tabs are scrolled left, in CSS pixels.
    scroll: f32,
    hover: Option<StripPart>,
    press: Option<Press>,
    /// Where the pointer is while a tab is dragged.
    pointer_x: f32,
    colors: Colors,
    /// Whether the window is a private one (M37): a band across the
    /// strip's top says so.
    private: bool,
    changed: bool,
}

/// The band across a private window's strip.
pub const PRIVATE_MARK: Rgba8 = Rgba8::rgb(0x7B, 0x3F, 0xC4);
/// How tall it is.
pub const PRIVATE_BAND: f32 = 3.0;

impl TabStrip {
    pub fn new(fonts: FontCollection) -> TabStrip {
        TabStrip {
            backend: SoftwareBackend::with_fonts(fonts.clone()),
            fonts,
            tabs: Vec::new(),
            shown: 0,
            width: 0,
            scroll: 0.0,
            hover: None,
            press: None,
            pointer_x: 0.0,
            colors: LIGHT,
            private: false,
            changed: true,
        }
    }

    /// The tabs as they now are, `shown` the one shown.
    pub fn update(&mut self, tabs: Vec<TabItem>, shown: usize) {
        let shown = shown.min(tabs.len().saturating_sub(1));
        if tabs != self.tabs || shown != self.shown {
            let moved = shown != self.shown || tabs.len() != self.tabs.len();
            self.tabs = tabs;
            self.shown = shown;
            self.changed = true;
            if moved {
                self.keep_shown_in_view();
            }
        }
    }

    /// Marks the strip as a private window's, or not.
    pub fn set_private(&mut self, private: bool) {
        if private != self.private {
            self.private = private;
            self.changed = true;
        }
    }

    /// Draws with `colors` from now on.
    pub fn set_colors(&mut self, colors: Colors) {
        if colors != self.colors {
            self.colors = colors;
            self.changed = true;
        }
    }

    /// The tabs' width: [`TAB_WIDTH`] while they fit, less as more come,
    /// to [`MIN_TAB_WIDTH`].
    pub fn tab_width(&self) -> f32 {
        let room = self.room();
        let count = self.tabs.len().max(1) as f32;
        (room / count).clamp(MIN_TAB_WIDTH, TAB_WIDTH)
    }

    /// The room the tabs have: the strip but the gap and the new-tab
    /// button.
    fn room(&self) -> f32 {
        (self.width as f32 - 2.0 * GAP - NEW_TAB - GAP).max(MIN_TAB_WIDTH)
    }

    /// The farthest the tabs scroll.
    fn max_scroll(&self) -> f32 {
        (self.tab_width() * self.tabs.len() as f32 - self.room()).max(0.0)
    }

    /// Scrolls so that the tab shown is all in view.
    fn keep_shown_in_view(&mut self) {
        let w = self.tab_width();
        let left = w * self.shown as f32;
        let room = self.room();
        if left < self.scroll {
            self.scroll = left;
        } else if left + w > self.scroll + room {
            self.scroll = left + w - room;
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    /// Where tab `index` is drawn: its left edge, the drag taken in.
    fn tab_x(&self, index: usize) -> f32 {
        GAP + self.tab_width() * index as f32 - self.scroll
    }

    /// The new-tab button's rectangle.
    fn new_tab_rect(&self) -> (f32, f32, f32, f32) {
        let after =
            GAP + (self.tab_width() * self.tabs.len() as f32 - self.scroll).min(self.room());
        let y = TAB_TOP + (HEIGHT as f32 - TAB_TOP - NEW_TAB) / 2.0;
        (after + GAP / 2.0, y, NEW_TAB, NEW_TAB)
    }

    /// Tab `index`'s close button's rectangle, when it has one: the tab
    /// shown and the one under the pointer, when wide enough.
    fn close_rect(&self, index: usize) -> Option<(f32, f32, f32, f32)> {
        let w = self.tab_width();
        let hovered =
            matches!(self.hover, Some(StripPart::Tab(i) | StripPart::Close(i)) if i == index);
        if (index != self.shown && !hovered) || w < 2.0 * PADDING + CLOSE + 12.0 {
            return None;
        }
        let x = self.tab_x(index) + w - PADDING - CLOSE + 4.0;
        let y = TAB_TOP + (HEIGHT as f32 - TAB_TOP - CLOSE) / 2.0;
        Some((x, y, CLOSE, CLOSE))
    }

    /// The part of the strip at `(x, y)`, in its CSS pixels.
    pub fn part_at(&self, x: f32, y: f32) -> Option<StripPart> {
        let inside = |(rx, ry, rw, rh): (f32, f32, f32, f32)| {
            x >= rx && x < rx + rw && y >= ry && y < ry + rh
        };
        if !(0.0..HEIGHT as f32).contains(&y) {
            return None;
        }
        if inside(self.new_tab_rect()) {
            return Some(StripPart::New);
        }
        if x < GAP || x >= GAP + self.room() || y < TAB_TOP {
            return None;
        }
        let index = ((x - GAP + self.scroll) / self.tab_width()).floor();
        if index < 0.0 || index as usize >= self.tabs.len() {
            return None;
        }
        let index = index as usize;
        match self.close_rect(index) {
            Some(rect) if inside(rect) => Some(StripPart::Close(index)),
            _ => Some(StripPart::Tab(index)),
        }
    }

    /// The pointer moved to `(x, y)`: the part under it is lit, and a
    /// tab held is dragged, moving among the others.
    pub fn pointer_moved(&mut self, x: f32, y: f32) -> Option<StripAction> {
        self.pointer_x = x;
        let (w, scroll, last) = (
            self.tab_width(),
            self.scroll,
            self.tabs.len().saturating_sub(1) as f32,
        );
        if let Some(press) = &mut self.press
            && let StripPart::Tab(index) = press.part
            && !press.middle
        {
            if press.dragging.is_none() && (x - press.x).abs() >= DRAG_THRESHOLD {
                let left = GAP + w * index as f32 - scroll;
                press.dragging = Some((index, press.x - left));
            }
            if let Some((at, grab)) = press.dragging {
                let left = x - grab - GAP + scroll;
                let to = ((left + w / 2.0) / w).floor().clamp(0.0, last) as usize;
                self.changed = true;
                if to != at {
                    press.dragging = Some((to, grab));
                    return Some(StripAction::Move { from: at, to });
                }
            }
            return None;
        }
        let hover = self.part_at(x, y);
        if hover != self.hover {
            self.hover = hover;
            self.changed = true;
        }
        None
    }

    /// The pointer left the strip.
    pub fn pointer_left(&mut self) {
        if self.hover.take().is_some() {
            self.changed = true;
        }
    }

    /// A button pressed at `(x, y)`, the middle one if `middle`: a tab
    /// pressed with the left one is shown at once.
    pub fn press(&mut self, x: f32, y: f32, middle: bool) -> Option<StripAction> {
        let part = self.part_at(x, y)?;
        self.press = Some(Press {
            part,
            x,
            middle,
            dragging: None,
        });
        self.changed = true;
        match part {
            StripPart::Tab(index) if !middle => Some(StripAction::Select(index)),
            _ => None,
        }
    }

    /// The button released at `(x, y)`: a close button or the new-tab
    /// button pressed and released on is a click, as is a middle click
    /// on a tab, which closes it.
    pub fn release(&mut self, x: f32, y: f32) -> Option<StripAction> {
        let press = self.press.take()?;
        self.changed = true;
        let at = self.part_at(x, y);
        match (press.part, at) {
            (StripPart::Tab(a), Some(StripPart::Tab(b) | StripPart::Close(b)))
                if press.middle && a == b =>
            {
                Some(StripAction::Close(a))
            }
            (StripPart::Close(a), Some(StripPart::Close(b))) if a == b && !press.middle => {
                Some(StripAction::Close(a))
            }
            (StripPart::New, Some(StripPart::New)) if !press.middle => Some(StripAction::New),
            _ => None,
        }
    }

    /// The wheel turned by `delta` pixels over the strip: the tabs
    /// scroll when they do not all fit.
    pub fn wheel(&mut self, delta: f32) {
        let scroll = (self.scroll + delta).clamp(0.0, self.max_scroll());
        if scroll != self.scroll {
            self.scroll = scroll;
            self.changed = true;
        }
    }

    /// Whether the strip wants drawing again.
    pub fn needs_frame(&self) -> bool {
        self.changed
    }

    /// Draws the strip `width` device pixels wide at the display's
    /// `scale`, [`HEIGHT`] CSS pixels high: RGBA8 pixels, lent until the
    /// next frame.
    pub fn frame(&mut self, width: u32, scale: f32) -> &[u8] {
        let css = (width as f32 / scale).round() as u32;
        if css != self.width {
            self.width = css;
            self.scroll = self.scroll.clamp(0.0, self.max_scroll());
            self.keep_shown_in_view();
        }
        self.changed = false;
        let mut list = self.display_list();
        paint::scaled(&mut list.items, scale, paint::GroupKey::Document(0));
        self.backend
            .rasterize(&list, width, crate::rows(HEIGHT, scale))
    }

    /// The display list of the strip as it stands.
    pub(crate) fn display_list(&self) -> DisplayList {
        let c = self.colors;
        let width = self.width as f32;
        let height = HEIGHT as f32;
        let mut items = vec![DisplayItem::Rect {
            x: 0.0,
            y: 0.0,
            w: width,
            h: height,
            color: c.strip,
        }];
        items.push(DisplayItem::PushClip {
            x: GAP,
            y: 0.0,
            w: self.room(),
            h: height,
        });
        let dragged = self.press.and_then(|press| press.dragging);
        for index in 0..self.tabs.len() {
            if dragged.is_some_and(|(at, _)| at == index) {
                continue;
            }
            self.draw_tab(index, self.tab_x(index), &mut items);
        }
        // The tab dragged, over the others, where the pointer holds it.
        if let Some((at, grab)) = dragged {
            self.draw_tab(at, self.pointer_x - grab, &mut items);
        }
        items.push(DisplayItem::PopClip);
        // The line under the strip, but under the tab shown.
        let shown_x = self.tab_x(self.shown);
        let w = self.tab_width();
        items.push(DisplayItem::Rect {
            x: 0.0,
            y: height - 1.0,
            w: shown_x.max(0.0),
            h: 1.0,
            color: c.edge,
        });
        items.push(DisplayItem::Rect {
            x: shown_x + w,
            y: height - 1.0,
            w: (width - shown_x - w).max(0.0),
            h: 1.0,
            color: c.edge,
        });
        let (x, y, w, h) = self.new_tab_rect();
        if self.hover == Some(StripPart::New) {
            items.push(DisplayItem::Rect {
                x,
                y,
                w,
                h,
                color: c.hover,
            });
        }
        items.extend(plus(x + w / 2.0, y + h / 2.0, c.dimmed));
        if self.private {
            items.push(DisplayItem::Rect {
                x: 0.0,
                y: 0.0,
                w: width,
                h: PRIVATE_BAND,
                color: PRIVATE_MARK,
            });
        }
        DisplayList { items }
    }

    fn draw_tab(&self, index: usize, x: f32, items: &mut Vec<DisplayItem>) {
        let c = self.colors;
        let w = self.tab_width();
        let (y, h) = (TAB_TOP, HEIGHT as f32 - TAB_TOP);
        let hovered =
            matches!(self.hover, Some(StripPart::Tab(i) | StripPart::Close(i)) if i == index);
        let fill = if index == self.shown {
            c.shown
        } else if hovered {
            c.hover
        } else {
            c.tab
        };
        items.push(DisplayItem::Rect {
            x,
            y,
            w,
            h,
            color: c.edge,
        });
        items.push(DisplayItem::Rect {
            x: x + 1.0,
            y: y + 1.0,
            w: w - 2.0,
            h: h - 1.0,
            color: fill,
        });
        let tab = &self.tabs[index];
        let mut text_x = x + PADDING;
        // A load under way: a dot before the title.
        if tab.loading {
            items.push(dot(text_x + 4.0, y + h / 2.0, 3.5, c.mark));
            text_x += 12.0;
        }
        let close = self
            .close_rect(index)
            .map(|(cx, ..)| cx - (x - self.tab_x(index)));
        let end = close.map_or(x + w - PADDING, |cx| cx - 4.0);
        let room = end - text_x;
        if room > 4.0 {
            let title = self.fit(&tab.title, room);
            let metrics = self.fonts.primary().metrics(FONT_SIZE);
            items.push(DisplayItem::PushClip {
                x: text_x,
                y,
                w: room,
                h,
            });
            items.push(DisplayItem::Text {
                text: title,
                x: text_x,
                y: y + (h + metrics.ascent - metrics.descent) / 2.0,
                size: FONT_SIZE,
                color: c.text,
            });
            items.push(DisplayItem::PopClip);
        }
        if let Some(cx) = close {
            let cy = y + (h - CLOSE) / 2.0;
            if self.hover == Some(StripPart::Close(index)) {
                items.push(DisplayItem::Rect {
                    x: cx,
                    y: cy,
                    w: CLOSE,
                    h: CLOSE,
                    color: c.hover,
                });
            }
            items.extend(small_cross(cx + CLOSE / 2.0, cy + CLOSE / 2.0, c.dimmed));
        }
    }

    /// `text` as it fits in `room` pixels: all of it, or its start and
    /// an ellipsis.
    fn fit(&self, text: &str, room: f32) -> String {
        if self.fonts.measure(text, FONT_SIZE) <= room {
            return text.to_owned();
        }
        let room = room - self.fonts.measure(ELLIPSIS, FONT_SIZE);
        let stops = self.fonts.caret_stops(text, FONT_SIZE);
        let fits = stops.iter().take_while(|&&x| x <= room).count().max(1) - 1;
        let end = text.char_indices().nth(fits).map_or(text.len(), |(i, _)| i);
        format!("{}{ELLIPSIS}", &text[..end])
    }
}

/// A disc of `radius` centred on `(cx, cy)`.
fn dot(cx: f32, cy: f32, radius: f32, color: Rgba8) -> DisplayItem {
    let points = (0..16)
        .map(|n| {
            let a = n as f32 * std::f32::consts::TAU / 16.0;
            (cx + radius * a.cos(), cy + radius * a.sin())
        })
        .collect();
    DisplayItem::Polygon { points, color }
}

/// A plus centred on `(cx, cy)`: a new tab.
fn plus(cx: f32, cy: f32, color: Rgba8) -> Vec<DisplayItem> {
    let (half, t) = (6.0, 0.75);
    vec![
        DisplayItem::Rect {
            x: cx - half,
            y: cy - t,
            w: 2.0 * half,
            h: 2.0 * t,
            color,
        },
        DisplayItem::Rect {
            x: cx - t,
            y: cy - half,
            w: 2.0 * t,
            h: 2.0 * half,
            color,
        },
    ]
}

/// A small cross centred on `(cx, cy)`: a tab's close button.
fn small_cross(cx: f32, cy: f32, color: Rgba8) -> Vec<DisplayItem> {
    let half = 4.0_f32;
    let o = 0.75 / std::f32::consts::SQRT_2;
    let bar = |sign: f32| DisplayItem::Polygon {
        points: vec![
            (cx - half + o, cy - sign * (half + o)),
            (cx + half + o, cy + sign * (half - o)),
            (cx + half - o, cy + sign * (half + o)),
            (cx - half - o, cy - sign * (half - o)),
        ],
        color,
    };
    vec![bar(1.0), bar(-1.0)]
}
