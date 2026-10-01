//! Computed values: the result of resolving the cascade for one element.
//!
//! [`ComputedStyle`] is the v1 field set from the roadmap (M5.1): color,
//! font size/family, background color, `display`, the box-model edges
//! (margin/padding/border-width, one value per side), and a single
//! border color + style (per-side border color/style is a later
//! refinement). [`ComputedStyle::default`] holds each property's CSS
//! *initial* value — what an element gets when nothing in the cascade
//! sets it and (for non-inherited properties) its parent's value doesn't
//! apply either.

/// An RGBA color, 0.0–1.0 channels (see `paint::Rgba8` for the 8-bit
/// form `render` actually blits — `style` owns its own color type per
/// the layering contract: `paint` sits *above* `style` and converts at
/// the boundary, not the other way around).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// Red channel.
    pub r: f32,
    /// Green channel.
    pub g: f32,
    /// Blue channel.
    pub b: f32,
    /// Alpha channel.
    pub a: f32,
}

impl Color {
    /// An opaque color from 0–255 channels (convenience for the named
    /// and hex-color tables).
    pub const fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Color {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: 1.0,
        }
    }

    /// Fully transparent black — `background-color`'s initial value.
    pub const TRANSPARENT: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    /// Opaque black — the conventional UA default for `color`.
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
}

/// `display` (css-display-3, v1 subset: box generation only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    /// Generates a block-level box.
    Block,
    /// Generates an inline-level box. The initial value.
    Inline,
    /// Generates no box at all — the element and its descendants are
    /// skipped entirely by layout and paint.
    None,
}

/// `border-style` (v1 subset: enough to decide whether a border paints
/// at all — CSS2 §8.5.3: a `none` border computes to zero width
/// regardless of `border-width`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    /// No border is painted. The initial value.
    None,
    /// A solid border is painted.
    Solid,
}

/// `text-align` (M7.5 v1: left/right/center only — `justify` is
/// deferred to M21, per the roadmap's own note on that item).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    /// The initial value.
    Left,
    Right,
    Center,
}

/// `text-transform` (M8.4 v1: the common case-mapping keywords —
/// `full-width`/`full-size-kana` are a CJK-specific refinement left for
/// later).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTransform {
    /// The initial value.
    None,
    Uppercase,
    Lowercase,
    /// Title-cases: the first letter of each word, uppercased.
    Capitalize,
}

/// `white-space` (M8.4 v1: the four classic values — `break-spaces` is
/// a finer-grained css-text-4 addition left for later).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpace {
    /// Collapse whitespace runs to one space; wrap at them. The initial
    /// value.
    Normal,
    /// Preserve whitespace verbatim (including newlines as forced
    /// breaks); never wrap.
    Pre,
    /// Preserve whitespace verbatim; wrap at it anyway.
    PreWrap,
    /// Collapse whitespace runs to one space, but keep newlines as
    /// forced breaks; wrap at ordinary spaces.
    PreLine,
}

/// `overflow` (M9.3 v1: the shorthand's four keywords, one value for
/// both axes — `overflow-x`/`overflow-y` are a later refinement).
/// Whether content is clipped is what this engine acts on today;
/// scrolling an element of its own is M10.5, and until then `scroll`
/// and `auto` clip exactly as `hidden` does (CSS2 §11.1.1: all three
/// clip to the padding box; only the viewport scrolls, M9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    /// Content is not clipped. The initial value.
    Visible,
    Hidden,
    Scroll,
    Auto,
}

impl Overflow {
    /// Whether this value clips the element's content to its padding
    /// box (every value but `visible`).
    pub fn clips(self) -> bool {
        self != Overflow::Visible
    }
}

/// Text direction (M8.3) — read from the HTML `dir` attribute (there's
/// no CSS `direction` property parsing yet; `dir` is the only source for
/// now, applied the same way the inline `style` attribute is: a direct
/// per-element lookup in `cascade`, not a normal cascaded declaration).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// The initial value.
    Ltr,
    Rtl,
}

