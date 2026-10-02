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

/// Past this many pixels across or down, tiny-skia fills without
/// anti-aliasing what it would otherwise anti-alias (its supersampling
/// works in 16-bit coordinates), so that a fill clipped to part of the
/// frame could differ from the same fill whole: a frame that large is
/// always rasterized whole.
const MAX_PARTIAL_SIDE: u32 = 8191;

/// At most this many damaged rectangles are redrawn one by one; more
/// are redrawn as the one rectangle around them all.
const MAX_DAMAGE_RECTS: usize = 32;

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

    /// The pixels inside, within a `width` by `height` frame: those a
    /// glyph drawn under the clip can change.
    fn pixels(self, width: u32, height: u32) -> Option<PixelRect> {
        let (x0, x1) = self.columns(width as i64);
        let (y0, y1) = self.rows(height as i64);
        PixelRect::new(x0, y0, x1, y1)
    }

    /// Every pixel it reaches into, within a `width` by `height` frame:
    /// those anything drawn under the clip can change, an anti-aliased
    /// edge along it included.
    fn reach(self, width: u32, height: u32) -> Option<PixelRect> {
        if self.is_empty() {
            return None;
        }
        PixelRect::around(self.x0, self.y0, self.x1, self.y1, width, height)
    }
}

/// A rectangle of whole pixels: columns `x0..x1` of rows `y0..y1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl PixelRect {
    /// The rectangle from `(x0, y0)` to `(x1, y1)`, those taken as they
    /// are; `None` when it holds no pixel.
    fn new(x0: i64, y0: i64, x1: i64, y1: i64) -> Option<Self> {
        let fit = |v: i64| v.clamp(0, u32::MAX as i64) as u32;
        let rect = PixelRect {
            x0: fit(x0),
            y0: fit(y0),
            x1: fit(x1),
            y1: fit(y1),
        };
        (rect.x1 > rect.x0 && rect.y1 > rect.y0).then_some(rect)
    }

    /// The pixels `[x0, x1)` by `[y0, y1)` reaches into, the edges
    /// rounded outwards and the whole kept within a `width` by
    /// `height` frame; `None` when it reaches into none.
    fn around(x0: f32, y0: f32, x1: f32, y1: f32, width: u32, height: u32) -> Option<Self> {
        let rect = PixelRect::new(
            x0.floor() as i64,
            y0.floor() as i64,
            x1.ceil() as i64,
            y1.ceil() as i64,
        )?;
        rect.intersect(PixelRect {
            x0: 0,
            y0: 0,
            x1: width,
            y1: height,
        })
    }

    /// How many pixels it holds.
    pub fn area(self) -> u64 {
        (self.x1 - self.x0) as u64 * (self.y1 - self.y0) as u64
    }

    fn intersect(self, other: PixelRect) -> Option<PixelRect> {
        PixelRect::new(
            self.x0.max(other.x0) as i64,
            self.y0.max(other.y0) as i64,
            self.x1.min(other.x1) as i64,
            self.y1.min(other.y1) as i64,
        )
    }

    /// Whether `other` is all inside this one.
    fn contains(self, other: PixelRect) -> bool {
        self.x0 <= other.x0 && self.y0 <= other.y0 && other.x1 <= self.x1 && other.y1 <= self.y1
    }

    /// Whether the two overlap or share an edge.
    fn touches(self, other: PixelRect) -> bool {
        self.x0 <= other.x1 && other.x0 <= self.x1 && self.y0 <= other.y1 && other.y0 <= self.y1
    }

    /// The rectangle around both.
    fn union(self, other: PixelRect) -> PixelRect {
        PixelRect {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    /// The same pixels as a clip.
    fn clip(self) -> Clip {
        Clip {
            x0: self.x0 as f32,
            y0: self.y0 as f32,
            x1: self.x1 as f32,
            y1: self.y1 as f32,
        }
    }
}

/// What the last raster drew anew (M13.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redrawn {
    /// The whole frame: there was no last frame of its size, or the
    /// damage covered half of it or more.
    Whole,
    /// These rectangles, each cleared and drawn again; none when the
    /// list was the last one's.
    Rects(Vec<PixelRect>),
}

/// The clips in force (M9.3), as a raster meets them in a list: each
/// entry is already intersected with the ones under it, so the
/// innermost is the whole clip, and the frame is the clip when none is.
struct Clips {
    frame: Clip,
    stack: Vec<Clip>,
}

impl Clips {
    fn new(width: u32, height: u32) -> Self {
        Clips {
            frame: Clip::new(0.0, 0.0, width as f32, height as f32),
            stack: Vec::new(),
        }
    }

    /// The clip in force.
    fn current(&self) -> Clip {
        self.stack.last().copied().unwrap_or(self.frame)
    }

    /// Takes `item` into account: a clip begins or ends, or for a
    /// drawing nothing changes. An unmatched end is ignored.
    fn apply(&mut self, item: &DisplayItem) {
        match item {
            DisplayItem::PushClip { x, y, w, h } => {
                let clip = self.current().intersect(Clip::new(*x, *y, *w, *h));
                self.stack.push(clip);
            }
            DisplayItem::PopClip => {
                self.stack.pop();
            }
            _ => {}
        }
    }
}

/// The frame a raster left, kept to draw the next one by (M13.3).
struct Retained {
    list: DisplayList,
    pixmap: tiny_skia::Pixmap,
}

