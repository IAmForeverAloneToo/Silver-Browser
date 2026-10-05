//! Rasterization backends: display list in, pixels out.
//!
//! Owns the pixel pipeline. The backend trait is the seam behind which the
//! future wgpu (GPU) backend will drop in; `SoftwareBackend` (tiny-skia +
//! CPU) is the default. Only this crate (and `shell`) touches pixel
//! buffers — per the layering contract in `docs/architecture.md`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::RangeInclusive;
use std::time::Instant;

use paint::{DisplayItem, DisplayList, GroupKey, LayerReason, Rgba8, TILE_HEIGHT};
use style::transform::Matrix;
use text::{
    Curve, FontCollection, FontError, GlyphMask, MAX_MASK_PIXELS, Script, ShapedGlyph,
    glyph_outline, rasterize_glyph,
};

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

/// How many edits apart two lists may be for [`align`] to match their
/// items one by one (M19): past this many, the items between their
/// shared start and end are all taken as changed, which is right if
/// more than it need be, and the cost of looking stays bounded.
const MAX_DIFF_EDITS: usize = 512;

/// How many bytes of layer pixmaps the backend holds unless told
/// otherwise (M19.6): sixteen frames of 1280 by 800, modest on purpose.
pub const DEFAULT_LAYER_BUDGET: usize = 64 << 20;

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

/// A group being drawn into (M19): the groups around it composed into
/// one matrix, from the group's coordinates to the raster's; whether
/// the group's pixels are resampled into the raster from a pixmap of
/// its own, a scratch or a layer, rather than drawn where a raster of
/// them would put them, which [`State::is_resampled`] decides; and how
/// many clips were in force when it began, since a clip the group
/// leaves open ends with it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GroupState {
    matrix: Matrix,
    resampled: bool,
    clips: usize,
}

/// The state a raster carries through a list (M9.3, M19): the clips in
/// force, each already intersected with the ones under it and mapped
/// to the raster's coordinates, so that the innermost is the whole clip
/// and the frame is the clip when none is; and the groups being drawn
/// into.
#[derive(Debug, Clone, PartialEq)]
struct State {
    frame: Clip,
    clips: Vec<Clip>,
    groups: Vec<GroupState>,
}

impl State {
    fn new(width: u32, height: u32) -> Self {
        State {
            frame: Clip::new(0.0, 0.0, width as f32, height as f32),
            clips: Vec::new(),
            groups: Vec::new(),
        }
    }

    /// The state of a walk over a group's items in the group's own
    /// coordinates, which reach any distance either way.
    fn unbounded() -> Self {
        State {
            frame: Clip::new(-1e9, -1e9, 2e9, 2e9),
            clips: Vec::new(),
            groups: Vec::new(),
        }
    }

    /// The state of a raster into a scratch or a layer whose
    /// coordinates are the list's mapped by `base`, which only
    /// translates and scales.
    fn based(width: u32, height: u32, base: Matrix) -> Self {
        let mut state = State::new(width, height);
        state.groups.push(GroupState {
            matrix: base,
            resampled: false,
            clips: 0,
        });
        state
    }

    /// The clip in force.
    fn clip(&self) -> Clip {
        self.clips.last().copied().unwrap_or(self.frame)
    }

    /// From the list's coordinates here to the raster's; `None` outside
    /// every group, where they are the same.
    fn matrix(&self) -> Option<Matrix> {
        self.groups.last().map(|g| g.matrix)
    }

    /// Whether a group resampled from a pixmap of its own is being
    /// walked into: its items are drawn there, and sampled from there
    /// into the raster.
    fn resampling(&self) -> bool {
        self.groups.last().is_some_and(|g| g.resampled)
    }

    /// The scale and translation the items here are drawn through, as
    /// `(sx, sy, tx, ty)`: the identity outside every group. A raster
    /// draws only through groups that have one; inside a scratch group
    /// it draws nothing here.
    fn through(&self) -> (f32, f32, f32, f32) {
        self.matrix()
            .and_then(|m| m.scale_and_translation())
            .unwrap_or((1.0, 1.0, 0.0, 0.0))
    }

    /// Whether a group with `matrix` and `opacity` is composited from a
    /// scratch of its own: its matrix does more than translate and
    /// scale, or its opacity is below one.
    fn needs_scratch(matrix: &Matrix, opacity: f32) -> bool {
        !(matrix.translates_and_scales_only() && opacity >= 1.0)
    }

    /// Whether a group with `matrix`, a layer when `layer` says so, is
    /// resampled from a pixmap of its own: a scratch, when the matrix
    /// does more than translate and scale; a layer, when it is kept and
    /// its matrix is not a translation by whole pixels, which lays the
    /// layer over the raster pixel for pixel. A group with an opacity
    /// alone is drawn through into a scratch in the raster's own
    /// coordinates, which resamples nothing. A layer the backend's
    /// budget refuses is drawn as a group that is no layer, which this
    /// cannot know: it then errs towards more reach, which only costs.
    fn is_resampled(matrix: &Matrix, layer: Option<LayerReason>) -> bool {
        !matrix.translates_and_scales_only()
            || (layer.is_some() && whole_pixel_translation(matrix).is_none())
    }

    /// The rectangle at `(x, y)` of size `w` by `h` in the list's
    /// coordinates here, in the raster's: exact under groups drawn
    /// through, the bounds of its image inside a scratch group. `None`
    /// when it has no image.
    fn map(&self, x: f32, y: f32, w: f32, h: f32) -> Option<Clip> {
        match self.matrix() {
            None => Some(Clip::new(x, y, w, h)),
            Some(m) => match m.scale_and_translation() {
                Some((sx, sy, tx, ty)) => Some(Clip::new(x * sx + tx, y * sy + ty, w * sx, h * sy)),
                None => m
                    .bounds(x, y, w, h)
                    .map(|(x0, y0, x1, y1)| Clip { x0, y0, x1, y1 }),
            },
        }
    }

    /// Takes `item` into account: a clip or a group begins or ends, or
    /// for a drawing nothing changes. An unmatched end is ignored, and
    /// a clip's end never reaches past the group it is in.
    fn apply(&mut self, item: &DisplayItem) {
        match item {
            DisplayItem::PushClip { x, y, w, h } => {
                // A clip with no image clips everything.
                let clip = self
                    .map(*x, *y, *w, *h)
                    .unwrap_or(Clip::new(0.0, 0.0, 0.0, 0.0));
                self.clips.push(self.clip().intersect(clip));
            }
            DisplayItem::PopClip => {
                let floor = self.groups.last().map_or(0, |g| g.clips);
                if self.clips.len() > floor {
                    self.clips.pop();
                }
            }
            DisplayItem::PushGroup { matrix, layer, .. } => {
                let composed = self.matrix().map_or(*matrix, |m| m.then(matrix));
                let resampled = self.resampling() || State::is_resampled(matrix, *layer);
                self.groups.push(GroupState {
                    matrix: composed,
                    resampled,
                    clips: self.clips.len(),
                });
            }
            DisplayItem::PopGroup => {
                if let Some(group) = self.groups.pop() {
                    self.clips.truncate(group.clips);
                }
            }
            _ => {}
        }
    }
}

/// The index of the [`DisplayItem::PopGroup`] that ends the group
/// `items[start]` begins, or `items.len()` when none does.
fn group_end(items: &[DisplayItem], start: usize) -> usize {
    let mut depth = 0usize;
    for (i, item) in items.iter().enumerate().skip(start + 1) {
        match item {
            DisplayItem::PushGroup { .. } => depth += 1,
            DisplayItem::PopGroup if depth == 0 => return i,
            DisplayItem::PopGroup => depth -= 1,
            _ => {}
        }
    }
    items.len()
}

/// The whole pixels `matrix` translates by, as `(dx, dy)`, when it
/// does nothing else: a pixmap drawn through it is a copy of its
/// pixels, moved.
fn whole_pixel_translation(matrix: &Matrix) -> Option<(i64, i64)> {
    let (sx, sy, tx, ty) = matrix.scale_and_translation()?;
    let whole = |v: f32| v.fract() == 0.0 && v.abs() < 1e9;
    (sx == 1.0 && sy == 1.0 && whole(tx) && whole(ty)).then_some((tx as i64, ty as i64))
}

/// The frame a raster left, kept to draw the next one by (M13.3).
struct Retained {
    list: DisplayList,
    pixmap: tiny_skia::Pixmap,
}

/// What the last raster did with the layers (M19): the groups it
/// composited from a layer, those whose layer it drew into, with how
/// many pixmaps of each — a whole layer is one, a layer of tiles as
/// many tiles as were drawn — and how many tiles it drew after the
/// frame, ahead of a scroll.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layered {
    pub composited: Vec<GroupKey>,
    pub rasterized: Vec<(GroupKey, u32)>,
    pub prepainted: u32,
}

/// A layer a backend holds (M19): its group, why it is a layer, how
/// many bytes of pixmaps it holds and in how many tiles, none for a
/// layer kept whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerInfo {
    pub key: GroupKey,
    pub reason: LayerReason,
    pub bytes: usize,
    pub tiles: u32,
}

/// A rasterizer: display list + viewport size in, framebuffer out.
pub trait RenderBackend {
    /// Rasterizes `list` into a `width` x `height` RGBA8 frame (row-major,
    /// 4 bytes per pixel, alpha-premultiplied as produced by tiny-skia —
    /// everything Milestone-1 draws is opaque, so premultiplied and
    /// straight alpha coincide), lent out until the next call: the
    /// backend keeps it. The frame is the list's whole raster, however
    /// the backend comes by it: it may draw again only what changed
    /// since the last frame (M13.3), and keep groups that are layers
    /// from frame to frame (M19), but the bytes are the same. A size
    /// the backend refuses gives an empty frame.
    fn rasterize(&mut self, list: &DisplayList, width: u32, height: u32) -> &[u8];

    /// [`RenderBackend::rasterize`] with the time the frame has (M19):
    /// work the frame can do without, the tiles of a scrolled layer
    /// next to what the viewport shows, is done after the frame until
    /// `deadline`, and not at all without one.
    fn rasterize_by(
        &mut self,
        list: &DisplayList,
        width: u32,
        height: u32,
        deadline: Option<Instant>,
    ) -> &[u8] {
        let _ = deadline;
        self.rasterize(list, width, height)
    }

    /// Sets how many bytes of layer pixmaps the backend may hold
    /// (M19.6); a backend without layers holds none.
    fn set_layer_budget(&mut self, bytes: usize) {
        let _ = bytes;
    }

    /// How many bytes of layer pixmaps the backend holds.
    fn layer_bytes(&self) -> usize {
        0
    }

    /// The layers the backend holds, in a fixed order by key.
    fn layers(&self) -> Vec<LayerInfo> {
        Vec::new()
    }

    /// What the last raster did with the layers.
    fn layered(&self) -> Layered {
        Layered::default()
    }
}

/// A layer (M19): a group's pixels kept from frame to frame under its
/// key, in the group's own coordinates, with the items they are of.
struct Layer {
    reason: LayerReason,
    items: Vec<DisplayItem>,
    store: Store,
    /// The last frame to composite the layer, or make it.
    used: u64,
}

/// Where a layer keeps its pixels.
enum Store {
    /// One pixmap of the group's whole reach, whose top-left texel is
    /// at `origin` in the group's coordinates.
    Whole {
        pixmap: tiny_skia::Pixmap,
        origin: (f32, f32),
    },
    /// Tiles (M19.4): strips `width` pixels across from `left`, each
    /// [`TILE_HEIGHT`] tall, by their row, row `r` holding the texels
    /// from `r * TILE_HEIGHT` down.
    Tiles {
        left: i32,
        width: u32,
        rows: BTreeMap<i32, Tile>,
    },
}

/// One tile of a layer, and the last frame to composite it, or make
/// it.
struct Tile {
    pixmap: tiny_skia::Pixmap,
    used: u64,
}

impl Layer {
    /// The bytes of pixmaps it holds.
    fn bytes(&self) -> usize {
        match &self.store {
            Store::Whole { pixmap, .. } => pixmap_bytes(pixmap),
            Store::Tiles { rows, .. } => rows.values().map(|t| pixmap_bytes(&t.pixmap)).sum(),
        }
    }

    /// How many tiles it holds; none when kept whole.
    fn tiles(&self) -> u32 {
        match &self.store {
            Store::Whole { .. } => 0,
            Store::Tiles { rows, .. } => rows.len() as u32,
        }
    }
}

fn pixmap_bytes(pixmap: &tiny_skia::Pixmap) -> usize {
    pixmap.data().len()
}

/// The rows of tiles that hold any texel row from `y0` to `y1`, as a
/// range, empty when there is none.
fn tile_rows(y0: f32, y1: f32) -> RangeInclusive<i32> {
    let tall = TILE_HEIGHT as f32;
    if !(y0.is_finite() && y1.is_finite()) || y1 <= y0 {
        return RangeInclusive::new(1, 0);
    }
    let row = |y: f32| (y / tall).floor().clamp(-1e8, 1e8) as i32;
    row(y0.floor())..=row(y1.ceil() - 1.0)
}