/// A `<length-percentage>` value, or `auto` (css-values-4 §6.3) — used
/// for `width`/`height` (M6, which accept all three) and `margin` (which
/// accepts `auto` and `<length>`, but not yet `<percentage>` — v1 scope,
/// see `cascade`'s module docs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthPercentageAuto {
    /// The `auto` keyword — its meaning is property- (and even
    /// context-) specific, so callers resolve it themselves; see
    /// [`LengthPercentageAuto::resolve`].
    Auto,
    /// An absolute length, in CSS pixels.
    Px(f32),
    /// A percentage, as written (`50%` is `Percent(50.0)`, not `0.5`) —
    /// resolved against a containing-block dimension at layout time.
    Percent(f32),
}

impl LengthPercentageAuto {
    /// Resolves against `basis` (the relevant containing-block dimension,
    /// for a percentage); `auto` resolves to whatever the caller's
    /// algorithm says `auto` means here (there's no one universal value).
    pub fn resolve(&self, basis: f32, if_auto: f32) -> f32 {
        match self {
            LengthPercentageAuto::Auto => if_auto,
            LengthPercentageAuto::Px(v) => *v,
            LengthPercentageAuto::Percent(p) => basis * p / 100.0,
        }
    }

    /// Whether this is the `auto` keyword.
    pub fn is_auto(&self) -> bool {
        matches!(self, LengthPercentageAuto::Auto)
    }
}

/// A `<length-percentage>` (css-values-4 §6.3), kept as written until
/// layout resolves the percentage against the containing block (M10.1):
/// `padding` and the min and max sizes take this.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthPercentage {
    /// An absolute length, in CSS pixels.
    Px(f32),
    /// A percentage, as written (`50%` is `Percent(50.0)`).
    Percent(f32),
}

impl LengthPercentage {
    /// Resolves against `basis`, the containing-block dimension the
    /// property's percentage refers to.
    pub fn resolve(&self, basis: f32) -> f32 {
        match self {
            LengthPercentage::Px(v) => *v,
            LengthPercentage::Percent(p) => basis * p / 100.0,
        }
    }
}

/// The box-model edges, one value per side, in CSS's `top, right,
/// bottom, left` order (matches `margin`/`padding`/`border-width`
/// shorthand order, so parsing and storage share one shape).
pub type Edges = [f32; 4];

/// Like [`Edges`], but for `margin`, whose values can be `auto`.
pub type MarginEdges = [LengthPercentageAuto; 4];

/// Like [`Edges`], but for `padding`, whose values can be percentages
/// (of the containing block's width, every side; CSS2 §8.4).
pub type PaddingEdges = [LengthPercentage; 4];

/// Index into an [`Edges`] array for the top edge (the rest follow the
/// same clockwise order: right, bottom, left).
pub const TOP: usize = 0;
/// Index into an [`Edges`] array for the right edge.
pub const RIGHT: usize = 1;
/// Index into an [`Edges`] array for the bottom edge.
pub const BOTTOM: usize = 2;
/// Index into an [`Edges`] array for the left edge.
pub const LEFT: usize = 3;