/// A rasterizer: display list + viewport size in, framebuffer out.
pub trait RenderBackend {
    /// Rasterizes `list` into a `width` x `height` RGBA8 buffer
    /// (row-major, 4 bytes per pixel, alpha-premultiplied as produced by
    /// tiny-skia — everything Milestone-1 draws is opaque, so premultiplied
    /// and straight alpha coincide). The result is the list's whole
    /// raster, however the backend comes by it: it may draw again only
    /// what changed since the last frame (M13.3), but the bytes are the
    /// same.
    fn rasterize(&mut self, list: &DisplayList, width: u32, height: u32) -> Vec<u8>;
}

/// The CPU rasterizer: tiny-skia for geometry, cached glyph masks for text.
///
/// It keeps the last frame and its list (M13.3). The next list is
/// compared with the last: the items both share at the start and at
/// the end draw alike, and the damage is what the items between can
/// have touched, in either list — with, where the clips in force when
/// the shared end begins differ, those clips, under which the shared
/// end draws. A frame of the same size whose damage covers less than
/// half of it is drawn by clearing each damaged rectangle and drawing
/// every item that meets it again, only inside it. An item all inside
/// the rectangle is drawn as a whole raster draws it; one across its
/// edge is kept inside by what cannot change the pixels it draws
/// there: a rectangle is clipped to whole pixels (tiny-skia
/// supersamples each pixel from the edges within it alone), a glyph and
/// an anti-aliased polygon write only the pixels inside, and a quad is
/// drawn on a copy whose inside is copied back. The bytes are a whole
/// raster's.
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
    shaped: HashMap<u32, HashMap<String, Run>>,
    /// Glyphs held in `shaped`, against [`SHAPE_CACHE_GLYPHS`].
    cached_glyphs: usize,
    /// The last frame and its list (M13.3).
    retained: Option<Retained>,
    /// A frame-sized copy a quad across a damaged rectangle's edge is
    /// drawn on.
    scratch: Option<tiny_skia::Pixmap>,
    /// What the last raster drew anew.
    redrawn: Redrawn,
}

/// A shaped run, and how far its ink reaches.
struct Run {
    glyphs: Vec<(Script, ShapedGlyph)>,
    /// The ink's extent from the run's origin, from its glyphs' masks:
    /// `None` until a raster asks, then `Some(None)` for a run that
    /// inks nothing.
    ink: Option<Option<Ink>>,
}

/// The extent of a run's glyph masks from its origin, in pixels.
#[derive(Debug, Clone, Copy)]
struct Ink {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
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
            retained: None,
            scratch: None,
            redrawn: Redrawn::Whole,
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

    /// Forgets the last frame: the next raster draws the whole frame.
    pub fn forget_frame(&mut self) {
        self.retained = None;
    }