/// What of a group whose coordinates `composed` maps to the raster's
/// can show under `clip`, in the group's own coordinates, as `(x0, y0,
/// x1, y1)`: the clip's bounds back through the matrix. `None` when
/// nothing can.
fn visible_in(composed: &Matrix, clip: Clip) -> Option<(f32, f32, f32, f32)> {
    if clip.is_empty() {
        return None;
    }
    let inverse = composed.inverse()?;
    let (x0, y0, x1, y1) =
        inverse.bounds(clip.x0, clip.y0, clip.x1 - clip.x0, clip.y1 - clip.y0)?;
    ([x0, y0, x1, y1].iter().all(|v| v.is_finite()) && x1 > x0 && y1 > y0)
        .then_some((x0, y0, x1, y1))
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
    /// The masks of glyphs scaled unevenly (M19), by script, glyph id,
    /// font size bits and the two scales' bits: their outlines filled
    /// through the scale, which a mask at another size cannot stand in
    /// for. Counted and emptied with `masks`.
    outlined: HashMap<(Script, u16, u32, u32, u32), GlyphMask>,
    /// Coverage bytes held in `masks` and `outlined`, against
    /// [`MASK_CACHE_BUDGET`].
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
    /// The layers (M19), by their groups' keys.
    layers: HashMap<GroupKey, Layer>,
    /// The bytes of pixmaps held in `layers`, against `layer_budget`.
    layer_bytes: usize,
    /// How many bytes of layer pixmaps may be held (M19.6).
    layer_budget: usize,
    /// Which raster this is, counting them: what a layer's and a tile's
    /// `used` are against.
    frame: u64,
    /// What the last raster did with the layers.
    layered: Layered,
    /// The groups whose key the last list gave to two groups: neither
    /// is composited from the layer under it, since a layer holds one.
    doubled: HashSet<GroupKey>,
    /// The tiles of the margin around the viewport not yet drawn, as
    /// the layer's key and the row, with how far the row is from the
    /// viewport's middle in rows; drawn after the frame, nearest first.
    deferred: Vec<(GroupKey, i32, i32)>,
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
            outlined: HashMap::new(),
            cached_bytes: 0,
            shaped: HashMap::new(),
            cached_glyphs: 0,
            retained: None,
            scratch: None,
            redrawn: Redrawn::Whole,
            layers: HashMap::new(),
            layer_bytes: 0,
            layer_budget: DEFAULT_LAYER_BUDGET,
            frame: 0,
            layered: Layered::default(),
            doubled: HashSet::new(),
            deferred: Vec::new(),
        }
    }

    /// Empties the glyph-mask and shaped-run caches: what a new
    /// document calls for, since the text it draws is not the last
    /// one's.
    pub fn clear_glyph_cache(&mut self) {
        self.masks.clear();
        self.outlined.clear();
        self.cached_bytes = 0;
        self.shaped.clear();
        self.cached_glyphs = 0;
    }

    /// Forgets the last frame and the layers (M19): the next raster
    /// draws the whole frame and every layer anew, as a new document
    /// calls for.
    pub fn forget_frame(&mut self) {
        self.retained = None;
        self.layers.clear();
        self.layer_bytes = 0;
        self.deferred.clear();
    }

    /// Forgets the last frame but keeps the layers: the next raster
    /// draws the whole frame from them.
    #[doc(hidden)]
    pub fn forget_last_frame(&mut self) {
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

    /// The mask of a glyph scaled by `(sx, sy)`, two different scales
    /// (M19): its outline at `size` filled through the scale, from the
    /// pen at a whole pixel, as [`outline_mask`] makes it.
    fn scaled_mask(
        &mut self,
        script: Script,
        glyph_id: u16,
        size: f32,
        (sx, sy): (f32, f32),
    ) -> &GlyphMask {
        let key = (script, glyph_id, size.to_bits(), sx.to_bits(), sy.to_bits());
        if !self.outlined.contains_key(&key) {
            let font = self.fonts.font_for(script).glyph_font();
            let mask = outline_mask(font, glyph_id, size, (sx, sy));
            let bytes = mask.coverage.len();
            if self.cached_bytes + bytes > MASK_CACHE_BUDGET {
                self.clear_glyph_cache();
            }
            self.cached_bytes += bytes;
            self.outlined.insert(key, mask);
        }
        &self.outlined[&key]
    }

    /// Draws `text` with its baseline's left end at `origin`, in the
    /// raster's coordinates, its glyphs scaled by `scale` (M19): the run
    /// is shaped at `size` and its glyphs placed from the scaled
    /// origin; a glyph is rasterized at the size the scale makes when
    /// the scale is the same both ways, which is as sharp as text of
    /// that size, and its outline is filled through the scale
    /// otherwise.
    #[allow(clippy::too_many_arguments)]
    fn draw_text(
        &mut self,
        pixmap: &mut tiny_skia::Pixmap,
        text: &str,
        origin: (f32, f32),
        size: f32,
        color: &Rgba8,
        clip: Clip,
        scale: (f32, f32),
    ) {
        let (x, y) = origin;
        let (sx, sy) = scale;
        let glyphs = self.shaped(text, size);
        for (script, g) in glyphs {
            let (gx, gy) = (x + g.x * sx, y + g.y * sy);
            let mask = if sx == sy {
                self.mask_for(script, g.glyph_id, size * sx)
            } else {
                self.scaled_mask(script, g.glyph_id, size, scale)
            };
            let base_x = gx + mask.left as f32;
            let base_y = gy + mask.top as f32;
            blit_mask(pixmap, mask, color, base_x, base_y, clip);
        }
    }

    /// Draws `list` whole onto `pixmap`, which is clear.
    fn draw_all(&mut self, pixmap: &mut tiny_skia::Pixmap, list: &DisplayList) {
        let state = State::new(pixmap.width(), pixmap.height());
        self.draw_items(pixmap, &list.items, state);
    }

    /// Whether the group `key`, a layer when `layer` says so, is
    /// composited from the layer held under its key (M19).
    fn is_held(&self, key: &GroupKey, layer: Option<LayerReason>) -> bool {
        layer.is_some() && self.layers.contains_key(key) && !self.doubled.contains(key)
    }

    /// Draws `items` onto `pixmap` from `state`, as a whole raster does:
    /// each through the groups it is in, a group held as a layer
    /// composited from it, and a group that needs a scratch of its own
    /// composited from one (M19).
    fn draw_items(
        &mut self,
        pixmap: &mut tiny_skia::Pixmap,
        items: &[DisplayItem],
        mut state: State,
    ) {
        let mut i = 0;
        while i < items.len() {
            let item = &items[i];
            if let DisplayItem::PushGroup {
                matrix,
                opacity,
                key,
                layer,
            } = item
                && (State::needs_scratch(matrix, *opacity) || self.is_held(key, *layer))
            {
                let end = group_end(items, i);
                let composed = state.matrix().map_or(*matrix, |m| m.then(matrix));
                let held = self.is_held(key, *layer).then_some(*key);
                self.composite(
                    pixmap,
                    &items[i + 1..end],
                    composed,
                    *opacity,
                    state.clip(),
                    None,
                    held,
                );
                i = end + 1;
                continue;
            }
            let clip = state.clip();
            let through = state.through();
            state.apply(item);
            self.draw(pixmap, item, clip, through);
            i += 1;
        }
    }

    /// Composites the group `items`, whose coordinates `matrix` maps to
    /// `pixmap`'s, over `pixmap` with `opacity` under `clip`: from the
    /// layer `held` names, when one is (M19), else from a scratch. When
    /// the matrix only translates and scales, the scratch is in the
    /// pixmap's coordinates and the items are drawn through the matrix
    /// into it, so that the pixels are those a raster of them there
    /// would give, and the scratch is laid over the pixmap pixel for
    /// pixel; otherwise the scratch is in the group's own coordinates,
    /// and is resampled into the pixmap through the matrix, bilinearly.
    /// Only the pixels of `only` are written when it is given (M13.3).
    /// A group whose scratch would have more pixels than a frame may is
    /// not drawn.
    #[allow(clippy::too_many_arguments)]
    fn composite(
        &mut self,
        pixmap: &mut tiny_skia::Pixmap,
        items: &[DisplayItem],
        matrix: Matrix,
        opacity: f32,
        clip: Clip,
        only: Option<PixelRect>,
        held: Option<GroupKey>,
    ) {
        let (width, height) = (pixmap.width(), pixmap.height());
        let Some(within) = clip.pixels(width, height) else {
            return;
        };
        let within = match only {
            Some(rect) => match within.intersect(rect) {
                Some(within) => within,
                None => return,
            },
            None => within,
        };
        if let Some(key) = held
            && self.composite_layer(pixmap, key, matrix, opacity, within)
        {
            return;
        }
        if matrix.translates_and_scales_only() {
            // The items' reach in the pixmap, drawn through.
            let mut state = State::based(width, height, matrix);
            let mut reach: Option<PixelRect> = None;
            for item in items {
                if let Some(bounds) = self.bounds(item, &state, width, height) {
                    reach = Some(reach.map_or(bounds, |r| r.union(bounds)));
                }
                state.apply(item);
            }
            let Some(region) = reach.and_then(|r| r.intersect(within)) else {
                return;
            };
            let (w, h) = (region.x1 - region.x0, region.y1 - region.y0);
            let Some(mut scratch) = tiny_skia::Pixmap::new(w, h) else {
                return;
            };
            let base =
                Matrix::translate(-(region.x0 as f32), -(region.y0 as f32), 0.0).then(&matrix);
            self.draw_items(&mut scratch, items, State::based(w, h, base));
            blit(
                pixmap,
                &scratch,
                (region.x0 as i64, region.y0 as i64),
                opacity,
                region,
            );
            return;
        }
        // The group's own coordinates: its items' reach there, within
        // what can land in the region, two pixels more all round for
        // the resampling.
        let Some(inverse) = matrix.inverse() else {
            return;
        };
        let (wx, wy) = (within.x0 as f32, within.y0 as f32);
        let (ww, wh) = (
            (within.x1 - within.x0) as f32,
            (within.y1 - within.y0) as f32,
        );
        let Some((lx0, ly0, lx1, ly1)) = inverse.bounds(wx, wy, ww, wh) else {
            return;
        };
        let landing = Clip {
            x0: lx0 - 2.0,
            y0: ly0 - 2.0,
            x1: lx1 + 2.0,
            y1: ly1 + 2.0,
        };
        let Some(reach) = self.reach(items) else {
            return;
        };
        // The scratch is the whole reach, so that a region of it
        // composited alone (M13.3) samples the same texels from the
        // same place as the whole, to the bit; only a reach with more
        // pixels than a frame may have is cut to what can land in the
        // region.
        let mut corners = whole_reach(reach);
        if !fits_a_frame(corners) {
            let cut = reach.intersect(landing);
            if cut.is_empty() {
                return;
            }
            corners = whole_reach(cut);
            if !fits_a_frame(corners) {
                return;
            }
        }
        let (gx0, gy0, gx1, gy1) = corners;
        let (gw, gh) = ((gx1 - gx0) as f64, (gy1 - gy0) as f64);
        if gw < 1.0 || gh < 1.0 {
            return;
        }
        let (gw, gh) = (gw as u32, gh as u32);
        let Some(mut scratch) = tiny_skia::Pixmap::new(gw, gh) else {
            return;
        };
        let base = Matrix::translate(-gx0, -gy0, 0.0);
        self.draw_items(&mut scratch, items, State::based(gw, gh, base));
        composite_pixmap(
            pixmap,
            &scratch,
            (gx0, gy0),
            &matrix,
            &inverse,
            opacity,
            within,
        );
    }

    /// How far `items`, in their own coordinates, reach: the union of
    /// their extents, walked without bounds; `None` when they draw
    /// nothing.
    fn reach(&mut self, items: &[DisplayItem]) -> Option<Clip> {
        let mut reach: Option<Clip> = None;
        let mut state = State::unbounded();
        for item in items {
            if let Some(extent) = self.extent(item, &state) {
                reach = Some(match reach {
                    None => extent,
                    Some(r) => Clip {
                        x0: r.x0.min(extent.x0),
                        y0: r.y0.min(extent.y0),
                        x1: r.x1.max(extent.x1),
                        y1: r.y1.max(extent.y1),
                    },
                });
            }
            state.apply(item);
        }
        reach.filter(|r| !r.is_empty())
    }

    /// Composites the layer held under `key`, if one is, over `pixmap`
    /// by `matrix`, from the layer's coordinates to the pixmap's, at
    /// `opacity`, writing the pixels of `within` alone (M19): a whole
    /// layer as one pixmap, a layer of tiles each tile the region
    /// meets, marking what it used for the budget. Whether one was.
    fn composite_layer(
        &mut self,
        pixmap: &mut tiny_skia::Pixmap,
        key: GroupKey,
        matrix: Matrix,
        opacity: f32,
        within: PixelRect,
    ) -> bool {
        let frame = self.frame;
        let Some(layer) = self.layers.get_mut(&key) else {
            return false;
        };
        layer.used = frame;
        if let Some(inverse) = matrix.inverse() {
            match &mut layer.store {
                Store::Whole {
                    pixmap: src,
                    origin,
                } => {
                    composite_pixmap(pixmap, src, *origin, &matrix, &inverse, opacity, within);
                }
                Store::Tiles { left, width, rows } => {
                    if whole_pixel_translation(&matrix).is_some() {
                        for (row, tile) in rows.iter_mut() {
                            let origin = (*left as f32, *row as f32 * TILE_HEIGHT as f32);
                            if composite_pixmap(
                                pixmap,
                                &tile.pixmap,
                                origin,
                                &matrix,
                                &inverse,
                                opacity,
                                within,
                            ) {
                                tile.used = frame;
                            }
                        }
                    } else {
                        // Resampled across the tiles as one, so that the
                        // seams between them sample both sides.
                        let tiled = Tiled {
                            rows,
                            left: *left,
                            width: *width,
                        };
                        let (x0, y0, x1, y1) = tiled.reach();
                        if let Some((fx0, fy0, fx1, fy1)) = matrix.bounds(x0, y0, x1 - x0, y1 - y0)
                            && let Some(landed) = PixelRect::around(
                                fx0 - 1.0,
                                fy0 - 1.0,
                                fx1 + 1.0,
                                fy1 + 1.0,
                                pixmap.width(),
                                pixmap.height(),
                            )
                            .and_then(|r| r.intersect(within))
                        {
                            resample(pixmap, &tiled, &inverse, opacity, landed);
                            for tile in rows.values_mut() {
                                tile.used = frame;
                            }
                        }
                    }
                }
            }
        }
        if !self.layered.composited.contains(&key) {
            self.layered.composited.push(key);
        }
        true
    }

    /// Marks the layers this frame's `items` use (M19.6), walked from
    /// `state`: a layer whose group is listed, and the tiles of it the
    /// raster shows, are this frame's, which the budget's eviction
    /// leaves alone, so that making room for one layer never costs
    /// another the frame needs.
    fn mark_layers(&mut self, items: &[DisplayItem], state: &State) {
        let mut state = state.clone();
        let mut i = 0;
        while i < items.len() {
            let item = &items[i];
            if let DisplayItem::PushGroup {
                matrix,
                key,
                layer: Some(_),
                ..
            } = item
            {
                let end = group_end(items, i);
                let composed = state.matrix().map_or(*matrix, |m| m.then(matrix));
                let clip = state.clip();
                let frame = self.frame;
                if let Some(layer) = self.layers.get_mut(key) {
                    layer.used = frame;
                    if let Store::Tiles { rows, .. } = &mut layer.store
                        && let Some((_, y0, _, y1)) = visible_in(&composed, clip)
                    {
                        for row in tile_rows(y0, y1) {
                            if let Some(tile) = rows.get_mut(&row) {
                                tile.used = frame;
                            }
                        }
                    }
                }
                let mut inner = state.clone();
                inner.apply(item);
                self.mark_layers(&items[i + 1..end], &inner);
                i = end + 1;
                continue;
            }
            state.apply(item);
            i += 1;
        }
    }

    /// Brings the layers of the groups among `items` up to date (M19):
    /// a group that is a layer is rasterized into its layer where its
    /// own items changed, inner groups first so that an outer one's
    /// raster finds them, and a layer the budget has no room for is let
    /// go, its group drawn as one that is no layer. `frame` is the walk's
    /// state in the raster's coordinates, for what can show; `local` in
    /// the coordinates of the layer being walked into, or the raster's
    /// outside every layer, where `extra` collects the rectangles whose
    /// pixels change for reasons the lists cannot tell: a group held as
    /// a layer that was not before, or the other way, is drawn another
    /// way, which can differ by a hair, so its reach is one. `seen` is
    /// the keys met: a key met twice names no layer, since a layer
    /// holds one group.
    fn update_layers(
        &mut self,
        items: &[DisplayItem],
        frame: &State,
        local: &State,
        seen: &mut HashSet<GroupKey>,
        extra: &mut Vec<Clip>,
    ) {
        let mut frame = frame.clone();
        let mut local = local.clone();
        let mut i = 0;
        while i < items.len() {
            let item = &items[i];
            if let DisplayItem::PushGroup {
                matrix,
                key,
                layer: Some(reason),
                ..
            } = item
            {
                let end = group_end(items, i);
                let inner = &items[i + 1..end];
                let composed = frame.matrix().map_or(*matrix, |m| m.then(matrix));
                let outward = local.matrix().map_or(*matrix, |m| m.then(matrix));
                let clip = frame.clip();
                let mut inner_frame = frame.clone();
                inner_frame.apply(item);
                let mut inner_extra = Vec::new();
                self.update_layers(
                    inner,
                    &inner_frame,
                    &State::unbounded(),
                    seen,
                    &mut inner_extra,
                );
                let was = self.layers.contains_key(key) && !self.doubled.contains(key);
                let held = if seen.insert(*key) {
                    self.update_layer(*key, *reason, inner, &composed, clip, &inner_extra)
                } else {
                    self.drop_layer(*key);
                    self.doubled.insert(*key);
                    false
                };
                // The group's reach outside it, where it is drawn
                // another way now, two pixels more for the resampling;
                // and what changed inside it, which an outer layer's
                // pixels of it hold too.
                if was != held
                    && let Some(reach) = self.reach(inner)
                    && let Some(mapped) = outward_clip(&outward, reach)
                {
                    extra.push(mapped);
                }
                for r in inner_extra {
                    if let Some(mapped) = outward_clip(&outward, r) {
                        extra.push(mapped);
                    }
                }
                i = end + 1;
                continue;
            }
            frame.apply(item);
            local.apply(item);
            i += 1;
        }
    }

    /// Brings the layer of the group `key` up to date with `items`, its
    /// content in its own coordinates, which `composed` maps to the
    /// raster's, where `clip` is in force, `extra` being rectangles of
    /// it whose pixels changed for reasons its lists cannot tell: a
    /// scrolled group's as tiles (M19.4), another's as one pixmap.
    /// Whether the layer is held.
    fn update_layer(
        &mut self,
        key: GroupKey,
        reason: LayerReason,
        items: &[DisplayItem],
        composed: &Matrix,
        clip: Clip,
        extra: &[Clip],
    ) -> bool {
        let Some(visible) = visible_in(composed, clip) else {
            self.drop_layer(key);
            return false;
        };
        let mut old = self.layers.remove(&key);
        if let Some(layer) = &old {
            self.layer_bytes -= layer.bytes();
        }
        if old.as_ref().is_some_and(|layer| layer.reason != reason) {
            old = None;
        }
        let kept = match reason {
            LayerReason::Scrolled => self.update_tiles(key, reason, old, items, visible, extra),
            LayerReason::Transformed | LayerReason::WillChange => {
                self.update_whole(key, reason, old, items, extra)
            }
        };
        match kept {
            Some(layer) => {
                self.layer_bytes += layer.bytes();
                self.layers.insert(key, layer);
                true
            }
            None => false,
        }
    }

    /// The layer of the group `key` kept whole: one pixmap of the
    /// items' reach, a clear texel all round, drawn again where the
    /// items changed since `old`, whose pixmap is kept when the reach
    /// is where it was. `None` when the items draw nothing, or the
    /// budget has no room.
    fn update_whole(
        &mut self,
        key: GroupKey,
        reason: LayerReason,
        old: Option<Layer>,
        items: &[DisplayItem],
        extra: &[Clip],
    ) -> Option<Layer> {
        let reach = self.reach(items)?;
        let corners = whole_reach(reach);
        if !fits_a_frame(corners) {
            return None;
        }
        let (gx0, gy0, gx1, gy1) = corners;
        let (gw, gh) = ((gx1 - gx0) as f64, (gy1 - gy0) as f64);
        if gw < 1.0 || gh < 1.0 {
            return None;
        }
        let (gw, gh) = (gw as u32, gh as u32);
        let origin = (gx0, gy0);
        let base = Matrix::translate(-gx0, -gy0, 0.0);
        let extra: Vec<PixelRect> = extra
            .iter()
            .filter_map(|r| {
                PixelRect::around(r.x0 - gx0, r.y0 - gy0, r.x1 - gx0, r.y1 - gy0, gw, gh)
            })
            .collect();
        if let Some(Layer {
            store:
                Store::Whole {
                    mut pixmap,
                    origin: was,
                },
            items: were,
            ..
        }) = old
            && was == origin
            && pixmap.width() == gw
            && pixmap.height() == gh
        {
            if were != items || !extra.is_empty() {
                let partial = gw <= MAX_PARTIAL_SIDE && gh <= MAX_PARTIAL_SIDE;
                let mut rects = self.damaged(&were, items, gw, gh, Some(base));
                rects.extend(extra);
                let rects = merge(rects);
                let area: u64 = rects.iter().map(|r| r.area()).sum();
                if partial && area * 2 < gw as u64 * gh as u64 {
                    for rect in &rects {
                        self.redraw(&mut pixmap, items, *rect, Some(base));
                    }
                } else {
                    pixmap.fill(tiny_skia::Color::TRANSPARENT);
                    self.draw_items(&mut pixmap, items, State::based(gw, gh, base));
                }
                if !rects.is_empty() {
                    self.layered.rasterized.push((key, 1));
                }
            }
            return Some(Layer {
                reason,
                items: items.to_vec(),
                store: Store::Whole { pixmap, origin },
                used: self.frame,
            });
        }
        if !self.reserve(gw as usize * gh as usize * 4, 0) {
            return None;
        }
        let mut pixmap = tiny_skia::Pixmap::new(gw, gh)?;
        self.draw_items(&mut pixmap, items, State::based(gw, gh, base));
        self.layered.rasterized.push((key, 1));
        Some(Layer {
            reason,
            items: items.to_vec(),
            store: Store::Whole { pixmap, origin },
            used: self.frame,
        })
    }

    /// The layer of the scrolled group `key` kept as tiles (M19.4):
    /// strips as wide as what of the group can show, `visible`, the
    /// tiles that meet it drawn now, those within a `visible`'s height
    /// above and below it, the margin, left for the time after the
    /// frame, and those further off let go. The tiles of `old` are
    /// kept when the strips are where they were, drawn again where the
    /// items changed. `None` when the budget has no room for a tile the
    /// raster shows.
    fn update_tiles(
        &mut self,
        key: GroupKey,
        reason: LayerReason,
        old: Option<Layer>,
        items: &[DisplayItem],
        (vx0, vy0, vx1, vy1): (f32, f32, f32, f32),
        extra: &[Clip],
    ) -> Option<Layer> {
        let (left, right) = (vx0.floor(), vx1.ceil());
        if !(left.is_finite() && right.is_finite())
            || right - left < 1.0
            || right - left > MAX_PARTIAL_SIDE as f32
            || left.abs() > 1e8
        {
            return None;
        }
        let (left, width) = (left as i32, (right - left) as u32);
        let tall = TILE_HEIGHT;
        let now = tile_rows(vy0, vy1);
        let span = vy1 - vy0;
        let margin = tile_rows(vy0 - span, vy1 + span);
        let tile_bytes = width as usize * tall as usize * 4;
        let base = |row: i32| Matrix::translate(-(left as f32), -(row as f32 * tall as f32), 0.0);
        let mut rows: BTreeMap<i32, Tile> = BTreeMap::new();
        let mut drawn = 0u32;
        if let Some(Layer {
            store:
                Store::Tiles {
                    left: had_left,
                    width: had_width,
                    rows: had,
                },
            items: were,
            ..
        }) = old
            && had_left == left
            && had_width == width
        {
            rows = had
                .into_iter()
                .filter(|(row, _)| margin.contains(row))
                .collect();
            if (were != items || !extra.is_empty()) && !rows.is_empty() {
                // The damage over the rows held, end to end, in the
                // coordinates of the first, cut to each tile.
                let first = *rows.keys().next().expect("a row");
                let last = *rows.keys().next_back().expect("a row");
                let height = ((last - first + 1) as u32).saturating_mul(tall);
                let top = first as f32 * tall as f32;
                let mut rects = self.damaged(&were, items, width, height, Some(base(first)));
                rects.extend(extra.iter().filter_map(|r| {
                    PixelRect::around(
                        r.x0 - left as f32,
                        r.y0 - top,
                        r.x1 - left as f32,
                        r.y1 - top,
                        width,
                        height,
                    )
                }));
                let mut per_row: BTreeMap<i32, PixelRect> = BTreeMap::new();
                for rect in rects {
                    for offset in tile_rows(rect.y0 as f32, rect.y1 as f32) {
                        let row = first + offset;
                        if !rows.contains_key(&row) {
                            continue;
                        }
                        let ty = offset as i64 * tall as i64;
                        let Some(cut) = PixelRect::new(
                            rect.x0 as i64,
                            rect.y0 as i64 - ty,
                            rect.x1 as i64,
                            (rect.y1 as i64 - ty).min(tall as i64),
                        ) else {
                            continue;
                        };
                        per_row
                            .entry(row)
                            .and_modify(|r| *r = r.union(cut))
                            .or_insert(cut);
                    }
                }
                let partial = width <= MAX_PARTIAL_SIDE;
                for (row, rect) in per_row {
                    let tile = rows.get_mut(&row).expect("a row held");
                    if partial && rect.area() * 2 < width as u64 * tall as u64 {
                        self.redraw(&mut tile.pixmap, items, rect, Some(base(row)));
                    } else {
                        tile.pixmap.fill(tiny_skia::Color::TRANSPARENT);
                        self.draw_items(
                            &mut tile.pixmap,
                            items,
                            State::based(width, tall, base(row)),
                        );
                    }
                    drawn += 1;
                }
            }
        }
        // The tiles the raster shows, which it cannot do without.
        let mut held: usize = rows.values().map(|t| pixmap_bytes(&t.pixmap)).sum();
        for row in now.clone() {
            if let Some(tile) = rows.get_mut(&row) {
                tile.used = self.frame;
                continue;
            }
            if !self.reserve(tile_bytes, held) {
                return None;
            }
            let mut pixmap = tiny_skia::Pixmap::new(width, tall)?;
            self.draw_items(&mut pixmap, items, State::based(width, tall, base(row)));
            held += tile_bytes;
            drawn += 1;
            rows.insert(
                row,
                Tile {
                    pixmap,
                    used: self.frame,
                },
            );
        }
        // The margin's tiles missing, for the time after the frame.
        let middle = (*now.start() as i64 + *now.end() as i64) / 2;
        for row in margin {
            if !rows.contains_key(&row) {
                self.deferred.push((
                    key,
                    row,
                    (row as i64 - middle).unsigned_abs().min(i32::MAX as u64) as i32,
                ));
            }
        }
        if drawn > 0 {
            self.layered.rasterized.push((key, drawn));
        }
        Some(Layer {
            reason,
            items: items.to_vec(),
            store: Store::Tiles { left, width, rows },
            used: self.frame,
        })
    }

    /// Lets the layer of `key` go.
    fn drop_layer(&mut self, key: GroupKey) {
        if let Some(layer) = self.layers.remove(&key) {
            self.layer_bytes -= layer.bytes();
        }
    }

    /// Makes room for `bytes` more of layer pixmaps, beside `held`
    /// bytes kept outside the count for the moment (M19.6): the layers
    /// and tiles this frame has not used go, least recently used first,
    /// until the budget holds it. Whether it does.
    fn reserve(&mut self, bytes: usize, held: usize) -> bool {
        while self.layer_bytes + held + bytes > self.layer_budget {
            if !self.evict_oldest(self.frame) {
                return false;
            }
        }
        true
    }

    /// Lets layers and tiles go, least recently used first, until the
    /// budget holds what is left (M19.6): what a lowered budget calls
    /// for, after the frame that found it so.
    fn trim(&mut self) {
        while self.layer_bytes > self.layer_budget && self.evict_oldest(u64::MAX) {}
    }

    /// Lets the layer or tile least recently used go, among those used
    /// before the frame `before`; whether there was one.
    fn evict_oldest(&mut self, before: u64) -> bool {
        let mut oldest: Option<(u64, String, GroupKey, Option<i32>)> = None;
        let mut consider = |used: u64, key: GroupKey, row: Option<i32>| {
            if used >= before {
                return;
            }
            // Ties by the key's name, so that the order is fixed.
            let name = format!("{key:?}");
            if oldest
                .as_ref()
                .is_none_or(|(u, n, _, _)| used < *u || (used == *u && name < *n))
            {
                oldest = Some((used, name, key, row));
            }
        };
        for (key, layer) in &self.layers {
            match &layer.store {
                Store::Whole { .. } => consider(layer.used, *key, None),
                Store::Tiles { rows, .. } => {
                    for (row, tile) in rows {
                        consider(tile.used, *key, Some(*row));
                    }
                }
            }
        }
        let Some((_, _, key, row)) = oldest else {
            return false;
        };
        match row {
            None => self.drop_layer(key),
            Some(row) => {
                let mut empty = false;
                if let Some(layer) = self.layers.get_mut(&key)
                    && let Store::Tiles { rows, .. } = &mut layer.store
                {
                    if let Some(tile) = rows.remove(&row) {
                        self.layer_bytes -= pixmap_bytes(&tile.pixmap);
                    }
                    empty = rows.is_empty();
                }
                if empty {
                    self.drop_layer(key);
                }
            }
        }
        true
    }

    /// Draws the tiles of the margin not yet drawn, nearest the
    /// viewport first, while there is time before `deadline` and room
    /// under the budget without letting anything go (M19.4); the rest
    /// wait for a frame that shows them.
    fn prepaint(&mut self, deadline: Option<Instant>) {
        let Some(deadline) = deadline else {
            self.deferred.clear();
            return;
        };
        let mut deferred = std::mem::take(&mut self.deferred);
        deferred.sort_by_key(|(_, _, distance)| *distance);
        for (key, row, _) in deferred {
            if Instant::now() >= deadline {
                break;
            }
            let Some(mut layer) = self.layers.remove(&key) else {
                continue;
            };
            let bytes = layer.bytes();
            self.layer_bytes -= bytes;
            if let Store::Tiles { left, width, rows } = &mut layer.store
                && !rows.contains_key(&row)
            {
                let (left, width) = (*left, *width);
                let tile_bytes = width as usize * TILE_HEIGHT as usize * 4;
                if self.layer_bytes + bytes + tile_bytes > self.layer_budget {
                    self.layer_bytes += bytes;
                    self.layers.insert(key, layer);
                    break;
                }
                if let Some(mut pixmap) = tiny_skia::Pixmap::new(width, TILE_HEIGHT) {
                    let base =
                        Matrix::translate(-(left as f32), -(row as f32 * TILE_HEIGHT as f32), 0.0);
                    self.draw_items(
                        &mut pixmap,
                        &layer.items,
                        State::based(width, TILE_HEIGHT, base),
                    );
                    if let Store::Tiles { rows, .. } = &mut layer.store {
                        rows.insert(
                            row,
                            Tile {
                                pixmap,
                                used: self.frame,
                            },
                        );
                    }
                    self.layered.prepainted += 1;
                }
            }
            self.layer_bytes += layer.bytes();
            self.layers.insert(key, layer);
        }
    }

    /// Draws one item under `clip`, through the scale and translation
    /// `(sx, sy, tx, ty)` of the groups it is in, as a whole raster does.
    fn draw(
        &mut self,
        pixmap: &mut tiny_skia::Pixmap,
        item: &DisplayItem,
        clip: Clip,
        (sx, sy, tx, ty): (f32, f32, f32, f32),
    ) {
        let point = |(x, y): (f32, f32)| (x * sx + tx, y * sy + ty);
        match item {
            DisplayItem::PushClip { .. }
            | DisplayItem::PopClip
            | DisplayItem::PushGroup { .. }
            | DisplayItem::PopGroup => {}
            DisplayItem::Rect { x, y, w, h, color } => {
                let r = Clip::new(x * sx + tx, y * sy + ty, w * sx, h * sy).intersect(clip);
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
                self.draw_text(pixmap, text, point((*x, *y)), *size, color, clip, (sx, sy));
            }
            // No anti-aliasing for a quad: quads that share an
            // edge must tile it without a seam (a border's bands in
            // one colour). A polygon is anti-aliased by the exact
            // area of each pixel it covers: its mitres to sides of
            // another colour blend by their share of the pixel, as
            // a browser's do (M11), and its axis-aligned edges lie
            // on whole pixels, where the area is all or nothing.
            DisplayItem::Quad { points, color } => {
                fill_polygon(pixmap, &points.map(point), color, clip);
            }
            DisplayItem::Polygon { points, color } => {
                let frame = PixelRect {
                    x0: 0,
                    y0: 0,
                    x1: pixmap.width(),
                    y1: pixmap.height(),
                };
                let points: Vec<(f32, f32)> = points.iter().map(|p| point(*p)).collect();
                fill_polygon_by_area(pixmap, &points, color, clip, frame);
            }
        }
    }

    /// How far `item`, drawn from `state`, reaches in the raster's
    /// coordinates, before the pixel grid, within the clip in force;
    /// `None` when it draws nothing. The item's reach in the group's
    /// coordinates is mapped to the raster's, grown first by what the
    /// group's own raster can add there and after by what the raster
    /// can add here: a rectangle and a polygon reach exactly as far as
    /// their raster does; a quad and a run of text are given a pixel
    /// more all round against rounding, the run's glyphs placed from the
    /// extent of their masks, and the run another under a scale, which
    /// the masks' rounding does not follow exactly; inside a scratch
    /// group, a texel more all round before the mapping, which is how
    /// far the resampling reaches, and a pixel after. Text whose origin
    /// is not a number may land anywhere in the clip.
    fn extent(&mut self, item: &DisplayItem, state: &State) -> Option<Clip> {
        let clip = state.clip();
        let in_scratch = state.resampling();
        let scaled = state.through() != (1.0, 1.0, 0.0, 0.0);
        let grown = |r: Clip, by: f32| Clip {
            x0: r.x0 - by,
            y0: r.y0 - by,
            x1: r.x1 + by,
            y1: r.y1 + by,
        };
        let hull = |points: &[(f32, f32)]| {
            let (x0, y0, x1, y1) = extent(points);
            [x0, y0, x1, y1]
                .iter()
                .all(|v| v.is_finite())
                .then_some(Clip { x0, y0, x1, y1 })
        };
        // The reach in the group's coordinates, the texels to add there
        // and the pixels to add after the mapping.
        let (reach, texels, pixels) = match item {
            DisplayItem::PushClip { .. }
            | DisplayItem::PopClip
            | DisplayItem::PushGroup { .. }
            | DisplayItem::PopGroup => return None,
            DisplayItem::Rect { x, y, w, h, .. } => (Clip::new(*x, *y, *w, *h), 0.0, 0.0),
            DisplayItem::Text {
                text, x, y, size, ..
            } => {
                if !x.is_finite() || !y.is_finite() {
                    return (!clip.is_empty()).then_some(clip);
                }
                let ink = self.ink(text, *size)?;
                let reach = Clip {
                    x0: x + ink.left,
                    y0: y + ink.top,
                    x1: x + ink.right,
                    y1: y + ink.bottom,
                };
                (reach, 1.0, 1.0 + if scaled { 1.0 } else { 0.0 })
            }
            DisplayItem::Quad { points, .. } => match hull(points) {
                Some(reach) => (reach, 0.0, 1.0),
                // tiny-skia fills no such path; were it to, its pixels
                // would be in the clip.
                None => return (!clip.is_empty()).then_some(clip),
            },
            DisplayItem::Polygon { points, .. } => (hull(points)?, 0.0, 0.0),
        };
        let texels = texels + if in_scratch { 1.0 } else { 0.0 };
        let pixels = pixels + if in_scratch { 1.0 } else { 0.0 };
        let reach = grown(reach, texels);
        let mapped = state.map(reach.x0, reach.y0, reach.x1 - reach.x0, reach.y1 - reach.y0)?;
        let extent = grown(mapped, pixels).intersect(clip);
        (!extent.is_empty()).then_some(extent)
    }

    /// The pixels `item`, drawn from `state`, can change in a `width` by
    /// `height` frame: its [`Self::extent`] on the pixel grid; `None`
    /// when it changes none.
    fn bounds(
        &mut self,
        item: &DisplayItem,
        state: &State,
        width: u32,
        height: u32,
    ) -> Option<PixelRect> {
        let extent = self.extent(item, state)?;
        PixelRect::around(extent.x0, extent.y0, extent.x1, extent.y1, width, height)
    }

    /// What drawing `b` over the raster `a` drew can change, in a
    /// `width` by `height` raster whose coordinates are the lists'
    /// mapped by `base`, if given: the items of either list that
    /// [`align`] does not match with one of the other, each through the
    /// groups it is in (M19), can change the pixels they reach; and an
    /// item matched can, in both lists, while the clips and groups in
    /// force differ between the two where it is — a group's matrix or
    /// opacity having changed, say, under which the item draws
    /// elsewhere. A group begun before a change is one scratch drawn
    /// again whole, but what did not change in it comes out the same,
    /// so the items of it that are matched are not damage. The
    /// rectangles overlap as they may.
    fn damaged(
        &mut self,
        a: &[DisplayItem],
        b: &[DisplayItem],
        width: u32,
        height: u32,
        base: Option<Matrix>,
    ) -> Vec<PixelRect> {
        let state = || match base {
            Some(base) => State::based(width, height, base),
            None => State::new(width, height),
        };
        let (mut sa, mut sb) = (state(), state());
        let mut rects = Vec::new();
        let (mut i, mut j) = (0, 0);
        for (pi, pj) in align(a, b).into_iter().chain([(a.len(), b.len())]) {
            while i < pi {
                rects.extend(self.bounds(&a[i], &sa, width, height));
                sa.apply(&a[i]);
                i += 1;
            }
            while j < pj {
                rects.extend(self.bounds(&b[j], &sb, width, height));
                sb.apply(&b[j]);
                j += 1;
            }
            if pi == a.len() {
                break;
            }
            if sa != sb {
                rects.extend(self.bounds(&a[pi], &sa, width, height));
                rects.extend(self.bounds(&b[pj], &sb, width, height));
            }
            sa.apply(&a[pi]);
            sb.apply(&b[pj]);
            i += 1;
            j += 1;
        }
        rects
    }

    /// Draws `items` again inside `rect` of `pixmap`, which holds their
    /// last raster, in coordinates the lists' mapped by `base`, if
    /// given: the rectangle is cleared, then every item that can change
    /// a pixel in it is drawn again there (see the type's
    /// documentation), every clip and its end met whatever its place,
    /// so that the pixels inside come out as a whole raster's. A group
    /// composited from a layer or a scratch is composited again, inside
    /// the rectangle alone (M19).
    fn redraw(
        &mut self,
        pixmap: &mut tiny_skia::Pixmap,
        items: &[DisplayItem],
        rect: PixelRect,
        base: Option<Matrix>,
    ) {
        let (width, height) = (pixmap.width(), pixmap.height());
        let row = width as usize * 4;
        let (x0, x1) = (rect.x0 as usize * 4, rect.x1 as usize * 4);
        for y in rect.y0 as usize..rect.y1 as usize {
            pixmap.data_mut()[y * row + x0..y * row + x1].fill(0);
        }
        let inside = rect.clip();
        let mut state = match base {
            Some(base) => State::based(width, height, base),
            None => State::new(width, height),
        };
        let mut i = 0;
        while i < items.len() {
            let item = &items[i];
            if let DisplayItem::PushGroup {
                matrix,
                opacity,
                key,
                layer,
            } = item
                && (State::needs_scratch(matrix, *opacity) || self.is_held(key, *layer))
            {
                let end = group_end(items, i);
                let composed = state.matrix().map_or(*matrix, |m| m.then(matrix));
                let held = self.is_held(key, *layer).then_some(*key);
                self.composite(
                    pixmap,
                    &items[i + 1..end],
                    composed,
                    *opacity,
                    state.clip(),
                    Some(rect),
                    held,
                );
                i = end + 1;
                continue;
            }
            i += 1;
            let clip = state.clip();
            let through = state.through();
            let bounds = self.bounds(item, &state, width, height);
            state.apply(item);
            let Some(bounds) = bounds else {
                continue;
            };
            if bounds.intersect(rect).is_none() {
                continue;
            }
            if rect.contains(bounds) {
                self.draw(pixmap, item, clip, through);
                continue;
            }
            match item {
                DisplayItem::Rect { .. } | DisplayItem::Text { .. } => {
                    self.draw(pixmap, item, clip.intersect(inside), through);
                }
                DisplayItem::Polygon { points, color } => {
                    let (sx, sy, tx, ty) = through;
                    let points: Vec<(f32, f32)> = points
                        .iter()
                        .map(|(x, y)| (x * sx + tx, y * sy + ty))
                        .collect();
                    fill_polygon_by_area(pixmap, &points, color, clip, rect);
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
                    self.draw(&mut scratch, item, clip, through);
                    if let Some(back) = bounds.intersect(rect) {
                        copy_pixels(&scratch, pixmap, back);
                    }
                    self.scratch = Some(scratch);
                }
                DisplayItem::PushClip { .. }
                | DisplayItem::PopClip
                | DisplayItem::PushGroup { .. }
                | DisplayItem::PopGroup => {}
            }
        }
    }
}

