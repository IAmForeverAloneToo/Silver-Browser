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

use std::sync::{Arc, LazyLock};

pub use crate::calc::Calc;
use crate::calc::Expression;
use crate::custom::CustomProperties;

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

/// `display` (css-display-3, the box-generation values of CSS2 §9.2.4,
/// the table values of §17.2, M12, the flex values of css-flexbox-1 §3,
/// M17, and the grid values of css-grid-2 §5.1, M18).
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
    /// A block-level flex container (css-flexbox-1 §3, M17): its
    /// children are flex items, laid out along its main axis.
    Flex,
    /// An inline-level flex container: atomic on its line like an
    /// inline-block, a flex container inside.
    InlineFlex,
    /// A block-level grid container (css-grid-2 §5.1, M18): its
    /// children are grid items, placed in its rows and columns.
    Grid,
    /// An inline-level grid container: atomic on its line like an
    /// inline-block, a grid container inside.
    InlineGrid,
}

impl Display {
    /// Whether this value is an atomic inline-level box (CSS2 §9.2.2):
    /// an inline-block, an inline table, an inline flex container or an
    /// inline grid container, one item on its line and a formatting
    /// context of its own inside.
    pub fn is_atomic_inline(self) -> bool {
        matches!(
            self,
            Display::InlineBlock | Display::InlineTable | Display::InlineFlex | Display::InlineGrid
        )
    }

    /// Whether this value makes a flex container (M17), whose children
    /// are flex items and so blockified (css-flexbox-1 §4).
    pub fn is_flex(self) -> bool {
        matches!(self, Display::Flex | Display::InlineFlex)
    }

    /// Whether this value makes a grid container (M18), whose children
    /// are grid items and so blockified (css-grid-2 §6.1).
    pub fn is_grid(self) -> bool {
        matches!(self, Display::Grid | Display::InlineGrid)
    }

    /// Whether this value makes a container whose in-flow children are
    /// its items, blockified: a flex or a grid container.
    pub fn has_items(self) -> bool {
        self.is_flex() || self.is_grid()
    }

    /// The value a float, an absolutely positioned box, a flex item or a
    /// grid item computes to (CSS2 §9.7's table, css-display-3 §2.7's
    /// blockification): an inline table becomes a table, an inline flex
    /// container a flex container and an inline grid container a grid
    /// container; an inline box, an inline-block and every
    /// table-internal box become a block; a block, a table, a flex
    /// container, a grid container and `none` stay what they are.
    pub fn blockified(self) -> Display {
        match self {
            Display::Block | Display::Table | Display::Flex | Display::Grid | Display::None => self,
            Display::InlineTable => Display::Table,
            Display::InlineFlex => Display::Flex,
            Display::InlineGrid => Display::Grid,
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
#[derive(Debug, Clone, PartialEq)]
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
    /// A `calc()` with a percentage of the line height in it (M21).
    Calc(Arc<Calc>),
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

/// A `<length-percentage>` value, or `auto` (css-values-4 §6.3): the
/// margins (M6, M10.1) and the insets (M11).
#[derive(Debug, Clone, PartialEq)]
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
    /// A `calc()` with a percentage in it (M21), resolved at layout
    /// time as a percentage is.
    Calc(Arc<Calc>),
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
            LengthPercentageAuto::Calc(c) => c.resolve(basis),
        }
    }

    /// Whether this is the `auto` keyword.
    pub fn is_auto(&self) -> bool {
        matches!(self, LengthPercentageAuto::Auto)
    }

    /// Whether this depends on its percentage's basis: a percentage, or
    /// a `calc()` with one in it.
    pub fn has_percent(&self) -> bool {
        matches!(
            self,
            LengthPercentageAuto::Percent(_) | LengthPercentageAuto::Calc(_)
        )
    }
}

/// A `<length-percentage>` (css-values-4 §6.3), kept as written until
/// layout resolves the percentage against the containing block (M10.1):
/// `padding` and the min and max sizes take this.
#[derive(Debug, Clone, PartialEq)]
pub enum LengthPercentage {
    /// An absolute length, in CSS pixels.
    Px(f32),
    /// A percentage, as written (`50%` is `Percent(50.0)`).
    Percent(f32),
    /// A `calc()` with a percentage in it (M21), resolved at layout
    /// time as a percentage is.
    Calc(Arc<Calc>),
}

impl LengthPercentage {
    /// Resolves against `basis`, the containing-block dimension the
    /// property's percentage refers to.
    pub fn resolve(&self, basis: f32) -> f32 {
        match self {
            LengthPercentage::Px(v) => *v,
            LengthPercentage::Percent(p) => basis * p / 100.0,
            LengthPercentage::Calc(c) => c.resolve(basis),
        }
    }

    /// Whether this depends on its percentage's basis: a percentage, or
    /// a `calc()` with one in it.
    pub fn has_percent(&self) -> bool {
        matches!(
            self,
            LengthPercentage::Percent(_) | LengthPercentage::Calc(_)
        )
    }

    /// `percent` of a basis plus `px`, the form of an offset along an
    /// axis — `right 10px` is `100%` minus `10px` (css-values-4 §4.3) —
    /// as a length, a percentage, or the `calc()` of the two.
    pub fn sum(percent: f32, px: f32) -> LengthPercentage {
        if percent == 0.0 {
            LengthPercentage::Px(px)
        } else if px == 0.0 {
            LengthPercentage::Percent(percent)
        } else {
            LengthPercentage::Calc(Arc::new(Calc::new(
                Expression::Sum(Box::new([
                    Expression::Percent(percent),
                    Expression::Value(px),
                ])),
                false,
            )))
        }
    }
}

/// A box's preferred or minimum size in one axis: `width`, `height`,
/// `min-width`, `min-height` and the `<'width'>` of `flex-basis`
/// (css-sizing-3 §3.2, M17), kept as written until layout resolves it.
/// The intrinsic keywords are sizes in the inline axis; in the block
/// axis a box's size, unless otherwise specified, takes them as its
/// automatic size (§3.2).
#[derive(Debug, Clone, PartialEq)]
pub enum Size {
    /// `auto`: what it means depends on the property and the layout. For
    /// `min-width` and `min-height` it is zero, but on a flex item's main
    /// axis (css-flexbox-1 §4.5).
    Auto,
    /// A length, in CSS pixels, never negative.
    Px(f32),
    /// A percentage, as written, never negative.
    Percent(f32),
    /// A `calc()` with a percentage in it (M21), never negative.
    Calc(Arc<Calc>),
    /// The box's min-content size.
    MinContent,
    /// The box's max-content size.
    MaxContent,
    /// `fit-content`: `min(max-content, max(min-content, stretch))`.
    FitContent,
}

impl Size {
    /// The size as a length or a percentage; `None` for `auto` and the
    /// keywords.
    pub fn length_percentage(&self) -> Option<LengthPercentage> {
        match self {
            Size::Px(px) => Some(LengthPercentage::Px(*px)),
            Size::Percent(p) => Some(LengthPercentage::Percent(*p)),
            Size::Calc(c) => Some(LengthPercentage::Calc(c.clone())),
            Size::Auto | Size::MinContent | Size::MaxContent | Size::FitContent => None,
        }
    }

    /// Whether this is `auto`.
    pub fn is_auto(&self) -> bool {
        *self == Size::Auto
    }

    /// Whether this depends on its percentage's basis: a percentage, or
    /// a `calc()` with one in it.
    pub fn has_percent(&self) -> bool {
        matches!(self, Size::Percent(_) | Size::Calc(_))
    }
}

/// A box's maximum size in one axis: `max-width` and `max-height`
/// (css-sizing-3 §3.2, M17). In the block axis the keywords are `none`.
#[derive(Debug, Clone, PartialEq)]
pub enum MaxSize {
    /// `none`: no limit. The initial value.
    None,
    /// A length, in CSS pixels, never negative.
    Px(f32),
    /// A percentage, as written, never negative.
    Percent(f32),
    /// A `calc()` with a percentage in it (M21), never negative.
    Calc(Arc<Calc>),
    /// The box's min-content size.
    MinContent,
    /// The box's max-content size.
    MaxContent,
    /// `fit-content`.
    FitContent,
}