    /// What the last raster drew anew (M13.3).
    pub fn redrawn(&self) -> &Redrawn {
        &self.redrawn
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
            return run.glyphs.clone();
        }
        // Itemized + per-script fallback-dispatched (M8.1), not always
        // the one primary font — see `FontCollection::shape`'s doc.
        let glyphs = self.fonts.shape(text, size);
        if self.cached_glyphs + glyphs.len() > SHAPE_CACHE_GLYPHS {
            self.shaped.clear();
            self.cached_glyphs = 0;
        }
        self.cached_glyphs += glyphs.len();
        self.shaped.entry(key).or_default().insert(
            text.to_string(),
            Run {
                glyphs: glyphs.clone(),
                ink: None,
            },
        );
        glyphs
    }

    /// How far the ink of `text` at `size` reaches from its origin:
    /// the union of its glyphs' masks where [`Self::draw_text`] puts
    /// them, before the origin's rounding.
    fn ink(&mut self, text: &str, size: f32) -> Option<Ink> {
        let key = size.to_bits();
        if let Some(ink) = self
            .shaped
            .get(&key)
            .and_then(|runs| runs.get(text))
            .and_then(|run| run.ink)
        {
            return ink;
        }
        let mut ink: Option<Ink> = None;
        for (script, g) in self.shaped(text, size) {
            let mask = self.mask_for(script, g.glyph_id, size);
            if mask.width == 0 || mask.height == 0 {
                continue;
            }
            let (left, top) = (g.x + mask.left as f32, g.y + mask.top as f32);
            let glyph = Ink {
                left,
                top,
                right: left + mask.width as f32,
                bottom: top + mask.height as f32,
            };
            ink = Some(match ink {
                None => glyph,
                Some(i) => Ink {
                    left: i.left.min(glyph.left),
                    top: i.top.min(glyph.top),
                    right: i.right.max(glyph.right),
                    bottom: i.bottom.max(glyph.bottom),
                },
            });
        }
        // A mask past the budget may have emptied the caches since.
        if let Some(run) = self
            .shaped
            .get_mut(&key)
            .and_then(|runs| runs.get_mut(text))
        {
            run.ink = Some(ink);
        }
        ink
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

    /// Draws `list` whole onto `pixmap`, which is clear.
    fn draw_all(&mut self, pixmap: &mut tiny_skia::Pixmap, list: &DisplayList) {
        let mut clips = Clips::new(pixmap.width(), pixmap.height());
        for item in &list.items {
            let clip = clips.current();
            clips.apply(item);
            self.draw(pixmap, item, clip);
        }
    }

    /// Draws one item under `clip`, as a whole raster does.
    fn draw(&mut self, pixmap: &mut tiny_skia::Pixmap, item: &DisplayItem, clip: Clip) {
        match item {
            DisplayItem::PushClip { .. } | DisplayItem::PopClip => {}
            DisplayItem::Rect { x, y, w, h, color } => {
                let r = Clip::new(*x, *y, *w, *h).intersect(clip);
                if r.is_empty() {
                    return;
                }
                let Some(rect) = tiny_skia::Rect::from_ltrb(r.x0, r.y0, r.x1, r.y1) else {
                    return;
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
                    return;
                }
                self.draw_text(pixmap, text, (*x, *y), *size, color, clip);
            }
            // No anti-aliasing for a quad: quads that share an
            // edge must tile it without a seam (a border's bands in
            // one colour). A polygon is anti-aliased by the exact
            // area of each pixel it covers: its mitres to sides of
            // another colour blend by their share of the pixel, as
            // a browser's do (M11), and its axis-aligned edges lie
            // on whole pixels, where the area is all or nothing.
            DisplayItem::Quad { points, color } => {
                fill_polygon(pixmap, points, color, clip);
            }
            DisplayItem::Polygon { points, color } => {
                let frame = PixelRect {
                    x0: 0,
                    y0: 0,
                    x1: pixmap.width(),
                    y1: pixmap.height(),
                };
                fill_polygon_by_area(pixmap, points, color, clip, frame);
            }
        }
    }

    /// The pixels `item`, drawn under `clip`, can change in a `width`
    /// by `height` frame; `None` when it changes none. A rectangle and a
    /// polygon reach exactly as far as their raster can; a quad and a
    /// run of text are given a pixel more all round, against rounding,
    /// the run's glyphs placed from the extent of their masks; text
    /// whose origin is not a number may land anywhere in the clip.
    fn bounds(
        &mut self,
        item: &DisplayItem,
        clip: Clip,
        width: u32,
        height: u32,
    ) -> Option<PixelRect> {
        match item {
            DisplayItem::PushClip { .. } | DisplayItem::PopClip => None,
            DisplayItem::Rect { x, y, w, h, .. } => {
                let r = Clip::new(*x, *y, *w, *h).intersect(clip);
                if r.is_empty() {
                    return None;
                }
                PixelRect::around(r.x0, r.y0, r.x1, r.y1, width, height)
            }
            DisplayItem::Text {
                text, x, y, size, ..
            } => {
                let within = clip.pixels(width, height)?;
                if !x.is_finite() || !y.is_finite() {
                    return Some(within);
                }
                let ink = self.ink(text, *size)?;
                PixelRect::around(
                    x + ink.left - 1.0,
                    y + ink.top - 1.0,
                    x + ink.right + 1.0,
                    y + ink.bottom + 1.0,
                    width,
                    height,
                )?
                .intersect(within)
            }
            DisplayItem::Quad { points, .. } => {
                let polygon = clip_polygon(points, clip);
                if polygon.len() < 3 {
                    return None;
                }
                let (x0, y0, x1, y1) = extent(&polygon);
                if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
                    // tiny-skia fills no such path; were it to, its
                    // pixels would be in the clip.
                    return clip.reach(width, height);
                }
                PixelRect::around(x0 - 1.0, y0 - 1.0, x1 + 1.0, y1 + 1.0, width, height)
            }
            DisplayItem::Polygon { points, .. } => {
                let polygon = clip_polygon(points, clip);
                if polygon.len() < 3 || polygon.iter().any(|p| !p.0.is_finite() || !p.1.is_finite())
                {
                    return None;
                }
                let (x0, y0, x1, y1) = extent(&polygon);
                PixelRect::around(x0, y0, x1, y1, width, height)
            }
        }
    }

    /// What drawing `new` over the frame `old` drew can change: the
    /// pixels the items between the lists' shared start and shared end
    /// can change, in either list, and wherever the shared end draws
    /// under clips that differ between the two, all of both clips.
    /// `None` when that is half the frame or more, which is better
    /// drawn whole.
    fn damage(
        &mut self,
        old: &DisplayList,
        new: &DisplayList,
        width: u32,
        height: u32,
    ) -> Option<Vec<PixelRect>> {
        let (a, b) = (&old.items, &new.items);
        let start = a.iter().zip(b).take_while(|(x, y)| x == y).count();
        let most = a.len().min(b.len()) - start;
        let end = a
            .iter()
            .rev()
            .zip(b.iter().rev())
            .take(most)
            .take_while(|(x, y)| x == y)
            .count();
        let mut rects = Vec::new();
        let mut clips = [Clips::new(width, height), Clips::new(width, height)];
        for (clips, items) in clips.iter_mut().zip([a, b]) {
            for item in &items[..start] {
                clips.apply(item);
            }
            for item in &items[start..items.len() - end] {
                rects.extend(self.bounds(item, clips.current(), width, height));
                clips.apply(item);
            }
        }
        // The shared end draws alike where the clips in force are the
        // same in both lists, and once the stacks are, they stay so. A
        // clip begun or ended in between, or a stack deeper in one list
        // than in the other, puts it under different clips, in which
        // it may draw differently anywhere.
        let [mut old_clips, mut new_clips] = clips;
        let mut added = None;
        for item in &b[b.len() - end..] {
            if old_clips.stack == new_clips.stack {
                break;
            }
            let pair = (old_clips.current(), new_clips.current());
            let draws = !matches!(item, DisplayItem::PushClip { .. } | DisplayItem::PopClip);
            if draws && pair.0 != pair.1 && added != Some(pair) {
                rects.extend(pair.0.reach(width, height));
                rects.extend(pair.1.reach(width, height));
                added = Some(pair);
            }
            old_clips.apply(item);
            new_clips.apply(item);
        }
        let rects = merge(rects);
        let area: u64 = rects.iter().map(|r| r.area()).sum();
        (area * 2 < width as u64 * height as u64).then_some(rects)
    }

    /// Draws `list` again inside `rect` of `pixmap`, which holds the
    /// last frame: the rectangle is cleared, then every item that can
    /// change a pixel in it is drawn again there (see the type's
    /// documentation), every clip and its end met whatever its place,
    /// so that the pixels inside come out as a whole raster's.
    fn redraw(&mut self, pixmap: &mut tiny_skia::Pixmap, list: &DisplayList, rect: PixelRect) {
        let (width, height) = (pixmap.width(), pixmap.height());
        let row = width as usize * 4;
        let (x0, x1) = (rect.x0 as usize * 4, rect.x1 as usize * 4);
        for y in rect.y0 as usize..rect.y1 as usize {
            pixmap.data_mut()[y * row + x0..y * row + x1].fill(0);
        }
        let inside = rect.clip();
        let mut clips = Clips::new(width, height);
        for item in &list.items {
            let clip = clips.current();
            clips.apply(item);
            let Some(bounds) = self.bounds(item, clip, width, height) else {
                continue;
            };
            if bounds.intersect(rect).is_none() {
                continue;
            }
            if rect.contains(bounds) {
                self.draw(pixmap, item, clip);
                continue;
            }
            match item {
                DisplayItem::Rect { .. } | DisplayItem::Text { .. } => {
                    self.draw(pixmap, item, clip.intersect(inside));
                }
                DisplayItem::Polygon { points, color } => {
                    fill_polygon_by_area(pixmap, points, color, clip, rect);
                }
                DisplayItem::Quad { .. } => {
                    // Drawn on a copy of what is under it, of which
                    // only the part inside the rectangle comes back.
                    let mut scratch = match self.scratch.take() {
                        Some(s) if s.width() == width && s.height() == height => s,
                        _ => match tiny_skia::Pixmap::new(width, height) {
                            Some(s) => s,
                            None => return,
                        },
                    };
                    copy_pixels(pixmap, &mut scratch, bounds);
                    self.draw(&mut scratch, item, clip);
                    if let Some(back) = bounds.intersect(rect) {
                        copy_pixels(&scratch, pixmap, back);
                    }
                    self.scratch = Some(scratch);
                }
                DisplayItem::PushClip { .. } | DisplayItem::PopClip => {}
            }
        }
    }
}

