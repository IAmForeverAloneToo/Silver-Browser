//! Font loading, text shaping, and glyph rasterization.
//!
//! Owns everything between "a string at a font size" and "alpha masks at
//! pixel positions": system font discovery (fontdb), shaping (rustybuzz),
//! and glyph outlines (ab_glyph). Consumers: `render` and `layout`
//! (inline layout / line breaking).
//!
//! **Kerning and ligatures (roadmap M8.2)** already happen, as of M1's
//! original [`FontStore::shape`] — not new work, just worth stating
//! plainly so it isn't re-discovered as a surprise or re-implemented by
//! accident: `shape`/`measure`'s shared `shape_raw` helper calls
//! `rustybuzz::shape(&face, &[], buffer)` with an *empty* features list,
//! which does not mean "no features" — it means "no additional
//! overrides on top of the shaping engine's own defaults." HarfBuzz's
//! (and rustybuzz's) OpenType shaper
//! applies a standard default feature set per script that includes
//! `kern`, `liga`, `clig`, and `calt` unless a caller explicitly
//! disables them (passing a `Feature` with value `0`), which this crate
//! never does. `font-feature-settings` (CSS, to add/remove specific
//! features beyond the defaults) is M21 scope, per that item's own note.

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use ab_glyph::{Font as _, Glyph, GlyphId, Point, PxScale};
// Re-exported: `Script` is already part of this crate's own public API
// (`itemize`, `FontCollection::font_for`/`shape`), so a consumer like
// `render` can name it as `text::Script` without taking its own direct
// `unicode-script` dependency just to spell the type its glyph-mask
// cache key needs (M8.1 — see `render::SoftwareBackend`).
pub use unicode_script::Script;
use unicode_script::UnicodeScript;

/// Errors produced by font loading and shaping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontError {
    /// No usable system font was found.
    NoFontFound,
    /// The font data could not be parsed.
    InvalidFont,
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FontError::NoFontFound => write!(f, "no usable system font found"),
            FontError::InvalidFont => write!(f, "the font data could not be parsed"),
        }
    }
}

impl std::error::Error for FontError {}

/// A shaped glyph positioned relative to its text run's origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedGlyph {
    /// Glyph id in the font's glyph table.
    pub glyph_id: u16,
    /// Horizontal offset from the run origin (pixels).
    pub x: f32,
    /// Vertical offset from the baseline; positive is downward (pixels).
    pub y: f32,
}

/// A rasterized glyph: 8-bit alpha coverage and placement metadata.
#[derive(Debug, Clone)]
pub struct GlyphMask {
    /// Mask width in pixels.
    pub width: u32,
    /// Mask height in pixels.
    pub height: u32,
    /// Horizontal offset of the mask's left edge from the pen (pixels).
    pub left: i32,
    /// Vertical offset of the mask's top edge from the baseline
    /// (pixels, positive down — negative for ascenders).
    pub top: i32,
    /// 8-bit alpha coverage, row-major.
    pub coverage: Vec<u8>,
}

impl GlyphMask {
    /// An empty mask (e.g. for blank glyphs).
    pub fn empty() -> Self {
        GlyphMask {
            width: 0,
            height: 0,
            left: 0,
            top: 0,
            coverage: Vec::new(),
        }
    }
}

/// A font's vertical metrics, scaled to CSS pixels for one font size
/// (roadmap M7.1: "line height = max of font metrics
/// (ascent+descent+leading v1)").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    /// Height above the baseline.
    pub ascent: f32,
    /// Height below the baseline (a positive value, even though the
    /// font file's own `descender` is conventionally negative).
    pub descent: f32,
    /// Extra inter-line spacing the font itself recommends (OpenType's
    /// `lineGap` / hhea table) — the "leading" in "ascent+descent+leading".
    pub line_gap: f32,
}

impl FontMetrics {
    /// The full line height these metrics imply: ascent + descent +
    /// line_gap.
    pub fn line_height(&self) -> f32 {
        self.ascent + self.descent + self.line_gap
    }
}

/// A loaded font, ready for shaping and rasterization. Cloning shares
/// the face data (an `Arc`) rather than copying it.
#[derive(Clone)]
pub struct FontStore {
    /// Raw face data.
    raw: Arc<Vec<u8>>,
    /// Face index within `raw` (nonzero for font collections).
    index: u32,
    /// The face's units per em, cached for shaping.
    units_per_em: f32,
    /// Vertical metrics, in font units (scaled to pixels on demand —
    /// see [`FontStore::metrics`]).
    ascender_units: f32,
    descender_units: f32,
    line_gap_units: f32,
    /// The OS/2 table's x-height, in font units, when the font gives one.
    x_height_units: Option<f32>,
    /// Whether the `head` table's flags ask for whole pixels per em in
    /// all of the scaler's arithmetic (bit 3), which FreeType honours
    /// when it hints an advance (see [`FontStore::advance`]).
    integer_ppem: bool,
    /// ab_glyph handle for outline rasterization.
    glyph_font: ab_glyph::FontArc,
}

impl fmt::Debug for FontStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FontStore")
            .field("index", &self.index)
            .field("units_per_em", &self.units_per_em)
            .field("bytes", &self.raw.len())
            .finish()
    }
}

impl FontStore {
    /// Loads the best available system font.
    ///
    /// Tries a family fallback list ending in the generic `sans-serif`,
    /// so this succeeds on any platform with fonts installed. Returns
    /// [`FontError::NoFontFound`] when nothing usable is installed — never
    /// panics.
    pub fn new() -> Result<Self, FontError> {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Self::query(&db, &["Segoe UI", "DejaVu Sans", "Helvetica", "Arial"])
    }

    /// Loads the best match for `family_names` (falling back to the
    /// generic `sans-serif`) out of an already-populated database —
    /// the shared plumbing behind [`FontStore::new`] and
    /// [`FontCollection::new`]'s per-script fallback queries (roadmap
    /// M8.1), which need to issue more than one query against the
    /// *same* loaded font set rather than re-scanning the system fonts
    /// per query.
    fn query(db: &fontdb::Database, family_names: &[&str]) -> Result<Self, FontError> {
        let families: Vec<fontdb::Family> = family_names
            .iter()
            .copied()
            .map(fontdb::Family::Name)
            .chain([fontdb::Family::SansSerif])
            .collect();
        let query = fontdb::Query {
            families: &families,
            weight: fontdb::Weight::NORMAL,
            style: fontdb::Style::Normal,
            stretch: fontdb::Stretch::Normal,
        };

        let Some(face_id) = db.query(&query) else {
            return Err(FontError::NoFontFound);
        };

        match db.with_face_data(face_id, |data, index| {
            FontStore::from_bytes(data.to_vec(), index)
        }) {
            Some(result) => result,
            None => Err(FontError::NoFontFound),
        }
    }