impl MaxSize {
    /// The limit as a length or a percentage; `None` for `none` and the
    /// keywords.
    pub fn length_percentage(&self) -> Option<LengthPercentage> {
        match self {
            MaxSize::Px(px) => Some(LengthPercentage::Px(*px)),
            MaxSize::Percent(p) => Some(LengthPercentage::Percent(*p)),
            MaxSize::Calc(c) => Some(LengthPercentage::Calc(c.clone())),
            MaxSize::None | MaxSize::MinContent | MaxSize::MaxContent | MaxSize::FitContent => None,
        }
    }

    /// Whether this depends on its percentage's basis: a percentage, or
    /// a `calc()` with one in it.
    pub fn has_percent(&self) -> bool {
        matches!(self, MaxSize::Percent(_) | MaxSize::Calc(_))
    }
}

/// `flex-direction` (css-flexbox-1 §5.1, M17): the main axis and its
/// direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    /// Along the inline axis, left to right here. The initial value.
    Row,
    RowReverse,
    /// Along the block axis, top to bottom.
    Column,
    ColumnReverse,
}

impl FlexDirection {
    /// Whether the main axis is the block axis, vertical here.
    pub fn is_column(self) -> bool {
        matches!(self, FlexDirection::Column | FlexDirection::ColumnReverse)
    }

    /// Whether main-start and main-end are swapped.
    pub fn is_reverse(self) -> bool {
        matches!(
            self,
            FlexDirection::RowReverse | FlexDirection::ColumnReverse
        )
    }
}

/// `flex-wrap` (css-flexbox-1 §5.2, M17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexWrap {
    /// One line. The initial value.
    NoWrap,
    Wrap,
    /// Wrapping, with cross-start and cross-end swapped.
    WrapReverse,
}

/// `flex-basis` (css-flexbox-1 §7.2.3, M17): `content`, or a size as
/// `width` takes one; `auto` is the initial value.
#[derive(Debug, Clone, PartialEq)]
pub enum FlexBasis {
    /// The item's content size.
    Content,
    /// `auto` (the item's main size property) or a size.
    Size(Size),
}

/// The positional alignment keywords (css-align-3 §4.1, M17), each
/// property taking the ones its grammar lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignPosition {
    Center,
    Start,
    End,
    SelfStart,
    SelfEnd,
    FlexStart,
    FlexEnd,
    Left,
    Right,
}

/// The overflow alignment written before a positional keyword
/// (css-align-3 §4.4, M17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowAlign {
    /// Neither: unsafe, as the reference browsers have it.
    Default,
    /// `safe`: an overflowing subject is aligned as `start`.
    Safe,
    /// `unsafe`.
    Unsafe,
}

/// Which baseline set a baseline alignment uses (css-align-3 §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaselinePosition {
    /// `baseline` or `first baseline`.
    First,
    /// `last baseline`.
    Last,
}

/// `justify-content` and `align-content` (css-align-3 §5.1, M17).
/// `justify-content` takes no baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentAlignment {
    /// The initial value.
    Normal,
    Baseline(BaselinePosition),
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Stretch,
    Position(AlignPosition, OverflowAlign),
}

/// `align-items` and `align-self` (css-align-3 §6, M17). `Auto` is
/// `align-self`'s alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfAlignment {
    /// `align-self: auto`, its initial value: the parent's
    /// `align-items`.
    Auto,
    /// `align-items`'s initial value.
    Normal,
    Stretch,
    Baseline(BaselinePosition),
    Position(AlignPosition, OverflowAlign),
}

/// `justify-items` (css-align-3 §6.1, M18): the `justify-self` that
/// `auto` gives a container's items, and whether it carries `legacy`,
/// which makes the value pass to the container's children as their own
/// `legacy` (§6.1's computed value).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JustifyItems {
    /// The value, never `auto`: `normal`, `stretch`, a baseline or a
    /// position, `left`, `right` and `center` among them.
    pub value: SelfAlignment,
    /// Written with `legacy`: `legacy left`, `legacy right` or `legacy
    /// center`.
    pub legacy: bool,
}

impl JustifyItems {
    /// `normal`, which a bare `legacy` computes to unless its parent's
    /// value carries `legacy`.
    pub const NORMAL: JustifyItems = JustifyItems {
        value: SelfAlignment::Normal,
        legacy: false,
    };

    /// What a bare `legacy`, the initial value, computes to under a
    /// parent whose value is `parent`: that value when it carries
    /// `legacy`, `normal` otherwise.
    pub fn legacy_under(parent: JustifyItems) -> JustifyItems {
        if parent.legacy {
            parent
        } else {
            JustifyItems::NORMAL
        }
    }
}

/// A line's names in a track list (css-grid-2 §7.2.2, M18): the
/// `<custom-ident>`s between `[` and `]`, as written, since a
/// `<custom-ident>` is case-sensitive.
pub type LineNames = Vec<Arc<str>>;

/// One end of a track's size, a `<track-breadth>` (css-grid-2 §7.2.1).
#[derive(Debug, Clone, PartialEq)]
pub enum TrackBreadth {
    /// A non-negative length or percentage: a fixed sizing function.
    Fixed(LengthPercentage),
    /// A non-negative `<flex>`, in `fr`: a flexible sizing function.
    Flex(f32),
    /// `min-content`.
    MinContent,
    /// `max-content`.
    MaxContent,
    /// `auto`.
    Auto,
}

/// A track's size, a `<track-size>` (css-grid-2 §7.2.1), as written.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackSize {
    /// One breadth, both the minimum and the maximum, a flex's minimum
    /// being `auto` (§12.2).
    Breadth(TrackBreadth),
    /// `minmax(min, max)`: the minimum is never a flex.
    MinMax(TrackBreadth, TrackBreadth),
    /// `fit-content(<length-percentage>)`.
    FitContent(LengthPercentage),
}

impl TrackSize {
    /// `auto`: `grid-auto-rows` and `grid-auto-columns`' initial value.
    pub const AUTO: TrackSize = TrackSize::Breadth(TrackBreadth::Auto);
}

/// How many times a `repeat()` repeats (css-grid-2 §7.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatCount {
    /// A number of times, at least one, as written; layout limits it.
    Count(u32),
    /// `auto-fill`: as many as fit.
    AutoFill,
    /// `auto-fit`: as many as fit, the empty ones collapsed.
    AutoFit,
}

impl RepeatCount {
    /// Whether the count is found at layout: `auto-fill` or `auto-fit`.
    pub fn is_automatic(self) -> bool {
        matches!(self, RepeatCount::AutoFill | RepeatCount::AutoFit)
    }
}

/// A `repeat()` in a track list (css-grid-2 §7.2.3): its tracks, each
/// after the names of the line before it, `names` having one more entry
/// than `tracks`, its last the names of the line after the last track.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackRepeat {
    pub count: RepeatCount,
    pub names: Vec<LineNames>,
    pub tracks: Vec<TrackSize>,
}

/// One section of a track list: a track, or a repetition of tracks.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackSection {
    Track(TrackSize),
    Repeat(TrackRepeat),
}

/// A track list, `grid-template-columns` or `grid-template-rows` when
/// not `none` (css-grid-2 §7.2.5's computed track list): its sections,
/// each after the names of the line before it, `names` having one more
/// entry than `sections`, its last the names of the line after the last.
/// Never empty, and with at most one automatic repetition.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackList {
    pub names: Vec<LineNames>,
    pub sections: Vec<TrackSection>,
}

impl TrackList {
    /// The list's `repeat(auto-fill, …)` or `repeat(auto-fit, …)`, if
    /// it has one.
    pub fn auto_repeat(&self) -> Option<&TrackRepeat> {
        self.sections.iter().find_map(|section| match section {
            TrackSection::Repeat(repeat) if repeat.count.is_automatic() => Some(repeat),
            _ => None,
        })
    }
}

/// A named grid area of `grid-template-areas` (css-grid-2 §7.3): its
/// rows and columns, zero-based, from the first to one past the last.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedArea {
    pub name: Arc<str>,
    pub rows: (u32, u32),
    pub columns: (u32, u32),
}

/// `grid-template-areas` when not `none` (css-grid-2 §7.3): the size of
/// the grid its strings make, and its named areas, each a rectangle,
/// worked out once by the cascade.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateAreas {
    pub rows: u32,
    pub columns: u32,
    pub areas: Vec<NamedArea>,
}