/// The least and greatest `x` and `y` of `points`.
fn extent(points: &[(f32, f32)]) -> (f32, f32, f32, f32) {
    points.iter().fold(
        (
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ),
        |(x0, y0, x1, y1), &(x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
    )
}

/// `rects` with every two that overlap or share an edge replaced by the
/// rectangle around them, until none do; more than
/// [`MAX_DAMAGE_RECTS`] become the one around them all.
fn merge(mut rects: Vec<PixelRect>) -> Vec<PixelRect> {
    if rects.len() > MAX_DAMAGE_RECTS {
        let all = rects.iter().copied().reduce(PixelRect::union);
        return all.into_iter().collect();
    }
    'merging: loop {
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                if rects[i].touches(rects[j]) {
                    rects[i] = rects[i].union(rects[j]);
                    rects.swap_remove(j);
                    continue 'merging;
                }
            }
        }
        return rects;
    }
}

/// Copies the pixels of `rect` from `from` to `to`, two pixmaps of one
/// size.
fn copy_pixels(from: &tiny_skia::Pixmap, to: &mut tiny_skia::Pixmap, rect: PixelRect) {
    let row = from.width() as usize * 4;
    let (x0, x1) = (rect.x0 as usize * 4, rect.x1 as usize * 4);
    for y in rect.y0 as usize..rect.y1 as usize {
        let span = y * row + x0..y * row + x1;
        to.data_mut()[span.clone()].copy_from_slice(&from.data()[span]);
    }
}

impl RenderBackend for SoftwareBackend {
    fn rasterize(&mut self, list: &DisplayList, width: u32, height: u32) -> Vec<u8> {
        self.redrawn = Redrawn::Whole;
        let last = self.retained.take();
        if width as u64 * height as u64 > MAX_FRAME_PIXELS {
            return Vec::new();
        }
        // The last frame, when it is this size, drawn again where the
        // lists differ; otherwise a frame drawn whole, on a new pixmap,
        // which comes cleared.
        let partial = width <= MAX_PARTIAL_SIDE && height <= MAX_PARTIAL_SIDE;
        let redrawn = last
            .filter(|r| partial && r.pixmap.width() == width && r.pixmap.height() == height)
            .and_then(
                |Retained {
                     list: old,
                     mut pixmap,
                 }| {
                    let rects = self.damage(&old, list, width, height)?;
                    for &rect in &rects {
                        self.redraw(&mut pixmap, list, rect);
                    }
                    self.redrawn = Redrawn::Rects(rects);
                    Some(pixmap)
                },
            );
        let pixmap = match redrawn {
            Some(pixmap) => pixmap,
            None => {
                let Some(mut pixmap) = tiny_skia::Pixmap::new(width, height) else {
                    return Vec::new();
                };
                self.draw_all(&mut pixmap, list);
                pixmap
            }
        };
        let pixels = pixmap.data().to_vec();
        self.retained = Some(Retained {
            list: list.clone(),
            pixmap,
        });
        pixels
    }
}

/// Fills the polygon `points`, clipped to `clip`, without
/// anti-aliasing: a pixel is painted when its centre is inside.
fn fill_polygon(pixmap: &mut tiny_skia::Pixmap, points: &[(f32, f32)], color: &Rgba8, clip: Clip) {
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
    paint.anti_alias = false;
    pixmap.fill_path(
        &path,
        &paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::identity(),
        None,
    );
}

