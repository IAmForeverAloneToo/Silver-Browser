//! Rasterization backends: display list in, pixels out.
//!
//! Owns the pixel pipeline. The backend trait is the seam behind which the
//! future wgpu (GPU) backend will drop in; `SoftwareBackend` (tiny-skia +
//! CPU) is the default. Only this crate (and `shell`) touches pixel
//! buffers — per the layering contract in `docs/architecture.md`.

use std::collections::HashMap;

use paint::{DisplayItem, DisplayList, Rgba8};
use text::{FontCollection, FontError, GlyphMask, Script, ShapedGlyph, rasterize_glyph};

/// The most pixels one frame may have: an 8192-by-8192 frame is 256 MiB
/// of pixels. A larger request gets an empty buffer rather than an
/// allocation that takes the process down; no window is that large.
pub const MAX_FRAME_PIXELS: u64 = 8192 * 8192;

/// How many bytes of glyph coverage the mask cache holds before it is
/// emptied. Every distinct (font, glyph, size) is an entry, and a page
/// can ask for as many sizes as it likes; the cache is a speed-up, not
/// a store, so it starts again when it is full.
const MASK_CACHE_BUDGET: usize = 64 << 20;

/// How many shaped glyphs the run cache holds before it is emptied: a
/// million glyphs is some sixteen megabytes of positions, far more text
/// than a viewport shows, and the cache is a speed-up, not a store.
const SHAPE_CACHE_GLYPHS: usize = 1 << 20;

/// An axis-aligned clip in pixel coordinates: a pixel is inside when
/// its centre is, `x0 <= px + 0.5 < x1` and likewise for `y`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Clip {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Clip {
    fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Clip {
            x0: x,
            y0: y,
            x1: x + w,
            y1: y + h,
        }
    }

    fn intersect(self, other: Clip) -> Clip {
        Clip {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        }
    }

    /// Whether no pixel at all is inside (a NaN edge counts as empty).
    fn is_empty(self) -> bool {
        !(self.x1 > self.x0 && self.y1 > self.y0)
    }

    /// The pixel columns inside, `min..max`, within `0..width`.
    fn columns(self, width: i64) -> (i64, i64) {
        (
            ((self.x0 - 0.5).ceil().max(0.0) as i64).min(width),
            ((self.x1 - 0.5).ceil().max(0.0) as i64).min(width),
        )
    }

    /// The pixel rows inside, `min..max`, within `0..height`.
    fn rows(self, height: i64) -> (i64, i64) {
        (
            ((self.y0 - 0.5).ceil().max(0.0) as i64).min(height),
            ((self.y1 - 0.5).ceil().max(0.0) as i64).min(height),
        )
    }
}

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
    /// Coverage bytes held in `masks`, against [`MASK_CACHE_BUDGET`].
    cached_bytes: usize,
    /// Shaped runs by font size (bits) and text (M9.5): a scroll draws
    /// the same runs frame after frame, and shaping them anew was most
    /// of a frame — the text stack of today parses the face on every
    /// call (decision D7). Bounded by [`SHAPE_CACHE_GLYPHS`]; emptied
    /// with the mask cache.
    shaped: HashMap<u32, HashMap<String, Vec<(Script, ShapedGlyph)>>>,
    /// Glyphs held in `shaped`, against [`SHAPE_CACHE_GLYPHS`].
    cached_glyphs: usize,
}

impl SoftwareBackend {
    /// Creates the backend, loading a system font plus whatever
    /// per-script fallbacks are available (M8.1).
    pub fn new() -> Result<Self, FontError> {
        Ok(Self::with_fonts(FontCollection::new()?))
    }

    /// Creates the backend over fonts already loaded.
    pub fn with_fonts(fonts: FontCollection) -> Self {
        SoftwareBackend {
            fonts,
            masks: HashMap::new(),
            cached_bytes: 0,
            shaped: HashMap::new(),
            cached_glyphs: 0,
        }
    }

    /// Empties the glyph-mask and shaped-run caches: what a new
    /// document calls for, since the text it draws is not the last
    /// one's.
    pub fn clear_glyph_cache(&mut self) {
        self.masks.clear();
        self.cached_bytes = 0;
        self.shaped.clear();
        self.cached_glyphs = 0;
    }

    /// Glyph masks cached right now.
    #[doc(hidden)]
    pub fn glyph_cache_len(&self) -> usize {
        self.masks.len()
    }