/// The mask of the glyph `glyph_id` of `font` at `size`, its outline
/// filled through the scale `(sx, sy)` with the pen at a whole pixel
/// (M19): coverage from tiny-skia's anti-aliased fill of the outline's
/// contours, placed like a mask [`rasterize_glyph`] makes. Empty for a
/// glyph without an outline, or one larger than [`MAX_MASK_PIXELS`].
fn outline_mask(
    font: &ab_glyph::FontArc,
    glyph_id: u16,
    size: f32,
    (sx, sy): (f32, f32),
) -> GlyphMask {
    let Some(curves) = glyph_outline(font, glyph_id, size) else {
        return GlyphMask::empty();
    };
    let mut pb = tiny_skia::PathBuilder::new();
    let mut at: Option<(f32, f32)> = None;
    for curve in curves {
        let (start, end) = match curve {
            Curve::Line(a, b) => (a, b),
            Curve::Quad(a, _, c) => (a, c),
            Curve::Cubic(a, _, _, d) => (a, d),
        };
        if at != Some(start) {
            if at.is_some() {
                pb.close();
            }
            pb.move_to(start.0 * sx, start.1 * sy);
        }
        match curve {
            Curve::Line(_, b) => pb.line_to(b.0 * sx, b.1 * sy),
            Curve::Quad(_, b, c) => pb.quad_to(b.0 * sx, b.1 * sy, c.0 * sx, c.1 * sy),
            Curve::Cubic(_, b, c, d) => {
                pb.cubic_to(b.0 * sx, b.1 * sy, c.0 * sx, c.1 * sy, d.0 * sx, d.1 * sy)
            }
        }
        at = Some(end);
    }
    if at.is_some() {
        pb.close();
    }
    let Some(path) = pb.finish() else {
        return GlyphMask::empty();
    };
    let bounds = path.bounds();
    let (left, top) = (bounds.left().floor() as f64, bounds.top().floor() as f64);
    let (right, bottom) = (bounds.right().ceil() as f64, bounds.bottom().ceil() as f64);
    let (width, height) = ((right - left).max(0.0), (bottom - top).max(0.0));
    if !(left.is_finite() && top.is_finite() && width >= 1.0 && height >= 1.0)
        || width * height > MAX_MASK_PIXELS as f64
    {
        return GlyphMask::empty();
    }
    let (Ok(left), Ok(top)) = (i32::try_from(left as i64), i32::try_from(top as i64)) else {
        return GlyphMask::empty();
    };
    let (width, height) = (width as u32, height as u32);
    let Some(mut pixmap) = tiny_skia::Pixmap::new(width, height) else {
        return GlyphMask::empty();
    };
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::WHITE);
    paint.anti_alias = true;
    pixmap.fill_path(
        &path,
        &paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::from_translate(-(left as f32), -(top as f32)),
        None,
    );
    GlyphMask {
        width,
        height,
        left,
        top,
        coverage: pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|px| px[3])
            .collect(),
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

