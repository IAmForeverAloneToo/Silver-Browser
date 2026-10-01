//! Rasterization backends: display list in, pixels out.
//!
//! Owns the pixel pipeline. The backend trait is the seam behind which the
//! future wgpu (GPU) backend will drop in; `SoftwareBackend` (tiny-skia +
//! CPU) is the default. Only this crate (and `shell`) touches pixel
//! buffers — per the layering contract in `docs/architecture.md`.

use std::collections::HashMap;

use paint::{DisplayItem, DisplayList, Rgba8};
use text::{FontCollection, FontError, GlyphMask, Script, rasterize_glyph};

/// A rasterizer: display list + viewport size in, framebuffer out.
pub trait RenderBackend {
    /// Rasterizes `list` into a `width` x `height` RGBA8 buffer
    /// (row-major, 4 bytes per pixel, alpha-premultiplied as produced by
    /// tiny-skia — everything Milestone-1 draws is opaque, so premultiplied
    /// and straight alpha coincide).
    fn rasterize(&mut self, list: &DisplayList, width: u32, height: u32) -> Vec<u8>;
}

/// The CPU rasterizer: tiny-skia for geometry, cached glyph masks for text.
pub struct SoftwareBackend {
    fonts: FontCollection,
    /// Glyph mask cache, keyed by (script, glyph id, font size bits).
    ///
    /// `Script` is part of the key, not just an input to computing it
    /// (M8.1): a `glyph_id` is only meaningful relative to the specific
    /// font that produced it, and two unrelated fonts routinely reuse
    /// the same small integer id for unrelated glyphs. Since
    /// `FontCollection::font_for` deterministically picks one font per
    /// script for this backend's whole lifetime, `Script` is a safe,
    /// stable stand-in for "which font" — cheaper than carrying a
    /// font-instance identity around, at the cost of (harmless)
    /// duplicate cache entries when two scripts happen to resolve to
    /// the same font (e.g. Hiragana and Katakana, see
    /// `fallback_families_for`).
    masks: HashMap<(Script, u16, u32), GlyphMask>,
}

impl SoftwareBackend {
    /// Creates the backend, loading a system font plus whatever
    /// per-script fallbacks are available (M8.1).
    pub fn new() -> Result<Self, FontError> {
        Ok(SoftwareBackend {
            fonts: FontCollection::new()?,
            masks: HashMap::new(),
        })
    }

    fn mask_for(&mut self, script: Script, glyph_id: u16, size: f32) -> &GlyphMask {
        let key = (script, glyph_id, size.to_bits());
        self.masks.entry(key).or_insert_with(|| {
            rasterize_glyph(self.fonts.font_for(script).glyph_font(), glyph_id, size)
        })
    }

    fn draw_text(
        &mut self,
        pixmap: &mut tiny_skia::Pixmap,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        color: &Rgba8,
    ) {
        // Itemized + per-script fallback-dispatched (M8.1), not always
        // the one primary font — see `FontCollection::shape`'s doc.
        let glyphs = self.fonts.shape(text, size);
        for (script, g) in glyphs {
            let mask = self.mask_for(script, g.glyph_id, size);
            let base_x = x + g.x + mask.left as f32;
            let base_y = y + g.y + mask.top as f32;
            blit_mask(pixmap, mask, color, base_x, base_y);
        }
    }
}

impl RenderBackend for SoftwareBackend {
    fn rasterize(&mut self, list: &DisplayList, width: u32, height: u32) -> Vec<u8> {
        let Some(mut pixmap) = tiny_skia::Pixmap::new(width, height) else {
            return Vec::new();
        };

        for item in &list.items {
            match item {
                DisplayItem::Rect { x, y, w, h, color } => {
                    let Some(rect) = tiny_skia::Rect::from_xywh(*x, *y, *w, *h) else {
                        continue;
                    };
                    let path = tiny_skia::PathBuilder::from_rect(rect);
                    let mut paint = tiny_skia::Paint::default();
                    paint.set_color(tiny_skia::Color::from_rgba8(
                        color.r, color.g, color.b, color.a,
                    ));
                    paint.anti_alias = true;
                    pixmap.fill_path(
                        &path,
                        &paint,
                        tiny_skia::FillRule::Winding,
                        tiny_skia::Transform::identity(),
                        None,
                    );
                }
                DisplayItem::Text {
                    text,
                    x,
                    y,
                    size,
                    color,
                } => {
                    self.draw_text(&mut pixmap, text, *x, *y, *size, color);
                }
            }
        }

        pixmap.data().to_vec()
    }
}