    /// Loads a font from raw face data at `index`.
    pub fn from_bytes(data: Vec<u8>, index: u32) -> Result<Self, FontError> {
        let ttf = ttf_parser::Face::parse(&data, index).map_err(|_| FontError::InvalidFont)?;
        let units_per_em = ttf.units_per_em() as f32;
        if units_per_em <= 0.0 {
            return Err(FontError::InvalidFont);
        }
        // Read everything out of `ttf` before `data` moves into the `Arc`
        // below (`ttf` borrows it).
        //
        // Vertical metrics (M8.5, corrected by M17): the browsers' —
        // `hhea`'s ascender, descender and line gap, the OS/2 table's
        // typographic ones only when the font sets its USE_TYPO_METRICS
        // bit, or when `hhea` has none, and then the Windows ones —
        // which is the choice ttf_parser's `ascender`, `descender` and
        // `line_gap` make. M8.5 preferred the typographic metrics
        // whenever a font had them, on the belief that browsers do; the
        // reference browser does not, and M17's acceptance pages, text
        // laid out on both sides in one font, found the lines apart.
        // ttf_parser's descender is conventionally <= 0 (how far below
        // the baseline); `.max(0.0)` guards the (rare, malformed-font)
        // case where that doesn't hold, so `descent` stays a sane
        // non-negative "height" rather than silently subtracting space.
        let ascender_units = f32::from(ttf.ascender());
        let descender_units = (-f32::from(ttf.descender())).max(0.0);
        let line_gap_units = f32::from(ttf.line_gap()).max(0.0);
        let x_height_units = ttf.x_height().map(f32::from).filter(|x| *x > 0.0);
        let integer_ppem = ttf
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"head"))
            .and_then(|head| head.get(16..18))
            .is_some_and(|flags| flags[1] & 0x08 != 0);

        // TODO(M2): honor non-zero indices for .ttc collections in ab_glyph.
        let glyph_font =
            ab_glyph::FontArc::try_from_vec(data.clone()).map_err(|_| FontError::InvalidFont)?;
        Ok(FontStore {
            raw: Arc::new(data),
            index,
            units_per_em,
            ascender_units,
            descender_units,
            line_gap_units,
            x_height_units,
            integer_ppem,
            glyph_font,
        })
    }

    /// The x-height at `font_size_px`, the height of a lowercase `x`
    /// above the baseline (what CSS's `ex` is): the OS/2 table's figure
    /// when the font gives one, else the `x` glyph's own outline, else
    /// half the font size, the fallback css-values-4 allows when nothing
    /// better is known.
    pub fn x_height(&self, font_size_px: f32) -> f32 {
        if let Some(units) = self.x_height_units {
            return units * font_size_px / self.units_per_em;
        }
        let id = self.glyph_font.glyph_id('x');
        if id.0 != 0
            && let Some(outline) = self
                .glyph_font
                .outline_glyph(id.with_scale(px_scale(&self.glyph_font, font_size_px.max(0.0))))
        {
            // Outline coordinates grow downward, the baseline at zero.
            let top = -outline.px_bounds().min.y;
            if top.is_finite() && top > 0.0 {
                return top;
            }
        }
        font_size_px * 0.5
    }

    /// The advance of the digit zero at `font_size_px` (what CSS's `ch`
    /// is); half the font size when the font has no zero.
    pub fn zero_advance(&self, font_size_px: f32) -> f32 {
        let advance = self.measure("0", font_size_px);
        if advance > 0.0 {
            advance
        } else {
            font_size_px * 0.5
        }
    }

    /// The ab_glyph handle, for glyph rasterization.
    pub fn glyph_font(&self) -> &ab_glyph::FontArc {
        &self.glyph_font
    }

    /// This font's vertical metrics at `font_size_px` (roadmap M7.1),
    /// each rounded to a whole pixel, half up, as the reference browser
    /// rounds them (M17): a line of normal height is a whole number of
    /// pixels tall, and its baseline a whole number below its top.
    pub fn metrics(&self, font_size_px: f32) -> FontMetrics {
        let scale = font_size_px / self.units_per_em;
        let px = |units: f32| (units * scale + 0.5).floor();
        FontMetrics {
            ascent: px(self.ascender_units),
            descent: px(self.descender_units),
            line_gap: px(self.line_gap_units),
        }
    }

    /// Shapes `text` at `font_size_px`, returning glyphs positioned
    /// relative to the run origin (baseline at y = 0).
    ///
    /// Returns an empty vector if the face cannot be constructed — never
    /// panics on untrusted input.
    pub fn shape(&self, text: &str, font_size_px: f32) -> Vec<ShapedGlyph> {
        let Some((face, shaped)) = self.shape_raw(text) else {
            return Vec::new();
        };
        let mut pen_x = 0.0_f32;
        let mut out = Vec::with_capacity(shaped.len());
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            out.push(ShapedGlyph {
                glyph_id: info.glyph_id as u16,
                x: pen_x + whole_pixels(pos.x_offset, font_size_px, self.units_per_em),
                // harfbuzz y_offset is positive-up; we store positive-down.
                y: -whole_pixels(pos.y_offset, font_size_px, self.units_per_em),
            });
            pen_x += self.advance(&face, info, pos, font_size_px);
        }
        out
    }

    /// `text`'s total advance width at `font_size_px` (roadmap M7.2) —
    /// everything [`FontStore::shape`] computes except the per-glyph
    /// positions `layout`'s word-measurement doesn't need. `0.0` if the
    /// face can't be constructed, same never-panics guarantee as `shape`.
    pub fn measure(&self, text: &str, font_size_px: f32) -> f32 {
        let Some((face, shaped)) = self.shape_raw(text) else {
            return 0.0;
        };
        shaped
            .glyph_infos()
            .iter()
            .zip(shaped.glyph_positions())
            .map(|(info, pos)| self.advance(&face, info, pos, font_size_px))
            .sum()
    }

    /// Where a caret may stand in `text` at `font_size_px` (M15): the x
    /// of each character boundary, from the run's start, one more than
    /// there are characters, the first 0 and the last the run's advance.
    /// They come from the shaper's clusters: a cluster's advance is
    /// shared evenly among the characters it covers, a ligature's
    /// among the letters it joins. The characters are taken in logical
    /// order, left to right; a run the shaper lays out right to left is
    /// not placed as it is drawn until bidirectional text comes to
    /// editing (M42). All zeros past the first if the face cannot be
    /// constructed.
    pub fn caret_stops(&self, text: &str, font_size_px: f32) -> Vec<f32> {
        let count = text.chars().count();
        let mut stops = Vec::with_capacity(count + 1);
        stops.push(0.0);
        if let Some((face, shaped)) = self.shape_raw(text) {
            let mut advances: std::collections::BTreeMap<usize, f32> =
                std::collections::BTreeMap::new();
            for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
                *advances.entry(info.cluster as usize).or_default() +=
                    self.advance(&face, info, pos, font_size_px);
            }
            let starts: Vec<usize> = advances.keys().copied().collect();
            let mut x = 0.0f32;
            for (k, &start) in starts.iter().enumerate() {
                let end = starts.get(k + 1).copied().unwrap_or(text.len());
                let Some(covered) = text.get(start..end) else {
                    continue;
                };
                let chars = covered.chars().count();
                let share = advances[&start] / chars.max(1) as f32;
                for _ in 0..chars {
                    x += share;
                    stops.push(x);
                }
            }
        }
        // A face that would not shape, or clusters that did not cover
        // every character, leave the rest where the last one ended.
        let last = stops.last().copied().unwrap_or(0.0);
        stops.resize(count + 1, last);
        stops
    }

    /// Shared rustybuzz plumbing for [`FontStore::shape`] and
    /// [`FontStore::measure`] — font-unit positions, unscaled, with the
    /// face they were shaped in, whose own advances
    /// [`FontStore::advance`] reads.
    fn shape_raw(&self, text: &str) -> Option<(rustybuzz::Face<'_>, rustybuzz::GlyphBuffer)> {
        let face = rustybuzz::Face::from_slice(&self.raw, self.index)?;
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        let shaped = rustybuzz::shape(&face, &[], buffer);
        Some((face, shaped))
    }

    /// A shaped glyph's advance at `font_size_px`, as the reference
    /// browser takes it with no subpixel positioning (M17, the flexbox
    /// note's "Vertical metrics" and what follows it): the font's own
    /// advance for the glyph hinted as FreeType hints it, and the
    /// shaper's adjustment to it — kerning — scaled at the size and
    /// rounded to a whole pixel, half up. A glyph the shaper gives no
    /// advance, a mark, has none.
    fn advance(
        &self,
        face: &rustybuzz::Face<'_>,
        info: &rustybuzz::GlyphInfo,
        pos: &rustybuzz::GlyphPosition,
        font_size_px: f32,
    ) -> f32 {
        if pos.x_advance == 0 {
            return 0.0;
        }
        let own = face
            .glyph_hor_advance(ttf_parser::GlyphId(info.glyph_id as u16))
            .map_or(0, i32::from);
        self.hinted(own, font_size_px)
            + whole_pixels(pos.x_advance - own, font_size_px, self.units_per_em)
    }

    /// `units` of advance at `font_size_px` as FreeType's hinting gives
    /// it: scaled in its 26.6 fixed point (`FT_DivFix`, `FT_MulFix`) at
    /// the pixels per em — the size, or the size rounded to a whole
    /// pixel when the font asks for that — and rounded to a whole pixel
    /// (`FT_PIX_ROUND`). In 128 bits, the size held below a trillion
    /// pixels and a size that is not a number taken as none, so that no
    /// size overflows it.
    fn hinted(&self, units: i32, font_size_px: f32) -> f32 {
        let size = if self.integer_ppem {
            (font_size_px + 0.5).floor()
        } else {
            font_size_px
        };
        let size26 = (f64::from(size) * 64.0).round().clamp(0.0, 64e12) as i128;
        let upem = self.units_per_em as i128;
        let x_scale = ((size26 << 16) + upem / 2) / upem;
        let product = i128::from(units) * x_scale;
        let scaled = product.signum() * ((product.abs() + 0x8000) >> 16);
        ((scaled + 32) >> 6) as f32
    }
}