/// The corners of a pixmap that holds all of `reach` with a clear
/// texel all round, `(x0, y0, x1, y1)` on whole texels: the resampling
/// then blends the content's edges with nothing rather than with the
/// edge texel again, which padding the pixmap with would give, and the
/// edges come out anti-aliased.
fn whole_reach(reach: Clip) -> (f32, f32, f32, f32) {
    (
        reach.x0.floor() - 1.0,
        reach.y0.floor() - 1.0,
        reach.x1.ceil() + 1.0,
        reach.y1.ceil() + 1.0,
    )
}

/// Whether a pixmap with these corners has no more pixels than a frame
/// may.
fn fits_a_frame((x0, y0, x1, y1): (f32, f32, f32, f32)) -> bool {
    [x0, y0, x1, y1].iter().all(|v| v.is_finite())
        && (x1 - x0) as f64 * (y1 - y0) as f64 <= MAX_FRAME_PIXELS as f64
}

/// The bounds of `reach` through `matrix`, two pixels more all round
/// for what a resampling adds; `None` when it has no image.
fn outward_clip(matrix: &Matrix, reach: Clip) -> Option<Clip> {
    let (x0, y0, x1, y1) =
        matrix.bounds(reach.x0, reach.y0, reach.x1 - reach.x0, reach.y1 - reach.y0)?;
    Some(Clip {
        x0: x0 - 2.0,
        y0: y0 - 2.0,
        x1: x1 + 2.0,
        y1: y1 + 2.0,
    })
}