impl TemplateAreas {
    /// The area called `name`, if there is one.
    pub fn area(&self, name: &str) -> Option<&NamedArea> {
        self.areas.iter().find(|area| &*area.name == name)
    }
}

/// A `<grid-line>`: one of `grid-row-start`, `grid-row-end`,
/// `grid-column-start` and `grid-column-end` (css-grid-2 §8.3).
#[derive(Debug, Clone, PartialEq)]
pub enum GridLine {
    /// `auto`: automatic placement, or a span of one.
    Auto,
    /// A `<custom-ident>` alone: a named area's edge, else the first
    /// line with that name.
    Ident(Arc<str>),
    /// `<integer> && <custom-ident>?`: the integer is never zero, and
    /// negative counts from the end of the explicit grid.
    Line(i32, Option<Arc<str>>),
    /// `span && [ <integer> || <custom-ident> ]`: the integer is at
    /// least one.
    Span(u32, Option<Arc<str>>),
}

/// `grid-auto-flow` (css-grid-2 §7.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GridAutoFlow {
    /// `column`: items fill each column in turn; `row` otherwise, the
    /// initial value.
    pub column: bool,
    /// `dense`: auto-placement goes back to fill holes.
    pub dense: bool,
}

/// The grid properties of one element (css-grid-2 §7, §8, M18), and
/// `justify-items` and `justify-self` (css-align-3 §6.1, §6.2), which
/// only grid containers, grid items and absolutely positioned boxes
/// read: what [`ComputedStyle::grid`] points to when any of them is not
/// its initial value. None of them is inherited.
#[derive(Debug, Clone, PartialEq)]
pub struct GridStyle {
    /// `grid-template-columns`; `None` is `none`.
    pub template_columns: Option<TrackList>,
    /// `grid-template-rows`; `None` is `none`.
    pub template_rows: Option<TrackList>,
    /// `grid-template-areas`; `None` is `none`.
    pub template_areas: Option<TemplateAreas>,
    /// `grid-auto-columns`: never empty, a pattern repeated.
    pub auto_columns: Vec<TrackSize>,
    /// `grid-auto-rows`: never empty.
    pub auto_rows: Vec<TrackSize>,
    /// `grid-auto-flow`.
    pub auto_flow: GridAutoFlow,
    /// `grid-row-start`, `grid-row-end`, `grid-column-start` and
    /// `grid-column-end`.
    pub row_start: GridLine,
    pub row_end: GridLine,
    pub column_start: GridLine,
    pub column_end: GridLine,
    /// `justify-items`: its initial value, `legacy`, takes the parent's
    /// when that carries `legacy`, and is `normal` otherwise.
    pub justify_items: JustifyItems,
    /// `justify-self`, initially `auto`.
    pub justify_self: SelfAlignment,
}

impl Default for GridStyle {
    /// Every grid property's initial value.
    fn default() -> Self {
        GridStyle {
            template_columns: None,
            template_rows: None,
            template_areas: None,
            auto_columns: vec![TrackSize::AUTO],
            auto_rows: vec![TrackSize::AUTO],
            auto_flow: GridAutoFlow::default(),
            row_start: GridLine::Auto,
            row_end: GridLine::Auto,
            column_start: GridLine::Auto,
            column_end: GridLine::Auto,
            justify_items: JustifyItems::NORMAL,
            justify_self: SelfAlignment::Auto,
        }
    }
}

/// The grid properties' initial values, which [`ComputedStyle::grid`]
/// gives a style that sets none of them.
static INITIAL_GRID: LazyLock<GridStyle> = LazyLock::new(GridStyle::default);

/// One function of a `transform` list (css-transforms-1 §7,
/// css-transforms-2 §13, M19), as computed: lengths absolute,
/// percentages kept for the box they are of, angles in degrees. The 2D
/// form of a function and its 3D form are kept as one, the 3D one,
/// which is the primitive they share when they are interpolated (M22,
/// css-transforms-2 §16); `matrix()` is kept apart from `matrix3d()`
/// for the same reason.
#[derive(Debug, Clone, PartialEq)]
pub enum TransformFunction {
    /// `matrix(a, b, c, d, e, f)`.
    Matrix([f32; 6]),
    /// `matrix3d()`: its sixteen numbers in column-major order, as
    /// written.
    Matrix3d([f32; 16]),
    /// `translate()`, `translateX()`, `translateY()`, `translateZ()` and
    /// `translate3d()`: the x and y translations, percentages of the
    /// reference box's width and height, and the z one, a length.
    Translate(LengthPercentage, LengthPercentage, f32),
    /// `scale()`, `scaleX()`, `scaleY()`, `scaleZ()` and `scale3d()`.
    Scale(f32, f32, f32),
    /// `rotate()`, `rotateX()`, `rotateY()`, `rotateZ()` and
    /// `rotate3d()`: the axis as written, and the angle in degrees.
    Rotate {
        /// The axis's x component.
        x: f32,
        /// The axis's y component.
        y: f32,
        /// The axis's z component.
        z: f32,
        /// The angle, in degrees.
        angle: f32,
    },
    /// `skew()`, `skewX()` and `skewY()`: the angles along x and y, in
    /// degrees.
    Skew(f32, f32),
    /// `perspective()`: `None` for `none`, else the depth, a length.
    Perspective(Option<f32>),
}

/// `transform-origin` (css-transforms-1 §8, css-transforms-2 §8): the
/// point the transform is about, offsets from the reference box's top
/// left, and a length along z. The initial value is `50% 50%`. An
/// offset is a percentage of the box's size along its axis plus a
/// length, which is what the forms of `<position>` (css-values-4 §4.3)
/// resolve to: `right 10px` is `calc(100% - 10px)`.
#[derive(Debug, Clone, PartialEq)]
pub struct TransformOrigin {
    /// Along x, of the box's width.
    pub x: LengthPercentage,
    /// Along y, of the box's height.
    pub y: LengthPercentage,
    /// Along z, a length.
    pub z: f32,
}

impl TransformOrigin {
    /// The initial value, `50% 50%`.
    pub const CENTER: TransformOrigin = TransformOrigin {
        x: LengthPercentage::Percent(50.0),
        y: LengthPercentage::Percent(50.0),
        z: 0.0,
    };
}

/// `perspective-origin` (css-transforms-2 §11): the vanishing point of
/// a box's `perspective`, offsets from its border box's top left. The
/// initial value is `50% 50%`.
#[derive(Debug, Clone, PartialEq)]
pub struct PerspectiveOrigin {
    /// Along x, of the box's width.
    pub x: LengthPercentage,
    /// Along y, of the box's height.
    pub y: LengthPercentage,
}

impl PerspectiveOrigin {
    /// The initial value, `50% 50%`.
    pub const CENTER: PerspectiveOrigin = PerspectiveOrigin {
        x: LengthPercentage::Percent(50.0),
        y: LengthPercentage::Percent(50.0),
    };
}

/// `transform-box` (css-transforms-1 §9): the reference box of the
/// transform, which its origin and its percentages are of. For an
/// element with a CSS layout box, which every element here has (SVG is
/// M32), `fill-box` is used as `content-box`, and `stroke-box` and
/// `view-box`, the initial value, as `border-box`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformBox {
    /// `content-box`.
    ContentBox,
    /// `border-box`.
    BorderBox,
    /// `fill-box`.
    FillBox,
    /// `stroke-box`.
    StrokeBox,
    /// `view-box`, the initial value.
    ViewBox,
}

impl TransformBox {
    /// Whether the reference box is the content box, rather than the
    /// border box, for an element with a CSS layout box.
    pub fn uses_content_box(self) -> bool {
        matches!(self, TransformBox::ContentBox | TransformBox::FillBox)
    }
}

/// The `translate` property (css-transforms-2 §2.1), other than `none`:
/// percentages of the reference box's width and height, and a length
/// along z.
#[derive(Debug, Clone, PartialEq)]
pub struct Translate {
    /// Along x.
    pub x: LengthPercentage,
    /// Along y.
    pub y: LengthPercentage,
    /// Along z, a length.
    pub z: f32,
}

/// The `rotate` property (css-transforms-2 §2.2), other than `none`: a
/// rotation about an axis. `rotate: 30deg` is about the z axis,
/// `(0, 0, 1)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotate {
    /// The axis's x component.
    pub x: f32,
    /// The axis's y component.
    pub y: f32,
    /// The axis's z component.
    pub z: f32,
    /// The angle, in degrees.
    pub angle: f32,
}