/// `units` of a font scaled to `font_size_px` and rounded to a whole
/// pixel, half up, as the reference browser places glyphs with no
/// subpixel positioning (M17). No units are no pixels at any size, an
/// infinite one included.
fn whole_pixels(units: i32, font_size_px: f32, units_per_em: f32) -> f32 {
    if units == 0 {
        return 0.0;
    }
    (units as f32 * font_size_px / units_per_em + 0.5).floor()
}

/// The most pixels one glyph mask may hold: a glyph drawn at a size
/// where a single letter is 4096 pixels on a side. Past it the glyph
/// is not drawn, rather than the mask's allocation (a 50,000-pixel
/// font asks for gigabytes per letter) taking the process down.
pub const MAX_MASK_PIXELS: u64 = 4096 * 4096;

/// `ab_glyph`'s scale for a font size of `size_px`. A font size is the
/// em, as the shaper's advances are scaled; `ab_glyph`'s `PxScale` is
/// instead the height from the descender to the ascender, which is
/// larger than the em in most fonts (2,384 units to DejaVu Sans's
/// 2,048), so given the font size as it is, every glyph would be drawn
/// that much smaller than the room its advance leaves it. A font whose
/// units per em `ab_glyph` cannot read, or whose height is not positive,
/// keeps the font size.
fn px_scale(font: &ab_glyph::FontArc, size_px: f32) -> PxScale {
    let height = font.height_unscaled();
    let scale = font
        .units_per_em()
        .map(|units_per_em| size_px * height / units_per_em)
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .unwrap_or(size_px);
    PxScale::from(scale)
}

/// Rasterizes one glyph of `font` at `size_px`, the em, into an alpha
/// mask, at the size the shaper's advances are measured at.
///
/// The mask is positioned relative to the baseline at the pen position
/// (`left`/`top` carry the placement offsets). Returns an empty mask for
/// blank or unoutlineable glyphs, for a size that is not a finite
/// positive number, and for a glyph larger than [`MAX_MASK_PIXELS`].
pub fn rasterize_glyph(font: &ab_glyph::FontArc, glyph_id: u16, size_px: f32) -> GlyphMask {
    if !size_px.is_finite() || size_px <= 0.0 {
        return GlyphMask::empty();
    }
    let glyph = Glyph {
        id: GlyphId(glyph_id),
        scale: px_scale(font, size_px),
        position: Point { x: 0.0, y: 0.0 },
    };
    let Some(outlined) = font.outline_glyph(glyph) else {
        return GlyphMask::empty();
    };

    let bounds = outlined.px_bounds();
    if !(bounds.min.x.is_finite()
        && bounds.min.y.is_finite()
        && bounds.max.x.is_finite()
        && bounds.max.y.is_finite())
    {
        return GlyphMask::empty();
    }
    // Bounds are measured in `f64` and `i64`: `as i32` would saturate
    // and then wrap in the subtraction for a glyph wider than two
    // billion pixels, which a hostile font size can ask for.
    let left_i64 = (bounds.min.x as f64).floor() as i64;
    let top_i64 = (bounds.min.y as f64).floor() as i64;
    let width_i64 = ((bounds.max.x as f64).ceil() as i64 - left_i64).max(0);
    let height_i64 = ((bounds.max.y as f64).ceil() as i64 - top_i64).max(0);
    if width_i64 == 0 || height_i64 == 0 || width_i64 * height_i64 > MAX_MASK_PIXELS as i64 {
        return GlyphMask::empty();
    }
    let (Ok(left), Ok(top)) = (i32::try_from(left_i64), i32::try_from(top_i64)) else {
        return GlyphMask::empty();
    };
    let (width, height) = (width_i64 as u32, height_i64 as u32);

    let mut coverage = vec![0u8; width as usize * height as usize];
    // draw() reports u32 coordinates RELATIVE to the glyph's bounding box
    // (not absolute), so reconstruct absolute positions via the bbox min
    // before indexing the mask.
    let min_x = bounds.min.x;
    let min_y = bounds.min.y;
    outlined.draw(|x, y, v| {
        let px = (x as f32 + min_x).floor() as i32 - left;
        let py = (y as f32 + min_y).floor() as i32 - top;
        if px >= 0 && py >= 0 && (px as u32) < width && (py as u32) < height {
            let idx = py as usize * width as usize + px as usize;
            let a = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            if a > coverage[idx] {
                coverage[idx] = a;
            }
        }
    });

    GlyphMask {
        width,
        height,
        left,
        top,
        coverage,
    }
}