    /// Shaped runs cached right now.
    #[doc(hidden)]
    pub fn shape_cache_len(&self) -> usize {
        self.shaped.values().map(HashMap::len).sum()
    }

    /// The shaped glyphs of `text` at `size`, from the run cache or
    /// shaped now and kept.
    fn shaped(&mut self, text: &str, size: f32) -> Vec<(Script, ShapedGlyph)> {
        let key = size.to_bits();
        if let Some(run) = self.shaped.get(&key).and_then(|runs| runs.get(text)) {
            return run.clone();
        }
        // Itemized + per-script fallback-dispatched (M8.1), not always
        // the one primary font — see `FontCollection::shape`'s doc.
        let run = self.fonts.shape(text, size);
        if self.cached_glyphs + run.len() > SHAPE_CACHE_GLYPHS {
            self.shaped.clear();
            self.cached_glyphs = 0;
        }
        self.cached_glyphs += run.len();
        self.shaped
            .entry(key)
            .or_default()
            .insert(text.to_string(), run.clone());
        run
    }

    fn mask_for(&mut self, script: Script, glyph_id: u16, size: f32) -> &GlyphMask {
        let key = (script, glyph_id, size.to_bits());
        if !self.masks.contains_key(&key) {
            let mask = rasterize_glyph(self.fonts.font_for(script).glyph_font(), glyph_id, size);
            let bytes = mask.coverage.len();
            if self.cached_bytes + bytes > MASK_CACHE_BUDGET {
                self.clear_glyph_cache();
            }
            self.cached_bytes += bytes;
            self.masks.insert(key, mask);
        }
        &self.masks[&key]
    }

    /// Draws `text` with its baseline's left end at `origin`.
    fn draw_text(
        &mut self,
        pixmap: &mut tiny_skia::Pixmap,
        text: &str,
        origin: (f32, f32),
        size: f32,
        color: &Rgba8,
        clip: Clip,
    ) {
        let (x, y) = origin;
        let glyphs = self.shaped(text, size);
        for (script, g) in glyphs {
            let mask = self.mask_for(script, g.glyph_id, size);
            let base_x = x + g.x + mask.left as f32;
            let base_y = y + g.y + mask.top as f32;
            blit_mask(pixmap, mask, color, base_x, base_y, clip);
        }
    }
}

impl RenderBackend for SoftwareBackend {
    fn rasterize(&mut self, list: &DisplayList, width: u32, height: u32) -> Vec<u8> {
        if width as u64 * height as u64 > MAX_FRAME_PIXELS {
            return Vec::new();
        }
        let Some(mut pixmap) = tiny_skia::Pixmap::new(width, height) else {
            return Vec::new();
        };

        // The clips in force (M9.3): each entry is already intersected
        // with the ones under it, so the innermost is the whole clip.
        let frame = Clip::new(0.0, 0.0, width as f32, height as f32);
        let mut clips: Vec<Clip> = Vec::new();
        for item in &list.items {
            let clip = clips.last().copied().unwrap_or(frame);
            match item {
                DisplayItem::PushClip { x, y, w, h } => {
                    clips.push(clip.intersect(Clip::new(*x, *y, *w, *h)));
                }
                DisplayItem::PopClip => {
                    clips.pop();
                }
                DisplayItem::Rect { x, y, w, h, color } => {
                    let r = Clip::new(*x, *y, *w, *h).intersect(clip);
                    if r.is_empty() {
                        continue;
                    }
                    let Some(rect) = tiny_skia::Rect::from_ltrb(r.x0, r.y0, r.x1, r.y1) else {
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
                    if clip.is_empty() {
                        continue;
                    }
                    self.draw_text(&mut pixmap, text, (*x, *y), *size, color, clip);
                }
                // No anti-aliasing for a quad: quads that share an
                // edge must tile it without a seam (a border's bands in
                // one colour). A polygon is anti-aliased: its mitres to
                // sides of another colour blend, as a browser's do
                // (M11), and its axis-aligned edges lie on whole pixels.
                DisplayItem::Quad { points, color } => {
                    fill_polygon(&mut pixmap, points, color, clip, false);
                }
                DisplayItem::Polygon { points, color } => {
                    fill_polygon(&mut pixmap, points, color, clip, true);
                }
            }
        }

        // The pixmap's own buffer, not a copy of it.
        pixmap.take()
    }
}

/// Fills the polygon `points`, clipped to `clip`, with or without
/// anti-aliasing.
fn fill_polygon(
    pixmap: &mut tiny_skia::Pixmap,
    points: &[(f32, f32)],
    color: &Rgba8,
    clip: Clip,
    anti_alias: bool,
) {
    let polygon = clip_polygon(points, clip);
    if polygon.len() < 3 {
        return;
    }
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(polygon[0].0, polygon[0].1);
    for &(px, py) in &polygon[1..] {
        pb.line_to(px, py);
    }
    pb.close();
    let Some(path) = pb.finish() else {
        return;
    };
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::from_rgba8(
        color.r, color.g, color.b, color.a,
    ));
    paint.anti_alias = anti_alias;
    pixmap.fill_path(
        &path,
        &paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::identity(),
        None,
    );
}