/// A resolved set of CSS property values for one element (roadmap M5.1).
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    /// `color` — inherited.
    pub color: Color,
    /// `background-color` — not inherited.
    pub background_color: Color,
    /// `font-size`, resolved to CSS pixels — inherited. `em` in an
    /// element's own `font-size` resolves against the *parent's*
    /// font-size (a property can't be relative to itself).
    pub font_size_px: f32,
    /// `font-family`, as declared (comma-separated family list, kept
    /// as one string) — inherited. Still not consumed by `text`'s font
    /// selection: M8.1 added per-*script* fallback (`text::
    /// FontCollection`), not per-declared-family selection — this
    /// string is parsed and carried through cascade/inheritance and
    /// then simply never read. See `cascade::parse_font_family`'s doc
    /// for why that's a real, separately-scoped gap, not an oversight.
    pub font_family: String,
    /// `display` — not inherited (every element's own box generation is
    /// independent of its parent's).
    pub display: Display,
    /// `text-align` (M7.5) — inherited (css-text-3 §7.1 lists it among
    /// the inherited properties): it describes how *this* element's own
    /// inline content is aligned, but a block that itself has no direct
    /// inline content still passes the value down to descendants that do.
    pub text_align: TextAlign,
    /// `text-transform` (M8.4) — inherited.
    pub text_transform: TextTransform,
    /// `white-space` (M8.4) — inherited.
    pub white_space: WhiteSpace,
    /// `letter-spacing`, CSS pixels, extra space after each character —
    /// inherited. `0.0` (the initial value) is the common case, kept
    /// cheap: `layout::inline` only splits a word into per-character
    /// fragments when this is non-zero (see its module docs).
    pub letter_spacing: f32,
    /// `word-spacing`, CSS pixels, *extra* space added on top of a
    /// normal space's own width between words — inherited.
    pub word_spacing: f32,
    /// `dir`/`direction` (M8.3) — inherited; see [`Direction`]'s doc for
    /// why this is read differently than most properties here.
    pub direction: Direction,
    /// `width` (M6) — not inherited. `auto` (the initial value) means
    /// "fill the containing block" in block layout (CSS2.1 §10.3.3);
    /// other formatting contexts (inline, later) mean something else,
    /// which is why this stays unresolved (not a plain `f32`) until a
    /// layout algorithm resolves it against a containing block.
    pub width: LengthPercentageAuto,
    /// `height` (M6) — not inherited. `auto` (the initial value) means
    /// "as tall as the content" in block layout. Percentage heights
    /// aren't resolved yet (roadmap M6.4 defers that to M10) — the
    /// *parser* already keeps `Percent` for `width`, but for `height` it
    /// deliberately never produces one yet (see `cascade`).
    pub height: LengthPercentageAuto,
    /// `margin-{top,right,bottom,left}` — not inherited. Unlike
    /// `padding`/`border-width`, `margin` accepts `auto` (M6.3's
    /// auto-centering).
    pub margin: MarginEdges,
    /// `padding-{top,right,bottom,left}` — not inherited. A percentage
    /// resolves against the containing block's *width*, for the vertical
    /// sides too (CSS2 §8.4), which only layout knows.
    pub padding: PaddingEdges,
    /// `border-{top,right,bottom,left}-width`, CSS pixels — not
    /// inherited. Per `border_style` below, a `none` border always
    /// paints as zero-width regardless of this value (CSS2 §8.5.3) —
    /// callers that need the *painted* width should check `border_style`
    /// first rather than reading `border_width` alone.
    pub border_width: Edges,
    /// `border-color` (v1: one shared color, not per-side) — not
    /// inherited. Defaults to `color` when unset (`currentcolor` is the
    /// spec initial value) — resolved once the rest of the cascade for
    /// this element has been applied, see `cascade::compute_style`.
    pub border_color: Color,
    /// `border-style` (v1: one shared style, not per-side) — not
    /// inherited.
    pub border_style: BorderStyle,
    /// `overflow` (M9.3) — not inherited.
    pub overflow: Overflow,
}

impl Default for ComputedStyle {
    /// The CSS initial values for the v1 property set.
    fn default() -> Self {
        ComputedStyle {
            color: Color::BLACK,
            background_color: Color::TRANSPARENT,
            font_size_px: 16.0, // 'medium', by UA convention
            font_family: "sans-serif".to_string(),
            display: Display::Inline,
            text_align: TextAlign::Left,
            text_transform: TextTransform::None,
            white_space: WhiteSpace::Normal,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            direction: Direction::Ltr,
            width: LengthPercentageAuto::Auto,
            height: LengthPercentageAuto::Auto,
            margin: [LengthPercentageAuto::Px(0.0); 4], // margin's initial value is `0`, not `auto`
            padding: [LengthPercentage::Px(0.0); 4],
            border_width: [0.0; 4],
            border_color: Color::BLACK, // corrected to `color` post-cascade
            border_style: BorderStyle::None,
            overflow: Overflow::Visible,
        }
    }
}