/// Alpha-composites a glyph mask onto the pixmap at `(base_x, base_y)`
/// (the mask's top-left, in pixel coordinates).
fn blit_mask(
    pixmap: &mut tiny_skia::Pixmap,
    mask: &GlyphMask,
    color: &Rgba8,
    base_x: f32,
    base_y: f32,
) {
    if mask.width == 0 || mask.height == 0 {
        return;
    }
    let pw = pixmap.width() as i64;
    let ph = pixmap.height() as i64;
    let origin_x = base_x.floor() as i64;
    let origin_y = base_y.floor() as i64;
    if origin_x >= pw
        || origin_y >= ph
        || origin_x + mask.width as i64 <= 0
        || origin_y + mask.height as i64 <= 0
    {
        return;
    }

    let data = pixmap.data_mut();
    let ca = color.a as f32 / 255.0;
    for row in 0..mask.height as i64 {
        let py = origin_y + row;
        if py < 0 || py >= ph {
            continue;
        }
        for col in 0..mask.width as i64 {
            let px = origin_x + col;
            if px < 0 || px >= pw {
                continue;
            }
            let alpha = mask.coverage[(row * mask.width as i64 + col) as usize] as f32 / 255.0;
            if alpha <= 0.0 {
                continue;
            }
            let a = alpha * ca;
            let idx = (py * pw + px) as usize * 4;
            // Milestone-1 colors are opaque, so straight==premultiplied
            // and we can leave the alpha channel untouched.
            data[idx] = (data[idx] as f32 * (1.0 - a) + color.r as f32 * a).round() as u8;
            data[idx + 1] = (data[idx + 1] as f32 * (1.0 - a) + color.g as f32 * a).round() as u8;
            data[idx + 2] = (data[idx + 2] as f32 * (1.0 - a) + color.b as f32 * a).round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paint::Rgba8;

    #[test]
    fn rasterizes_rects() {
        // Skips silently on machines with no system font.
        let Ok(mut backend) = SoftwareBackend::new() else {
            return;
        };
        let list = DisplayList {
            items: vec![
                DisplayItem::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 10.0,
                    color: Rgba8::rgb(0x11, 0x22, 0x33),
                },
                DisplayItem::Rect {
                    x: 4.0,
                    y: 4.0,
                    w: 2.0,
                    h: 2.0,
                    color: Rgba8::rgb(0xff, 0xff, 0xff),
                },
            ],
        };
        let buf = backend.rasterize(&list, 10, 10);
        assert_eq!(buf.len(), 10 * 10 * 4);
        assert_eq!(&buf[0..4], &[0x11, 0x22, 0x33, 0xff]);
        // Center pixel is white.
        let center = (5 * 10 + 5) as usize * 4;
        assert_eq!(&buf[center..center + 4], &[0xff, 0xff, 0xff, 0xff]);
    }

    #[test]
    fn zero_sized_viewport_returns_empty() {
        let Ok(mut backend) = SoftwareBackend::new() else {
            return;
        };
        let buf = backend.rasterize(&DisplayList::default(), 0, 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn mask_cache_does_not_collide_across_scripts() {
        // M8.1: a `glyph_id` is only meaningful relative to the font
        // that produced it, so the *same* (glyph_id, size) from two
        // different scripts — which `FontCollection::font_for` may
        // resolve to two different fonts — must not alias onto one
        // cache entry. The glyph id itself doesn't need to be real;
        // this is about the cache key, not what gets rasterized.
        let Ok(mut backend) = SoftwareBackend::new() else {
            return;
        };
        backend.mask_for(Script::Latin, 1, 16.0);
        backend.mask_for(Script::Han, 1, 16.0);
        assert_eq!(
            backend.masks.len(),
            2,
            "same (glyph_id, size) from two scripts must not share a cache slot"
        );
    }
}