/// `transform-style` (css-transforms-2 §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformStyle {
    /// `flat`, the initial value: the box's children are drawn into
    /// its plane.
    Flat,
    /// `preserve-3d`: the box's children keep their place in 3D, in
    /// the 3D rendering context the box opens or takes part in.
    Preserve3d,
}

/// `backface-visibility` (css-transforms-2 §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfaceVisibility {
    /// `visible`, the initial value.
    Visible,
    /// `hidden`: the box is not drawn when its accumulated matrix turns
    /// its back to the viewer.
    Hidden,
}

/// `will-change`'s computed value (css-will-change-1 §2, M19): which of
/// the features this engine acts on the declaration names. `auto`, and
/// a list naming only other properties, keep none of them; the names
/// themselves are not kept, beyond these bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WillChange {
    bits: u32,
}

impl WillChange {
    /// `auto`, the initial value: nothing is expected to change.
    pub const AUTO: WillChange = WillChange { bits: 0 };
    /// Names `transform`.
    pub const TRANSFORM: WillChange = WillChange { bits: 1 };
    /// Names `translate`.
    pub const TRANSLATE: WillChange = WillChange { bits: 1 << 1 };
    /// Names `rotate`.
    pub const ROTATE: WillChange = WillChange { bits: 1 << 2 };
    /// Names `scale`.
    pub const SCALE: WillChange = WillChange { bits: 1 << 3 };
    /// Names `opacity`.
    pub const OPACITY: WillChange = WillChange { bits: 1 << 4 };
    /// Names `perspective`.
    pub const PERSPECTIVE: WillChange = WillChange { bits: 1 << 5 };
    /// Names `transform-style`.
    pub const TRANSFORM_STYLE: WillChange = WillChange { bits: 1 << 6 };
    /// Names `scroll-position`.
    pub const SCROLL_POSITION: WillChange = WillChange { bits: 1 << 7 };
    /// Names `contents`.
    pub const CONTENTS: WillChange = WillChange { bits: 1 << 8 };
    /// Names `filter`, a property this engine has not got (M23) whose
    /// other values make a stacking context and a containing block for
    /// all descendants, which naming it does too.
    pub const FILTER: WillChange = WillChange { bits: 1 << 9 };
    /// Names `backdrop-filter`: as `filter`.
    pub const BACKDROP_FILTER: WillChange = WillChange { bits: 1 << 10 };
    /// Names `contain`, whose `paint` and `layout` make a containing
    /// block for all descendants.
    pub const CONTAIN: WillChange = WillChange { bits: 1 << 11 };
    /// Names `offset-path` (Motion Path 1): as `filter`.
    pub const OFFSET_PATH: WillChange = WillChange { bits: 1 << 12 };
    /// Names `position`, whose other values make a containing block for
    /// absolutely positioned descendants and, `fixed` and `sticky`, a
    /// stacking context.
    pub const POSITION: WillChange = WillChange { bits: 1 << 13 };
    /// Names `z-index`, whose other values make a positioned box or an
    /// item a stacking context.
    pub const Z_INDEX: WillChange = WillChange { bits: 1 << 14 };
    /// Names `clip-path`, `mask`, `mask-image`, `mask-border`,
    /// `isolation`, `mix-blend-mode` or `view-transition-name`,
    /// properties this engine has not got whose other values make a
    /// stacking context.
    pub const COMPOSITED: WillChange = WillChange { bits: 1 << 15 };

    /// The feature `name` stands for, or `None` for a name this engine
    /// does not act on. A `-webkit-` name stands for the property it is
    /// an alias of.
    pub fn feature(name: &str) -> Option<WillChange> {
        let name = name.strip_prefix("-webkit-").unwrap_or(name);
        Some(match name {
            "transform" => WillChange::TRANSFORM,
            "translate" => WillChange::TRANSLATE,
            "rotate" => WillChange::ROTATE,
            "scale" => WillChange::SCALE,
            "opacity" => WillChange::OPACITY,
            "perspective" => WillChange::PERSPECTIVE,
            "transform-style" => WillChange::TRANSFORM_STYLE,
            "scroll-position" => WillChange::SCROLL_POSITION,
            "contents" => WillChange::CONTENTS,
            "filter" => WillChange::FILTER,
            "backdrop-filter" => WillChange::BACKDROP_FILTER,
            "contain" => WillChange::CONTAIN,
            "offset-path" => WillChange::OFFSET_PATH,
            "position" => WillChange::POSITION,
            "z-index" => WillChange::Z_INDEX,
            "clip-path"
            | "mask"
            | "mask-image"
            | "mask-border"
            | "isolation"
            | "mix-blend-mode"
            | "view-transition-name" => WillChange::COMPOSITED,
            _ => return None,
        })
    }

    /// This value with `other`'s features named too.
    pub fn with(self, other: WillChange) -> WillChange {
        WillChange {
            bits: self.bits | other.bits,
        }
    }

    /// Whether every feature of `features` is named.
    pub fn names(self, features: WillChange) -> bool {
        self.bits & features.bits == features.bits
    }

    /// Whether any feature of `features` is named.
    pub fn names_any(self, features: WillChange) -> bool {
        self.bits & features.bits != 0
    }

    /// Whether this is `auto`, or names nothing this engine acts on.
    pub fn is_auto(self) -> bool {
        self.bits == 0
    }

    /// The features whose non-initial values make a box a containing
    /// block for all its descendants, the fixed ones included: the
    /// transform properties, `perspective`, `transform-style`,
    /// `filter`, `backdrop-filter`, `contain` and `offset-path`
    /// (css-will-change-1 §2.2, css-transforms-1 §2, css-transforms-2
    /// §2, §9, §10).
    pub const CONTAINING: WillChange = WillChange {
        bits: WillChange::TRANSFORM.bits
            | WillChange::TRANSLATE.bits
            | WillChange::ROTATE.bits
            | WillChange::SCALE.bits
            | WillChange::PERSPECTIVE.bits
            | WillChange::TRANSFORM_STYLE.bits
            | WillChange::FILTER.bits
            | WillChange::BACKDROP_FILTER.bits
            | WillChange::CONTAIN.bits
            | WillChange::OFFSET_PATH.bits,
    };

    /// The features whose non-initial values make a box a containing
    /// block for its absolutely positioned descendants:
    /// [`WillChange::CONTAINING`] and `position`.
    pub const CONTAINING_ABSOLUTES: WillChange = WillChange {
        bits: WillChange::CONTAINING.bits | WillChange::POSITION.bits,
    };

    /// The features whose non-initial values make a box a stacking
    /// context: [`WillChange::CONTAINING`], `opacity`, `position` and
    /// the composited properties; `z-index` on a positioned box or an
    /// item, which the box decides.
    pub const STACKING: WillChange = WillChange {
        bits: WillChange::CONTAINING.bits
            | WillChange::OPACITY.bits
            | WillChange::POSITION.bits
            | WillChange::COMPOSITED.bits,
    };

    /// The features that make a box's group a layer of its own (the
    /// compositing note's policy): the author's word that its matrix or
    /// its opacity is about to change.
    pub const LAYER: WillChange = WillChange {
        bits: WillChange::TRANSFORM.bits
            | WillChange::TRANSLATE.bits
            | WillChange::ROTATE.bits
            | WillChange::SCALE.bits
            | WillChange::PERSPECTIVE.bits
            | WillChange::TRANSFORM_STYLE.bits
            | WillChange::OPACITY.bits,
    };
}

/// The transform, opacity and will-change properties of one element
/// (css-transforms-1, css-transforms-2, css-color-4 §3.2,
/// css-will-change-1; M19): what [`ComputedStyle::effects`] points to
/// when any of them is not its initial value. None of them is inherited.
#[derive(Debug, Clone, PartialEq)]
pub struct Effects {
    /// `transform`: its functions in order; empty is `none`.
    pub transform: Vec<TransformFunction>,
    /// `transform-origin`.
    pub transform_origin: TransformOrigin,
    /// `transform-box`.
    pub transform_box: TransformBox,
    /// `translate`; `None` is `none`.
    pub translate: Option<Translate>,
    /// `rotate`; `None` is `none`.
    pub rotate: Option<Rotate>,
    /// `scale`, its factors along x, y and z; `None` is `none`.
    pub scale: Option<[f32; 3]>,
    /// `transform-style`.
    pub transform_style: TransformStyle,
    /// `perspective`, a length; `None` is `none`. A value below one
    /// pixel is used as one pixel (css-transforms-2 §10).
    pub perspective: Option<f32>,
    /// `perspective-origin`.
    pub perspective_origin: PerspectiveOrigin,
    /// `backface-visibility`.
    pub backface_visibility: BackfaceVisibility,
    /// `opacity`, in `[0, 1]`.
    pub opacity: f32,
    /// `will-change`.
    pub will_change: WillChange,
}