/// Fills the simple polygon `points`, clipped to `clip`, anti-aliased
/// by exact area: each pixel takes the colour in the share of it the
/// polygon covers, blended over what is there. The coverage comes from
/// the signed-area accumulation of the polygon's edges (the way font
/// rasterizers compute it): every edge adds, to the pixels of each row
/// it crosses, the area it sweeps and the cover it carries to the
/// pixels right of it, and a running sum along the row gives each
/// pixel's covered area, exact for a polygon that does not cross
/// itself whichever way it winds. Only the pixels of `write` are
/// written, each as it would be were all of them (M13.3).
fn fill_polygon_by_area(
    pixmap: &mut tiny_skia::Pixmap,
    points: &[(f32, f32)],
    color: &Rgba8,
    clip: Clip,
    write: PixelRect,
) {
    let polygon = clip_polygon(points, clip);
    if polygon.len() < 3 || polygon.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return;
    }
    let (pw, ph) = (pixmap.width() as i64, pixmap.height() as i64);
    // The pixels the polygon can touch: its bounds, within the frame.
    let min_x = polygon
        .iter()
        .map(|p| p.0)
        .fold(f32::INFINITY, f32::min)
        .floor() as i64;
    let min_y = polygon
        .iter()
        .map(|p| p.1)
        .fold(f32::INFINITY, f32::min)
        .floor() as i64;
    let max_x = polygon
        .iter()
        .map(|p| p.0)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil() as i64;
    let max_y = polygon
        .iter()
        .map(|p| p.1)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil() as i64;
    let (x0, y0) = (min_x.max(0), min_y.max(0));
    let (x1, y1) = (max_x.min(pw), max_y.min(ph));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    // One extra column: an edge's cover spills into the pixel after it.
    let stride = bw + 2;
    let mut acc = vec![0.0f32; stride * bh];
    for i in 0..polygon.len() {
        let (ax, ay) = polygon[i];
        let (bx, by) = polygon[(i + 1) % polygon.len()];
        accumulate_edge(
            &mut acc,
            stride,
            bh,
            (ax - x0 as f32, ay - y0 as f32),
            (bx - x0 as f32, by - y0 as f32),
        );
    }
    let data = pixmap.data_mut();
    for row in 0..bh {
        let mut sum = 0.0f32;
        let py = y0 + row as i64;
        let row_written = (write.y0 as i64..write.y1 as i64).contains(&py);
        for col in 0..bw {
            sum += acc[row * stride + col];
            let coverage = sum.abs().min(1.0);
            let px = x0 + col as i64;
            if coverage <= 0.0 || !row_written || !(write.x0 as i64..write.x1 as i64).contains(&px)
            {
                continue;
            }
            let idx = (py * pw + px) as usize * 4;
            let a = coverage * color.a as f32 / 255.0;
            data[idx] = (data[idx] as f32 * (1.0 - a) + color.r as f32 * a).round() as u8;
            data[idx + 1] = (data[idx + 1] as f32 * (1.0 - a) + color.g as f32 * a).round() as u8;
            data[idx + 2] = (data[idx + 2] as f32 * (1.0 - a) + color.b as f32 * a).round() as u8;
        }
    }
}