/// Splits `text` into maximal same-script runs (roadmap M8.1's
/// "itemization into shape runs"), as byte-offset ranges into it — the
/// unit [`FontCollection`] dispatches to a per-script font.
///
/// "Same script" bends slightly for `Common`/`Inherited` characters
/// (spaces, digits, punctuation, combining marks): per standard text-
/// itemization practice (matching HarfBuzz/ICU's own run iterators),
/// these don't start a run of their own — they merge into whichever
/// run they're adjacent to, so e.g. a space or a run of Western digits
/// in the middle of a Han sentence doesn't get split off into its own
/// `Common`-tagged run that would otherwise ping-pong font selection
/// back to the primary font mid-sentence. A run that's *entirely*
/// `Common`/`Inherited` (no strong-script character anywhere in it,
/// e.g. a lone "123") stays tagged `Common` — [`fallback_families_for`]
/// has no entry for it, so it resolves to the primary font, same as
/// today's one-font behavior.
pub fn itemize(text: &str) -> Vec<(Range<usize>, Script)> {
    let mut runs = Vec::new();
    let mut run_start = 0usize;
    let mut run_script = Script::Common;

    for (idx, ch) in text.char_indices() {
        let s = ch.script();
        if idx == 0 {
            run_script = s;
            continue;
        }
        // A so-far-unclaimed `Common` run accepts *any* real script
        // next (there's nothing for it to conflict with yet) — not
        // just another `Common`/`Inherited` character — which is what
        // lets a real script arriving after leading digits/punctuation
        // retroactively claim the run below, instead of splitting off
        // what's seen so far into a separate, pointless `Common` run.
        if s == run_script
            || s == Script::Common
            || s == Script::Inherited
            || run_script == Script::Common
        {
            if run_script == Script::Common && s != Script::Common && s != Script::Inherited {
                run_script = s;
            }
            continue;
        }
        runs.push((run_start..idx, run_script));
        run_start = idx;
        run_script = s;
    }
    if !text.is_empty() {
        runs.push((run_start..text.len(), run_script));
    }
    runs
}

/// Per-script system family names to try before falling back to the
/// primary font (roadmap M8.1). `None` means "use the primary font,
/// unchanged from before M8.1" — right for every script this engine
/// doesn't have a specific, system-realistic family list for, which is
/// most of them: ordinary system UI fonts (the ones [`FontStore::new`]
/// already finds) tend to cover Latin/Cyrillic/Greek and plenty more
/// well enough that a separate query would just find the same font
/// again.
///
/// Scope cut (M8.1 "v1", matching the roadmap item's own "Latin → CJK"
/// framing): only Han/Hiragana/Katakana/Hangul get a real fallback
/// list here — the concrete scenario the roadmap names, and the one
/// most likely to actually matter (a Latin-oriented primary font with
/// no CJK glyphs at all, vs. most other non-Latin scripts where the
/// primary font at least has *some* partial, if imperfect, coverage).
/// Extending this to Arabic, Devanagari, Thai, etc. is the same
/// mechanism — just more entries — once there's a concrete reason
/// (and a system to test family-name choices against) to tune lists
/// for them.
fn fallback_families_for(script: Script) -> Option<&'static [&'static str]> {
    match script {
        Script::Han => Some(&[
            "Microsoft YaHei",
            "SimSun",
            "PingFang SC",
            "Noto Sans CJK SC",
            "Noto Sans SC",
            "Noto Sans CJK",
        ]),
        Script::Hiragana | Script::Katakana => Some(&[
            "Yu Gothic",
            "MS Gothic",
            "Hiragino Sans",
            "Noto Sans CJK JP",
            "Noto Sans JP",
            "Noto Sans CJK",
        ]),
        Script::Hangul => Some(&[
            "Malgun Gothic",
            "Apple SD Gothic Neo",
            "Noto Sans CJK KR",
            "Noto Sans KR",
            "Noto Sans CJK",
        ]),
        _ => None,
    }
}

/// A primary font plus resolved per-script fallback fonts (roadmap
/// M8.1) — e.g. a Latin-oriented primary font paired with a
/// CJK-capable fallback, each queried from the same loaded `fontdb`
/// database once at construction (not re-scanning system fonts per
/// script). Falls back are resolved eagerly rather than lazily: there
/// are only a handful of scripts [`fallback_families_for`] knows about,
/// so the up-front cost is small and it sidesteps needing interior
/// mutability to cache query results behind `&self`.
///
/// A system missing every family in a script's fallback list is not an
/// error — [`FontCollection::font_for`] just returns the primary font
/// for that script, same as if M8.1 didn't exist; this mirrors
/// `FontStore::shape`'s own "never panics on what it can't do" stance.
#[derive(Clone)]
pub struct FontCollection {
    primary: FontStore,
    fallbacks: HashMap<Script, FontStore>,
}

impl FontCollection {
    /// A collection of one font, from its face data, with no per-script
    /// fallbacks: what a test wants, so that it measures and draws the
    /// same font on every machine rather than whichever system font
    /// [`FontCollection::new`] finds there.
    pub fn from_bytes(data: Vec<u8>, index: u32) -> Result<Self, FontError> {
        Ok(FontCollection {
            primary: FontStore::from_bytes(data, index)?,
            fallbacks: HashMap::new(),
        })
    }

    /// Loads the primary font (same search as [`FontStore::new`]) plus
    /// whatever per-script fallbacks are available on this system.
    pub fn new() -> Result<Self, FontError> {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let primary = FontStore::query(&db, &["Segoe UI", "DejaVu Sans", "Helvetica", "Arial"])?;

        let mut fallbacks = HashMap::new();
        for script in [
            Script::Han,
            Script::Hiragana,
            Script::Katakana,
            Script::Hangul,
        ] {
            if let Some(families) = fallback_families_for(script)
                && let Ok(store) = FontStore::query(&db, families)
            {
                fallbacks.insert(script, store);
            }
        }
        Ok(FontCollection { primary, fallbacks })
    }

    /// The font every script used before M8.1, and still the font for
    /// any script with no resolved fallback.
    pub fn primary(&self) -> &FontStore {
        &self.primary
    }

    /// The best available font for `script`: a resolved fallback
    /// (M8.1) if this system has one, else the primary font.
    pub fn font_for(&self, script: Script) -> &FontStore {
        self.fallbacks.get(&script).unwrap_or(&self.primary)
    }

