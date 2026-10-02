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

use std::sync::Arc;

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

/// `display` (css-display-3, the box-generation values of CSS2 §9.2.4
/// and the table values of §17.2, M12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    /// Generates a block-level box.
    Block,
    /// Generates an inline-level box. The initial value.
    Inline,
    /// Generates an inline-level box that is a block container inside
    /// (CSS2 §9.2.4, M10.4): atomic on its line, its content laid out
    /// as a block formatting context of its own.
    InlineBlock,
    /// Generates no box at all — the element and its descendants are
    /// skipped entirely by layout and paint.
    None,
    /// A block-level table (CSS2 §17.2).
    Table,
    /// An inline-level table: atomic on its line like an inline-block.
    InlineTable,
    /// A group of rows: `tbody`.
    TableRowGroup,
    /// A group of rows displayed before every other row: `thead`.
    TableHeaderGroup,
    /// A group of rows displayed after every other row: `tfoot`.
    TableFooterGroup,
    /// A row of cells: `tr`.
    TableRow,
    /// A group of columns: `colgroup`.
    TableColumnGroup,
    /// A column: `col`. Its box holds no content, only a column of the
    /// grid.
    TableColumn,
    /// A cell: `td`, `th`.
    TableCell,
    /// A table's caption: `caption`.
    TableCaption,
}

impl Display {
    /// Whether this value is an atomic inline-level box (CSS2 §9.2.2):
    /// an inline-block or an inline table, one item on its line and a
    /// formatting context of its own inside.
    pub fn is_atomic_inline(self) -> bool {
        matches!(self, Display::InlineBlock | Display::InlineTable)
    }

    /// The value a float or an absolutely positioned box computes to
    /// (CSS2 §9.7's table, css-display-3 §2.7's blockification): an
    /// inline table becomes a table; an inline box, an inline-block and
    /// every table-internal box become a block; a block, a table and
    /// `none` stay what they are.
    pub fn blockified(self) -> Display {
        match self {
            Display::Block | Display::Table | Display::None => self,
            Display::InlineTable => Display::Table,
            Display::Inline
            | Display::InlineBlock
            | Display::TableRowGroup
            | Display::TableHeaderGroup
            | Display::TableFooterGroup
            | Display::TableRow
            | Display::TableColumnGroup
            | Display::TableColumn
            | Display::TableCell
            | Display::TableCaption => Display::Block,
        }
    }
}

/// `table-layout` (CSS2 §17.5.2, M12): which algorithm sizes a table's
/// columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableLayout {
    /// The automatic algorithm, from the cells' content. The initial
    /// value.
    Auto,
    /// The fixed algorithm, from the table's width and the first row.
    Fixed,
}

/// `border-collapse` (CSS2 §17.6, M12): which border model a table
/// uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderCollapse {
    /// Every cell has its own border, `border-spacing` between them.
    /// The initial value.
    Separate,
    /// Adjacent borders are merged into one, by §17.6.2's conflict
    /// rules.
    Collapse,
}

/// `border-spacing` (CSS2 §17.6.1, M12): the distance between adjacent
/// cells' borders in the separated model, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderSpacing {
    /// Between columns, and between the table's border and its first
    /// and last columns.
    pub horizontal: f32,
    /// Between rows, and between the table's border and its first and
    /// last rows.
    pub vertical: f32,
}

/// `caption-side` (CSS2 §17.4.1, M12): where a table's caption goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionSide {
    /// Above the table box. The initial value.
    Top,
    /// Below it.
    Bottom,
}

/// `empty-cells` (CSS2 §17.6.1.1, M12): whether a cell with no content
/// paints its background and borders in the separated model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyCells {
    /// It does. The initial value.
    Show,
    /// It paints nothing.
    Hide,
}

/// `vertical-align` (CSS2 §10.8.1 and §17.5.3, M12), parsed in full.
/// This engine reads it on table cells alone, where `top`, `middle`
/// and `bottom` align the cell in its row as §17.5.3 says and every
/// other value is `baseline`, as §17.5.3 says too; its meaning on
/// inline boxes is M42's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VerticalAlign {
    /// The initial value.
    Baseline,
    Sub,
    Super,
    Top,
    TextTop,
    Middle,
    Bottom,
    TextBottom,
    /// A length, in CSS pixels: the baseline raised by it (negative,
    /// lowered).
    Length(f32),
    /// A percentage of the line height, as written.
    Percent(f32),
}