impl Default for Effects {
    /// Every property's initial value.
    fn default() -> Self {
        Effects {
            transform: Vec::new(),
            transform_origin: TransformOrigin::CENTER,
            transform_box: TransformBox::ViewBox,
            translate: None,
            rotate: None,
            scale: None,
            transform_style: TransformStyle::Flat,
            perspective: None,
            perspective_origin: PerspectiveOrigin::CENTER,
            backface_visibility: BackfaceVisibility::Visible,
            opacity: 1.0,
            will_change: WillChange::AUTO,
        }
    }
}

impl Effects {
    /// Whether a transformable box with these properties is a
    /// containing block for all its descendants, the fixed-positioned
    /// ones included: it has a transform, a `perspective`,
    /// `transform-style: preserve-3d`, or a `will-change` naming any
    /// property that would make one (css-transforms-1 §2,
    /// css-transforms-2 §2, §9, §10, css-will-change-1 §2.2). A `fixed`
    /// box inside it is positioned against its padding box, and scrolls
    /// with it. On a box that is not transformable, an inline box, only
    /// the `will-change` counts: see [`Effects::contains_all_of`].
    pub fn contains_all_descendants(&self) -> bool {
        self.contains_all_of(true)
    }

    /// [`Effects::contains_all_descendants`] for a box that is
    /// transformable or not: the transform properties apply to a
    /// transformable element alone (css-transforms-1 §3), while a
    /// `will-change` naming a `filter` applies to any box.
    pub fn contains_all_of(&self, transformable: bool) -> bool {
        (transformable
            && (self.has_transform()
                || self.perspective.is_some()
                || self.transform_style == TransformStyle::Preserve3d))
            || self.will_change.names_any(WillChange::CONTAINING)
    }

    /// Whether the box is a containing block for its absolutely
    /// positioned descendants by these properties:
    /// [`Effects::contains_all_of`], or a `will-change` naming
    /// `position` (css-will-change-1 §2.2).
    pub fn contains_absolutes_of(&self, transformable: bool) -> bool {
        self.contains_all_of(transformable)
            || self.will_change.names_any(WillChange::CONTAINING_ABSOLUTES)
    }

    /// Whether a transformable box is a stacking context by these
    /// properties: [`Effects::contains_all_descendants`], an `opacity`
    /// below one, or a `will-change` naming any property that would
    /// make one (css-color-4 §3.2, css-transforms-2 §9, css-will-change-1
    /// §2.2).
    pub fn is_stacking_context(&self) -> bool {
        self.is_stacking_context_of(true)
    }

    /// [`Effects::is_stacking_context`] for a box that is transformable
    /// or not: `perspective` on an inline box makes none.
    pub fn is_stacking_context_of(&self, transformable: bool) -> bool {
        self.contains_all_of(transformable)
            || self.opacity < 1.0
            || self.will_change.names_any(WillChange::STACKING)
    }

    /// The used `transform-style` (css-transforms-2 §9.1): `flat` when
    /// the box has a grouping property, an `overflow` that clips or an
    /// `opacity` below one, whatever was written.
    pub fn used_transform_style(&self, overflow: Overflow) -> TransformStyle {
        if overflow.clips() || self.opacity < 1.0 {
            TransformStyle::Flat
        } else {
            self.transform_style
        }
    }
}

/// The effects properties' initial values, which
/// [`ComputedStyle::effects`] gives a style that sets none of them.
static INITIAL_EFFECTS: LazyLock<Effects> = LazyLock::new(Effects::default);

/// `break-before` and `break-after` (css-break-3 §3.1; M20): whether a
/// break may, must or should not come between two boxes. The column and
/// region values are taken and mean nothing on a page, which has no
/// columns or regions to break.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreakBetween {
    #[default]
    Auto,
    Avoid,
    AvoidPage,
    /// A forced page break.
    Page,
    /// A forced break, then another if need be, so that the next page
    /// is a left page.
    Left,
    /// The same, to a right page.
    Right,
    /// `recto`: a right page, the document being left to right.
    Recto,
    /// `verso`: a left page.
    Verso,
    AvoidColumn,
    Column,
    AvoidRegion,
    Region,
}

impl BreakBetween {
    /// Whether the value forces a page break.
    pub fn forces_page(self) -> bool {
        matches!(
            self,
            BreakBetween::Page
                | BreakBetween::Left
                | BreakBetween::Right
                | BreakBetween::Recto
                | BreakBetween::Verso
        )
    }

    /// Whether the value asks for no page break.
    pub fn avoids_page(self) -> bool {
        matches!(self, BreakBetween::Avoid | BreakBetween::AvoidPage)
    }
}

/// `break-inside` (css-break-3 §3.2; M20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreakInside {
    #[default]
    Auto,
    Avoid,
    AvoidPage,
    AvoidColumn,
    AvoidRegion,
}

impl BreakInside {
    /// Whether the value asks for no page break inside the box.
    pub fn avoids_page(self) -> bool {
        matches!(self, BreakInside::Avoid | BreakInside::AvoidPage)
    }
}

/// `print-color-adjust` (css-color-adjust-1 §3.3; M20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrintColorAdjust {
    /// The user agent may economise on ink; this engine does not.
    #[default]
    Economy,
    Exact,
}

/// The break properties, `orphans`, `widows` and `print-color-adjust`
/// (css-break-3 §3, css-color-adjust-1 §3.3; M20): what
/// [`ComputedStyle::fragmentation`] points to when any of them is not
/// its initial value. `orphans`, `widows` and `print-color-adjust` are
/// inherited; the break properties are not.
#[derive(Debug, Clone, PartialEq)]
pub struct Fragmentation {
    /// `break-before`.
    pub break_before: BreakBetween,
    /// `break-after`.
    pub break_after: BreakBetween,
    /// `break-inside`.
    pub break_inside: BreakInside,
    /// `orphans`: the fewest lines of a block left at the bottom of a
    /// page, at least one.
    pub orphans: u32,
    /// `widows`: the fewest lines of a block carried to the top of a
    /// page, at least one.
    pub widows: u32,
    /// `print-color-adjust`.
    pub print_color_adjust: PrintColorAdjust,
}

impl Default for Fragmentation {
    /// Every property's initial value.
    fn default() -> Self {
        Fragmentation {
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            break_inside: BreakInside::Auto,
            orphans: 2,
            widows: 2,
            print_color_adjust: PrintColorAdjust::Economy,
        }
    }
}

impl Fragmentation {
    /// These properties as a child inherits them: the inherited ones
    /// kept, the others initial.
    fn inherited(&self) -> Fragmentation {
        Fragmentation {
            orphans: self.orphans,
            widows: self.widows,
            print_color_adjust: self.print_color_adjust,
            ..Fragmentation::default()
        }
    }
}