    /// `text`'s total advance width at `font_size_px`, itemizing it
    /// into per-script runs (M8.1) and measuring each with
    /// [`FontCollection::font_for`]'s pick rather than always the
    /// primary font — the fallback-aware counterpart to
    /// [`FontStore::measure`], and what [`layout`][crate] should call
    /// instead of it so a CJK word's measured width (and therefore
    /// line-wrapping) reflects the font it'll actually be shown in.
    pub fn measure(&self, text: &str, font_size_px: f32) -> f32 {
        itemize(text)
            .iter()
            .map(|(range, script)| {
                self.font_for(*script)
                    .measure(&text[range.clone()], font_size_px)
            })
            .sum()
    }

    /// The primary font's x-height at `font_size_px` (see
    /// [`FontStore::x_height`]).
    pub fn x_height(&self, font_size_px: f32) -> f32 {
        self.primary().x_height(font_size_px)
    }

    /// The primary font's advance for the digit zero at `font_size_px`
    /// (see [`FontStore::zero_advance`]).
    pub fn zero_advance(&self, font_size_px: f32) -> f32 {
        self.primary().zero_advance(font_size_px)
    }

    /// The line-height-relevant metrics across *every* script actually
    /// present in `text` at `font_size_px` (M8.1) — the max
    /// ascent/descent/line-gap over each per-script font `text`'s
    /// content dispatches to, extending `layout::inline::flush_line`'s
    /// existing "shared baseline sits at the tallest metrics on the
    /// line" rule (M7.4) to mixed scripts/fonts, not just mixed sizes.
    /// Matters even for a single-script word: today, *every* word's
    /// line-height contribution comes from the one primary font
    /// regardless of its own script, which is already subtly wrong for
    /// e.g. a Han word if a resolved CJK fallback's vertical metrics
    /// differ from the Latin primary font's — this is what fixes that,
    /// not just the literal mixed-script-in-one-word case.
    ///
    /// Zeroed metrics for empty `text` — matching `flush_line`'s own
    /// zero-initialized fold starting point, so composing this via
    /// `.max()` across words never needs a special empty case.
    pub fn metrics_for_text(&self, text: &str, font_size_px: f32) -> FontMetrics {
        itemize(text)
            .iter()
            .map(|(_, script)| self.font_for(*script).metrics(font_size_px))
            .fold(
                FontMetrics {
                    ascent: 0.0,
                    descent: 0.0,
                    line_gap: 0.0,
                },
                |acc, m| FontMetrics {
                    ascent: acc.ascent.max(m.ascent),
                    descent: acc.descent.max(m.descent),
                    line_gap: acc.line_gap.max(m.line_gap),
                },
            )
    }

    /// Shapes `text` at `font_size_px`, itemizing it into per-script
    /// runs (M8.1) and shaping each with [`FontCollection::font_for`]'s
    /// pick — the fallback-aware counterpart to [`FontStore::shape`].
    /// Each glyph's `x`/`y` are adjusted to be relative to the whole
    /// `text`'s origin (continuing the pen position across runs), so a
    /// caller can treat the result exactly like one `FontStore::shape`
    /// call's output — except each glyph now carries the script whose
    /// font actually produced it, which a renderer *must* key any
    /// glyph-mask cache on once more than one font is in play: a
    /// `glyph_id` is only meaningful relative to the specific font that
    /// produced it, and two unrelated fonts routinely reuse the same
    /// small integer id for unrelated glyphs.
    pub fn shape(&self, text: &str, font_size_px: f32) -> Vec<(Script, ShapedGlyph)> {
        let mut out = Vec::new();
        let mut pen_x = 0.0f32;
        for (range, script) in itemize(text) {
            let font = self.font_for(script);
            let run_text = &text[range];
            for g in font.shape(run_text, font_size_px) {
                out.push((
                    script,
                    ShapedGlyph {
                        glyph_id: g.glyph_id,
                        x: g.x + pen_x,
                        y: g.y,
                    },
                ));
            }
            pen_x += font.measure(run_text, font_size_px);
        }
        out
    }

    /// Where a caret may stand in `text` at `font_size_px` (M15), as
    /// [`FontStore::caret_stops`] has it, the runs itemized and each
    /// measured in the font [`FontCollection::font_for`] picks, as
    /// [`FontCollection::shape`] places them: one stop per character
    /// boundary, the last at the whole text's advance.
    pub fn caret_stops(&self, text: &str, font_size_px: f32) -> Vec<f32> {
        let mut stops = vec![0.0];
        let mut pen_x = 0.0f32;
        for (range, script) in itemize(text) {
            let font = self.font_for(script);
            let run_text = &text[range];
            let run = font.caret_stops(run_text, font_size_px);
            stops.extend(run.iter().skip(1).map(|x| x + pen_x));
            pen_x += font.measure(run_text, font_size_px);
        }
        stops
    }
}

/// A base text direction, for [`bidi_reorder`] — this crate's own copy
/// rather than a shared type: `text` sits *below* `style` in the layer
/// stack (see `docs/architecture.md`), so it can't name
/// `style::computed::Direction` without creating an upward dependency.
/// `layout` (which depends on both) converts at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

/// Reorders `text` into visual order — left-to-right as written, this
/// produces what should actually be *displayed* left-to-right — per
/// UAX#9's bidirectional algorithm (roadmap M8.3), via the `unicode-bidi`
/// crate. `base_direction` is the explicit direction (from `dir`/
/// `direction`), if any; `None` lets the algorithm determine it itself
/// via first-strong detection (UAX#9 P2/P3) — exactly what passing `None`
/// as `unicode_bidi::BidiInfo::new`'s paragraph level means.
///
/// Scope cut (M8.3 "level 1"): this treats all of `text` as a single
/// paragraph and reorders it as one unit. Reordering that correctly
/// interacts with *line breaking* (a bidi run can legitimately span a
/// soft-wrap point, and per UAX#9 reordering is properly a per-displayed-
/// line operation, not a per-paragraph one) is real complexity the
/// roadmap itself defers further (M42, "full bidi + ICU4X integration").
/// `layout::inline` calls this once per text box, before word-splitting
/// — correct for the common case this scope targets (one direction, or
/// one embedded opposite-direction run, within what ends up as a
/// single text node), not guaranteed correct across arbitrary nested
/// differently-styled spans within one bidi run.
///
/// `unicode-bidi` splits the text into paragraphs at every Bidi class B
/// character (LF, CR, NEL, U+2029 and the information separators), and
/// each paragraph is reordered on its own; nothing after the first one
/// is dropped.
pub fn bidi_reorder(text: &str, base_direction: Option<Direction>) -> String {
    if text.is_empty() {
        return String::new();
    }
    let level = base_direction.map(|d| match d {
        Direction::Ltr => unicode_bidi::Level::ltr(),
        Direction::Rtl => unicode_bidi::Level::rtl(),
    });
    let info = unicode_bidi::BidiInfo::new(text, level);
    if info.paragraphs.is_empty() {
        return text.to_string(); // no strong/explicit content at all
    }
    info.paragraphs
        .iter()
        .map(|para| info.reorder_line(para, para.range.clone()))
        .collect()
}

