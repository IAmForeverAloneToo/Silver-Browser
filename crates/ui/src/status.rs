//! The status bubble (docs/design/navigation.md, "The status bubble"): a
//! small box at the bottom left of the page, over it, saying where the
//! link under the pointer goes, or the latest notice. The shell puts its
//! pixels over the page's.

use paint::{DisplayItem, DisplayList, Rgba8};
use render::{RenderBackend as _, SoftwareBackend};
use text::FontCollection;

/// The bubble's height.
pub const HEIGHT: u32 = 22;
const PADDING: f32 = 6.0;
const FONT_SIZE: f32 = 13.0;
const BACKGROUND: Rgba8 = Rgba8::rgb(0xF2, 0xF2, 0xF2);
const EDGE: Rgba8 = Rgba8::rgb(0xBD, 0xBD, 0xBD);
const TEXT: Rgba8 = Rgba8::rgb(0x30, 0x30, 0x30);
/// What stands for the end of a text cut short.
const ELLIPSIS: &str = "…";

/// The status bubble.
pub struct StatusBubble {
    fonts: FontCollection,
    backend: SoftwareBackend,
}

impl StatusBubble {
    pub fn new(fonts: FontCollection) -> StatusBubble {
        StatusBubble {
            backend: SoftwareBackend::with_fonts(fonts.clone()),
            fonts,
        }
    }

    /// The text the bubble shows for `text` in at most `max_width`
    /// pixels: all of it, or its start and an ellipsis. A URL's scheme
    /// and host come first, so they are what stays.
    pub fn fit(&self, text: &str, max_width: f32) -> String {
        let room = max_width - 2.0 * PADDING;
        if self.fonts.measure(text, FONT_SIZE) <= room {
            return text.to_owned();
        }
        let room = room - self.fonts.measure(ELLIPSIS, FONT_SIZE);
        let stops = self.fonts.caret_stops(text, FONT_SIZE);
        let fits = stops.iter().take_while(|&&x| x <= room).count().max(1) - 1;
        let end = text.char_indices().nth(fits).map_or(text.len(), |(i, _)| i);
        format!("{}{ELLIPSIS}", &text[..end])
    }

    /// Draws the bubble for `text`, at most `max_width` pixels wide:
    /// its width, and its RGBA8 pixels, [`HEIGHT`] high, lent until the
    /// next frame. Nothing for an empty text or no room.
    pub fn frame(&mut self, text: &str, max_width: u32) -> Option<(u32, &[u8])> {
        if text.is_empty() || (max_width as f32) < 2.0 * PADDING + 8.0 {
            return None;
        }
        let shown = self.fit(text, max_width as f32);
        let width =
            ((self.fonts.measure(&shown, FONT_SIZE) + 2.0 * PADDING).ceil() as u32).min(max_width);
        let (w, h) = (width as f32, HEIGHT as f32);
        let metrics = self.fonts.primary().metrics(FONT_SIZE);
        let list = DisplayList {
            items: vec![
                DisplayItem::Rect {
                    x: 0.0,
                    y: 0.0,
                    w,
                    h,
                    color: EDGE,
                },
                DisplayItem::Rect {
                    x: 0.0,
                    y: 1.0,
                    w: w - 1.0,
                    h: h - 1.0,
                    color: BACKGROUND,
                },
                DisplayItem::Text {
                    text: shown,
                    x: PADDING,
                    y: (h + metrics.ascent - metrics.descent) / 2.0,
                    size: FONT_SIZE,
                    color: TEXT,
                },
            ],
        };
        let pixels = self.backend.rasterize(&list, width, HEIGHT);
        Some((width, pixels))
    }
}