/// Adds one edge of a polygon to the accumulation buffer `acc` of
/// `stride` columns and `height` rows, in that buffer's coordinates:
/// for each row the edge crosses, the signed area it sweeps across the
/// pixels it passes through, so that the running sum along the row is
/// the covered area of each pixel. Points left of the buffer are
/// clamped to its left edge, which keeps their cover and drops the area
/// outside, and rows above or below it are skipped.
fn accumulate_edge(acc: &mut [f32], stride: usize, height: usize, p0: (f32, f32), p1: (f32, f32)) {
    if (p0.1 - p1.1).abs() <= f32::EPSILON {
        return;
    }
    let (dir, (x0, y0), (x1, y1)) = if p0.1 < p1.1 {
        (1.0, p0, p1)
    } else {
        (-1.0, p1, p0)
    };
    let dxdy = (x1 - x0) / (y1 - y0);
    let mut x = x0;
    if y0 < 0.0 {
        x -= y0 * dxdy;
    }
    let first_row = y0.max(0.0).floor() as usize;
    let last_row = (y1.ceil().max(0.0) as usize).min(height);
    let right = (stride - 1) as f32;
    for row in first_row..last_row {
        let line = row * stride;
        let dy = ((row + 1) as f32).min(y1) - (row as f32).max(y0);
        let xnext = x + dxdy * dy;
        let d = dy * dir;
        let (xa, xb) = if x < xnext { (x, xnext) } else { (xnext, x) };
        let (xa, xb) = (xa.clamp(0.0, right), xb.clamp(0.0, right));
        let xa_floor = xa.floor();
        let xa_i = xa_floor as usize;
        let xb_ceil = xb.ceil();
        let xb_i = (xb_ceil as usize).min(stride - 1);
        if xb_i <= xa_i + 1 {
            // Within one pixel column: its area left of the edge's
            // middle goes to it, the rest carries to the next.
            let xmf = 0.5 * (xa + xb) - xa_floor;
            acc[line + xa_i] += d - d * xmf;
            acc[line + xa_i + 1] += d * xmf;
        } else {
            // Across several columns: the triangles at each end and
            // the parallelogram slices between.
            let s = (xb - xa).recip();
            let xaf = xa - xa_floor;
            let a0 = 0.5 * s * (1.0 - xaf) * (1.0 - xaf);
            let xbf = xb - xb_ceil + 1.0;
            let am = 0.5 * s * xbf * xbf;
            acc[line + xa_i] += d * a0;
            if xb_i == xa_i + 2 {
                acc[line + xa_i + 1] += d * (1.0 - a0 - am);
            } else {
                let a1 = s * (1.5 - xaf);
                acc[line + xa_i + 1] += d * (a1 - a0);
                for col in xa_i + 2..xb_i - 1 {
                    acc[line + col] += d * s;
                }
                let a2 = a1 + (xb_i - xa_i - 3) as f32 * s;
                acc[line + xb_i - 1] += d * (1.0 - a2 - am);
            }
            acc[line + xb_i] += d * am;
        }
        x = xnext;
    }
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
        // the diagonal are the two blended by the exact area covered,
        // half, and its whole-pixel edges stay exact.
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
        for (x, y) in [(5, 5), (2, 2), (8, 8)] {
            assert_eq!(
                px(&buf, 10, x, y),
                [128, 128, 128, 0xff],
                "({x}, {y}) half covered"
            );
        }
        // A pixel the edge crosses off its diagonal takes its exact
        // share: along the edge of the triangle (0, 0), (10, 0),
        // (10, 5), a slope of one half, the pixels alternate a quarter
        // and three quarters covered.
        let list = DisplayList {
            items: vec![
                white_frame(10.0, 10.0),
                DisplayItem::Polygon {
                    points: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 5.0)],
                    color: black,
                },
            ],
        };
        let buf = backend.rasterize(&list, 10, 10);
        for (x, y, value) in [(0, 0, 191), (1, 0, 64), (2, 1, 191), (3, 1, 64), (2, 0, 0)] {
            assert_eq!(
                px(&buf, 10, x, y),
                [value, value, value, 0xff],
                "({x}, {y})"
            );
        }
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

    /// A small deterministic generator: xorshift64, so that a failing
    /// case names the seed that replays it.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }

        /// A coordinate from `lo` to `hi`: mostly anywhere between,
        /// sometimes a whole pixel or a hair off one, where rounding
        /// decides.
        fn coord(&mut self, lo: f32, hi: f32) -> f32 {
            let v = lo + (self.next() % 65536) as f32 / 65536.0 * (hi - lo);
            match self.below(6) {
                0 => v.round(),
                1 => v.round() + 1.0 / 4096.0,
                2 => v.round() - 1.0 / 4096.0,
                _ => v,
            }
        }

        fn color(&mut self) -> Rgba8 {
            let a = [255, 255, 200, 128, 30][self.below(5)];
            Rgba8 {
                r: self.next() as u8,
                g: self.next() as u8,
                b: self.next() as u8,
                a,
            }
        }
    }

    /// A random item for a `w` by `h` frame, of every kind, reaching
    /// past the frame's edges now and then.
    fn random_item(rng: &mut Rng, w: f32, h: f32) -> DisplayItem {
        let (x, y) = (rng.coord(-8.0, w), rng.coord(-8.0, h));
        match rng.below(10) {
            0..=2 => DisplayItem::Rect {
                x,
                y,
                w: rng.coord(0.0, w / 3.0),
                h: rng.coord(0.0, h / 3.0),
                color: rng.color(),
            },
            3..=4 => DisplayItem::Text {
                text: ["Ag", "hello", "M", "x y z", "\u{5d0}\u{5d1}"][rng.below(5)].to_string(),
                x,
                y: rng.coord(4.0, h + 8.0),
                size: [8.0, 12.5, 16.0, 23.0][rng.below(4)],
                color: rng.color(),
            },
            5 => {
                let (dx, dy) = (rng.coord(1.0, 20.0), rng.coord(1.0, 12.0));
                let skew = rng.coord(-6.0, 6.0);
                DisplayItem::Quad {
                    points: [
                        (x, y),
                        (x + dx, y),
                        (x + dx + skew, y + dy),
                        (x + skew, y + dy),
                    ],
                    color: rng.color(),
                }
            }
            6 => {
                let points = (0..3 + rng.below(3))
                    .map(|k| {
                        let angle = k as f32 * 1.3;
                        let r = rng.coord(2.0, 14.0);
                        (x + r * angle.cos(), y + r * angle.sin())
                    })
                    .collect();
                DisplayItem::Polygon {
                    points,
                    color: rng.color(),
                }
            }
            7..=8 => DisplayItem::PushClip {
                x,
                y,
                w: rng.coord(0.0, w),
                h: rng.coord(0.0, h),
            },
            _ => DisplayItem::PopClip,
        }
    }

    /// `n` random items over a background: a white frame, nothing, or
    /// a translucent rectangle over part of it, so that what a raster
    /// starts from shows.
    fn random_list(rng: &mut Rng, n: usize, w: u32, h: u32) -> DisplayList {
        let background = match rng.below(4) {
            0 | 1 => white_frame(w as f32, h as f32),
            2 => DisplayItem::PopClip,
            _ => DisplayItem::Rect {
                x: rng.coord(0.0, w as f32 / 2.0),
                y: rng.coord(0.0, h as f32 / 2.0),
                w: w as f32 / 2.0,
                h: h as f32 / 2.0,
                color: Rgba8 {
                    r: 90,
                    g: 200,
                    b: 20,
                    a: 128,
                },
            },
        };
        let mut items = vec![background];
        items.extend((0..n).map(|_| random_item(rng, w as f32, h as f32)));
        DisplayList { items }
    }

    #[test]
    fn redrawing_a_rectangle_gives_a_whole_rasters_pixels_there_and_touches_nothing_else() {
        // Whatever the frame held, drawing a list again inside a
        // rectangle of whole pixels leaves there what a whole raster of
        // the list has, byte for byte, and every pixel outside as it
        // was (M13.3): what each damaged rectangle relies on, for every
        // kind of item across its edge or inside it.
        let (w, h) = (96u32, 64u32);
        let mut backend = test_backend();
        for seed in 1..=1500u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            let n = 1 + rng.below(12);
            let list = random_list(&mut rng, n, w, h);
            let n = rng.below(8);
            let before = random_list(&mut rng, n, w, h);
            let x0 = rng.below(w as usize) as u32;
            let y0 = rng.below(h as usize) as u32;
            let rect = PixelRect {
                x0,
                y0,
                x1: x0 + 1 + rng.below((w - x0) as usize) as u32,
                y1: y0 + 1 + rng.below((h - y0) as usize) as u32,
            };
            backend.forget_frame();
            let whole = backend.rasterize(&list, w, h);
            backend.forget_frame();
            let old = backend.rasterize(&before, w, h);
            let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("a frame");
            pixmap.data_mut().copy_from_slice(&old);
            backend.redraw(&mut pixmap, &list, rect);
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let inside = (rect.x0 as usize..rect.x1 as usize).contains(&x)
                        && (rect.y0 as usize..rect.y1 as usize).contains(&y);
                    let expect = if inside { &whole } else { &old };
                    assert_eq!(
                        px(pixmap.data(), w as usize, x, y),
                        px(expect, w as usize, x, y),
                        "seed {seed}: ({x}, {y}), {} {rect:?}, list {:?}",
                        if inside { "inside" } else { "outside" },
                        list.items
                    );
                }
            }
        }
    }

    #[test]
    fn a_frame_after_edits_is_a_whole_raster_and_mostly_redrawn_in_part() {
        // Random lists edited a step at a time — an item changed, put
        // in, taken out or moved, a clip among them — each frame drawn
        // from the last, and held to a whole raster of its list.
        let (w, h) = (160u32, 120u32);
        let mut retained = test_backend();
        let mut whole = test_backend();
        let (mut partial, mut none, mut whole_frames) = (0, 0, 0);
        for seed in 1..=300u64 {
            let mut rng = Rng(seed.wrapping_mul(0x2545_f491_4f6c_dd1d));
            let n = 4 + rng.below(16);
            let mut list = random_list(&mut rng, n, w, h);
            retained.forget_frame();
            retained.rasterize(&list, w, h);
            for step in 0..12 {
                let at = 1 + rng.below(list.items.len());
                match rng.below(5) {
                    0 if at < list.items.len() => {
                        list.items[at] = random_item(&mut rng, w as f32, h as f32);
                    }
                    1 => list
                        .items
                        .insert(at, random_item(&mut rng, w as f32, h as f32)),
                    2 if at < list.items.len() => {
                        list.items.remove(at);
                    }
                    3 if at < list.items.len() => {
                        let item = list.items.remove(at);
                        let to = 1 + rng.below(list.items.len());
                        list.items.insert(to, item);
                    }
                    _ => {}
                }
                let pixels = retained.rasterize(&list, w, h);
                whole.forget_frame();
                assert!(
                    pixels == whole.rasterize(&list, w, h),
                    "seed {seed}, step {step}: {:?} differs from a whole raster of {:?}",
                    retained.redrawn(),
                    list.items
                );
                match retained.redrawn() {
                    Redrawn::Rects(rects) if rects.is_empty() => none += 1,
                    Redrawn::Rects(_) => partial += 1,
                    Redrawn::Whole => whole_frames += 1,
                }
            }
        }
        // Of the frames an edit changed, most were drawn in part.
        assert!(
            partial > 2 * whole_frames && none > 0,
            "{partial} frames drawn in part, {whole_frames} whole, {none} not drawn"
        );
    }

    #[test]
    fn a_rectangle_clipped_to_whole_pixels_draws_the_same_pixels_inside() {
        // What a damaged rectangle's redraw relies on for a rectangle
        // across its edge (M13.3): tiny-skia supersamples each pixel
        // from the edges within it, so an edge moved to a pixel
        // boundary changes no pixel inside it. Random rectangles,
        // opaque and translucent, their edges on, near and off whole
        // pixels, under clips of their own.
        let (w, h) = (48u32, 32u32);
        let mut backend = test_backend();
        for seed in 1..=20_000u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            let item = DisplayItem::Rect {
                x: rng.coord(-6.0, 50.0),
                y: rng.coord(-6.0, 34.0),
                w: rng.coord(0.0, 30.0),
                h: rng.coord(0.0, 20.0),
                color: rng.color(),
            };
            let frame = Clip::new(0.0, 0.0, w as f32, h as f32);
            let clip = match rng.below(2) {
                0 => frame,
                _ => frame.intersect(Clip::new(
                    rng.coord(-2.0, 40.0),
                    rng.coord(-2.0, 30.0),
                    rng.coord(0.0, 48.0),
                    rng.coord(0.0, 32.0),
                )),
            };
            let x0 = rng.below(w as usize) as u32;
            let y0 = rng.below(h as usize) as u32;
            let rect = PixelRect {
                x0,
                y0,
                x1: x0 + 1 + rng.below((w - x0) as usize) as u32,
                y1: y0 + 1 + rng.below((h - y0) as usize) as u32,
            };
            let mut whole = tiny_skia::Pixmap::new(w, h).expect("a frame");
            whole.fill(tiny_skia::Color::from_rgba8(40, 90, 200, 255));
            let mut clipped = whole.clone();
            backend.draw(&mut whole, &item, clip);
            backend.draw(&mut clipped, &item, clip.intersect(rect.clip()));
            for y in rect.y0 as usize..rect.y1 as usize {
                for x in rect.x0 as usize..rect.x1 as usize {
                    assert_eq!(
                        px(clipped.data(), w as usize, x, y),
                        px(whole.data(), w as usize, x, y),
                        "seed {seed}: ({x}, {y}) of {item:?} under {clip:?} in {rect:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_quad_across_a_rectangles_edge_is_drawn_whole_and_copied_back() {
        // Clipped to the rectangle, this quad's slanted edges would be
        // edges from new end points, which tiny-skia snaps to its grid
        // differently: a pixel inside comes out otherwise. Drawn whole
        // on a copy, of which the inside comes back, it is a whole
        // raster's.
        let (w, h) = (48, 32);
        let points = [
            (34.000244, 18.231445),
            (48.67418, 18.231445),
            (55.940536, 29.231201),
            (41.2666, 29.231201),
        ];
        let black = Rgba8::rgb(0, 0, 0);
        let rect = PixelRect {
            x0: 35,
            y0: 11,
            x1: 46,
            y1: 25,
        };
        let frame = Clip::new(0.0, 0.0, w as f32, h as f32);
        let mut whole = tiny_skia::Pixmap::new(w, h).expect("a frame");
        fill_polygon(&mut whole, &points, &black, frame);
        let mut clipped = tiny_skia::Pixmap::new(w, h).expect("a frame");
        fill_polygon(&mut clipped, &points, &black, frame.intersect(rect.clip()));
        assert_ne!(
            px(clipped.data(), w as usize, 35, 20),
            px(whole.data(), w as usize, 35, 20),
            "clipping the quad changes a pixel inside"
        );
        let list = DisplayList {
            items: vec![DisplayItem::Quad {
                points,
                color: black,
            }],
        };
        let mut backend = test_backend();
        let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("a frame");
        backend.redraw(&mut pixmap, &list, rect);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let inside = (35..46).contains(&x) && (11..25).contains(&y);
                let expect = if inside {
                    px(whole.data(), w as usize, x, y)
                } else {
                    [0, 0, 0, 0]
                };
                assert_eq!(px(pixmap.data(), w as usize, x, y), expect, "({x}, {y})");
            }
        }
    }

    #[test]
    fn the_same_list_redraws_nothing_and_a_new_size_redraws_everything() {
        let mut backend = test_backend();
        let list = DisplayList {
            items: vec![
                white_frame(20.0, 20.0),
                DisplayItem::Rect {
                    x: 2.5,
                    y: 3.25,
                    w: 4.0,
                    h: 4.0,
                    color: Rgba8::rgb(9, 9, 9),
                },
            ],
        };
        let first = backend.rasterize(&list, 20, 20);
        assert_eq!(backend.redrawn(), &Redrawn::Whole);
        assert_eq!(backend.rasterize(&list, 20, 20), first);
        assert_eq!(backend.redrawn(), &Redrawn::Rects(Vec::new()));
        // One rectangle moved: the pixels it left and those it reached.
        let mut moved = list.clone();
        moved.items[1] = DisplayItem::Rect {
            x: 12.5,
            y: 3.25,
            w: 4.0,
            h: 4.0,
            color: Rgba8::rgb(9, 9, 9),
        };
        backend.rasterize(&moved, 20, 20);
        assert_eq!(
            backend.redrawn(),
            &Redrawn::Rects(vec![
                PixelRect {
                    x0: 2,
                    y0: 3,
                    x1: 7,
                    y1: 8
                },
                PixelRect {
                    x0: 12,
                    y0: 3,
                    x1: 17,
                    y1: 8
                },
            ])
        );
        backend.rasterize(&moved, 21, 20);
        assert_eq!(backend.redrawn(), &Redrawn::Whole);
        // Damage over half the frame: drawn whole.
        let mut wide = moved.clone();
        wide.items.push(DisplayItem::Rect {
            x: 0.0,
            y: 0.0,
            w: 21.0,
            h: 11.0,
            color: Rgba8::rgb(1, 2, 3),
        });
        backend.rasterize(&wide, 21, 20);
        assert_eq!(backend.redrawn(), &Redrawn::Whole);
    }

    #[test]
    fn a_clip_that_changed_under_a_shared_end_damages_what_it_held() {
        // The lists differ only in a clip's extent; the rectangle after
        // it is the same item in both, drawn under either clip.
        let mut backend = test_backend();
        let list = |clip_w: f32| DisplayList {
            items: vec![
                white_frame(20.0, 20.0),
                DisplayItem::PushClip {
                    x: 0.0,
                    y: 0.0,
                    w: clip_w,
                    h: 20.0,
                },
                DisplayItem::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 20.0,
                    h: 5.0,
                    color: Rgba8::rgb(0, 0, 0),
                },
                DisplayItem::PopClip,
            ],
        };
        backend.rasterize(&list(4.0), 20, 20);
        let pixels = backend.rasterize(&list(9.0), 20, 20);
        let mut whole = test_backend();
        assert_eq!(pixels, whole.rasterize(&list(9.0), 20, 20));
        assert_eq!(
            backend.redrawn(),
            &Redrawn::Rects(vec![PixelRect {
                x0: 0,
                y0: 0,
                x1: 9,
                y1: 20
            }])
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