/// The fragmentation properties' initial values, which
/// [`ComputedStyle::fragmentation`] gives a style that sets none of them.
static INITIAL_FRAGMENTATION: LazyLock<Fragmentation> = LazyLock::new(Fragmentation::default);

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
    /// Shared, not copied, by the styles that inherit it and the
    /// fragments that copy their styles (M16.5).
    pub font_family: Arc<str>,
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
    /// `width` (M6; the intrinsic keywords since M17) — not inherited.
    /// `auto` (the initial value) means "fill the containing block" in
    /// block layout (CSS2.1 §10.3.3); other formatting contexts mean
    /// something else, which is why this stays unresolved (not a plain
    /// `f32`) until a layout algorithm resolves it against a containing
    /// block.
    pub width: Size,
    /// `height` (M6) — not inherited. `auto` (the initial value) means
    /// "as tall as the content" in block layout; a percentage is of the
    /// containing block's height when that is definite (M10), and
    /// `auto` otherwise, as an intrinsic keyword is (M17).
    pub height: Size,
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
    /// containing block's width. The initial value is `auto` (M17),
    /// which is `0` but on a flex item's main axis.
    pub min_width: Size,
    /// `max-width` (M10.3) — not inherited; `none` is the initial value.
    pub max_width: MaxSize,
    /// `min-height` (M10.3) — not inherited; a percentage of the
    /// containing block's height when that is definite, else `0`. The
    /// initial value is `auto`, as for `min-width`.
    pub min_height: Size,
    /// `max-height` (M10.3) — not inherited; `none` is the initial
    /// value, and a percentage of an indefinite containing-block height
    /// counts as it.
    pub max_height: MaxSize,
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
    /// `flex-direction` (M17) — not inherited.
    pub flex_direction: FlexDirection,
    /// `flex-wrap` (M17) — not inherited.
    pub flex_wrap: FlexWrap,
    /// `order` (M17) — not inherited: where a flex item goes among its
    /// siblings, which keep document order among equals.
    pub order: i32,
    /// `flex-grow` (M17) — not inherited; finite, never negative.
    pub flex_grow: f32,
    /// `flex-shrink` (M17) — not inherited; finite, never negative.
    pub flex_shrink: f32,
    /// `flex-basis` (M17) — not inherited.
    pub flex_basis: FlexBasis,
    /// `justify-content` (M17) — not inherited.
    pub justify_content: ContentAlignment,
    /// `align-content` (M17) — not inherited.
    pub align_content: ContentAlignment,
    /// `align-items` (M17) — not inherited; never `auto`.
    pub align_items: SelfAlignment,
    /// `align-self` (M17) — not inherited.
    pub align_self: SelfAlignment,
    /// `row-gap` (M17) — not inherited; `None` is the initial `normal`,
    /// which is zero in a flex container.
    pub row_gap: Option<LengthPercentage>,
    /// `column-gap` (M17) — not inherited; `None` is `normal`.
    pub column_gap: Option<LengthPercentage>,
    /// The grid properties and the two justify properties (M18), behind
    /// one pointer, so that the many elements that set none of them
    /// carry eight bytes for them; `None` is every initial value. Read
    /// through [`ComputedStyle::grid`].
    pub grid: Option<Arc<GridStyle>>,
    /// The transform, opacity and will-change properties (M19), behind
    /// one pointer likewise; `None` is every initial value. Read through
    /// [`ComputedStyle::effects`].
    pub effects: Option<Arc<Effects>>,
    /// The break properties, `orphans`, `widows` and
    /// `print-color-adjust` (M20), behind one pointer likewise; `None` is
    /// every initial value. Read through [`ComputedStyle::fragmentation`].
    pub fragmentation: Option<Arc<Fragmentation>>,
    /// The custom properties (M21.1) — inherited, as every custom
    /// property is; `None` when none has a value. Shared with the parent
    /// unless the element declares one. Read through
    /// [`ComputedStyle::custom_property`].
    pub custom: Option<Arc<CustomProperties>>,
}

/// `font-family`'s initial value, one for every style that has it.
static SANS_SERIF: LazyLock<Arc<str>> = LazyLock::new(|| Arc::from("sans-serif"));