/// The pairs `(i, j)` with `a[i] == b[j]` matched in order, as many as
/// can be: the lists' shared start and end, and between them a longest
/// common subsequence (Myers, 1986). The items of either list left out
/// are what changed between them. Lists more than [`MAX_DIFF_EDITS`]
/// edits apart match their shared start and end alone.
fn align(a: &[DisplayItem], b: &[DisplayItem]) -> Vec<(usize, usize)> {
    let start = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let most = a.len().min(b.len()) - start;
    let end = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(most)
        .take_while(|(x, y)| x == y)
        .count();
    let mut pairs: Vec<(usize, usize)> = (0..start).map(|k| (k, k)).collect();
    let (x, y) = (&a[start..a.len() - end], &b[start..b.len() - end]);
    if !x.is_empty() && !y.is_empty() {
        pairs.extend(
            myers(x, y)
                .unwrap_or_default()
                .into_iter()
                .map(|(i, j)| (i + start, j + start)),
        );
    }
    pairs.extend((0..end).map(|k| (a.len() - end + k, b.len() - end + k)));
    pairs
}

/// Myers's shortest edit script between `x` and `y`, as the pairs it
/// matches, in order; `None` past [`MAX_DIFF_EDITS`] edits.
fn myers(x: &[DisplayItem], y: &[DisplayItem]) -> Option<Vec<(usize, usize)>> {
    let (n, m) = (x.len() as isize, y.len() as isize);
    let max = (n + m).min(MAX_DIFF_EDITS as isize);
    // `v[k]` is the furthest `x` reached on the diagonal `k = x - y`,
    // kept for `k` from `-max - 1` to `max + 1`; its state before each
    // step `d` is kept for the walk back, the diagonals `-d - 1` to
    // `d + 1` of it.
    let at = |k: isize| (k + max + 1) as usize;
    let mut v = vec![0isize; (2 * max + 3) as usize];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut shortest = None;
    'steps: for d in 0..=max {
        trace.push(v[at(-d - 1)..=at(d + 1)].to_vec());
        for k in (-d..=d).step_by(2) {
            let mut px = if k == -d || (k != d && v[at(k - 1)] < v[at(k + 1)]) {
                v[at(k + 1)]
            } else {
                v[at(k - 1)] + 1
            };
            let mut py = px - k;
            while px < n && py < m && x[px as usize] == y[py as usize] {
                px += 1;
                py += 1;
            }
            v[at(k)] = px;
            if px >= n && py >= m {
                shortest = Some(d);
                break 'steps;
            }
        }
    }
    let shortest = shortest?;
    let mut pairs = Vec::new();
    let (mut px, mut py) = (n, m);
    for d in (0..=shortest).rev() {
        let v = &trace[d as usize];
        let vk = |k: isize| v[(k + d + 1) as usize];
        let k = px - py;
        let prev_k = if k == -d || (k != d && vk(k - 1) < vk(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = vk(prev_k);
        let prev_y = prev_x - prev_k;
        while px > prev_x && py > prev_y {
            px -= 1;
            py -= 1;
            pairs.push((px as usize, py as usize));
        }
        if d > 0 {
            px = prev_x;
            py = prev_y;
        }
    }
    pairs.reverse();
    Some(pairs)
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
    fn rasterize(&mut self, list: &DisplayList, width: u32, height: u32) -> &[u8] {
        self.rasterize_by(list, width, height, None)
    }

    fn rasterize_by(
        &mut self,
        list: &DisplayList,
        width: u32,
        height: u32,
        deadline: Option<Instant>,
    ) -> &[u8] {
        self.redrawn = Redrawn::Whole;
        self.frame += 1;
        self.layered = Layered::default();
        self.deferred.clear();
        self.doubled.clear();
        let last = self.retained.take();
        if width as u64 * height as u64 > MAX_FRAME_PIXELS {
            return &[];
        }
        // The layers first (M19): this frame's are marked, so that the
        // budget never lets one go for another, then each is drawn
        // where its items changed, and the groups whose layer came or
        // went are damage.
        let base = State::new(width, height);
        self.mark_layers(&list.items, &base);
        let mut seen = HashSet::new();
        let mut transitions = Vec::new();
        self.update_layers(&list.items, &base, &base, &mut seen, &mut transitions);
        let transitions: Vec<PixelRect> = transitions
            .into_iter()
            .filter_map(|r| PixelRect::around(r.x0, r.y0, r.x1, r.y1, width, height))
            .collect();
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
                    let mut rects = self.damaged(&old.items, &list.items, width, height, None);
                    rects.extend(transitions);
                    let rects = merge(rects);
                    let area: u64 = rects.iter().map(|r| r.area()).sum();
                    if area * 2 >= width as u64 * height as u64 {
                        return None;
                    }
                    for &rect in &rects {
                        self.redraw(&mut pixmap, &list.items, rect, None);
                    }
                    self.redrawn = Redrawn::Rects(rects);
                    Some(pixmap)
                },
            );
        let pixmap = match redrawn {
            Some(pixmap) => pixmap,
            None => {
                let Some(mut pixmap) = tiny_skia::Pixmap::new(width, height) else {
                    return &[];
                };
                self.draw_all(&mut pixmap, list);
                pixmap
            }
        };
        self.prepaint(deadline);
        self.trim();
        let retained = self.retained.insert(Retained {
            list: list.clone(),
            pixmap,
        });
        retained.pixmap.data()
    }

    fn set_layer_budget(&mut self, bytes: usize) {
        self.layer_budget = bytes;
    }

    fn layer_bytes(&self) -> usize {
        self.layer_bytes
    }

    fn layers(&self) -> Vec<LayerInfo> {
        let mut layers: Vec<LayerInfo> = self
            .layers
            .iter()
            .map(|(key, layer)| LayerInfo {
                key: *key,
                reason: layer.reason,
                bytes: layer.bytes(),
                tiles: layer.tiles(),
            })
            .collect();
        layers.sort_by_key(|layer| format!("{:?}", layer.key));
        layers
    }

    fn layered(&self) -> Layered {
        self.layered.clone()
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
            blend(&mut data[idx..idx + 4], color, a);
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

/// Composites the texels of `src`, a scratch or the tiles of a layer in
/// the group's coordinates, into the pixels of `region` of `dst`,
/// through `inverse`, from the raster's coordinates to the group's
/// (M19): each pixel's centre is mapped back, and where it lands within
/// the source's reach, the source is sampled bilinearly between texel
/// centres, the texels beyond the reach standing in as the source
/// says, and the sample blended over the pixel at `opacity`,
/// premultiplied. The mapping is of the raster's point alone, the
/// whole-texel origin of the source taken off after, so that the same
/// pixel samples the same place whatever the origin, and a region
/// composited on its own comes out as the whole does.
fn resample(
    dst: &mut tiny_skia::Pixmap,
    src: &impl Texels,
    inverse: &Matrix,
    opacity: f32,
    region: PixelRect,
) {
    let (rx0, ry0, rx1, ry1) = src.reach();
    let stride = dst.width() as usize;
    let data = dst.data_mut();
    for y in region.y0..region.y1 {
        for x in region.x0..region.x1 {
            let Some((qx, qy)) = inverse.project(x as f32 + 0.5, y as f32 + 0.5) else {
                continue;
            };
            if !(qx >= rx0 && qy >= ry0 && qx < rx1 && qy < ry1) {
                continue;
            }
            let (sx, sy) = (qx - 0.5, qy - 0.5);
            let (fx, fy) = (sx.floor(), sy.floor());
            let (wx, wy) = (sx - fx, sy - fy);
            let (ix, iy) = (fx as i64, fy as i64);
            let mut c = [0.0f32; 4];
            for (dx, dy, weight) in [
                (0, 0, (1.0 - wx) * (1.0 - wy)),
                (1, 0, wx * (1.0 - wy)),
                (0, 1, (1.0 - wx) * wy),
                (1, 1, wx * wy),
            ] {
                let t = src.texel(ix + dx, iy + dy);
                for (k, channel) in c.iter_mut().enumerate() {
                    *channel += t[k] * weight;
                }
            }
            let i = (y as usize * stride + x as usize) * 4;
            over(&mut data[i..i + 4], c, opacity);
        }
    }
}

/// Lays the premultiplied colour `src`, at `opacity`, over the
/// premultiplied pixel `px`: source-over on every channel, rounded.
fn over(px: &mut [u8], src: [f32; 4], opacity: f32) {
    let a = src[3] * opacity / 255.0;
    for (k, channel) in src.iter().enumerate() {
        px[k] = (channel * opacity + px[k] as f32 * (1.0 - a)).round() as u8;
    }
}

/// A source of texels for [`resample`]: where it reaches in its own
/// coordinates, as `(x0, y0, x1, y1)`, outside which nothing is drawn,
/// and the texel at a point, which stands in beyond its edges.
trait Texels {
    fn reach(&self) -> (f32, f32, f32, f32);
    fn texel(&self, x: i64, y: i64) -> [f32; 4];
}

/// A pixmap whose top-left texel is at `origin`, a whole texel: its
/// edge texels stand in beyond its edges.
struct Placed<'a> {
    pixmap: &'a tiny_skia::Pixmap,
    origin: (f32, f32),
}

