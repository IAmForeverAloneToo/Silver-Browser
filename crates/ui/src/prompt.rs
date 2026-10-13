//! The permission prompt (M33.4, docs/design/hardening.md): a strip
//! under the toolbar, over the top of the page, saying what a site asks
//! for, with "Allow" and "Block" at its right. The shell puts its pixels
//! over the page's and gives it the pointer's presses there.

use paint::{DisplayItem, DisplayList, Rgba8};
use render::{RenderBackend as _, SoftwareBackend};
use text::FontCollection;

/// The bar's height.
pub const HEIGHT: u32 = 34;
const PADDING: f32 = 10.0;
const FONT_SIZE: f32 = 13.0;
const BUTTON_W: f32 = 64.0;
const BUTTON_H: f32 = 24.0;
const GAP: f32 = 8.0;
const BAR: Rgba8 = Rgba8::rgb(0xFF, 0xF8, 0xE1);
const EDGE: Rgba8 = Rgba8::rgb(0xC9, 0xC9, 0xC9);
const BUTTON: Rgba8 = Rgba8::rgb(0xFF, 0xFF, 0xFF);
const BUTTON_EDGE: Rgba8 = Rgba8::rgb(0xB8, 0xB8, 0xB8);
const TEXT: Rgba8 = Rgba8::rgb(0x1F, 0x1F, 0x1F);
/// What stands for the end of a text cut short.
const ELLIPSIS: &str = "...";

/// A button of the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Allow,
    Block,
}

impl Choice {
    fn label(self) -> &'static str {
        match self {
            Choice::Allow => "Allow",
            Choice::Block => "Block",
        }
    }

    /// Its left edge in a bar `width` wide: Block at the right, Allow
    /// before it.
    fn x(self, width: f32) -> f32 {
        match self {
            Choice::Block => width - PADDING - BUTTON_W,
            Choice::Allow => width - PADDING - 2.0 * BUTTON_W - GAP,
        }
    }
}

/// The permission prompt's bar.
pub struct PromptBar {
    fonts: FontCollection,
    backend: SoftwareBackend,
}

impl PromptBar {
    pub fn new(fonts: FontCollection) -> PromptBar {
        PromptBar {
            backend: SoftwareBackend::with_fonts(fonts.clone()),
            fonts,
        }
    }

    /// The button at `(x, y)`, in the bar's own CSS pixels, in a bar
    /// `width` CSS pixels wide.
    pub fn choice_at(width: f32, x: f32, y: f32) -> Option<Choice> {
        let top = (HEIGHT as f32 - BUTTON_H) / 2.0;
        if !(top..top + BUTTON_H).contains(&y) {
            return None;
        }
        [Choice::Allow, Choice::Block]
            .into_iter()
            .find(|choice| (choice.x(width)..choice.x(width) + BUTTON_W).contains(&x))
    }

    /// `text` as it fits in `room` pixels: all of it, or its start and an
    /// ellipsis.
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

    /// Draws the bar saying `text`, `width` device pixels wide at the
    /// display's `scale`: its RGBA8 pixels, [`HEIGHT`] CSS pixels high,
    /// lent until the next frame.
    pub fn frame(&mut self, text: &str, width: u32, scale: f32) -> &[u8] {
        let w = width as f32 / scale;
        let h = HEIGHT as f32;
        let metrics = self.fonts.primary().metrics(FONT_SIZE);
        let baseline =
            |top: f32, height: f32| top + (height + metrics.ascent - metrics.descent) / 2.0;
        let room = (Choice::Allow.x(w) - GAP - PADDING).max(0.0);
        let mut items = vec![
            DisplayItem::Rect {
                x: 0.0,
                y: 0.0,
                w,
                h,
                color: EDGE,
            },
            DisplayItem::Rect {
                x: 0.0,
                y: 0.0,
                w,
                h: h - 1.0,
                color: BAR,
            },
            DisplayItem::Text {
                text: self.fit(text, room),
                x: PADDING,
                y: baseline(0.0, h),
                size: FONT_SIZE,
                color: TEXT,
            },
        ];
        let top = (h - BUTTON_H) / 2.0;
        for choice in [Choice::Allow, Choice::Block] {
            let x = choice.x(w);
            let label = choice.label();
            let inset = (BUTTON_W - self.fonts.measure(label, FONT_SIZE)) / 2.0;
            items.extend([
                DisplayItem::Rect {
                    x,
                    y: top,
                    w: BUTTON_W,
                    h: BUTTON_H,
                    color: BUTTON_EDGE,
                },
                DisplayItem::Rect {
                    x: x + 1.0,
                    y: top + 1.0,
                    w: BUTTON_W - 2.0,
                    h: BUTTON_H - 2.0,
                    color: BUTTON,
                },
                DisplayItem::Text {
                    text: label.to_owned(),
                    x: x + inset,
                    y: baseline(top, BUTTON_H),
                    size: FONT_SIZE,
                    color: TEXT,
                },
            ]);
        }
        let mut list = DisplayList { items };
        paint::scaled(&mut list.items, scale, paint::GroupKey::Document(0));
        self.backend
            .rasterize(&list, width.max(1), crate::rows(HEIGHT, scale))
    }
}