/// Clips a polygon to `clip` (Sutherland–Hodgman against the four
/// edges). A convex input stays convex, and a simple one stays a
/// simple fill under the winding rule; fewer than three points means
/// nothing is left.
fn clip_polygon(points: &[(f32, f32)], clip: Clip) -> Vec<(f32, f32)> {
    let mut polygon = points.to_vec();
    polygon = clip_against(
        &polygon,
        |p| p.0 >= clip.x0,
        |a, b| intersect_x(a, b, clip.x0),
    );
    polygon = clip_against(
        &polygon,
        |p| p.0 <= clip.x1,
        |a, b| intersect_x(a, b, clip.x1),
    );
    polygon = clip_against(
        &polygon,
        |p| p.1 >= clip.y0,
        |a, b| intersect_y(a, b, clip.y0),
    );
    polygon = clip_against(
        &polygon,
        |p| p.1 <= clip.y1,
        |a, b| intersect_y(a, b, clip.y1),
    );
    polygon
}

/// One Sutherland–Hodgman pass: the part of `input` on the `inside`
/// side of an edge, `intersect` giving where a segment crosses it.
fn clip_against(
    input: &[(f32, f32)],
    inside: impl Fn((f32, f32)) -> bool,
    intersect: impl Fn((f32, f32), (f32, f32)) -> (f32, f32),
) -> Vec<(f32, f32)> {
    let mut output = Vec::with_capacity(input.len() + 1);
    for i in 0..input.len() {
        let current = input[i];
        let previous = input[(i + input.len() - 1) % input.len()];
        let (cur_in, prev_in) = (inside(current), inside(previous));
        if cur_in {
            if !prev_in {
                output.push(intersect(previous, current));
            }
            output.push(current);
        } else if prev_in {
            output.push(intersect(previous, current));
        }
    }
    output
}

/// Where the segment `a`-`b` crosses the vertical line `x`.
fn intersect_x(a: (f32, f32), b: (f32, f32), x: f32) -> (f32, f32) {
    let t = if b.0 == a.0 {
        0.0
    } else {
        (x - a.0) / (b.0 - a.0)
    };
    (x, a.1 + (b.1 - a.1) * t)
}

/// Where the segment `a`-`b` crosses the horizontal line `y`.
fn intersect_y(a: (f32, f32), b: (f32, f32), y: f32) -> (f32, f32) {
    let t = if b.1 == a.1 {
        0.0
    } else {
        (y - a.1) / (b.1 - a.1)
    };
    (a.0 + (b.0 - a.0) * t, y)
}