/// `border-style` (CSS2 §8.5.3, all ten values since M10). `none` and
/// `hidden` paint nothing and compute to a zero width regardless of
/// `border-width`; the others paint, `solid` and `double` exactly and
/// the rest as `paint` approximates them (see its border painting).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    /// No border is painted. The initial value.
    None,
    /// As `none`, except in table border conflicts (M12).
    Hidden,
    Solid,
    Double,
    Dashed,
    Dotted,
    Groove,
    Ridge,
    Inset,
    Outset,
}

impl BorderStyle {
    /// Whether this style paints anything — and so whether the side's
    /// width counts (CSS2 §8.5.3).
    pub fn paints(self) -> bool {
        !matches!(self, BorderStyle::None | BorderStyle::Hidden)
    }
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

/// `box-sizing` (css-sizing-3 §4.1, M10.2): which box the `width`,
/// `height` and the min and max sizes measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxSizing {
    /// The content box. The initial value.
    ContentBox,
    /// The border box: padding and border are inside the given size.
    BorderBox,
}

/// `float` (CSS2 §9.5.1, M11.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Float {
    /// In flow. The initial value.
    None,
    Left,
    Right,
}

/// `clear` (CSS2 §9.5.2, M11.1): which floats a box must be below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clear {
    /// The initial value.
    None,
    Left,
    Right,
    Both,
}

/// `position` (CSS2 §9.3.1 and css-position-3 for `sticky`, M11.3 to
/// M11.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// In flow, the offsets ignored. The initial value.
    Static,
    /// In flow, then moved by the offsets.
    Relative,
    /// Out of flow, against the nearest positioned ancestor's padding
    /// box or the initial containing block.
    Absolute,
    /// Out of flow, against the viewport, not scrolling with the document.
    Fixed,
    /// In flow, shifted to keep its offsets from the scrollport's edges
    /// while its containing block allows.
    Sticky,
}

impl Position {
    /// Whether a box with this value is positioned (CSS2 §9.3.1): it
    /// is the containing block of its absolutely positioned descendants
    /// and takes part in stacking (M11.6).
    pub fn is_positioned(self) -> bool {
        self != Position::Static
    }

    /// Whether a box with this value is taken out of the normal flow.
    pub fn is_out_of_flow(self) -> bool {
        matches!(self, Position::Absolute | Position::Fixed)
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

/// `content`'s computed value (CSS 2.2 §12.2, M16), which only
/// `::before` and `::after` read.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Content {
    /// `normal`, the initial value; on `::before` and `::after` it
    /// computes to `none`.
    #[default]
    Normal,
    /// `none`: no box.
    None,
    /// The parts, in order: strings with each `attr()` replaced by the
    /// attribute's value, and the quote keywords. A `url()` is left out
    /// until images (M23), so the list may be empty: a box with nothing
    /// in it.
    Items(Arc<[ContentItem]>),
}

/// One part of a [`Content`] list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentItem {
    /// A string, or the value of an `attr()`.
    Text(String),
    /// `open-quote`: the opening mark at the current depth, which it
    /// then deepens.
    OpenQuote,
    /// `close-quote`: makes the depth shallower, then gives its closing
    /// mark.
    CloseQuote,
    /// `no-open-quote`: deepens without a mark.
    NoOpenQuote,
    /// `no-close-quote`: makes the depth shallower without a mark.
    NoCloseQuote,
}

/// `quotes`' computed value (CSS 2.2 §12.3.1, M16).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Quotes {
    /// `auto`, the initial value: “ ” and ‘ ’ in every language, until
    /// the choice by language that needs locale data (M42).
    #[default]
    Auto,
    /// `none`: the quote keywords give no marks.
    None,
    /// The pairs of opening and closing marks, outermost first; never
    /// empty.
    Pairs(Arc<[(String, String)]>),
}