impl Texels for Placed<'_> {
    fn reach(&self) -> (f32, f32, f32, f32) {
        let (ox, oy) = self.origin;
        (
            ox,
            oy,
            ox + self.pixmap.width() as f32,
            oy + self.pixmap.height() as f32,
        )
    }

    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        let (sw, sh) = (self.pixmap.width() as i64, self.pixmap.height() as i64);
        let tx = (x - self.origin.0 as i64).clamp(0, sw - 1) as usize;
        let ty = (y - self.origin.1 as i64).clamp(0, sh - 1) as usize;
        texel_of(self.pixmap, tx, ty)
    }
}

/// The tiles of a layer (M19.4): `width` texels across from `left`, and
/// the rows held, from the first to the last; a row not held is clear,
/// and the texels at the sides stand in beyond them.
struct Tiled<'a> {
    rows: &'a BTreeMap<i32, Tile>,
    left: i32,
    width: u32,
}

impl Texels for Tiled<'_> {
    fn reach(&self) -> (f32, f32, f32, f32) {
        let tall = TILE_HEIGHT as f32;
        let first = self.rows.keys().next().copied().unwrap_or(0) as f32;
        let last = self.rows.keys().next_back().copied().unwrap_or(-1) as f32;
        (
            self.left as f32,
            first * tall,
            self.left as f32 + self.width as f32,
            (last + 1.0) * tall,
        )
    }

    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        let tall = TILE_HEIGHT as i64;
        let tx = (x - self.left as i64).clamp(0, self.width as i64 - 1) as usize;
        let row = y.div_euclid(tall);
        let Some(tile) = i32::try_from(row).ok().and_then(|row| self.rows.get(&row)) else {
            return [0.0; 4];
        };
        texel_of(&tile.pixmap, tx, (y - row * tall) as usize)
    }
}

/// The texel at column `x` of row `y` of `pixmap`, as floats.
fn texel_of(pixmap: &tiny_skia::Pixmap, x: usize, y: usize) -> [f32; 4] {
    let i = (y * pixmap.width() as usize + x) * 4;
    let source = pixmap.data();
    [
        source[i] as f32,
        source[i + 1] as f32,
        source[i + 2] as f32,
        source[i + 3] as f32,
    ]
}

/// Draws `src`, whose top-left texel is at `origin` in its own
/// coordinates, which `matrix` maps to `dst`'s and `inverse` back, over
/// `dst` at `opacity`, writing the pixels of `within` alone: a copy of
/// its pixels, moved, when the matrix is a translation by whole pixels
/// and the origin is whole, else each pixel sampled as [`resample`]
/// does. Whether any pixel was reached.
fn composite_pixmap(
    dst: &mut tiny_skia::Pixmap,
    src: &tiny_skia::Pixmap,
    origin: (f32, f32),
    matrix: &Matrix,
    inverse: &Matrix,
    opacity: f32,
    within: PixelRect,
) -> bool {
    let (sw, sh) = (src.width() as i64, src.height() as i64);
    if let Some((dx, dy)) = whole_pixel_translation(matrix)
        && origin.0.fract() == 0.0
        && origin.1.fract() == 0.0
    {
        let at = (origin.0 as i64 + dx, origin.1 as i64 + dy);
        let Some(region) =
            PixelRect::new(at.0, at.1, at.0 + sw, at.1 + sh).and_then(|r| r.intersect(within))
        else {
            return false;
        };
        blit(dst, src, at, opacity, region);
        return true;
    }
    let Some((fx0, fy0, fx1, fy1)) = matrix.bounds(origin.0, origin.1, sw as f32, sh as f32) else {
        return false;
    };
    let Some(landed) = PixelRect::around(
        fx0 - 1.0,
        fy0 - 1.0,
        fx1 + 1.0,
        fy1 + 1.0,
        dst.width(),
        dst.height(),
    )
    .and_then(|r| r.intersect(within)) else {
        return false;
    };
    resample(
        dst,
        &Placed {
            pixmap: src,
            origin,
        },
        inverse,
        opacity,
        landed,
    );
    true
}

/// Lays `src`, its top-left pixel at `at` in `dst`'s coordinates, over
/// `dst` at `opacity`, pixel for pixel, writing the pixels of `region`
/// alone, which is inside both: an opaque pixel at full opacity is
/// copied, any other laid over as [`over`] does.
fn blit(
    dst: &mut tiny_skia::Pixmap,
    src: &tiny_skia::Pixmap,
    at: (i64, i64),
    opacity: f32,
    region: PixelRect,
) {
    let (dw, sw) = (dst.width() as usize, src.width() as usize);
    let source = src.data();
    let data = dst.data_mut();
    for y in region.y0..region.y1 {
        let sy = (y as i64 - at.1) as usize;
        for x in region.x0..region.x1 {
            let sx = (x as i64 - at.0) as usize;
            let s = (sy * sw + sx) * 4;
            let d = (y as usize * dw + x as usize) * 4;
            if opacity >= 1.0 && source[s + 3] == 255 {
                data[d..d + 4].copy_from_slice(&source[s..s + 4]);
            } else {
                let src = [
                    source[s] as f32,
                    source[s + 1] as f32,
                    source[s + 2] as f32,
                    source[s + 3] as f32,
                ];
                over(&mut data[d..d + 4], src, opacity);
            }
        }
    }
}