/// The face data of DejaVu Sans, bundled under `fonts/` for the tests of
/// this crate and of the crates above it: every test measures and draws
/// the same font on every machine, and a machine without system fonts
/// still runs them. Not part of the library's own font loading.
#[doc(hidden)]
pub const TEST_FONT: &[u8] = include_bytes!("../fonts/DejaVuSans.ttf");

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> FontStore {
        FontStore::from_bytes(TEST_FONT.to_vec(), 0).expect("the bundled test font parses")
    }

    fn test_fonts() -> FontCollection {
        FontCollection::from_bytes(TEST_FONT.to_vec(), 0).expect("the bundled test font parses")
    }

    /// A glyph is drawn at the size its advance is measured at, the em:
    /// its mask covers its outline's bounding box scaled by the font
    /// size over the units per em, give or take the pixels the box's
    /// edges are rounded out to and the coverage that rounds to none.
    #[test]
    fn glyphs_are_drawn_at_the_size_the_shaper_measures() {
        let store = test_store();
        let face = ttf_parser::Face::parse(TEST_FONT, 0).expect("the bundled font parses");
        for size in [11.0_f32, 16.0, 40.0] {
            let scale = size / face.units_per_em() as f32;
            for c in ['m', 'W', 'H', 'o', 'l'] {
                let id = face.glyph_index(c).expect("the glyph");
                let bbox = face.glyph_bounding_box(id).expect("an outline");
                let mask = rasterize_glyph(store.glyph_font(), id.0, size);
                let left = bbox.x_min as f32 * scale;
                let right = bbox.x_max as f32 * scale;
                let top = -(bbox.y_max as f32) * scale;
                let bottom = -(bbox.y_min as f32) * scale;
                let (mask_left, mask_top) = (mask.left as f32, mask.top as f32);
                let mask_right = mask_left + mask.width as f32;
                let mask_bottom = mask_top + mask.height as f32;
                for (what, drawn, outline) in [
                    ("left", mask_left, left),
                    ("right", mask_right, right),
                    ("top", mask_top, top),
                    ("bottom", mask_bottom, bottom),
                ] {
                    assert!(
                        (drawn - outline).abs() <= 1.0,
                        "{c:?} at {size}px: {what} drawn at {drawn}, outline at {outline}"
                    );
                }
            }
        }
    }

    #[test]
    fn caret_stops_stand_at_every_character_boundary() {
        let fonts = test_fonts();
        for text in [
            "",
            "a",
            "Hello, world",
            "café ünïcode",
            "office ﬁ ffi",
            "日本語 mixed",
        ] {
            let stops = fonts.caret_stops(text, 15.0);
            assert_eq!(stops.len(), text.chars().count() + 1, "{text:?}");
            assert_eq!(stops[0], 0.0);
            assert!(
                stops.windows(2).all(|pair| pair[0] <= pair[1]),
                "{text:?}: {stops:?}"
            );
            let whole = fonts.measure(text, 15.0);
            assert!((stops[stops.len() - 1] - whole).abs() < 0.01, "{text:?}");
        }
        // Each boundary of plain text is where the text up to it ends.
        let text = "Hello, world";
        let stops = fonts.caret_stops(text, 15.0);
        for (n, (i, _)) in text.char_indices().enumerate() {
            let prefix = fonts.measure(&text[..i], 15.0);
            assert!(
                (stops[n] - prefix).abs() < 0.5,
                "at {i}: {} vs {prefix}",
                stops[n]
            );
        }
    }

    #[test]
    fn shape_hello_advances_reasonably() {
        let store = test_store();
        let glyphs = store.shape("Hello", 28.0);
        assert!(!glyphs.is_empty(), "shaping produced no glyphs");
        let advance = glyphs.last().map(|g| g.x).unwrap_or(0.0);
        // "Hello" at 28px spans roughly 20–300px. A units mix-up (e.g.
        // dividing by 64, or failing to scale) lands far outside this band.
        assert!(
            advance > 20.0 && advance < 300.0,
            "suspicious total advance: {advance}px"
        );
    }

    #[test]
    fn measure_is_at_least_as_wide_as_shapes_last_glyph_start() {
        let store = test_store();
        let glyphs = store.shape("Hello", 28.0);
        let last = glyphs.last().expect("non-empty");
        // `shape`'s `ShapedGlyph.x` is where that glyph's pen *starts* —
        // it doesn't include that glyph's own advance, so the true total
        // width `measure` computes must be at least that large (strictly
        // larger, for any glyph with a non-zero advance of its own).
        assert!(store.measure("Hello", 28.0) >= last.x);
    }

    #[test]
    fn measure_hello_is_in_the_same_reasonable_band_as_shape() {
        let store = test_store();
        let advance = store.measure("Hello", 28.0);
        // Same band as `shape_hello_advances_reasonably` above — both
        // describe the same text at the same size.
        assert!(
            advance > 20.0 && advance < 300.0,
            "suspicious total advance: {advance}px"
        );
    }

    #[test]
    fn measure_scales_with_font_size_a_glyph_at_a_time_in_whole_pixels() {
        // Each of the four glyphs is rounded at its size (M17), so twice
        // the size is twice the width within half a pixel a glyph at
        // each size.
        let store = test_store();
        let at_10 = store.measure("word", 10.0);
        let at_20 = store.measure("word", 20.0);
        assert!(at_10 > 0.0);
        assert_eq!((at_10.fract(), at_20.fract()), (0.0, 0.0));
        assert!(
            (at_20 - at_10 * 2.0).abs() <= 4.0 * 1.5,
            "{at_20} should be about twice {at_10}"
        );
    }

    #[test]
    fn glyphs_are_placed_as_the_reference_browser_places_them_with_no_subpixel_positioning() {
        // Widths the reference browser gives runs of DejaVu Sans (M17):
        // each glyph's own advance hinted at a whole pixel per em, the
        // kerning rounded on its own. At 20px `z` is 1075 units, 10.498
        // pixels exactly but 10.5 in FreeType's fixed point, so 11; at
        // 10.5px a run is as wide as at 11; "AV" and "To" kern.
        let store = test_store();
        for (text, size, width) in [
            ("The quick brown fox jumps over the lazy dog", 16.0, 358.0),
            ("The quick brown fox jumps over the lazy dog", 20.0, 449.0),
            ("Navigation", 13.0, 71.0),
            ("AVAWAy To", 16.0, 86.0),
            ("AVAWAy To", 12.0, 62.0),
            ("lazy", 10.5, 23.0),
            ("Zebra puzzle", 15.5, 103.0),
        ] {
            assert_eq!(store.measure(text, size), width, "{text:?} at {size}px");
        }
        // The glyphs' own positions add up to the same, and a size that
        // is not a number measures nothing.
        let glyphs = store.shape("To", 16.0);
        assert_eq!(
            glyphs[1].x,
            store.measure("To", 16.0) - store.measure("o", 16.0)
        );
        assert_eq!(store.measure("Ab", f32::NAN), 0.0);
        assert!(store.measure("Ab", f32::INFINITY).is_finite());
        // A combining mark adds nothing to the width, as there, and its
        // offset is on whole pixels too: the acute over `x` is 90 units
        // left of the pen, 0.7 pixels at 16px, so one.
        let marked = store.shape("x\u{301}", 16.0);
        assert_eq!(store.measure("x\u{301}", 16.0), store.measure("x", 16.0));
        assert_eq!((marked[0].x, marked[1].x), (0.0, 8.0));
    }

    #[test]
    fn metrics_are_hheas_rounded_to_whole_pixels_as_the_reference_browser_has_them() {
        // DejaVu Sans does not set OS/2's USE_TYPO_METRICS: its `hhea`
        // ascender 1901 and descender 483 (of 2048) are taken, not its
        // typographic 1556, 492 and 410, and each is rounded at the
        // size, half up. The lines and baselines the reference browser
        // gives it, as measured (M17).
        let store = test_store();
        for (size, ascent, descent) in [
            (1.0, 1.0, 0.0),
            (3.0, 3.0, 1.0),
            (7.0, 6.0, 2.0),
            (10.5, 10.0, 2.0),
            (12.0, 11.0, 3.0),
            (13.0, 12.0, 3.0),
            (16.0, 15.0, 4.0),
            (21.0, 19.0, 5.0),
            (24.0, 22.0, 6.0),
            (48.0, 45.0, 11.0),
        ] {
            let m = store.metrics(size);
            assert_eq!(
                (m.ascent, m.descent, m.line_gap),
                (ascent, descent, 0.0),
                "at {size}px"
            );
        }
    }

    #[test]
    fn metrics_scale_with_font_size_and_are_non_negative() {
        let store = test_store();
        let small = store.metrics(10.0);
        let big = store.metrics(20.0);
        assert!(small.ascent >= 0.0);
        assert!(small.descent >= 0.0);
        assert!(small.line_gap >= 0.0);
        assert!(
            big.ascent > small.ascent,
            "ascent should grow with font size"
        );
        assert_eq!(
            small.line_height(),
            small.ascent + small.descent + small.line_gap
        );
    }

    #[test]
    fn itemize_pure_latin_text_is_one_run() {
        let runs = itemize("hello world");
        assert_eq!(runs, vec![(0..11, Script::Latin)]);
    }

    #[test]
    fn itemize_splits_latin_and_han() {
        let text = "hello世界";
        let runs = itemize(text);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].1, Script::Latin);
        assert_eq!(&text[runs[0].0.clone()], "hello");
        assert_eq!(runs[1].1, Script::Han);
        assert_eq!(&text[runs[1].0.clone()], "世界");
    }

    #[test]
    fn itemize_merges_common_characters_into_the_adjacent_run() {
        // Digits, spaces, and punctuation (`Common`) don't split a run
        // on their own — this stays a single Latin run end to end.
        let text = "hello, world! 123";
        let runs = itemize(text);
        assert_eq!(runs, vec![(0..text.len(), Script::Latin)]);
    }

    #[test]
    fn itemize_leading_common_text_is_claimed_by_the_first_real_script() {
        // Leading digits with no preceding script get retroactively
        // claimed by whatever real script follows, rather than
        // splitting off into their own pointless `Common` run.
        let text = "123世界";
        let runs = itemize(text);
        assert_eq!(runs, vec![(0..text.len(), Script::Han)]);
    }

    #[test]
    fn itemize_a_lone_common_run_with_no_strong_script_stays_common() {
        assert_eq!(itemize("123"), vec![(0..3, Script::Common)]);
    }

    #[test]
    fn itemize_inherited_characters_do_not_claim_an_unclaimed_common_run() {
        // A combining mark (`Inherited`) is not a *strong* script —
        // unlike `itemize_leading_common_text_is_claimed_by_the_first_
        // real_script`, it must not retroactively claim a still-`Common`
        // run the way a real script (e.g. Han) does.
        let text = " \u{0301}"; // space, then a combining acute accent
        assert_eq!(itemize(text), vec![(0..text.len(), Script::Common)]);
    }

    #[test]
    fn itemize_empty_input_is_empty() {
        assert_eq!(itemize(""), Vec::new());
    }

    #[test]
    fn fallback_families_cover_the_scoped_cjk_scripts() {
        for script in [
            Script::Han,
            Script::Hiragana,
            Script::Katakana,
            Script::Hangul,
        ] {
            assert!(
                fallback_families_for(script).is_some(),
                "{script:?} should have a fallback list"
            );
        }
    }

    #[test]
    fn fallback_families_has_no_entry_outside_the_scoped_scripts() {
        for script in [
            Script::Latin,
            Script::Common,
            Script::Cyrillic,
            Script::Greek,
            Script::Arabic,
            Script::Hebrew,
        ] {
            assert!(
                fallback_families_for(script).is_none(),
                "{script:?} should fall through to the primary font"
            );
        }
    }

    #[test]
    fn font_collection_measure_matches_primary_for_pure_latin_text() {
        let fonts = test_fonts();
        // Latin has no fallback entry, so `FontCollection::measure`
        // must agree exactly with plain `FontStore::measure` on the
        // primary font.
        assert_eq!(
            fonts.measure("hello world", 16.0),
            fonts.primary().measure("hello world", 16.0)
        );
    }

    #[test]
    fn font_collection_measure_sums_per_run_widths_with_the_right_font_each() {
        let fonts = test_fonts();
        let text = "hello世界";
        // Independently assembled from the same public building blocks
        // `FontCollection::measure` itself uses, so this still catches
        // a regression where it stopped itemizing/dispatching at all
        // (e.g. a "simplification" to always measure against the
        // primary font) on a machine with a real CJK fallback font —
        // not just restating the implementation.
        let expected: f32 = itemize(text)
            .iter()
            .map(|(range, script)| fonts.font_for(*script).measure(&text[range.clone()], 16.0))
            .sum();
        assert_eq!(fonts.measure(text, 16.0), expected);
    }

    #[test]
    fn font_collection_metrics_for_text_matches_primary_for_pure_latin_text() {
        let fonts = test_fonts();
        assert_eq!(
            fonts.metrics_for_text("hello", 16.0),
            fonts.primary().metrics(16.0)
        );
    }

    #[test]
    fn font_collection_metrics_for_text_is_zeroed_for_empty_input() {
        let fonts = test_fonts();
        let m = fonts.metrics_for_text("", 16.0);
        assert_eq!(m.ascent, 0.0);
        assert_eq!(m.descent, 0.0);
        assert_eq!(m.line_gap, 0.0);
    }

    #[test]
    fn font_collection_shape_matches_primary_for_pure_latin_text() {
        let fonts = test_fonts();
        let via_collection: Vec<ShapedGlyph> = fonts
            .shape("hello", 16.0)
            .into_iter()
            .map(|(_, g)| g)
            .collect();
        assert_eq!(via_collection, fonts.primary().shape("hello", 16.0));
    }

    #[test]
    fn font_collection_shape_tags_each_glyph_with_its_run_script() {
        let fonts = test_fonts();
        let glyphs = fonts.shape("hello世界", 16.0);
        if glyphs.is_empty() {
            return; // no measurable glyphs for this system font
        }
        // "hello世界" itemizes into a Latin run then a Han run (see
        // `itemize_splits_latin_and_han`) — shaping never drops a
        // character (an unsupported one still maps to *some* glyph,
        // typically `.notdef`), so every shaped glyph must carry one of
        // exactly those two tags, with at least one of each present.
        assert!(
            glyphs
                .iter()
                .all(|(s, _)| *s == Script::Latin || *s == Script::Han),
            "{glyphs:?}"
        );
        assert!(
            glyphs.iter().any(|(s, _)| *s == Script::Latin),
            "{glyphs:?}"
        );
        assert!(glyphs.iter().any(|(s, _)| *s == Script::Han), "{glyphs:?}");
    }

    #[test]
    fn font_collection_shape_continues_the_pen_position_across_runs() {
        let fonts = test_fonts();
        let width = fonts.measure("hello世界", 16.0);
        if width <= 0.0 {
            return;
        }
        let glyphs = fonts.shape("hello世界", 16.0);
        let last_x = glyphs.last().map(|(_, g)| g.x).unwrap_or(0.0);
        // Same "pen-start, not full advance" relationship as
        // `FontStore::shape` within a single run (see
        // `measure_is_at_least_as_wide_as_shapes_last_glyph_start`),
        // just carried across the run boundary: the last glyph's own
        // pen-start must land strictly before the text's total
        // measured width, not reset back near 0 by a dropped offset.
        assert!(
            last_x < width,
            "last glyph (x={last_x}) must start before the total width ({width})"
        );
    }

    #[test]
    fn bidi_reorder_leaves_pure_ltr_text_unchanged() {
        assert_eq!(bidi_reorder("hello world", None), "hello world");
        assert_eq!(
            bidi_reorder("hello world", Some(Direction::Ltr)),
            "hello world"
        );
    }

    #[test]
    fn bidi_reorder_reverses_a_pure_rtl_run() {
        // Hebrew "shalom" (hello). A single-direction run's visual order
        // is exactly its logical order reversed — no weak/neutral
        // characters here to complicate the result.
        let hebrew = "שלום";
        let reversed: String = hebrew.chars().rev().collect();
        assert_eq!(bidi_reorder(hebrew, Some(Direction::Rtl)), reversed);
    }

    #[test]
    fn bidi_reorder_auto_detects_rtl_via_first_strong() {
        let hebrew = "שלום";
        // `None` (no explicit `dir`) must resolve the same way as
        // explicit `Rtl` for text that *is* unambiguously RTL — that's
        // what UAX#9's first-strong detection (P2/P3) is for.
        assert_eq!(
            bidi_reorder(hebrew, None),
            bidi_reorder(hebrew, Some(Direction::Rtl))
        );
    }

    #[test]
    fn bidi_reorder_keeps_embedded_digits_in_forward_order() {
        // Digits are internally LTR even inside an RTL run (UAX#9's
        // weak-type resolution) — "123" must never come out as "321",
        // regardless of exactly how the surrounding Hebrew words move.
        let mixed = "שלום 123 עולם";
        let out = bidi_reorder(mixed, Some(Direction::Rtl));
        let find = |s: &str, needle: char| s.find(needle);
        let (p1, p2, p3) = (find(&out, '1'), find(&out, '2'), find(&out, '3'));
        assert!(
            p1.is_some() && p2.is_some() && p3.is_some(),
            "all three digits present: {out:?}"
        );
        assert!(
            p1 < p2 && p2 < p3,
            "digits must stay in forward order within {out:?}"
        );
    }

    #[test]
    fn bidi_reorder_empty_input_is_empty() {
        assert_eq!(bidi_reorder("", None), "");
        assert_eq!(bidi_reorder("", Some(Direction::Ltr)), "");
    }

    #[test]
    fn rasterize_nonblank_glyph_has_coverage() {
        let store = test_store();
        let glyphs = store.shape("A", 28.0);
        let Some(g) = glyphs.first() else {
            return;
        };
        let mask = rasterize_glyph(store.glyph_font(), g.glyph_id, 28.0);
        assert!(mask.width > 0 && mask.height > 0, "glyph mask is empty");
        assert!(
            mask.coverage.iter().any(|&a| a > 0),
            "glyph mask has no coverage"
        );
    }

    #[test]
    fn a_system_font_loads_or_the_machine_has_none_installed() {
        // The one test that asks the system: a machine with any font
        // at all must find one. The others use the bundled font.
        match FontCollection::new() {
            Ok(fonts) => assert!(fonts.primary().measure("x", 16.0) > 0.0),
            Err(FontError::NoFontFound) => {
                eprintln!("no system font installed; the bundled font covers the rest")
            }
            Err(e) => panic!("a system font was found but failed to load: {e}"),
        }
    }

    #[test]
    fn x_height_and_zero_advance_come_from_the_font() {
        // DejaVu Sans: an x-height a little over half the em (the OS/2
        // table says 1120 of 2048 units), and a zero as wide as `measure`
        // says. Both scale with the size.
        let fonts = test_fonts();
        let ex = fonts.x_height(16.0);
        assert!((8.0..10.4).contains(&ex), "{ex}");
        assert_eq!(fonts.x_height(32.0), ex * 2.0);
        assert_eq!(fonts.zero_advance(16.0), fonts.measure("0", 16.0));
        assert!(fonts.zero_advance(16.0) > 0.0);
        // A hostile size does not break either.
        assert!(fonts.x_height(0.0) >= 0.0);
        assert!(fonts.zero_advance(-1.0).is_finite());
    }

    #[test]
    fn rasterize_refuses_sizes_that_would_allocate_gigabytes() {
        let store = test_store();
        let glyph_id = store.shape("M", 16.0)[0].glyph_id;
        for size in [1e9, 50_000.0, f32::INFINITY, f32::NAN, -16.0, 0.0] {
            let mask = rasterize_glyph(store.glyph_font(), glyph_id, size);
            assert_eq!(mask.width, 0, "size {size}");
            assert!(mask.coverage.is_empty(), "size {size}");
        }
        // A large but affordable size still draws.
        let mask = rasterize_glyph(store.glyph_font(), glyph_id, 1000.0);
        assert!(mask.width > 100 && (mask.width as u64) * (mask.height as u64) <= MAX_MASK_PIXELS);
    }

    #[test]
    fn bidi_reorder_keeps_every_paragraph() {
        // Every Bidi class B character starts a new paragraph; the text
        // after it used to be dropped.
        for sep in ['\n', '\r', '\u{85}', '\u{2029}', '\u{1c}'] {
            let text = format!("one{sep}two three");
            assert_eq!(
                bidi_reorder(&text, Some(Direction::Ltr)),
                text,
                "separator {sep:?}"
            );
        }
        assert_eq!(bidi_reorder("one\u{2029}two", None), "one\u{2029}two");
    }
}