impl Quotes {
    /// The opening and closing marks for a quote `depth` levels deep,
    /// the outermost being 0: past the last pair, the last pair (CSS 2.2
    /// §12.3.2); `None` for `quotes: none`.
    pub fn marks(&self, depth: usize) -> Option<(&str, &str)> {
        const AUTO: [(&str, &str); 2] = [("\u{201C}", "\u{201D}"), ("\u{2018}", "\u{2019}")];
        match self {
            Quotes::Auto => Some(AUTO[depth.min(AUTO.len() - 1)]),
            Quotes::None => None,
            Quotes::Pairs(pairs) => pairs
                .get(depth.min(pairs.len().saturating_sub(1)))
                .map(|(open, close)| (open.as_str(), close.as_str())),
        }
    }
}

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
    /// inherited. The initial value is `medium`, 3px. A side whose
    /// style paints nothing has a used width of zero regardless of this
    /// value (CSS2 §8.5.3): callers that need the width that counts
    /// read [`ComputedStyle::used_border_width`], never this alone.
    pub border_width: Edges,
    /// `border-{top,right,bottom,left}-color` (per side since M10) —
    /// not inherited. Defaults to `color` when unset (`currentcolor` is
    /// the spec initial value) — resolved once the rest of the cascade
    /// for this element has been applied, see `cascade::compute_style`.
    pub border_color: [Color; 4],
    /// `border-{top,right,bottom,left}-style` (per side since M10) —
    /// not inherited.
    pub border_style: [BorderStyle; 4],
    /// `overflow` (M9.3) — not inherited.
    pub overflow: Overflow,
    /// `box-sizing` (M10.2) — not inherited.
    pub box_sizing: BoxSizing,
    /// `min-width` (M10.3) — not inherited; a percentage of the
    /// containing block's width. The initial value is `0`.
    pub min_width: LengthPercentage,
    /// `max-width` (M10.3) — not inherited; `None` is the initial
    /// `none`.
    pub max_width: Option<LengthPercentage>,
    /// `min-height` (M10.3) — not inherited; a percentage of the
    /// containing block's height when that is definite, else `0`.
    pub min_height: LengthPercentage,
    /// `max-height` (M10.3) — not inherited; `None` is `none`, and so
    /// is a percentage of an indefinite containing-block height.
    pub max_height: Option<LengthPercentage>,
    /// `float` (M11.1) — not inherited. `none` on an absolutely
    /// positioned box whatever was written (CSS2 §9.7).
    pub float: Float,
    /// `clear` (M11.1) — not inherited.
    pub clear: Clear,
    /// `position` (M11.3 to M11.5) — not inherited.
    pub position: Position,
    /// `top`, `right`, `bottom` and `left` (M11.3), in that order like
    /// every edge array — not inherited; kept as written, since a
    /// percentage is of the containing block's height for `top` and
    /// `bottom` and of its width for `left` and `right` (CSS2 §9.3.2),
    /// which only layout knows. `auto` is the initial value.
    pub inset: MarginEdges,
    /// `z-index` (M11.6) — not inherited; `None` is `auto`.
    pub z_index: Option<i32>,
    /// `table-layout` (M12) — not inherited.
    pub table_layout: TableLayout,
    /// `border-collapse` (M12) — inherited.
    pub border_collapse: BorderCollapse,
    /// `border-spacing` (M12) — inherited. The initial value is `0`.
    pub border_spacing: BorderSpacing,
    /// `caption-side` (M12) — inherited.
    pub caption_side: CaptionSide,
    /// `empty-cells` (M12) — inherited.
    pub empty_cells: EmptyCells,
    /// `vertical-align` (M12) — not inherited; the UA sheet's `inherit`
    /// on `tr`, `td` and `th` is what carries a row group's `middle`
    /// down to an HTML cell.
    pub vertical_align: VerticalAlign,
    /// `content` (M16) — not inherited, and set only on `::before` and
    /// `::after`, the pseudo-elements it applies to (CSS 2.2 §12.2); an
    /// element keeps `normal`.
    pub content: Content,
    /// `quotes` (M16) — inherited.
    pub quotes: Quotes,
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
            border_width: [3.0; 4],          // `medium`
            border_color: [Color::BLACK; 4], // corrected to `color` post-cascade
            border_style: [BorderStyle::None; 4],
            overflow: Overflow::Visible,
            box_sizing: BoxSizing::ContentBox,
            min_width: LengthPercentage::Px(0.0),
            max_width: None,
            min_height: LengthPercentage::Px(0.0),
            max_height: None,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            inset: [LengthPercentageAuto::Auto; 4],
            z_index: None,
            table_layout: TableLayout::Auto,
            border_collapse: BorderCollapse::Separate,
            border_spacing: BorderSpacing {
                horizontal: 0.0,
                vertical: 0.0,
            },
            caption_side: CaptionSide::Top,
            empty_cells: EmptyCells::Show,
            vertical_align: VerticalAlign::Baseline,
            content: Content::Normal,
            quotes: Quotes::Auto,
        }
    }
}