impl Default for ComputedStyle {
    /// The CSS initial values for the v1 property set.
    fn default() -> Self {
        ComputedStyle {
            color: Color::BLACK,
            background_color: Color::TRANSPARENT,
            font_size_px: 16.0, // 'medium', by UA convention
            font_family: Arc::clone(&SANS_SERIF),
            display: Display::Inline,
            text_align: TextAlign::Left,
            text_transform: TextTransform::None,
            white_space: WhiteSpace::Normal,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            direction: Direction::Ltr,
            width: Size::Auto,
            height: Size::Auto,
            margin: [const { LengthPercentageAuto::Px(0.0) }; 4], // margin's initial value is `0`, not `auto`
            padding: [const { LengthPercentage::Px(0.0) }; 4],
            border_width: [3.0; 4],          // `medium`
            border_color: [Color::BLACK; 4], // corrected to `color` post-cascade
            border_style: [BorderStyle::None; 4],
            overflow: Overflow::Visible,
            box_sizing: BoxSizing::ContentBox,
            min_width: Size::Auto,
            max_width: MaxSize::None,
            min_height: Size::Auto,
            max_height: MaxSize::None,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            inset: [const { LengthPercentageAuto::Auto }; 4],
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
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            order: 0,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: FlexBasis::Size(Size::Auto),
            justify_content: ContentAlignment::Normal,
            align_content: ContentAlignment::Normal,
            align_items: SelfAlignment::Normal,
            align_self: SelfAlignment::Auto,
            row_gap: None,
            column_gap: None,
            grid: None,
            effects: None,
            fragmentation: None,
            custom: None,
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
    ///
    /// `justify-items` is not inherited, but its initial value, `legacy`,
    /// computes to the parent's when that carries `legacy` (css-align-3
    /// §6.1).
    pub fn inherited_from(parent: &ComputedStyle) -> Self {
        let mut style = ComputedStyle::default().inheriting(parent);
        let legacy = JustifyItems::legacy_under(parent.grid().justify_items);
        if legacy != JustifyItems::NORMAL {
            style.grid_mut().justify_items = legacy;
        }
        style
    }

    /// The grid properties (M18): this style's, or their initial values.
    pub fn grid(&self) -> &GridStyle {
        self.grid.as_deref().unwrap_or(&INITIAL_GRID)
    }

    /// The grid properties to change, this style's own copy made first
    /// if it shares them or has none.
    pub fn grid_mut(&mut self) -> &mut GridStyle {
        Arc::make_mut(self.grid.get_or_insert_with(Default::default))
    }

    /// The transform, opacity and will-change properties (M19): this
    /// style's, or their initial values.
    pub fn effects(&self) -> &Effects {
        self.effects.as_deref().unwrap_or(&INITIAL_EFFECTS)
    }

    /// The effects properties to change, this style's own copy made
    /// first if it shares them or has none.
    pub fn effects_mut(&mut self) -> &mut Effects {
        Arc::make_mut(self.effects.get_or_insert_with(Default::default))
    }

    /// The break properties, `orphans`, `widows` and
    /// `print-color-adjust` (M20): this style's, or their initial values.
    pub fn fragmentation(&self) -> &Fragmentation {
        self.fragmentation
            .as_deref()
            .unwrap_or(&INITIAL_FRAGMENTATION)
    }

    /// The fragmentation properties to change, this style's own copy made
    /// first if it shares them or has none.
    pub fn fragmentation_mut(&mut self) -> &mut Fragmentation {
        Arc::make_mut(self.fragmentation.get_or_insert_with(Default::default))
    }

    /// The value of the custom property `name` (M21.1), its substitution
    /// functions replaced; `None` when it has the guaranteed-invalid
    /// value, as one never declared has.
    pub fn custom_property(&self, name: &str) -> Option<&[css::Token]> {
        self.custom.as_ref()?.get(name).map(|value| &**value)
    }

    /// Whether a box with this style is a stacking context (CSS2
    /// Appendix E, M11; M19): a positioned box, or a flex or grid
    /// `item` (css-flexbox-1 §4.3, css-grid-2 §6.5), whose `z-index` is
    /// not `auto`; or a box whose transform, opacity or will-change
    /// properties make one ([`Effects::is_stacking_context`]). The root
    /// element's box is one besides, which the paint order knows.
    pub fn is_stacking_context(&self, item: bool) -> bool {
        self.is_stacking_context_of(item, true)
    }

    /// [`ComputedStyle::is_stacking_context`] for a box that is
    /// transformable or not (css-transforms-1 §3): an inline box's
    /// transform properties do nothing, so make no context, while its
    /// `opacity` and `will-change` do. A `will-change` naming `z-index`
    /// counts as a `z-index` would.
    pub fn is_stacking_context_of(&self, item: bool, transformable: bool) -> bool {
        ((self.position.is_positioned() || item)
            && (self.z_index.is_some() || self.effects().will_change.names(WillChange::Z_INDEX)))
            || self.effects().is_stacking_context_of(transformable)
    }

    /// This style with its inherited properties taken from `from` and
    /// the others kept: the parent a child on the first formatted line
    /// inherits from, its inherited properties from the `::first-line`
    /// and the others from its parent (Pseudo-Elements 4 §2.1.3, M16).
    pub fn with_inherited_from(&self, from: &ComputedStyle) -> Self {
        self.clone().inheriting(from)
    }

    /// [`ComputedStyle::with_inherited_from`], taking this style rather
    /// than copying it.
    fn inheriting(self, from: &ComputedStyle) -> Self {
        ComputedStyle {
            color: from.color,
            font_size_px: from.font_size_px,
            font_family: from.font_family.clone(),
            text_align: from.text_align,
            text_transform: from.text_transform,
            white_space: from.white_space,
            letter_spacing: from.letter_spacing,
            word_spacing: from.word_spacing,
            direction: from.direction,
            border_collapse: from.border_collapse,
            border_spacing: from.border_spacing,
            caption_side: from.caption_side,
            empty_cells: from.empty_cells,
            quotes: from.quotes.clone(),
            fragmentation: inherited_fragmentation(from),
            custom: from.custom.clone(),
            ..self
        }
    }
}

/// The fragmentation properties a child of `from` inherits: none when
/// they are the initial values, `from`'s own when it sets nothing but
/// the inherited ones, which shares them, and a copy of those otherwise.
fn inherited_fragmentation(from: &ComputedStyle) -> Option<Arc<Fragmentation>> {
    let own = from.fragmentation.as_ref()?;
    let inherited = own.inherited();
    if inherited == Fragmentation::default() {
        None
    } else if **own == inherited {
        Some(Arc::clone(own))
    } else {
        Some(Arc::new(inherited))
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
        assert_eq!(s.width, Size::Auto);
        assert_eq!(s.height, Size::Auto);
        assert_eq!(s.margin, [const { LengthPercentageAuto::Px(0.0) }; 4]);
        assert_eq!(s.border_style, [BorderStyle::None; 4]);
        assert_eq!(s.border_width, [3.0; 4], "medium");
        assert_eq!(s.used_border_width(), [0.0; 4], "none paints nothing");
        assert!(!BorderStyle::Hidden.paints() && BorderStyle::Dotted.paints());
        assert_eq!(s.overflow, Overflow::Visible);
        assert!(!s.overflow.clips());
        assert_eq!(s.box_sizing, BoxSizing::ContentBox);
        assert_eq!(s.min_width, Size::Auto);
        assert_eq!(s.max_width, MaxSize::None);
        assert_eq!(s.min_height, Size::Auto);
        assert_eq!(s.max_height, MaxSize::None);
        assert_eq!(s.float, Float::None);
        assert_eq!(s.clear, Clear::None);
        assert_eq!(s.position, Position::Static);
        assert!(!s.position.is_positioned() && !s.position.is_out_of_flow());
        assert!(Position::Relative.is_positioned() && !Position::Relative.is_out_of_flow());
        assert!(Position::Fixed.is_positioned() && Position::Fixed.is_out_of_flow());
        assert_eq!(s.inset, [const { LengthPercentageAuto::Auto }; 4]);
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
        assert_eq!(s.flex_direction, FlexDirection::Row);
        assert_eq!(s.flex_wrap, FlexWrap::NoWrap);
        assert_eq!(s.order, 0);
        assert_eq!((s.flex_grow, s.flex_shrink), (0.0, 1.0));
        assert_eq!(s.flex_basis, FlexBasis::Size(Size::Auto));
        assert_eq!(s.justify_content, ContentAlignment::Normal);
        assert_eq!(s.align_content, ContentAlignment::Normal);
        assert_eq!(s.align_items, SelfAlignment::Normal);
        assert_eq!(s.align_self, SelfAlignment::Auto);
        assert_eq!((s.row_gap, s.column_gap), (None, None));
    }

    #[test]
    fn the_style_is_the_size_it_is_meant_to_be() {
        // Every byte here is copied with every box and fragment (M17's
        // T4.3). The grid properties (M18) cost a pointer and no more:
        // 424 bytes before them; the transform, opacity and will-change
        // properties (M19) another, 432 before them; the break properties,
        // `orphans`, `widows` and `print-color-adjust` (M20) another, 440
        // before them. `calc()` (M21.2) grew each value that may hold a
        // percentage, 22 of them, from 8 bytes to 16, to hold its
        // expression beside the length and the percentage: 448 before
        // it (`docs/design/values.md` says why not smaller). The custom
        // properties (M21.1) cost a pointer, 624 before them. Growing
        // this is a decision, not an accident; say why where the number
        // changes.
        assert_eq!(std::mem::size_of::<ComputedStyle>(), 632);
        assert_eq!(ComputedStyle::default().grid, None);
        assert_eq!(*ComputedStyle::default().grid(), GridStyle::default());
        assert_eq!(ComputedStyle::default().effects, None);
        assert_eq!(*ComputedStyle::default().effects(), Effects::default());
        assert_eq!(ComputedStyle::default().fragmentation, None);
        assert_eq!(
            *ComputedStyle::default().fragmentation(),
            Fragmentation::default()
        );
    }

    #[test]
    fn the_fragmentation_properties_inherit_orphans_widows_and_the_colour_adjustment() {
        let mut parent = ComputedStyle::default();
        assert_eq!(ComputedStyle::inherited_from(&parent).fragmentation, None);
        // The break properties alone: nothing to inherit.
        parent.fragmentation_mut().break_before = BreakBetween::Page;
        parent.fragmentation_mut().break_inside = BreakInside::Avoid;
        assert_eq!(ComputedStyle::inherited_from(&parent).fragmentation, None);
        // An inherited one: a copy of those alone.
        parent.fragmentation_mut().orphans = 4;
        let child = ComputedStyle::inherited_from(&parent);
        assert_eq!(
            *child.fragmentation(),
            Fragmentation {
                orphans: 4,
                ..Fragmentation::default()
            }
        );
        // Inherited ones alone: shared, not copied.
        let grandchild = ComputedStyle::inherited_from(&child);
        assert!(Arc::ptr_eq(
            grandchild.fragmentation.as_ref().unwrap(),
            child.fragmentation.as_ref().unwrap()
        ));
        let mut exact = ComputedStyle::default();
        exact.fragmentation_mut().print_color_adjust = PrintColorAdjust::Exact;
        exact.fragmentation_mut().widows = 7;
        let child = ComputedStyle::inherited_from(&exact);
        assert_eq!(
            child.fragmentation().print_color_adjust,
            PrintColorAdjust::Exact
        );
        assert_eq!(child.fragmentation().widows, 7);
        assert!(BreakBetween::Left.forces_page() && !BreakBetween::Column.forces_page());
        assert!(BreakBetween::AvoidPage.avoids_page() && !BreakBetween::AvoidColumn.avoids_page());
        assert!(BreakInside::Avoid.avoids_page() && !BreakInside::AvoidRegion.avoids_page());
    }

    #[test]
    fn the_effects_default_to_their_initial_values_and_mean_nothing() {
        let e = Effects::default();
        assert!(e.transform.is_empty());
        assert_eq!(e.transform_origin, TransformOrigin::CENTER);
        assert_eq!(e.transform_box, TransformBox::ViewBox);
        assert_eq!((&e.translate, e.rotate, e.scale), (&None, None, None));
        assert_eq!(e.transform_style, TransformStyle::Flat);
        assert_eq!(e.perspective, None);
        assert_eq!(e.perspective_origin, PerspectiveOrigin::CENTER);
        assert_eq!(e.backface_visibility, BackfaceVisibility::Visible);
        assert_eq!(e.opacity, 1.0);
        assert!(e.will_change.is_auto());
        assert!(!e.has_transform());
        assert!(!e.contains_all_descendants());
        assert!(!e.is_stacking_context());
        assert_eq!(LengthPercentage::Percent(50.0).resolve(200.0), 100.0);
        assert_eq!(LengthPercentage::sum(100.0, -10.0).resolve(200.0), 190.0);
    }

    #[test]
    fn the_effects_say_what_a_box_becomes() {
        // A transform, an identity one included, a perspective, and a
        // will-change naming either make a containing block for all
        // descendants and a stacking context (css-transforms-1 §2,
        // css-transforms-2 §2, §10, css-will-change-1 §2.2); an opacity
        // below one, preserve-3d and a will-change naming them a
        // stacking context only.
        let e = Effects {
            translate: Some(Translate {
                x: LengthPercentage::Px(0.0),
                y: LengthPercentage::Px(0.0),
                z: 0.0,
            }),
            ..Default::default()
        };
        assert!(e.has_transform() && e.contains_all_descendants() && e.is_stacking_context());
        let e = Effects {
            perspective: Some(500.0),
            ..Default::default()
        };
        assert!(!e.has_transform() && e.contains_all_descendants());
        let mut e = Effects {
            will_change: WillChange::SCALE,
            ..Default::default()
        };
        assert!(e.contains_all_descendants());
        e.will_change = WillChange::OPACITY.with(WillChange::CONTENTS);
        assert!(!e.contains_all_descendants() && e.is_stacking_context());
        assert!(e.will_change.names(WillChange::OPACITY));
        assert!(e.will_change.names(WillChange::CONTENTS));
        assert!(
            !e.will_change
                .names(WillChange::OPACITY.with(WillChange::SCALE))
        );
        assert!(
            e.will_change
                .names_any(WillChange::OPACITY.with(WillChange::SCALE))
        );
        e.will_change = WillChange::SCROLL_POSITION;
        assert!(!e.is_stacking_context());
        let e = Effects {
            opacity: 0.5,
            ..Default::default()
        };
        assert!(!e.contains_all_descendants() && e.is_stacking_context());
        // A `preserve-3d` box is a containing block for its descendants,
        // as the reference browser has it (M19 step 7).
        let mut e = Effects {
            transform_style: TransformStyle::Preserve3d,
            ..Default::default()
        };
        assert!(e.contains_all_descendants() && e.is_stacking_context());
        // The used transform-style is flat under a grouping property.
        assert_eq!(
            e.used_transform_style(Overflow::Visible),
            TransformStyle::Preserve3d
        );
        assert_eq!(
            e.used_transform_style(Overflow::Hidden),
            TransformStyle::Flat
        );
        e.opacity = 0.9;
        assert_eq!(
            e.used_transform_style(Overflow::Visible),
            TransformStyle::Flat
        );
        assert_eq!(
            WillChange::feature("transform"),
            Some(WillChange::TRANSFORM)
        );
        assert_eq!(WillChange::feature("left"), None);
    }

    #[test]
    fn effects_mut_copies_a_shared_value_once() {
        let mut a = ComputedStyle::default();
        a.effects_mut().opacity = 0.5;
        let b = a.clone();
        assert!(Arc::ptr_eq(
            a.effects.as_ref().unwrap(),
            b.effects.as_ref().unwrap()
        ));
        let mut c = b.clone();
        c.effects_mut().perspective = Some(100.0);
        assert!(!Arc::ptr_eq(
            b.effects.as_ref().unwrap(),
            c.effects.as_ref().unwrap()
        ));
        assert_eq!((b.effects().opacity, b.effects().perspective), (0.5, None));
        assert_eq!(
            (c.effects().opacity, c.effects().perspective),
            (0.5, Some(100.0))
        );
    }

    #[test]
    fn grid_mut_copies_a_shared_value_once() {
        let mut a = ComputedStyle::default();
        a.grid_mut().auto_flow.dense = true;
        let b = a.clone();
        assert!(Arc::ptr_eq(
            a.grid.as_ref().unwrap(),
            b.grid.as_ref().unwrap()
        ));
        let mut c = b.clone();
        c.grid_mut().auto_flow.column = true;
        assert!(!Arc::ptr_eq(
            b.grid.as_ref().unwrap(),
            c.grid.as_ref().unwrap()
        ));
        assert!(b.grid().auto_flow.dense && !b.grid().auto_flow.column);
        assert!(c.grid().auto_flow.dense && c.grid().auto_flow.column);
    }

    #[test]
    fn a_bare_legacy_takes_a_legacy_parents_justify_items() {
        // css-align-3 §6.1: `legacy`, the initial value, computes to the
        // parent's value when that carries `legacy`, else to `normal`.
        let mut parent = ComputedStyle::default();
        assert_eq!(
            ComputedStyle::inherited_from(&parent).grid,
            None,
            "nothing to carry"
        );
        let center = JustifyItems {
            value: SelfAlignment::Position(AlignPosition::Center, OverflowAlign::Default),
            legacy: true,
        };
        parent.grid_mut().justify_items = center;
        assert_eq!(
            ComputedStyle::inherited_from(&parent).grid().justify_items,
            center
        );
        parent.grid_mut().justify_items = JustifyItems {
            legacy: false,
            ..center
        };
        assert_eq!(
            ComputedStyle::inherited_from(&parent).grid().justify_items,
            JustifyItems::NORMAL
        );
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
        assert!(Display::InlineFlex.is_atomic_inline());
        assert!(!Display::Flex.is_atomic_inline());
        assert!(Display::Flex.is_flex() && Display::InlineFlex.is_flex());
        assert!(!Display::Block.is_flex() && !Display::InlineBlock.is_flex());
        assert!(Display::InlineGrid.is_atomic_inline());
        assert!(!Display::Grid.is_atomic_inline());
        assert!(Display::Grid.is_grid() && Display::InlineGrid.is_grid());
        assert!(!Display::Flex.is_grid() && !Display::Grid.is_flex());
        assert!(Display::Grid.has_items() && Display::InlineFlex.has_items());
        assert!(!Display::Block.has_items() && !Display::Table.has_items());
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
            (Display::Flex, Display::Flex),
            (Display::InlineFlex, Display::Flex),
            (Display::Grid, Display::Grid),
            (Display::InlineGrid, Display::Grid),
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

    #[test]
    fn effects_make_stacking_contexts_and_containing_blocks_as_their_properties_would() {
        // css-will-change-1 §2.2, css-transforms-1 §3, css-transforms-2
        // §9 (M19): what a transformable box's effects make of it, and
        // what an inline box's do, whose transform properties do
        // nothing.
        let with = |will_change: WillChange| Effects {
            will_change,
            ..Default::default()
        };
        let stacking = [
            WillChange::FILTER,
            WillChange::BACKDROP_FILTER,
            WillChange::CONTAIN,
            WillChange::OFFSET_PATH,
            WillChange::POSITION,
            WillChange::COMPOSITED,
            WillChange::OPACITY,
            WillChange::TRANSFORM_STYLE,
        ];
        for feature in stacking {
            assert!(with(feature).is_stacking_context_of(true), "{feature:?}");
            assert!(with(feature).is_stacking_context_of(false), "{feature:?}");
        }
        for feature in [
            WillChange::FILTER,
            WillChange::BACKDROP_FILTER,
            WillChange::CONTAIN,
            WillChange::OFFSET_PATH,
            WillChange::TRANSFORM_STYLE,
        ] {
            assert!(with(feature).contains_all_of(false), "{feature:?}");
        }
        assert!(!with(WillChange::POSITION).contains_all_of(true));
        assert!(with(WillChange::POSITION).contains_absolutes_of(true));
        for feature in [
            WillChange::Z_INDEX,
            WillChange::SCROLL_POSITION,
            WillChange::CONTENTS,
            WillChange::AUTO,
        ] {
            assert!(!with(feature).is_stacking_context_of(true), "{feature:?}");
            assert!(!with(feature).contains_absolutes_of(true), "{feature:?}");
        }
        // A `z-index` named applies where a `z-index` would: on a
        // positioned box or an item.
        let mut style = ComputedStyle::default();
        style.effects_mut().will_change = WillChange::Z_INDEX;
        assert!(!style.is_stacking_context(false));
        assert!(style.is_stacking_context(true));
        style.position = Position::Relative;
        assert!(style.is_stacking_context(false));
        // The transform properties on an inline box make nothing.
        let transformed = Effects {
            perspective: Some(100.0),
            ..Default::default()
        };
        assert!(transformed.is_stacking_context_of(true));
        assert!(!transformed.is_stacking_context_of(false));
        assert!(transformed.contains_all_of(true));
        assert!(!transformed.contains_all_of(false));
        let preserved = Effects {
            transform_style: TransformStyle::Preserve3d,
            ..Default::default()
        };
        assert!(preserved.contains_all_of(true));
        assert!(!preserved.contains_all_of(false));
        let faded = Effects {
            opacity: 0.5,
            ..Default::default()
        };
        assert!(faded.is_stacking_context_of(false));
        assert!(!faded.contains_absolutes_of(true));
    }
}