/// Blends `color` at the coverage `a`, in `[0, 1]`, over the
/// premultiplied pixel `px`: source-over on every channel, so that a
/// pixel of a clear scratch (M19) comes out premultiplied as a pixmap's
/// must be, and an opaque frame's pixel is the straight blend it was.
fn blend(px: &mut [u8], color: &Rgba8, a: f32) {
    let over = |dst: u8, src: u8| (dst as f32 * (1.0 - a) + src as f32 * a).round() as u8;
    px[0] = over(px[0], color.r);
    px[1] = over(px[1], color.g);
    px[2] = over(px[2], color.b);
    px[3] = over(px[3], 255);
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
            blend(&mut data[idx..idx + 4], color, a);
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
                    px(buf, 10, x, y),
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
                    px(buf, 10, x, y),
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
        let unclipped = backend
            .rasterize(
                &DisplayList {
                    items: vec![white_frame(10.0, 10.0), text.clone()],
                },
                10,
                10,
            )
            .to_vec();
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
                    assert_eq!(px(clipped, 10, x, y), px(&unclipped, 10, x, y));
                    inked_inside += (px(clipped, 10, x, y) != WHITE) as u32;
                } else {
                    assert_eq!(px(clipped, 10, x, y), WHITE, "({x}, {y}) outside the clip");
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
        let first = backend.rasterize(&list, 40, 20).to_vec();
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
                let p = px(buf, 10, x, y);
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
        assert_eq!(px(buf, 10, 8, 2), BLACK, "well inside");
        assert_eq!(px(buf, 10, 2, 8), WHITE, "well outside");
        assert_eq!(
            px(buf, 10, 9, 0),
            BLACK,
            "the edge on whole pixels is exact"
        );
        for (x, y) in [(5, 5), (2, 2), (8, 8)] {
            assert_eq!(
                px(buf, 10, x, y),
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
            assert_eq!(px(buf, 10, x, y), [value, value, value, 0xff], "({x}, {y})");
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
                assert_eq!(px(buf, 10, x, y), BLACK, "({x}, {y})");
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
                    px(buf, 10, x, y),
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

    /// A key for a group in a test: every group the same one.
    fn group_key() -> GroupKey {
        let document = dom::Document::from_node(&dom::Node::document(Vec::new()));
        GroupKey::Box((document.root(), None))
    }

    /// A key no other group in any test has.
    fn fresh_key() -> GroupKey {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        GroupKey::Document(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    /// A random item for a `w` by `h` frame, of every kind, reaching
    /// past the frame's edges now and then: the groups (M19) translate,
    /// scale, rotate or fade, and nest or go unmatched as the clips do.
    fn random_item(rng: &mut Rng, w: f32, h: f32) -> DisplayItem {
        let (x, y) = (rng.coord(-8.0, w), rng.coord(-8.0, h));
        match rng.below(13) {
            10 => {
                let (dx, dy) = (rng.coord(-20.0, 20.0), rng.coord(-20.0, 20.0));
                let (matrix, opacity) = match rng.below(5) {
                    0 => (Matrix::translate(dx, dy, 0.0), 1.0),
                    1 => (
                        Matrix::translate(dx, dy, 0.0).then(&Matrix::scale(
                            rng.coord(0.5, 2.5),
                            rng.coord(0.5, 2.5),
                            1.0,
                        )),
                        1.0,
                    ),
                    2 => (
                        Matrix::translate(x, y, 0.0)
                            .then(&Matrix::rotate_z(rng.coord(-180.0, 180.0)))
                            .then(&Matrix::translate(-x, -y, 0.0)),
                        1.0,
                    ),
                    3 => (Matrix::translate(dx, dy, 0.0), rng.coord(0.2, 0.95)),
                    _ => (
                        Matrix::translate(x, y, 0.0)
                            .then(&Matrix::rotate_z(rng.coord(-180.0, 180.0)))
                            .then(&Matrix::scale(rng.coord(0.5, 2.0), 1.0, 1.0))
                            .then(&Matrix::translate(-x, -y, 0.0)),
                        rng.coord(0.2, 0.95),
                    ),
                };
                // A layer now and then (M19), of every reason, under a
                // key the test's lists share, so that one list's layer
                // serves the next.
                let layer = [
                    None,
                    None,
                    Some(LayerReason::Transformed),
                    Some(LayerReason::WillChange),
                    Some(LayerReason::Scrolled),
                ][rng.below(5)];
                DisplayItem::PushGroup {
                    matrix,
                    opacity,
                    key: GroupKey::Document(rng.below(6) as u32),
                    layer,
                }
            }
            11..=12 => DisplayItem::PopGroup,
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
            // The layers are brought to `list` again after the raster
            // of `before`, as a raster of `list` would bring them, so
            // that the redraw composites them as the whole raster did:
            // a group held as a layer and one drawn otherwise differ
            // where the layer is resampled; and a layer drawn again
            // where its items changed must come out as one drawn anew.
            backend.forget_frame();
            let whole = backend.rasterize(&list, w, h).to_vec();
            backend.forget_last_frame();
            let old = backend.rasterize(&before, w, h).to_vec();
            let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("a frame");
            pixmap.data_mut().copy_from_slice(&old);
            backend.frame += 1;
            backend.doubled.clear();
            let base = State::new(w, h);
            backend.mark_layers(&list.items, &base);
            let (mut seen, mut extra) = (HashSet::new(), Vec::new());
            backend.update_layers(&list.items, &base, &base, &mut seen, &mut extra);
            backend.redraw(&mut pixmap, &list.items, rect, None);
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
            backend.draw(&mut whole, &item, clip, (1.0, 1.0, 0.0, 0.0));
            backend.draw(
                &mut clipped,
                &item,
                clip.intersect(rect.clip()),
                (1.0, 1.0, 0.0, 0.0),
            );
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
        backend.redraw(&mut pixmap, &list.items, rect, None);
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
        let first = backend.rasterize(&list, 20, 20).to_vec();
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
        // it is the same item in both, drawn under either clip: the
        // damage is what it draws under each, the wider clip's 9 by 5.
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
                y1: 5
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

    /// The pixels of `list` rasterized whole on a `w` by `h` frame.
    fn raster(list: &DisplayList, w: u32, h: u32) -> Vec<u8> {
        test_backend().rasterize(list, w, h).to_vec()
    }

    fn rect(x: f32, y: f32, w: f32, h: f32, color: Rgba8) -> DisplayItem {
        DisplayItem::Rect { x, y, w, h, color }
    }

    fn group(matrix: Matrix, opacity: f32) -> DisplayItem {
        DisplayItem::PushGroup {
            matrix,
            opacity,
            key: group_key(),
            layer: None,
        }
    }

    /// The pixels where two frames differ, as `(x, y, a, b)`.
    fn differing(a: &[u8], b: &[u8], w: usize, h: usize) -> Vec<(usize, usize, [u8; 4], [u8; 4])> {
        let mut out = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if px(a, w, x, y) != px(b, w, x, y) {
                    out.push((x, y, px(a, w, x, y), px(b, w, x, y)));
                }
            }
        }
        out
    }

    #[test]
    fn a_group_drawn_through_gives_the_pixels_of_a_raster_of_its_items_there() {
        // M19: a group that only translates and scales is drawn into the
        // frame, its items mapped, so that a box moved or scaled matches
        // a box laid out there, byte for byte.
        let (w, h) = (64u32, 48u32);
        let red = Rgba8::rgb(255, 0, 0);
        let list = |items: Vec<DisplayItem>| {
            let mut all = vec![white_frame(w as f32, h as f32)];
            all.extend(items);
            DisplayList { items: all }
        };
        // Moved by (10, 6), fractional edge and all.
        let moved = list(vec![
            group(Matrix::translate(10.0, 6.0, 0.0), 1.0),
            rect(5.5, 4.0, 20.0, 10.0, red),
            DisplayItem::PopGroup,
        ]);
        let direct = list(vec![rect(15.5, 10.0, 20.0, 10.0, red)]);
        assert_eq!(raster(&moved, w, h), raster(&direct, w, h));
        // Scaled by 2 about the origin.
        let scaled = list(vec![
            group(Matrix::scale(2.0, 2.0, 1.0), 1.0),
            rect(5.0, 4.0, 10.0, 5.0, red),
            DisplayItem::PopGroup,
        ]);
        let direct = list(vec![rect(10.0, 8.0, 20.0, 10.0, red)]);
        assert_eq!(raster(&scaled, w, h), raster(&direct, w, h));
        // A clip inside the group is mapped with it.
        let clipped = list(vec![
            group(Matrix::scale(2.0, 2.0, 1.0), 1.0),
            DisplayItem::PushClip {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 5.0,
            },
            rect(0.0, 0.0, 30.0, 30.0, red),
            DisplayItem::PopClip,
            DisplayItem::PopGroup,
        ]);
        let direct = list(vec![rect(0.0, 0.0, 20.0, 10.0, red)]);
        assert_eq!(raster(&clipped, w, h), raster(&direct, w, h));
        // Groups nest, the matrices composing from the inside out.
        let nested = list(vec![
            group(Matrix::translate(10.0, 0.0, 0.0), 1.0),
            group(Matrix::scale(2.0, 2.0, 1.0), 1.0),
            rect(1.0, 1.0, 4.0, 4.0, red),
            DisplayItem::PopGroup,
            DisplayItem::PopGroup,
        ]);
        let direct = list(vec![rect(12.0, 2.0, 8.0, 8.0, red)]);
        assert_eq!(raster(&nested, w, h), raster(&direct, w, h));
        // Text in a group scaled by 2 is rasterized at twice its size:
        // the glyph of 32px text, placed from the mapped origin.
        let text = |x: f32, y: f32, size: f32| DisplayItem::Text {
            text: "M".to_string(),
            x,
            y,
            size,
            color: Rgba8::rgb(0, 0, 0),
        };
        let scaled = list(vec![
            group(Matrix::scale(2.0, 2.0, 1.0), 1.0),
            text(4.0, 20.0, 16.0),
            DisplayItem::PopGroup,
        ]);
        let direct = list(vec![text(8.0, 40.0, 32.0)]);
        let (a, b) = (raster(&scaled, w, h), raster(&direct, w, h));
        assert_eq!(differing(&a, &b, w as usize, h as usize), []);
        assert_ne!(a, raster(&list(vec![]), w, h), "the glyph is drawn");
    }

    #[test]
    fn an_opacity_group_is_composited_as_one() {
        // css-color-4 §3.2 (M19): the group's items do not show through
        // one another; the whole is blended at the opacity.
        let (w, h) = (48u32, 32u32);
        let list = DisplayList {
            items: vec![
                white_frame(w as f32, h as f32),
                group(Matrix::IDENTITY, 0.5),
                rect(10.0, 10.0, 20.0, 15.0, Rgba8::rgb(255, 0, 0)),
                rect(20.0, 10.0, 20.0, 15.0, Rgba8::rgb(0, 0, 255)),
                DisplayItem::PopGroup,
            ],
        };
        let pixels = raster(&list, w, h);
        let near = |got: [u8; 4], want: [u8; 4]| {
            got.iter()
                .zip(want.iter())
                .all(|(g, w)| (*g as i32 - *w as i32).abs() <= 1)
        };
        let at = |x, y| px(&pixels, w as usize, x, y);
        assert!(near(at(15, 15), [255, 128, 128, 255]), "{:?}", at(15, 15));
        assert!(
            near(at(25, 15), [128, 128, 255, 255]),
            "where the two overlap, the later one alone at half: {:?}",
            at(25, 15)
        );
        assert_eq!(at(5, 5), [255, 255, 255, 255]);
    }

    #[test]
    fn a_rotated_group_is_composited_from_a_scratch_and_a_right_angle_is_exact() {
        // M19: a 20 by 10 box at (20, 10) turned a quarter about its
        // centre (30, 15) is the 10 by 20 box at (25, 5), pixel for
        // pixel, the resampling landing on whole pixels.
        let (w, h) = (64u32, 48u32);
        let red = Rgba8::rgb(255, 0, 0);
        let about = Matrix::translate(30.0, 15.0, 0.0)
            .then(&Matrix::rotate_z(90.0))
            .then(&Matrix::translate(-30.0, -15.0, 0.0));
        let rotated = DisplayList {
            items: vec![
                white_frame(w as f32, h as f32),
                group(about, 1.0),
                rect(20.0, 10.0, 20.0, 10.0, red),
                DisplayItem::PopGroup,
            ],
        };
        let direct = DisplayList {
            items: vec![
                white_frame(w as f32, h as f32),
                rect(25.0, 5.0, 10.0, 20.0, red),
            ],
        };
        let (a, b) = (raster(&rotated, w, h), raster(&direct, w, h));
        assert_eq!(differing(&a, &b, w as usize, h as usize), []);
        // Turned by 45 degrees, the box's corners are where the matrix
        // puts them: its centre stays red, the old corners are white.
        let about = Matrix::translate(30.0, 15.0, 0.0)
            .then(&Matrix::rotate_z(45.0))
            .then(&Matrix::translate(-30.0, -15.0, 0.0));
        let turned = DisplayList {
            items: vec![
                white_frame(w as f32, h as f32),
                group(about, 1.0),
                rect(20.0, 10.0, 20.0, 10.0, red),
                DisplayItem::PopGroup,
            ],
        };
        let pixels = raster(&turned, w, h);
        assert_eq!(px(&pixels, w as usize, 30, 15), [255, 0, 0, 255]);
        assert_eq!(px(&pixels, w as usize, 30, 20), [255, 0, 0, 255]);
        assert_eq!(px(&pixels, w as usize, 38, 11), [255, 255, 255, 255]);
        assert_eq!(px(&pixels, w as usize, 22, 19), [255, 255, 255, 255]);
    }

    /// A reference for a group composited from a scratch: `items`
    /// rasterized in their own coordinates on a clear pixmap at
    /// `(ox, oy)`, then each pixel of a `w` by `h` frame over `under`
    /// taken from the point `matrix` maps to its centre, sampled
    /// bilinearly between texel centres, clamped to the scratch, and
    /// blended over at `opacity`; `None` where the frame is left alone.
    fn reference_composite(
        items: &[DisplayItem],
        (ox, oy, sw, sh): (f32, f32, u32, u32),
        matrix: Matrix,
        opacity: f32,
        under: &[u8],
        w: u32,
        h: u32,
    ) -> Vec<u8> {
        let mut backend = test_backend();
        let mut scratch = tiny_skia::Pixmap::new(sw, sh).unwrap();
        let base = Matrix::translate(-ox, -oy, 0.0);
        backend.draw_items(&mut scratch, items, State::based(sw, sh, base));
        let inverse = matrix.inverse().unwrap();
        let texel = |x: i64, y: i64| -> [f32; 4] {
            let x = x.clamp(0, sw as i64 - 1) as usize;
            let y = y.clamp(0, sh as i64 - 1) as usize;
            let p = px(scratch.data(), sw as usize, x, y);
            p.map(|c| c as f32 / 255.0)
        };
        let mut out = under.to_vec();
        for y in 0..h as usize {
            for x in 0..w as usize {
                let (gx, gy) = inverse.project(x as f32 + 0.5, y as f32 + 0.5).unwrap();
                // Inside the scratch's rectangle, as the fill of it is.
                let (lx, ly) = (gx - ox, gy - oy);
                if lx < 0.0 || ly < 0.0 || lx >= sw as f32 || ly >= sh as f32 {
                    continue;
                }
                let (sx, sy) = (lx - 0.5, ly - 0.5);
                let (x0, y0) = (sx.floor(), sy.floor());
                let (fx, fy) = (sx - x0, sy - y0);
                let (x0, y0) = (x0 as i64, y0 as i64);
                let mut c = [0.0f32; 4];
                for (dx, dy, wgt) in [
                    (0, 0, (1.0 - fx) * (1.0 - fy)),
                    (1, 0, fx * (1.0 - fy)),
                    (0, 1, (1.0 - fx) * fy),
                    (1, 1, fx * fy),
                ] {
                    let t = texel(x0 + dx, y0 + dy);
                    for k in 0..4 {
                        c[k] += t[k] * wgt;
                    }
                }
                let i = (y * w as usize + x) * 4;
                for k in 0..4 {
                    let src = c[k] * opacity;
                    let dst = out[i + k] as f32 / 255.0;
                    out[i + k] = ((src + dst * (1.0 - c[3] * opacity)) * 255.0).round() as u8;
                }
            }
        }
        out
    }

    #[test]
    fn a_scratch_composite_samples_as_the_reference_does() {
        // The backend's composite of a turned group against a reference
        // written out in full: where they differ says how tiny-skia
        // samples. A translucent box with fractional edges, turned by
        // 30 degrees, over a white frame.
        let (w, h) = (64u32, 48u32);
        let color = Rgba8 {
            r: 122,
            g: 238,
            b: 153,
            a: 200,
        };
        let about = Matrix::translate(30.0, 20.0, 0.0)
            .then(&Matrix::rotate_z(30.0))
            .then(&Matrix::translate(-30.0, -20.0, 0.0));
        let items = vec![rect(21.4, 14.8, 18.6, 11.2, color)];
        let list = DisplayList {
            items: vec![
                white_frame(w as f32, h as f32),
                group(about, 1.0),
                items[0].clone(),
                DisplayItem::PopGroup,
            ],
        };
        let got = raster(&list, w, h);
        let white = raster(
            &DisplayList {
                items: vec![white_frame(w as f32, h as f32)],
            },
            w,
            h,
        );
        let want = reference_composite(&items, (20.0, 13.0, 22, 15), about, 1.0, &white, w, h);
        let diff = differing(&got, &want, w as usize, h as usize);
        let big: Vec<&(usize, usize, [u8; 4], [u8; 4])> = diff
            .iter()
            .filter(|(_, _, a, b)| {
                a.iter()
                    .zip(b)
                    .any(|(x, y)| (*x as i32 - *y as i32).abs() > 2)
            })
            .collect();
        assert!(
            big.is_empty(),
            "{big:?}; {} pixels differ in all",
            diff.len()
        );
    }

    #[test]
    fn a_layer_is_drawn_once_and_composited_after_wherever_its_matrix_puts_it() {
        // M19: a group that is a layer is rasterized into its layer the
        // first time and composited from it after; a new matrix or
        // opacity draws nothing into it, and the frame is a fresh
        // raster's, whether the matrix moves it by whole pixels, by a
        // fraction, or turns it.
        let (w, h) = (48u32, 40u32);
        let mut backend = test_backend();
        let key = fresh_key();
        let list = |matrix: Matrix, opacity: f32| DisplayList {
            items: vec![
                white_frame(w as f32, h as f32),
                DisplayItem::PushGroup {
                    matrix,
                    opacity,
                    key,
                    layer: Some(LayerReason::WillChange),
                },
                rect(4.0, 4.0, 10.0, 10.0, Rgba8::rgb(0, 0, 255)),
                DisplayItem::Text {
                    text: "Ag".to_string(),
                    x: 4.0,
                    y: 30.0,
                    size: 12.0,
                    color: Rgba8::rgb(0, 0, 0),
                },
                DisplayItem::PopGroup,
            ],
        };
        let first = list(Matrix::translate(10.0, 10.0, 0.0), 1.0);
        let pixels = backend.rasterize(&first, w, h).to_vec();
        assert_eq!(backend.layered().rasterized, vec![(key, 1)]);
        assert_eq!(backend.layered().composited, vec![key]);
        assert_eq!(px(&pixels, w as usize, 15, 15), [0, 0, 255, 255]);
        let layers = backend.layers();
        assert_eq!(layers.len(), 1);
        assert_eq!(
            (layers[0].key, layers[0].reason),
            (key, LayerReason::WillChange)
        );
        assert_eq!(layers[0].tiles, 0);
        assert!(layers[0].bytes > 0 && backend.layer_bytes() == layers[0].bytes);
        for (matrix, opacity) in [
            (Matrix::translate(20.0, 12.0, 0.0), 1.0),
            (Matrix::translate(20.5, 12.25, 0.0), 1.0),
            (Matrix::translate(20.0, 12.0, 0.0), 0.5),
            (
                Matrix::translate(24.0, 20.0, 0.0)
                    .then(&Matrix::rotate_z(30.0))
                    .then(&Matrix::translate(-9.0, -9.0, 0.0)),
                0.8,
            ),
            (Matrix::scale(1.5, 1.5, 1.0), 1.0),
        ] {
            let moved = list(matrix, opacity);
            let pixels = backend.rasterize(&moved, w, h).to_vec();
            let layered = backend.layered();
            assert!(layered.rasterized.is_empty(), "{layered:?}");
            assert_eq!(layered.composited, vec![key]);
            let mut fresh = test_backend();
            assert_eq!(
                pixels,
                fresh.rasterize(&moved, w, h).to_vec(),
                "{matrix:?} {opacity}"
            );
        }
    }

    #[test]
    fn a_scrolled_layer_is_tiles_that_a_scroll_composites_and_time_draws_ahead() {
        // M19.4: a scrolled group is kept as tiles of the rows the
        // raster shows, with the rows a viewport's height either side
        // drawn after the frame while there is time; a scroll by whole
        // pixels composites the tiles, drawing into none while the
        // rows it shows are held, and the frame is a fresh raster's.
        let (w, h) = (40u32, 64u32);
        let mut backend = test_backend();
        let key = fresh_key();
        let list = |scroll: f32| {
            let mut items = vec![
                white_frame(w as f32, h as f32),
                DisplayItem::PushGroup {
                    matrix: Matrix::translate(0.0, -scroll, 0.0),
                    opacity: 1.0,
                    key,
                    layer: Some(LayerReason::Scrolled),
                },
            ];
            for i in 0..40 {
                let y = i as f32 * 50.0;
                items.push(rect(
                    4.0,
                    y + 2.0,
                    20.0,
                    30.0,
                    Rgba8::rgb(i * 6, 0, 255 - i * 6),
                ));
            }
            items.push(DisplayItem::PopGroup);
            DisplayList { items }
        };
        backend.rasterize_by(&list(0.0), w, h, None);
        let layers = backend.layers();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].reason, LayerReason::Scrolled);
        assert_eq!(layers[0].tiles, 1, "the row the frame shows alone");
        assert_eq!(backend.layered().rasterized, vec![(key, 1)]);
        assert_eq!(backend.layered().prepainted, 0);
        // With time after the frame, the margin's rows are drawn.
        let far = Instant::now() + std::time::Duration::from_secs(5);
        backend.rasterize_by(&list(0.0), w, h, Some(far));
        assert!(backend.layered().prepainted >= 1);
        assert!(backend.layers()[0].tiles >= 2, "{:?}", backend.layers());
        assert!(backend.layered().rasterized.is_empty());
        // A scroll within the rows held, each frame drawing the rows
        // the margin reaches next ahead of time: composited, nothing
        // drawn into the rows shown.
        let mut prepainted = 0;
        for scroll in [30.0, 100.0, 140.0, 250.0, 300.0] {
            let pixels = backend
                .rasterize_by(&list(scroll), w, h, Some(far))
                .to_vec();
            let layered = backend.layered();
            assert!(layered.rasterized.is_empty(), "{scroll}: {layered:?}");
            assert_eq!(layered.composited, vec![key]);
            prepainted += layered.prepainted;
            let mut fresh = test_backend();
            assert_eq!(
                pixels,
                fresh.rasterize(&list(scroll), w, h).to_vec(),
                "{scroll}"
            );
        }
        assert!(prepainted >= 1);
        // A scroll far off: the rows it shows, two across a tile's
        // edge, are drawn now, and the rows far behind let go.
        let pixels = backend.rasterize_by(&list(1500.0), w, h, None).to_vec();
        assert_eq!(backend.layered().rasterized, vec![(key, 2)]);
        let mut fresh = test_backend();
        assert_eq!(pixels, fresh.rasterize(&list(1500.0), w, h).to_vec());
        assert!(backend.layers()[0].tiles <= 3, "{:?}", backend.layers());
        // A change in the content draws again in the tiles it reaches,
        // and nowhere else: one rectangle's colour, in the row shown.
        let mut changed = list(1500.0);
        changed.items[2 + 30] = rect(4.0, 1502.0, 20.0, 30.0, Rgba8::rgb(0, 255, 0));
        let pixels = backend.rasterize_by(&changed, w, h, None).to_vec();
        assert_eq!(backend.layered().rasterized, vec![(key, 1)]);
        assert_eq!(px(&pixels, w as usize, 10, 10), [0, 255, 0, 255]);
        let mut fresh = test_backend();
        assert_eq!(pixels, fresh.rasterize(&changed, w, h).to_vec());
    }

    #[test]
    fn the_budget_caps_the_layers_and_a_frame_without_room_draws_through() {
        // M19.6: a thousand groups that are layers hold no more than
        // the budget allows, the frame is a fresh raster's all the
        // same, and a budget of nothing keeps no layer at all.
        let (w, h) = (64u32, 64u32);
        let mut items = vec![white_frame(w as f32, h as f32)];
        for i in 0..1000u32 {
            items.push(DisplayItem::PushGroup {
                matrix: Matrix::translate((i % 50) as f32, (i / 50) as f32 * 3.0, 0.0),
                opacity: 1.0,
                key: GroupKey::Document(i),
                layer: Some(LayerReason::WillChange),
            });
            items.push(rect(
                0.0,
                0.0,
                6.0,
                6.0,
                Rgba8::rgb((i * 7) as u8, (i * 3) as u8, 9),
            ));
            items.push(DisplayItem::PopGroup);
        }
        let list = DisplayList { items };
        let mut unbounded = test_backend();
        let want = unbounded.rasterize(&list, w, h).to_vec();
        assert_eq!(unbounded.layers().len(), 1000);
        let mut capped = test_backend();
        capped.set_layer_budget(20_000);
        assert_eq!(capped.rasterize(&list, w, h), &want[..]);
        assert!(capped.layer_bytes() <= 20_000, "{}", capped.layer_bytes());
        let held = capped.layers().len();
        assert!(held > 0 && held < 1000, "{held}");
        // The same list again: the same layers held, none let go for
        // another, and the frame the same.
        assert_eq!(capped.rasterize(&list, w, h), &want[..]);
        assert_eq!(capped.layers().len(), held);
        let mut none = test_backend();
        none.set_layer_budget(0);
        assert_eq!(none.rasterize(&list, w, h), &want[..]);
        assert!(none.layers().is_empty());
        assert_eq!(none.layer_bytes(), 0);
    }

    #[test]
    fn eviction_takes_the_least_recently_used_and_never_a_tile_the_frame_shows() {
        // M19.6: under a budget of two tiles, a scrolled layer keeps
        // the row the frame shows and one more, the one used last; a
        // whole layer used by this frame is never let go for a tile.
        let (w, h) = (16u32, 32u32);
        let key = fresh_key();
        let other = fresh_key();
        let list = |scroll: f32| {
            let mut items = vec![
                white_frame(w as f32, h as f32),
                DisplayItem::PushGroup {
                    matrix: Matrix::translate(0.0, -scroll, 0.0),
                    opacity: 1.0,
                    key,
                    layer: Some(LayerReason::Scrolled),
                },
            ];
            for i in 0..20 {
                items.push(rect(
                    2.0,
                    i as f32 * 100.0,
                    8.0,
                    8.0,
                    Rgba8::rgb(i * 10, 0, 0),
                ));
            }
            items.push(DisplayItem::PopGroup);
            items.push(DisplayItem::PushGroup {
                matrix: Matrix::translate(8.0, 8.0, 0.0),
                opacity: 1.0,
                key: other,
                layer: Some(LayerReason::WillChange),
            });
            items.push(rect(0.0, 0.0, 4.0, 4.0, Rgba8::rgb(0, 0, 255)));
            items.push(DisplayItem::PopGroup);
            DisplayList { items }
        };
        let tile = w as usize * TILE_HEIGHT as usize * 4;
        let mut backend = test_backend();
        backend.set_layer_budget(2 * tile + 1024);
        let far = Instant::now() + std::time::Duration::from_secs(5);
        backend.rasterize_by(&list(0.0), w, h, Some(far));
        let tiles = |backend: &SoftwareBackend| {
            backend
                .layers()
                .iter()
                .find(|l| l.key == key)
                .map_or(0, |l| l.tiles)
        };
        assert!(tiles(&backend) <= 2, "{:?}", backend.layers());
        assert!(backend.layers().iter().any(|l| l.key == other));
        // Row by row down the content: the budget holds, the frame is
        // right, and the whole layer stays.
        for scroll in [300.0, 600.0, 900.0, 300.0] {
            let pixels = backend
                .rasterize_by(&list(scroll), w, h, Some(far))
                .to_vec();
            assert!(backend.layer_bytes() <= 2 * tile + 1024);
            assert!(
                tiles(&backend) >= 1 && tiles(&backend) <= 2,
                "{:?}",
                backend.layers()
            );
            assert!(backend.layers().iter().any(|l| l.key == other));
            let mut fresh = test_backend();
            assert_eq!(
                pixels,
                fresh.rasterize(&list(scroll), w, h).to_vec(),
                "{scroll}"
            );
        }
        // Under a budget below one tile, the scrolled group is drawn
        // through, and the frame is still right.
        backend.set_layer_budget(tile / 2);
        let pixels = backend.rasterize_by(&list(300.0), w, h, Some(far)).to_vec();
        assert_eq!(tiles(&backend), 0);
        let mut fresh = test_backend();
        assert_eq!(pixels, fresh.rasterize(&list(300.0), w, h).to_vec());
    }

    #[test]
    fn a_key_two_groups_share_names_no_layer_and_a_layer_that_comes_or_goes_is_damage() {
        // Two groups under one key are drawn as groups that are no
        // layers, since a layer holds one; and a group that becomes a
        // layer, or stops being one, with a matrix that resamples, is
        // drawn again where it is, as a fresh raster has it.
        let (w, h) = (40u32, 30u32);
        let key = fresh_key();
        let twice = DisplayList {
            items: vec![
                white_frame(w as f32, h as f32),
                DisplayItem::PushGroup {
                    matrix: Matrix::translate(2.0, 2.0, 0.0),
                    opacity: 1.0,
                    key,
                    layer: Some(LayerReason::WillChange),
                },
                rect(0.0, 0.0, 8.0, 8.0, Rgba8::rgb(255, 0, 0)),
                DisplayItem::PopGroup,
                DisplayItem::PushGroup {
                    matrix: Matrix::translate(20.0, 2.0, 0.0),
                    opacity: 1.0,
                    key,
                    layer: Some(LayerReason::WillChange),
                },
                rect(0.0, 0.0, 8.0, 8.0, Rgba8::rgb(0, 255, 0)),
                DisplayItem::PopGroup,
            ],
        };
        let mut backend = test_backend();
        let pixels = backend.rasterize(&twice, w, h).to_vec();
        assert_eq!(px(&pixels, w as usize, 5, 5), [255, 0, 0, 255]);
        assert_eq!(px(&pixels, w as usize, 23, 5), [0, 255, 0, 255]);
        assert!(backend.layered().composited.is_empty());
        let fractional = |layer: Option<LayerReason>| DisplayList {
            items: vec![
                white_frame(w as f32, h as f32),
                DisplayItem::PushGroup {
                    matrix: Matrix::translate(2.5, 2.5, 0.0),
                    opacity: 1.0,
                    key,
                    layer,
                },
                rect(0.0, 0.0, 8.0, 8.0, Rgba8::rgb(255, 0, 0)),
                DisplayItem::PopGroup,
            ],
        };
        let mut backend = test_backend();
        backend.rasterize(&fractional(None), w, h);
        let pixels = backend
            .rasterize(&fractional(Some(LayerReason::WillChange)), w, h)
            .to_vec();
        let mut fresh = test_backend();
        assert_eq!(
            pixels,
            fresh.rasterize(&fractional(Some(LayerReason::WillChange)), w, h)
        );
        let pixels = backend.rasterize(&fractional(None), w, h).to_vec();
        let mut fresh = test_backend();
        assert_eq!(pixels, fresh.rasterize(&fractional(None), w, h));
    }

    #[test]
    fn alignment_matches_the_most_items_in_order_and_tile_rows_hold_the_texels() {
        let items: Vec<DisplayItem> = (0..8)
            .map(|i| rect(i as f32, 0.0, 1.0, 1.0, Rgba8::rgb(i, 0, 0)))
            .collect();
        // Two taken from the start, two added at the end: the six in
        // the middle match.
        let a: Vec<DisplayItem> = items[..6].to_vec();
        let b: Vec<DisplayItem> = items[2..8].to_vec();
        assert_eq!(
            align(&a, &b),
            (2..6).map(|i| (i, i - 2)).collect::<Vec<_>>()
        );
        // One moved from the start to the end: the rest match.
        let mut moved = a.clone();
        let first = moved.remove(0);
        moved.push(first);
        assert_eq!(
            align(&a, &moved),
            (1..6).map(|i| (i, i - 1)).collect::<Vec<_>>()
        );
        assert_eq!(align(&a, &a), (0..6).map(|i| (i, i)).collect::<Vec<_>>());
        assert_eq!(align(&a, &[]), Vec::new());
        // Past the edit limit, the shared start and end alone.
        let many: Vec<DisplayItem> = (0..MAX_DIFF_EDITS as u32 + 4)
            .map(|i| rect(0.0, i as f32, 1.0, 1.0, Rgba8::rgb(0, 0, 0)))
            .collect();
        let mut other = many.clone();
        other.reverse();
        other[0] = many[0].clone();
        assert_eq!(align(&many, &other), vec![(0, 0)]);
        assert_eq!(tile_rows(0.0, 256.0), 0..=0);
        assert_eq!(tile_rows(0.0, 256.5), 0..=1);
        assert_eq!(tile_rows(-0.5, 10.0), -1..=0);
        assert!(tile_rows(300.0, 300.0).is_empty());
        assert!(tile_rows(10.0, f32::NAN).is_empty());
        assert_eq!(tile_rows(1000.0, 1001.0), 3..=3);
    }
}