impl ComputedStyle {
    /// The border widths that count, per side: `border-width` where the
    /// side's style paints, zero where it is `none` or `hidden` (CSS2
    /// §8.5.3), and whole device pixels either way (css-backgrounds-3
    /// §4.1): a fractional width is rounded down, one between zero and
    /// a pixel up to one, as the reference browser has it (M11). What
    /// layout, paint and hit testing read.
    pub fn used_border_width(&self) -> Edges {
        let mut used = [0.0; 4];
        for (side, width) in used.iter_mut().enumerate() {
            if self.border_style[side].paints() {
                let specified = self.border_width[side];
                *width = if specified > 0.0 {
                    specified.floor().max(1.0)
                } else {
                    0.0
                };
            }
        }
        used
    }

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
            border_collapse: parent.border_collapse,
            border_spacing: parent.border_spacing,
            caption_side: parent.caption_side,
            empty_cells: parent.empty_cells,
            quotes: parent.quotes.clone(),
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
        assert_eq!(s.border_style, [BorderStyle::None; 4]);
        assert_eq!(s.border_width, [3.0; 4], "medium");
        assert_eq!(s.used_border_width(), [0.0; 4], "none paints nothing");
        assert!(!BorderStyle::Hidden.paints() && BorderStyle::Dotted.paints());
        assert_eq!(s.overflow, Overflow::Visible);
        assert!(!s.overflow.clips());
        assert_eq!(s.box_sizing, BoxSizing::ContentBox);
        assert_eq!(s.min_width, LengthPercentage::Px(0.0));
        assert_eq!(s.max_width, None);
        assert_eq!(s.min_height, LengthPercentage::Px(0.0));
        assert_eq!(s.max_height, None);
        assert_eq!(s.float, Float::None);
        assert_eq!(s.clear, Clear::None);
        assert_eq!(s.position, Position::Static);
        assert!(!s.position.is_positioned() && !s.position.is_out_of_flow());
        assert!(Position::Relative.is_positioned() && !Position::Relative.is_out_of_flow());
        assert!(Position::Fixed.is_positioned() && Position::Fixed.is_out_of_flow());
        assert_eq!(s.inset, [LengthPercentageAuto::Auto; 4]);
        assert_eq!(s.z_index, None);
        assert_eq!(s.table_layout, TableLayout::Auto);
        assert_eq!(s.border_collapse, BorderCollapse::Separate);
        assert_eq!(
            (s.border_spacing.horizontal, s.border_spacing.vertical),
            (0.0, 0.0)
        );
        assert_eq!(s.caption_side, CaptionSide::Top);
        assert_eq!(s.empty_cells, EmptyCells::Show);
        assert_eq!(s.vertical_align, VerticalAlign::Baseline);
    }

    #[test]
    fn inherited_from_carries_only_inherited_properties() {
        let parent = ComputedStyle {
            color: Color::rgb8(255, 0, 0),
            font_size_px: 20.0,
            text_align: TextAlign::Center,
            background_color: Color::rgb8(0, 255, 0), // not inherited
            display: Display::Block,                  // not inherited
            border_collapse: BorderCollapse::Collapse,
            border_spacing: BorderSpacing {
                horizontal: 3.0,
                vertical: 4.0,
            },
            caption_side: CaptionSide::Bottom,
            empty_cells: EmptyCells::Hide,
            table_layout: TableLayout::Fixed,      // not inherited
            vertical_align: VerticalAlign::Middle, // not inherited
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
        assert_eq!(child.border_collapse, BorderCollapse::Collapse);
        assert_eq!(child.border_spacing, parent.border_spacing);
        assert_eq!(child.caption_side, CaptionSide::Bottom);
        assert_eq!(child.empty_cells, EmptyCells::Hide);
        // Non-inherited properties reset to their initial value, not the
        // parent's — a block parent's child doesn't become block-by-default.
        assert_eq!(child.background_color, Color::TRANSPARENT);
        assert_eq!(child.display, Display::Inline);
        assert_eq!(child.table_layout, TableLayout::Auto);
        assert_eq!(child.vertical_align, VerticalAlign::Baseline);
    }

    #[test]
    fn display_knows_its_atomic_inlines_and_blockifies_as_css2_section_9_7_says() {
        assert!(Display::InlineBlock.is_atomic_inline());
        assert!(Display::InlineTable.is_atomic_inline());
        assert!(!Display::Inline.is_atomic_inline());
        assert!(!Display::Table.is_atomic_inline());
        for (value, expected) in [
            (Display::Block, Display::Block),
            (Display::Inline, Display::Block),
            (Display::InlineBlock, Display::Block),
            (Display::None, Display::None),
            (Display::Table, Display::Table),
            (Display::InlineTable, Display::Table),
            (Display::TableRowGroup, Display::Block),
            (Display::TableHeaderGroup, Display::Block),
            (Display::TableFooterGroup, Display::Block),
            (Display::TableRow, Display::Block),
            (Display::TableColumnGroup, Display::Block),
            (Display::TableColumn, Display::Block),
            (Display::TableCell, Display::Block),
            (Display::TableCaption, Display::Block),
        ] {
            assert_eq!(value.blockified(), expected, "{value:?}");
        }
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