/// Alpha-composites a glyph mask onto the pixmap at `(base_x, base_y)`
/// (the mask's top-left, in pixel coordinates), touching only pixels
/// inside `clip` (M9.3).
fn blit_mask(
    pixmap: &mut tiny_skia::Pixmap,
    mask: &GlyphMask,
    color: &Rgba8,
    base_x: f32,
    base_y: f32,
    clip: Clip,
) {
    if mask.width == 0 || mask.height == 0 {
        return;
    }
    let pw = pixmap.width() as i64;
    let ph = pixmap.height() as i64;
    let (min_x, max_x) = clip.columns(pw);
    let (min_y, max_y) = clip.rows(ph);
    let origin_x = base_x.floor() as i64;
    let origin_y = base_y.floor() as i64;
    if origin_x >= max_x
        || origin_y >= max_y
        || origin_x + mask.width as i64 <= min_x
        || origin_y + mask.height as i64 <= min_y
    {
        return;
    }

    let data = pixmap.data_mut();
    let ca = color.a as f32 / 255.0;
    for row in 0..mask.height as i64 {
        let py = origin_y + row;
        if py < min_y || py >= max_y {
            continue;
        }
        for col in 0..mask.width as i64 {
            let px = origin_x + col;
            if px < min_x || px >= max_x {
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

    /// A backend over the bundled test font: the same glyphs on every
    /// machine, and a machine without fonts still runs the tests.
    fn test_backend() -> SoftwareBackend {
        SoftwareBackend::with_fonts(
            FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
                .expect("the bundled font parses"),
        )
    }

    #[test]
    fn rasterizes_rects() {
        let mut backend = test_backend();
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

    /// The pixel at `(x, y)` of a `width`-wide RGBA8 buffer.
    fn px(buf: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * width + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    const WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
    const BLACK: [u8; 4] = [0, 0, 0, 0xff];

    fn white_frame(w: f32, h: f32) -> DisplayItem {
        DisplayItem::Rect {
            x: 0.0,
            y: 0.0,
            w,
            h,
            color: Rgba8::rgb(0xff, 0xff, 0xff),
        }
    }

    #[test]
    fn a_clip_keeps_a_rect_inside_it_and_an_unmatched_pop_is_harmless() {
        let mut backend = test_backend();
        let list = DisplayList {
            items: vec![
                white_frame(10.0, 10.0),
                DisplayItem::PopClip, // nothing to pop: ignored
                DisplayItem::PushClip {
                    x: 3.0,
                    y: 3.0,
                    w: 4.0,
                    h: 4.0,
                },
                DisplayItem::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 10.0,
                    color: Rgba8::rgb(0, 0, 0),
                },
                DisplayItem::PopClip,
            ],
        };
        let buf = backend.rasterize(&list, 10, 10);
        for y in 0..10 {
            for x in 0..10 {
                let inside = (3..7).contains(&x) && (3..7).contains(&y);
                assert_eq!(
                    px(&buf, 10, x, y),
                    if inside { BLACK } else { WHITE },
                    "({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn nested_clips_intersect_and_end_with_their_pop() {
        let mut backend = test_backend();
        let black = |x: f32, y: f32, w: f32, h: f32| DisplayItem::Rect {
            x,
            y,
            w,
            h,
            color: Rgba8::rgb(0, 0, 0),
        };
        let list = DisplayList {
            items: vec![
                white_frame(10.0, 10.0),
                DisplayItem::PushClip {
                    x: 0.0,
                    y: 0.0,
                    w: 5.0,
                    h: 5.0,
                },
                DisplayItem::PushClip {
                    x: 3.0,
                    y: 3.0,
                    w: 5.0,
                    h: 5.0,
                },
                black(0.0, 0.0, 10.0, 10.0), // only 3..5 x 3..5
                DisplayItem::PopClip,
                black(0.0, 0.0, 1.0, 10.0), // the outer clip: column 0, rows 0..5
                DisplayItem::PopClip,
                black(9.0, 9.0, 1.0, 1.0), // no clip: the corner
            ],
        };
        let buf = backend.rasterize(&list, 10, 10);
        for y in 0..10 {
            for x in 0..10 {
                let expect = ((3..5).contains(&x) && (3..5).contains(&y))
                    || (x == 0 && y < 5)
                    || (x == 9 && y == 9);
                assert_eq!(
                    px(&buf, 10, x, y),
                    if expect { BLACK } else { WHITE },
                    "({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn a_clip_keeps_text_inside_it() {
        // A 16px "M" with its baseline at 12 covers most of a 10 x 10
        // frame; clipped to the middle 4 x 4, only those pixels change.
        let mut backend = test_backend();
        let text = DisplayItem::Text {
            text: "M".to_string(),
            x: -2.0,
            y: 12.0,
            size: 16.0,
            color: Rgba8::rgb(0, 0, 0),
        };
        let unclipped = backend.rasterize(
            &DisplayList {
                items: vec![white_frame(10.0, 10.0), text.clone()],
            },
            10,
            10,
        );
        let clipped = backend.rasterize(
            &DisplayList {
                items: vec![
                    white_frame(10.0, 10.0),
                    DisplayItem::PushClip {
                        x: 3.0,
                        y: 3.0,
                        w: 4.0,
                        h: 4.0,
                    },
                    text,
                    DisplayItem::PopClip,
                ],
            },
            10,
            10,
        );
        let mut inked_inside = 0;
        let mut inked_outside_unclipped = 0;
        for y in 0..10 {
            for x in 0..10 {
                let inside = (3..7).contains(&x) && (3..7).contains(&y);
                if inside {
                    assert_eq!(px(&clipped, 10, x, y), px(&unclipped, 10, x, y));
                    inked_inside += (px(&clipped, 10, x, y) != WHITE) as u32;
                } else {
                    assert_eq!(px(&clipped, 10, x, y), WHITE, "({x}, {y}) outside the clip");
                    inked_outside_unclipped += (px(&unclipped, 10, x, y) != WHITE) as u32;
                }
            }
        }
        assert!(inked_inside > 0, "the glyph reaches into the clip");
        assert!(
            inked_outside_unclipped > 0,
            "without the clip the glyph inks outside it"
        );
    }

    #[test]
    fn shaped_runs_are_cached_across_frames_bounded_and_cleared() {
        let mut backend = test_backend();
        let item = |text: &str, size: f32| DisplayItem::Text {
            text: text.to_string(),
            x: 0.0,
            y: 10.0,
            size,
            color: Rgba8::rgb(0, 0, 0),
        };
        let list = DisplayList {
            items: vec![
                item("hello", 16.0),
                item("hello", 16.0),
                item("hello", 20.0),
            ],
        };
        let first = backend.rasterize(&list, 40, 20);
        assert_eq!(backend.shape_cache_len(), 2, "one run per (text, size)");
        let second = backend.rasterize(&list, 40, 20);
        assert_eq!(first, second, "a cached run draws the same pixels");
        assert_eq!(backend.shape_cache_len(), 2);
        // Past the glyph budget the cache starts again.
        backend.cached_glyphs = SHAPE_CACHE_GLYPHS - 1;
        backend.rasterize(
            &DisplayList {
                items: vec![item("another", 16.0)],
            },
            40,
            20,
        );
        assert_eq!(backend.shape_cache_len(), 1);
        assert!(backend.cached_glyphs <= SHAPE_CACHE_GLYPHS);
        backend.clear_glyph_cache();
        assert_eq!((backend.shape_cache_len(), backend.cached_glyphs), (0, 0));
    }

    #[test]
    fn polygons_are_anti_aliased_where_quads_are_not() {
        // The same triangle, half of a 10 by 10 square along its
        // diagonal, as a quad and as a polygon: the quad's pixels are
        // all black or white, each by its centre; the polygon's along
        // the diagonal are the two blended, half covered, and its
        // whole-pixel edges stay exact.
        let mut backend = test_backend();
        let black = Rgba8::rgb(0, 0, 0);
        let triangle = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        let list = DisplayList {
            items: vec![
                white_frame(10.0, 10.0),
                DisplayItem::Quad {
                    points: [triangle[0], triangle[1], triangle[2], triangle[2]],
                    color: black,
                },
            ],
        };
        let buf = backend.rasterize(&list, 10, 10);
        for y in 0..10 {
            for x in 0..10 {
                let p = px(&buf, 10, x, y);
                assert!(p == BLACK || p == WHITE, "({x}, {y}): {p:?}");
            }
        }
        let list = DisplayList {
            items: vec![
                white_frame(10.0, 10.0),
                DisplayItem::Polygon {
                    points: triangle.to_vec(),
                    color: black,
                },
            ],
        };
        let buf = backend.rasterize(&list, 10, 10);
        assert_eq!(px(&buf, 10, 8, 2), BLACK, "well inside");
        assert_eq!(px(&buf, 10, 2, 8), WHITE, "well outside");
        assert_eq!(
            px(&buf, 10, 9, 0),
            BLACK,
            "the edge on whole pixels is exact"
        );
        let on_diagonal = px(&buf, 10, 5, 5);
        assert!(
            on_diagonal != BLACK && on_diagonal != WHITE,
            "blended: {on_diagonal:?}"
        );
        // Skia's supersampled coverage, which tiny-skia keeps, puts
        // this pixel nearer three eighths than half: a blend, not a
        // guess at the exact share.
        assert!(
            (64..=192).contains(&on_diagonal[0]),
            "partly covered: {on_diagonal:?}"
        );
    }

    #[test]
    fn quads_tile_without_seams_and_respect_the_clip() {
        // Two triangles (as degenerate quads) that share the diagonal of
        // a 10 by 10 square: every pixel is black, no seam along it.
        let mut backend = test_backend();
        let black = Rgba8::rgb(0, 0, 0);
        let list = DisplayList {
            items: vec![
                white_frame(10.0, 10.0),
                DisplayItem::Quad {
                    points: [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (10.0, 10.0)],
                    color: black,
                },
                DisplayItem::Quad {
                    points: [(0.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 10.0)],
                    color: black,
                },
            ],
        };
        let buf = backend.rasterize(&list, 10, 10);
        for y in 0..10 {
            for x in 0..10 {
                assert_eq!(px(&buf, 10, x, y), BLACK, "({x}, {y})");
            }
        }
        // Clipped to the middle, a quad covering everything paints only
        // there, and the polygon clipper keeps the corners right.
        let list = DisplayList {
            items: vec![
                white_frame(10.0, 10.0),
                DisplayItem::PushClip {
                    x: 3.0,
                    y: 3.0,
                    w: 4.0,
                    h: 4.0,
                },
                DisplayItem::Quad {
                    points: [(-5.0, -5.0), (15.0, -5.0), (15.0, 15.0), (-5.0, 15.0)],
                    color: black,
                },
                DisplayItem::PopClip,
            ],
        };
        let buf = backend.rasterize(&list, 10, 10);
        for y in 0..10 {
            for x in 0..10 {
                let inside = (3..7).contains(&x) && (3..7).contains(&y);
                assert_eq!(
                    px(&buf, 10, x, y),
                    if inside { BLACK } else { WHITE },
                    "({x}, {y})"
                );
            }
        }
        let clipped = clip_polygon(
            &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
            Clip::new(2.0, 2.0, 5.0, 5.0),
        );
        let mut corners = clipped.clone();
        corners.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        assert_eq!(
            corners,
            vec![(2.0, 2.0), (2.0, 7.0), (7.0, 2.0), (7.0, 7.0)]
        );
        assert!(
            clip_polygon(
                &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
                Clip::new(5.0, 5.0, 1.0, 1.0)
            )
            .len()
                < 3
        );
    }

    #[test]
    fn zero_sized_viewport_returns_empty() {
        let mut backend = test_backend();
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
        let mut backend = test_backend();
        backend.mask_for(Script::Latin, 1, 16.0);
        backend.mask_for(Script::Han, 1, 16.0);
        assert_eq!(
            backend.masks.len(),
            2,
            "same (glyph_id, size) from two scripts must not share a cache slot"
        );
    }

    #[test]
    fn a_frame_past_the_pixel_cap_is_refused_not_allocated() {
        let mut backend = test_backend();
        assert!(
            backend
                .rasterize(&DisplayList::default(), 100_000, 100_000)
                .is_empty()
        );
        assert!(
            backend
                .rasterize(&DisplayList::default(), u32::MAX, 1)
                .is_empty()
        );
        assert_eq!(
            backend.rasterize(&DisplayList::default(), 8192, 8192).len(),
            8192 * 8192 * 4
        );
    }

    #[test]
    fn the_glyph_cache_is_bounded_and_can_be_cleared() {
        let mut backend = test_backend();
        let item = |size: f32| DisplayItem::Text {
            text: "M".to_string(),
            x: 0.0,
            y: 10.0,
            size,
            color: Rgba8::rgb(0, 0, 0),
        };
        // Each 2000px `M` is a mask of over a megabyte; sixty of them
        // pass the budget, and the cache starts again rather than grow.
        let sizes: Vec<DisplayItem> = (0..60).map(|i| item(2000.0 + i as f32)).collect();
        backend.rasterize(&DisplayList { items: sizes }, 8, 8);
        assert!(
            backend.glyph_cache_len() < 60,
            "the cache was emptied at the budget"
        );
        assert!(backend.cached_bytes <= MASK_CACHE_BUDGET);
        backend.rasterize(
            &DisplayList {
                items: vec![item(16.0)],
            },
            8,
            8,
        );
        assert!(backend.glyph_cache_len() >= 1);
        backend.clear_glyph_cache();
        assert_eq!(backend.glyph_cache_len(), 0);
        assert_eq!(backend.cached_bytes, 0);
    }
}