impl ComputedStyle {
    /// The starting point for a child's computed style: inherited
    /// properties carry the parent's *computed* values forward
    /// (css-cascade-5 §7.1); everything else starts at its initial
    /// value, exactly like [`ComputedStyle::default`]. The cascade then
    /// overwrites whatever this element's own matched declarations set.
    pub fn inherited_from(parent: &ComputedStyle) -> Self {
        ComputedStyle {
            color: parent.color,
            font_size_px: parent.font_size_px,
            font_family: parent.font_family.clone(),
            text_align: parent.text_align,
            text_transform: parent.text_transform,
            white_space: parent.white_space,
            letter_spacing: parent.letter_spacing,
            word_spacing: parent.word_spacing,
            direction: parent.direction,
            ..ComputedStyle::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_css_initial_values() {
        let s = ComputedStyle::default();
        assert_eq!(s.color, Color::BLACK);
        assert_eq!(s.background_color, Color::TRANSPARENT);
        assert_eq!(s.display, Display::Inline);
        assert_eq!(s.text_align, TextAlign::Left);
        assert_eq!(s.text_transform, TextTransform::None);
        assert_eq!(s.white_space, WhiteSpace::Normal);
        assert_eq!(s.letter_spacing, 0.0);
        assert_eq!(s.word_spacing, 0.0);
        assert_eq!(s.direction, Direction::Ltr);
        assert_eq!(s.width, LengthPercentageAuto::Auto);
        assert_eq!(s.height, LengthPercentageAuto::Auto);
        assert_eq!(s.margin, [LengthPercentageAuto::Px(0.0); 4]);
        assert_eq!(s.border_style, BorderStyle::None);
        assert_eq!(s.overflow, Overflow::Visible);
        assert!(!s.overflow.clips());
    }

    #[test]
    fn inherited_from_carries_only_inherited_properties() {
        let parent = ComputedStyle {
            color: Color::rgb8(255, 0, 0),
            font_size_px: 20.0,
            text_align: TextAlign::Center,
            background_color: Color::rgb8(0, 255, 0), // not inherited
            display: Display::Block,                  // not inherited
            ..ComputedStyle::default()
        };

        let child = ComputedStyle::inherited_from(&parent);
        assert_eq!(child.color, parent.color);
        assert_eq!(child.font_size_px, parent.font_size_px);
        assert_eq!(child.text_align, TextAlign::Center);
        assert_eq!(child.text_transform, parent.text_transform);
        assert_eq!(child.white_space, parent.white_space);
        assert_eq!(child.letter_spacing, parent.letter_spacing);
        assert_eq!(child.word_spacing, parent.word_spacing);
        assert_eq!(child.direction, parent.direction);
        // Non-inherited properties reset to their initial value, not the
        // parent's — a block parent's child doesn't become block-by-default.
        assert_eq!(child.background_color, Color::TRANSPARENT);
        assert_eq!(child.display, Display::Inline);
    }

    #[test]
    fn length_percentage_resolves_each_variant() {
        assert_eq!(LengthPercentage::Px(10.0).resolve(200.0), 10.0);
        assert_eq!(LengthPercentage::Percent(25.0).resolve(200.0), 50.0);
    }

    #[test]
    fn length_percentage_auto_resolves_each_variant() {
        assert_eq!(LengthPercentageAuto::Auto.resolve(200.0, 42.0), 42.0);
        assert_eq!(LengthPercentageAuto::Px(10.0).resolve(200.0, 42.0), 10.0);
        assert_eq!(
            LengthPercentageAuto::Percent(50.0).resolve(200.0, 42.0),
            100.0
        );
        assert!(LengthPercentageAuto::Auto.is_auto());
        assert!(!LengthPercentageAuto::Px(0.0).is_auto());
    }
}
