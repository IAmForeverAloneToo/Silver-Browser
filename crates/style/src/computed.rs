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
use crate::cascade::MAX_LENGTH_PX;
use crate::custom::CustomProperties;
use crate::easing::Easing;

/// An RGBA color, 0.0–1.0 channels (see `paint::Rgba8` for the 8-bit
/// form `render` actually blits — `style` owns its own color type per
/// the layering contract: `paint` sits *above* `style` and converts at
/// the boundary, not the other way around). The channels are sRGB's, a
/// colour of a wider gamut's outside 0 to 1, clipped when drawn; the
/// colour keeps the space it was written in and its components there
/// (gap batch E, D48, [`crate::color`]).
#[derive(Debug, Clone, Copy)]
pub struct Color {
    /// Red channel.
    pub r: f32,
    /// Green channel.
    pub g: f32,
    /// Blue channel.
    pub b: f32,
    /// Alpha channel.
    pub a: f32,
    /// The space it was written in.
    pub(crate) space: crate::color::Space,
    /// Its components there; the channels for a legacy colour.
    pub(crate) c: [f32; 3],
    /// Which components are `none`, a bit each, alpha's the fourth.
    pub(crate) missing: u8,
}

impl Color {
    /// An opaque color from 0–255 channels (convenience for the named
    /// and hex-color tables).
    pub const fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Color::rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
    }

    /// Fully transparent black — `background-color`'s initial value.
    pub const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);

    /// Opaque black — the conventional UA default for `color`.
    pub const BLACK: Color = Color::rgba(0.0, 0.0, 0.0, 1.0);
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
    /// A block-level list item (Lists 3 §2, M21.6): a block container
    /// that generates a `::marker` box and increments the `list-item`
    /// counter. Blockified, it stays one.
    ListItem,
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
    /// table-internal box become a block; a block, a list item, a table,
    /// a flex container, a grid container and `none` stay what they
    /// are.
    pub fn blockified(self) -> Display {
        match self {
            Display::Block
            | Display::ListItem
            | Display::Table
            | Display::Flex
            | Display::Grid
            | Display::None => self,
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

/// `outline-style` (CSS UI 4 section 5.3, M26.4): `none`, `auto`, the
/// focus ring a browser draws, or one of the border styles but `hidden`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineStyle {
    /// No outline: the initial value.
    None,
    Auto,
    Solid,
    Double,
    Dashed,
    Dotted,
    Groove,
    Ridge,
    Inset,
    Outset,
}

impl OutlineStyle {
    /// Its keyword.
    pub fn keyword(self) -> &'static str {
        match self {
            OutlineStyle::None => "none",
            OutlineStyle::Auto => "auto",
            OutlineStyle::Solid => "solid",
            OutlineStyle::Double => "double",
            OutlineStyle::Dashed => "dashed",
            OutlineStyle::Dotted => "dotted",
            OutlineStyle::Groove => "groove",
            OutlineStyle::Ridge => "ridge",
            OutlineStyle::Inset => "inset",
            OutlineStyle::Outset => "outset",
        }
    }

    /// The style of the keyword, ASCII case-insensitively.
    pub fn from_keyword(keyword: &str) -> Option<OutlineStyle> {
        [
            OutlineStyle::None,
            OutlineStyle::Auto,
            OutlineStyle::Solid,
            OutlineStyle::Double,
            OutlineStyle::Dashed,
            OutlineStyle::Dotted,
            OutlineStyle::Groove,
            OutlineStyle::Ridge,
            OutlineStyle::Inset,
            OutlineStyle::Outset,
        ]
        .into_iter()
        .find(|style| keyword.eq_ignore_ascii_case(style.keyword()))
    }
}

/// `cursor` (CSS UI 4 section 4.1, M26.4): its keyword. A URL's image is
/// not shown: the keyword after it, which the syntax requires, is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    /// The browser's choice by what is under the pointer: the initial
    /// value.
    Auto,
    Default,
    None,
    ContextMenu,
    Help,
    Pointer,
    Progress,
    Wait,
    Cell,
    Crosshair,
    Text,
    VerticalText,
    Alias,
    Copy,
    Move,
    NoDrop,
    NotAllowed,
    Grab,
    Grabbing,
    EResize,
    NResize,
    NeResize,
    NwResize,
    SResize,
    SeResize,
    SwResize,
    WResize,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ColResize,
    RowResize,
    AllScroll,
    ZoomIn,
    ZoomOut,
}

/// The cursors and their keywords.
const CURSORS: [(Cursor, &str); 36] = [
    (Cursor::Auto, "auto"),
    (Cursor::Default, "default"),
    (Cursor::None, "none"),
    (Cursor::ContextMenu, "context-menu"),
    (Cursor::Help, "help"),
    (Cursor::Pointer, "pointer"),
    (Cursor::Progress, "progress"),
    (Cursor::Wait, "wait"),
    (Cursor::Cell, "cell"),
    (Cursor::Crosshair, "crosshair"),
    (Cursor::Text, "text"),
    (Cursor::VerticalText, "vertical-text"),
    (Cursor::Alias, "alias"),
    (Cursor::Copy, "copy"),
    (Cursor::Move, "move"),
    (Cursor::NoDrop, "no-drop"),
    (Cursor::NotAllowed, "not-allowed"),
    (Cursor::Grab, "grab"),
    (Cursor::Grabbing, "grabbing"),
    (Cursor::EResize, "e-resize"),
    (Cursor::NResize, "n-resize"),
    (Cursor::NeResize, "ne-resize"),
    (Cursor::NwResize, "nw-resize"),
    (Cursor::SResize, "s-resize"),
    (Cursor::SeResize, "se-resize"),
    (Cursor::SwResize, "sw-resize"),
    (Cursor::WResize, "w-resize"),
    (Cursor::EwResize, "ew-resize"),
    (Cursor::NsResize, "ns-resize"),
    (Cursor::NeswResize, "nesw-resize"),
    (Cursor::NwseResize, "nwse-resize"),
    (Cursor::ColResize, "col-resize"),
    (Cursor::RowResize, "row-resize"),
    (Cursor::AllScroll, "all-scroll"),
    (Cursor::ZoomIn, "zoom-in"),
    (Cursor::ZoomOut, "zoom-out"),
];

impl Cursor {
    /// Its keyword.
    pub fn keyword(self) -> &'static str {
        CURSORS
            .iter()
            .find(|(cursor, _)| *cursor == self)
            .map_or("auto", |(_, keyword)| keyword)
    }

    /// The cursor of the keyword, ASCII case-insensitively.
    pub fn from_keyword(keyword: &str) -> Option<Cursor> {
        CURSORS
            .iter()
            .find(|(_, name)| keyword.eq_ignore_ascii_case(name))
            .map(|(cursor, _)| *cursor)
    }
}

/// The outline properties (CSS UI 4 section 5, M26.4), which few
/// elements set, behind one pointer in the style: drawn outside the
/// border box, taking no room.
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    pub style: OutlineStyle,
    /// `outline-width` as declared; [`ComputedStyle::used_outline_width`]
    /// is what is drawn.
    pub width: f32,
    /// `outline-color`; `None` for `currentcolor`, its initial value.
    pub color: Option<Color>,
    pub offset: f32,
}

impl Default for Outline {
    fn default() -> Outline {
        Outline {
            style: OutlineStyle::None,
            width: 3.0,
            color: None,
            offset: 0.0,
        }
    }
}

static INITIAL_OUTLINE: LazyLock<Outline> = LazyLock::new(Outline::default);

/// `text-align-all`'s computed value, which `text-align` sets (css-text-3
/// §7.1, M7.5, M21.7): `match-parent` computes to the parent's value with
/// `start` and `end` made `left` or `right` by the parent's direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    /// The start of the line in the block's direction: the initial
    /// value.
    Start,
    /// Its end.
    End,
    Left,
    Right,
    Center,
    /// Both edges, the space left shared among the justification
    /// opportunities (`text-justify`).
    Justify,
}

impl TextAlign {
    /// `match-parent` (css-text-3 §7.1): this, the parent's value, with
    /// `start` and `end` made `left` or `right` by `direction`, the
    /// parent's.
    pub fn matched(self, direction: Direction) -> TextAlign {
        match (self, direction) {
            (TextAlign::Start, Direction::Ltr) | (TextAlign::End, Direction::Rtl) => {
                TextAlign::Left
            }
            (TextAlign::Start, Direction::Rtl) | (TextAlign::End, Direction::Ltr) => {
                TextAlign::Right
            }
            (other, _) => other,
        }
    }
}

/// `text-align-last`'s computed value (css-text-3 §7.2, M21.7): how the
/// last line of a block and a line before a forced break are aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlignLast {
    /// As `text-align-all`, but `start` where that is `justify`: the
    /// initial value.
    Auto,
    /// As the [`TextAlign`] value.
    Align(TextAlign),
}

/// `text-justify`'s computed value (css-text-3 §7.4, M21.7): where a
/// justified line's space goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextJustify {
    /// The user agent's choice, `inter-word` here: the initial value.
    Auto,
    /// Nowhere: a justified line is aligned to its start.
    None,
    /// Between words.
    InterWord,
    /// Between words and between the characters of a word;
    /// `distribute` too.
    InterCharacter,
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
    /// Content is clipped, but the box is no scroll container
    /// (css-overflow-3 section 3, M26.7): nothing scrolls it, and an
    /// inline-block keeps its line's baseline. A form control's.
    Clip,
}

impl Overflow {
    /// Whether this value clips the element's content to its padding
    /// box (every value but `visible`).
    pub fn clips(self) -> bool {
        self != Overflow::Visible
    }

    /// Whether this value makes the box a scroll container: every value
    /// but `visible` and `clip`.
    pub fn scrolls(self) -> bool {
        !matches!(self, Overflow::Visible | Overflow::Clip)
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
            LengthPercentageAuto::Percent(p) => percent_of(basis, *p),
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

/// `p` percent of `basis`, in CSS pixels: `NaN` is 0, and the length is
/// clamped to [`MAX_LENGTH_PX`] either way, as a math function's is
/// ([`Calc::resolve`]). A percentage is kept as written, as large as an
/// `f32` holds, or infinite where a math function made it so (M21), and
/// of a basis of any size would be an infinite length, or of a basis of
/// zero `NaN`.
fn percent_of(basis: f32, p: f32) -> f32 {
    let v = basis * p / 100.0;
    if v.is_nan() {
        0.0
    } else {
        v.clamp(-MAX_LENGTH_PX, MAX_LENGTH_PX)
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
            LengthPercentage::Percent(p) => percent_of(basis, *p),
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
    /// `skew()`: the angles along x and y, in degrees, the second zero
    /// when it is not written.
    Skew(f32, f32),
    /// `skewX()`, in degrees. Kept apart from `skew()` and `skewY()`:
    /// the three share no primitive, and interpolate pairwise only with
    /// their own kind (css-transforms-1 §10, M22), as the reference
    /// browser has them.
    SkewX(f32),
    /// `skewY()`, in degrees.
    SkewY(f32),
    /// `perspective()`: `None` for `none`, else the depth, a length.
    Perspective(Option<f32>),
    /// An interpolation the box must finish (M22, css-transforms-1 §9):
    /// the rests of two lists that met as matrices with a translation's
    /// percentage among them, which only the reference box can make a
    /// matrix of, kept with the progress between them. No declaration
    /// computes to one; an animation or a transition does.
    Interpolated(Arc<TransformMix>),
}

/// The two lists and the progress of a [`TransformFunction::Interpolated`].
#[derive(Debug, Clone, PartialEq)]
pub struct TransformMix {
    /// The list interpolated from.
    pub from: Vec<TransformFunction>,
    /// The list interpolated to.
    pub to: Vec<TransformFunction>,
    /// How far from `from` to `to`, outside [0, 1] when an easing
    /// overshoots.
    pub progress: f64,
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
    /// Not a property: what the element's animations current or in
    /// effect animate, which the engine acts as if `will-change` named
    /// (Web Animations §5.6, M22.2), so that an animated transform or
    /// opacity makes the box what it will be to layout for the whole
    /// run. The cascade sets it beside the properties.
    pub animated: WillChange,
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
            animated: WillChange::AUTO,
        }
    }
}

impl Effects {
    /// `will-change` with what the element's animations act as if it
    /// named ([`Effects::animated`]).
    pub fn will_change_in_effect(&self) -> WillChange {
        self.will_change.with(self.animated)
    }

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
            || self
                .will_change_in_effect()
                .names_any(WillChange::CONTAINING)
    }

    /// Whether the box is a containing block for its absolutely
    /// positioned descendants by these properties:
    /// [`Effects::contains_all_of`], or a `will-change` naming
    /// `position` (css-will-change-1 §2.2).
    pub fn contains_absolutes_of(&self, transformable: bool) -> bool {
        self.contains_all_of(transformable)
            || self
                .will_change_in_effect()
                .names_any(WillChange::CONTAINING_ABSOLUTES)
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
            || self.will_change_in_effect().names_any(WillChange::STACKING)
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

// ---- SVG (M32, docs/design/svg.md) ----------------------------------------

/// An SVG paint (`fill`, `stroke`, SVG 2 section 13.2), as computed:
/// `currentcolor` kept as the keyword, the element's `color` where it is
/// used, as a colour's `currentcolor` is.
#[derive(Debug, Clone, PartialEq)]
pub enum SvgPaint {
    /// `none`.
    None,
    /// A colour.
    Color(Color),
    /// `currentcolor`.
    CurrentColor,
    /// `context-fill`: kept, and drawn as `none` (G87).
    ContextFill,
    /// `context-stroke`: kept, and drawn as `none` (G87).
    ContextStroke,
    /// `url(...)`, the reference as written, and what stands in when it
    /// names no paint server: `None` when no fallback is written, else
    /// `none`, a colour or `currentcolor`.
    Url(Arc<str>, Option<Box<SvgPaint>>),
}

/// `fill-rule` and `clip-rule`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillRule {
    /// `nonzero`.
    #[default]
    NonZero,
    /// `evenodd`.
    EvenOdd,
}

/// `stroke-linecap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StrokeLinecap {
    /// `butt`.
    #[default]
    Butt,
    /// `round`.
    Round,
    /// `square`.
    Square,
}

/// `stroke-linejoin`: SVG 2's `miter-clip` and `arcs` are no values, as
/// the reference browser has them (G87).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StrokeLinejoin {
    /// `miter`.
    #[default]
    Miter,
    /// `round`.
    Round,
    /// `bevel`.
    Bevel,
}

/// `text-anchor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAnchor {
    /// `start`.
    #[default]
    Start,
    /// `middle`.
    Middle,
    /// `end`.
    End,
}

/// `dominant-baseline` (CSS Inline 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DominantBaseline {
    /// `auto`.
    #[default]
    Auto,
    /// `text-bottom`.
    TextBottom,
    /// `alphabetic`.
    Alphabetic,
    /// `ideographic`.
    Ideographic,
    /// `middle`.
    Middle,
    /// `central`.
    Central,
    /// `mathematical`.
    Mathematical,
    /// `hanging`.
    Hanging,
    /// `text-top`.
    TextTop,
}

/// `shape-rendering`: `crispEdges` draws without anti-aliasing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShapeRendering {
    /// `auto`.
    #[default]
    Auto,
    /// `optimizeSpeed`.
    OptimizeSpeed,
    /// `crispEdges`.
    CrispEdges,
    /// `geometricPrecision`.
    GeometricPrecision,
}

/// `vector-effect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VectorEffect {
    /// `none`.
    #[default]
    None,
    /// `non-scaling-stroke`.
    NonScalingStroke,
}

/// One of the layers `paint-order` orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintLayer {
    /// The fill.
    Fill,
    /// The stroke.
    Stroke,
    /// The markers.
    Markers,
}

/// `paint-order`: the three layers in the order painted, or `None` for
/// `normal`, which paints them as `fill stroke markers` does and is
/// told from it, as the reference browser serializes the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PaintOrder(pub Option<[PaintLayer; 3]>);

impl PaintOrder {
    /// The layers in the order painted.
    pub fn layers(self) -> [PaintLayer; 3] {
        self.0
            .unwrap_or([PaintLayer::Fill, PaintLayer::Stroke, PaintLayer::Markers])
    }
}

/// SVG's inherited properties (M32), behind one pointer a child shares
/// with its parent: what [`ComputedStyle::svg`] points to when any is not
/// its initial value.
#[derive(Debug, Clone, PartialEq)]
pub struct SvgInherited {
    /// `fill`.
    pub fill: SvgPaint,
    /// `fill-opacity`, in `[0, 1]`.
    pub fill_opacity: f32,
    /// `fill-rule`.
    pub fill_rule: FillRule,
    /// `stroke`.
    pub stroke: SvgPaint,
    /// `stroke-opacity`, in `[0, 1]`.
    pub stroke_opacity: f32,
    /// `stroke-width`, not negative.
    pub stroke_width: LengthPercentage,
    /// `stroke-linecap`.
    pub stroke_linecap: StrokeLinecap,
    /// `stroke-linejoin`.
    pub stroke_linejoin: StrokeLinejoin,
    /// `stroke-miterlimit`, at least 1.
    pub stroke_miterlimit: f32,
    /// `stroke-dasharray`; `None` is `none`.
    pub stroke_dasharray: Option<Arc<[LengthPercentage]>>,
    /// `stroke-dashoffset`.
    pub stroke_dashoffset: LengthPercentage,
    /// `clip-rule`.
    pub clip_rule: FillRule,
    /// `text-anchor`.
    pub text_anchor: TextAnchor,
    /// `dominant-baseline`.
    pub dominant_baseline: DominantBaseline,
    /// `paint-order`.
    pub paint_order: PaintOrder,
    /// `shape-rendering`.
    pub shape_rendering: ShapeRendering,
}

impl Default for SvgInherited {
    fn default() -> Self {
        SvgInherited {
            fill: SvgPaint::Color(Color::BLACK),
            fill_opacity: 1.0,
            fill_rule: FillRule::NonZero,
            stroke: SvgPaint::None,
            stroke_opacity: 1.0,
            stroke_width: LengthPercentage::Px(1.0),
            stroke_linecap: StrokeLinecap::Butt,
            stroke_linejoin: StrokeLinejoin::Miter,
            stroke_miterlimit: 4.0,
            stroke_dasharray: None,
            stroke_dashoffset: LengthPercentage::Px(0.0),
            clip_rule: FillRule::NonZero,
            text_anchor: TextAnchor::Start,
            dominant_baseline: DominantBaseline::Auto,
            paint_order: PaintOrder::default(),
            shape_rendering: ShapeRendering::Auto,
        }
    }
}

static INITIAL_SVG: LazyLock<SvgInherited> = LazyLock::new(SvgInherited::default);

/// SVG's properties an element does not inherit (M32): the stops', the
/// clipping path, `vector-effect` and the geometry properties (SVG 2
/// section 7; `width` and `height` are CSS's own).
#[derive(Debug, Clone, PartialEq)]
pub struct SvgOwn {
    /// `stop-color`: a colour or `currentcolor`.
    pub stop_color: SvgPaint,
    /// `stop-opacity`, in `[0, 1]`.
    pub stop_opacity: f32,
    /// `vector-effect`.
    pub vector_effect: VectorEffect,
    /// `clip-path`: the reference of a `url()`; `None` is `none`.
    pub clip_path: Option<Arc<str>>,
    /// `x`.
    pub x: LengthPercentage,
    /// `y`.
    pub y: LengthPercentage,
    /// `cx`.
    pub cx: LengthPercentage,
    /// `cy`.
    pub cy: LengthPercentage,
    /// `r`, not negative.
    pub r: LengthPercentage,
    /// `rx`, not negative.
    pub rx: LengthPercentageAuto,
    /// `ry`, not negative.
    pub ry: LengthPercentageAuto,
}

impl Default for SvgOwn {
    fn default() -> Self {
        SvgOwn {
            stop_color: SvgPaint::Color(Color::BLACK),
            stop_opacity: 1.0,
            vector_effect: VectorEffect::None,
            clip_path: None,
            x: LengthPercentage::Px(0.0),
            y: LengthPercentage::Px(0.0),
            cx: LengthPercentage::Px(0.0),
            cy: LengthPercentage::Px(0.0),
            r: LengthPercentage::Px(0.0),
            rx: LengthPercentageAuto::Auto,
            ry: LengthPercentageAuto::Auto,
        }
    }
}

static INITIAL_SVG_OWN: LazyLock<SvgOwn> = LazyLock::new(SvgOwn::default);

/// SVG's properties (M32), behind the one pointer of
/// [`ComputedStyle::svg`]: those an element inherits and its own.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Svg {
    /// The inherited ones.
    pub inherited: SvgInherited,
    /// The ones an element does not inherit.
    pub own: SvgOwn,
}

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

/// The counter properties (Lists 3 §4, M21.4) and the list properties
/// (§3, M21.6): what [`ComputedStyle::lists`] points to when any of them
/// is not its initial value. The counter properties are not inherited;
/// the list properties are.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Lists {
    /// `counter-reset`: the counters the element instantiates, in order,
    /// each name once, its last; empty for `none`.
    pub counter_reset: Vec<CounterReset>,
    /// `counter-increment`: the counters added to and by how much, in
    /// order, a name as often as written; empty for `none`.
    pub counter_increment: Vec<CounterChange>,
    /// `counter-set`: the counters set and to what, in order; empty for
    /// `none`.
    pub counter_set: Vec<CounterChange>,
    /// `list-style-type`: what a list item's marker says when its
    /// `content` is `normal` (§3.4).
    pub list_style_type: ListStyleType,
    /// `list-style-position`: where a list item's marker goes (§3.5).
    pub list_style_position: ListStylePosition,
    /// `list-style-image`: the image a marker is (M23.3), or `None` for
    /// `none`; `list-style-type` stands in until it loads and when it
    /// cannot be shown (section 3.3).
    pub list_style_image: Option<Image>,
}

impl Lists {
    /// The values a child inherits: the list properties, the counter
    /// properties initial.
    fn inherited(&self) -> Lists {
        Lists {
            list_style_type: self.list_style_type.clone(),
            list_style_position: self.list_style_position,
            list_style_image: self.list_style_image.clone(),
            ..Lists::default()
        }
    }
}

/// A `<ratio>` (css-values-4 section 6.2): a width over a height, as
/// written, both finite and never negative.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ratio {
    pub width: f32,
    pub height: f32,
}

impl Ratio {
    /// Width over height; `None` for a degenerate ratio, one with a zero
    /// on either side, which is no ratio (css-values-4 section 6.2).
    pub fn value(self) -> Option<f32> {
        (self.width > 0.0 && self.height > 0.0).then(|| self.width / self.height)
    }
}

/// `aspect-ratio` (css-sizing-4 section 5.1, M23.2).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AspectRatio {
    /// A replaced element's natural ratio, else none.
    #[default]
    Auto,
    /// This ratio.
    Ratio(Ratio),
    /// `auto && <ratio>`: a replaced element's natural ratio, else this
    /// one.
    AutoOr(Ratio),
}

/// `object-fit` (CSS Images 3 section 4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectFit {
    #[default]
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

/// `object-position` (CSS Images 3 section 4.6): where the image goes
/// in its box, as a `<position>`.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectPosition {
    /// Along x, of the room left beside the image.
    pub x: LengthPercentage,
    /// Along y, the same way.
    pub y: LengthPercentage,
}

impl ObjectPosition {
    /// The initial value, `50% 50%`.
    pub const CENTER: ObjectPosition = ObjectPosition {
        x: LengthPercentage::Percent(50.0),
        y: LengthPercentage::Percent(50.0),
    };
}

impl Default for ObjectPosition {
    fn default() -> Self {
        ObjectPosition::CENTER
    }
}

/// `image-orientation` (CSS Images 3 section 5.1): the deprecated angles
/// and `flip` are not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageOrientation {
    /// The orientation the image's EXIF data gives.
    #[default]
    FromImage,
    /// The pixels as stored.
    None,
}

/// `image-rendering` (CSS Images 3 section 5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageRendering {
    #[default]
    Auto,
    Smooth,
    HighQuality,
    Pixelated,
    CrispEdges,
}

impl ImageRendering {
    /// Whether an image scaled is drawn with its pixels kept sharp:
    /// `pixelated` and `crisp-edges`, nearest neighbour.
    pub fn is_sharp(self) -> bool {
        matches!(self, ImageRendering::Pixelated | ImageRendering::CrispEdges)
    }
}

/// `aspect-ratio` and the image properties of CSS Images 3 (M23.2):
/// what [`ComputedStyle::image`] points to when any of them is not its
/// initial value. `aspect-ratio`, `object-fit` and `object-position` are
/// not inherited; `image-orientation` and `image-rendering` are.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImageStyle {
    pub aspect_ratio: AspectRatio,
    pub object_fit: ObjectFit,
    pub object_position: ObjectPosition,
    pub image_orientation: ImageOrientation,
    pub image_rendering: ImageRendering,
}

impl ImageStyle {
    /// The values a child inherits: the inherited ones kept, the others
    /// initial.
    fn inherited(&self) -> ImageStyle {
        ImageStyle {
            image_orientation: self.image_orientation,
            image_rendering: self.image_rendering,
            ..ImageStyle::default()
        }
    }
}

/// An `<image>` (CSS Images 3 section 2, M23.3): what a background, a
/// border image, a list marker and `content` show.
#[derive(Debug, Clone, PartialEq)]
pub enum Image {
    /// A `url()`, or the option an `image-set()` chose (Images 4 section
    /// 2.4): absolute where its sheet's URL was known, the loader having
    /// made it so; as written in a `style` attribute or element, which
    /// the document's base resolves. `resolution` is the image's natural
    /// resolution, in image pixels a CSS pixel: 1 but for an
    /// `image-set()` option that says otherwise.
    Url { url: Arc<str>, resolution: f32 },
    /// A gradient (section 3).
    Gradient(Arc<Gradient>),
}

impl Image {
    /// The URL the image is fetched from, if it is fetched.
    pub fn url(&self) -> Option<&str> {
        match self {
            Image::Url { url, .. } => Some(url),
            Image::Gradient(_) => None,
        }
    }
}

/// The most colour stops a gradient may have (M23.3): one with more is
/// invalid.
pub const MAX_GRADIENT_STOPS: usize = 256;

/// A gradient (CSS Images 3 section 3, M23.3): its line or ending shape,
/// whether it repeats, and its colour stops and transition hints.
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub shape: GradientShape,
    /// A `repeating-` gradient: its stops repeat along its line.
    pub repeating: bool,
    /// The stops and hints as written, a double position being two
    /// stops: a stop first and last, a hint only between two stops.
    pub items: Vec<GradientItem>,
}

/// What a gradient's line is (sections 3.1 and 3.2).
#[derive(Debug, Clone, PartialEq)]
pub enum GradientShape {
    /// `linear-gradient()`: its line's direction.
    Linear(LineDirection),
    /// `radial-gradient()`: an ellipse or a circle, its size, and its
    /// centre, a position in the gradient box.
    Radial {
        ellipse: bool,
        size: RadialSize,
        center: [LengthPercentage; 2],
    },
}

/// A linear gradient's direction (section 3.1.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineDirection {
    /// An angle in degrees, 0 up and clockwise: `to top` is 0, `to
    /// right` 90.
    Angle(f32),
    /// `to` a corner: towards the right or the left, the bottom or the
    /// top.
    Corner { right: bool, bottom: bool },
}

/// A radial gradient's size (section 3.2.1).
#[derive(Debug, Clone, PartialEq)]
pub enum RadialSize {
    ClosestSide,
    FarthestSide,
    ClosestCorner,
    FarthestCorner,
    /// A circle's radius, in pixels.
    Circle(f32),
    /// An ellipse's radii, a percentage of the box's side.
    Ellipse(LengthPercentage, LengthPercentage),
}

/// One of a gradient's colour stops or transition hints (section 3.4).
#[derive(Debug, Clone, PartialEq)]
pub enum GradientItem {
    /// A colour, at a position along the line or where the fixup puts
    /// it.
    Stop(Color, Option<LengthPercentage>),
    /// Where the colour halfway between the stops on either side is.
    Hint(LengthPercentage),
}

/// A side of `border-image-slice` (CSS Backgrounds 3 section 6.2,
/// M23.3): image pixels, or a percentage of the image's size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SliceSide {
    Number(f32),
    Percent(f32),
}

/// A side of `border-image-width` (section 6.3): a multiple of the
/// border's width; a length, or a percentage of the border image area;
/// or `auto`, the slice's own size.
#[derive(Debug, Clone, PartialEq)]
pub enum BorderImageWidth {
    Number(f32),
    Length(LengthPercentage),
    Auto,
}

/// A side of `border-image-outset` (section 6.4): a multiple of the
/// border's width, or a length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outset {
    Number(f32),
    Px(f32),
}

/// One axis of `border-image-repeat` (section 6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderImageRepeat {
    Stretch,
    Repeat,
    Round,
    Space,
}

/// The `border-image-*` properties (section 6, M23.3), kept apart while
/// any is not its initial value; none is inherited. Sides are top,
/// right, bottom, left.
#[derive(Debug, Clone, PartialEq)]
pub struct BorderImage {
    /// `border-image-source`: `None` for `none`.
    pub source: Option<Image>,
    pub slice: [SliceSide; 4],
    /// `fill` in `border-image-slice`: the middle is drawn too.
    pub fill: bool,
    pub width: [BorderImageWidth; 4],
    pub outset: [Outset; 4],
    /// Along x, then along y.
    pub repeat: [BorderImageRepeat; 2],
}

impl Default for BorderImage {
    fn default() -> Self {
        BorderImage {
            source: None,
            slice: [SliceSide::Percent(100.0); 4],
            fill: false,
            width: std::array::from_fn(|_| BorderImageWidth::Number(1.0)),
            outset: [Outset::Number(0.0); 4],
            repeat: [BorderImageRepeat::Stretch; 2],
        }
    }
}

/// One axis of `background-repeat` (CSS Backgrounds 3 section 2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Repeat,
    Space,
    Round,
    NoRepeat,
}

/// One layer's `background-size` (section 2.4): `cover`, `contain`, or a
/// width and a height, `None` for `auto`.
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundSize {
    Cover,
    Contain,
    Sized(Option<LengthPercentage>, Option<LengthPercentage>),
}

impl Default for BackgroundSize {
    fn default() -> Self {
        BackgroundSize::Sized(None, None)
    }
}

/// A box `background-origin` and `background-clip` name (sections 2.6
/// and 2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundBox {
    BorderBox,
    PaddingBox,
    ContentBox,
}

/// One layer's `background-attachment` (section 2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Attachment {
    #[default]
    Scroll,
    Fixed,
    Local,
}

/// The most layers a box's background may have (M23.3): a list longer
/// than this makes its declaration invalid.
pub const MAX_IMAGE_LAYERS: usize = 256;

/// The background properties but `background-color` (CSS Backgrounds 3
/// section 2, M23.3): what [`ComputedStyle::background`] points to when
/// any of them is not its initial value. Each is a list of one value a
/// layer, the first on top; none is inherited. There are as many layers
/// as `background-image` has values, the other lists repeated to that
/// number or cut to it (section 2.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Backgrounds {
    /// `background-image`: `None` for `none`.
    pub image: Vec<Option<Image>>,
    /// `background-repeat`, along x and along y.
    pub repeat: Vec<[Repeat; 2]>,
    /// `background-position`, each from the left and the top edge.
    pub position: Vec<[LengthPercentage; 2]>,
    pub size: Vec<BackgroundSize>,
    pub origin: Vec<BackgroundBox>,
    pub clip: Vec<BackgroundBox>,
    pub attachment: Vec<Attachment>,
}

impl Default for Backgrounds {
    fn default() -> Self {
        Backgrounds {
            image: vec![None],
            repeat: vec![[Repeat::Repeat; 2]],
            position: vec![[
                LengthPercentage::Percent(0.0),
                LengthPercentage::Percent(0.0),
            ]],
            size: vec![BackgroundSize::default()],
            origin: vec![BackgroundBox::PaddingBox],
            clip: vec![BackgroundBox::BorderBox],
            attachment: vec![Attachment::Scroll],
        }
    }
}

/// One background layer, each of its values taken from its list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundLayer<'a> {
    pub image: Option<&'a Image>,
    pub repeat: [Repeat; 2],
    pub position: &'a [LengthPercentage; 2],
    pub size: &'a BackgroundSize,
    pub origin: BackgroundBox,
    pub clip: BackgroundBox,
    pub attachment: Attachment,
}

impl Backgrounds {
    /// The layers, the first on top.
    pub fn layers(&self) -> impl Iterator<Item = BackgroundLayer<'_>> {
        fn at<T>(list: &[T], i: usize) -> &T {
            &list[i % list.len()]
        }
        (0..self.image.len()).map(move |i| BackgroundLayer {
            image: self.image[i].as_ref(),
            repeat: *at(&self.repeat, i),
            position: at(&self.position, i),
            size: at(&self.size, i),
            origin: *at(&self.origin, i),
            clip: *at(&self.clip, i),
            attachment: *at(&self.attachment, i),
        })
    }

    /// The box the colour is painted in: the bottom layer's
    /// `background-clip` (section 2.7).
    pub fn color_clip(&self) -> BackgroundBox {
        self.clip[(self.image.len() - 1) % self.clip.len()]
    }
}

/// The text decoration properties (Text Decoration 3 and 4, M21.7):
/// what [`ComputedStyle::decoration`] points to when any of them is not
/// its initial value. The line, its style, colour and thickness are not
/// inherited; the underline's offset and position are. An element's
/// decoration is not inherited either way but propagated by the box
/// tree to the text it decorates, which keeps the element's values.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Decoration {
    /// `text-decoration-line`.
    pub line: DecorationLine,
    /// `text-decoration-style`.
    pub style: DecorationStyle,
    /// `text-decoration-color`: `None` for `currentcolor`, the initial
    /// value, the element's `color`.
    pub color: Option<Color>,
    /// `text-decoration-thickness`.
    pub thickness: DecorationThickness,
    /// `text-underline-offset`: `None` for `auto`, a percentage of the
    /// font size otherwise.
    pub underline_offset: Option<LengthPercentage>,
    /// `text-underline-position`.
    pub underline_position: UnderlinePosition,
}

impl Decoration {
    /// The values a child inherits: the underline's offset and
    /// position, the rest initial.
    fn inherited(&self) -> Decoration {
        Decoration {
            underline_offset: self.underline_offset.clone(),
            underline_position: self.underline_position,
            ..Decoration::default()
        }
    }
}

/// `text-decoration-line`'s computed value: the lines, none of them for
/// `none`, the initial value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DecorationLine {
    pub underline: bool,
    pub overline: bool,
    pub line_through: bool,
    /// Taken, and never drawn: the user agent may not blink (Text
    /// Decoration 3 §2.1).
    pub blink: bool,
}

impl DecorationLine {
    /// Whether it draws a line.
    pub fn draws(&self) -> bool {
        self.underline || self.overline || self.line_through
    }
}

/// The transition and animation properties (Transitions 1 and 2 §2,
/// Animations 1 and 2 §4; M22): what [`ComputedStyle::motion`] points to
/// when any of them is not its initial value. None is inherited. Each is
/// a list: an element's transitions are as many as the items of
/// `transition-property`, its animations as those of `animation-name`,
/// and each other list of the group is repeated or cut to that length
/// (Values 4 Appendix A), as [`Motion::transitions`] and
/// [`Motion::animations`] give them. Every list but
/// `transition-property`'s has at least one item.
#[derive(Debug, Clone, PartialEq)]
pub struct Motion {
    /// `transition-property`: empty for `none`.
    pub transition_property: Vec<TransitionProperty>,
    /// `transition-duration`, in seconds, none below zero.
    pub transition_duration: Vec<f64>,
    /// `transition-timing-function`.
    pub transition_timing_function: Vec<Easing>,
    /// `transition-delay`, in seconds.
    pub transition_delay: Vec<f64>,
    /// `transition-behavior` (Transitions 2).
    pub transition_behavior: Vec<TransitionBehavior>,
    /// `animation-name`: `None` for `none`, the name as written
    /// otherwise, an identifier's or a string's.
    pub animation_name: Vec<Option<Arc<str>>>,
    /// `animation-duration`, in seconds, none below zero: `auto`
    /// (Animations 2) is zero, as it is for an animation on a
    /// document's timeline.
    pub animation_duration: Vec<f64>,
    /// `animation-timing-function`.
    pub animation_timing_function: Vec<Easing>,
    /// `animation-iteration-count`: none below zero, `infinite` an
    /// infinity.
    pub animation_iteration_count: Vec<f64>,
    /// `animation-direction`.
    pub animation_direction: Vec<AnimationDirection>,
    /// `animation-play-state`.
    pub animation_play_state: Vec<AnimationPlayState>,
    /// `animation-delay`, in seconds.
    pub animation_delay: Vec<f64>,
    /// `animation-fill-mode`.
    pub animation_fill_mode: Vec<AnimationFillMode>,
}

impl Default for Motion {
    fn default() -> Self {
        Motion {
            transition_property: vec![TransitionProperty::All],
            transition_duration: vec![0.0],
            transition_timing_function: vec![Easing::EASE],
            transition_delay: vec![0.0],
            transition_behavior: vec![TransitionBehavior::Normal],
            animation_name: vec![None],
            animation_duration: vec![0.0],
            animation_timing_function: vec![Easing::EASE],
            animation_iteration_count: vec![1.0],
            animation_direction: vec![AnimationDirection::Normal],
            animation_play_state: vec![AnimationPlayState::Running],
            animation_delay: vec![0.0],
            animation_fill_mode: vec![AnimationFillMode::None],
        }
    }
}

/// The item of a coordinated list at `index`, the list repeated as far
/// as it must be (Values 4 Appendix A); the list has at least one item.
fn coordinated<T>(list: &[T], index: usize) -> &T {
    &list[index % list.len()]
}

impl Motion {
    /// The element's transitions, one for each item of
    /// `transition-property`, with the other properties' items at its
    /// place.
    pub fn transitions(&self) -> impl Iterator<Item = SingleTransition<'_>> {
        self.transition_property
            .iter()
            .enumerate()
            .map(|(i, property)| SingleTransition {
                property,
                duration: *coordinated(&self.transition_duration, i),
                timing_function: coordinated(&self.transition_timing_function, i),
                delay: *coordinated(&self.transition_delay, i),
                behavior: *coordinated(&self.transition_behavior, i),
            })
    }

    /// The element's animations, one for each item of `animation-name`,
    /// `none` included, with the other properties' items at its place.
    pub fn animations(&self) -> impl Iterator<Item = SingleAnimation<'_>> {
        self.animation_name
            .iter()
            .enumerate()
            .map(|(i, name)| SingleAnimation {
                name: name.as_ref(),
                duration: *coordinated(&self.animation_duration, i),
                timing_function: coordinated(&self.animation_timing_function, i),
                iteration_count: *coordinated(&self.animation_iteration_count, i),
                direction: *coordinated(&self.animation_direction, i),
                play_state: *coordinated(&self.animation_play_state, i),
                delay: *coordinated(&self.animation_delay, i),
                fill_mode: *coordinated(&self.animation_fill_mode, i),
            })
    }
}

/// One item of `transition-property`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionProperty {
    /// `all`: every property that can transition.
    All,
    /// A property by its name, ASCII-lowercased unless it is a custom
    /// property's; one that names no property is kept and matches
    /// nothing (Transitions 1 §2.1).
    Name(Arc<str>),
}

/// One item of `transition-behavior` (Transitions 2 §2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionBehavior {
    /// The initial value: a discrete change does not transition.
    Normal,
    /// `allow-discrete`: a discrete change transitions too, flipping half
    /// way.
    AllowDiscrete,
}

/// One item of `animation-direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationDirection {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

/// One item of `animation-play-state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationPlayState {
    Running,
    Paused,
}

/// One item of `animation-fill-mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationFillMode {
    None,
    Forwards,
    Backwards,
    Both,
}

/// One transition an element's style asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SingleTransition<'m> {
    pub property: &'m TransitionProperty,
    /// In seconds.
    pub duration: f64,
    pub timing_function: &'m Easing,
    /// In seconds.
    pub delay: f64,
    pub behavior: TransitionBehavior,
}

/// One animation an element's style asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SingleAnimation<'m> {
    /// `None` for `none`, which runs nothing but keeps its place.
    pub name: Option<&'m Arc<str>>,
    /// In seconds.
    pub duration: f64,
    pub timing_function: &'m Easing,
    pub iteration_count: f64,
    pub direction: AnimationDirection,
    pub play_state: AnimationPlayState,
    /// In seconds.
    pub delay: f64,
    pub fill_mode: AnimationFillMode,
}

/// `text-decoration-style`'s computed value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecorationStyle {
    /// The initial value.
    #[default]
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

/// `text-decoration-thickness`'s computed value.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum DecorationThickness {
    /// The user agent's choice, the initial value.
    #[default]
    Auto,
    /// The font's own underline thickness.
    FromFont,
    /// A length, or a percentage of the font size.
    Length(LengthPercentage),
}

/// `text-underline-position`'s computed value, `left` and `right`
/// taken as `auto`, which they are in horizontal text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnderlinePosition {
    /// The initial value.
    #[default]
    Auto,
    /// Below the text's descent.
    Under,
    /// Where the font puts it.
    FromFont,
}

/// `list-style-type`'s computed value (Lists 3 §3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListStyleType {
    /// A counter style, by name: a predefined style's ASCII-lowercased,
    /// any other kept as written (Counter Styles 3 §3); `disc` is the
    /// initial value.
    Style(Arc<str>),
    /// A string, the marker's text as it is.
    String(Arc<str>),
    /// `none`: no marker, unless `::marker`'s `content` makes one.
    None,
}

impl Default for ListStyleType {
    fn default() -> Self {
        ListStyleType::Style(Arc::from("disc"))
    }
}

/// `list-style-position`'s computed value (Lists 3 §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListStylePosition {
    /// The marker is outside the list item's box, before its first line:
    /// the initial value.
    #[default]
    Outside,
    /// The marker is an inline box at the start of the list item's
    /// content.
    Inside,
}

/// One counter `counter-reset` instantiates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterReset {
    /// Its name, case-sensitive.
    pub name: Arc<str>,
    /// Whether it is `reversed()`.
    pub reversed: bool,
    /// Its starting value: the integer written, 0 when none is; `None`
    /// for a reversed counter written without one, whose value the box
    /// tree calculates from what increments it (§4.4.2).
    pub value: Option<i32>,
}

/// One counter `counter-increment` or `counter-set` changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterChange {
    /// Its name, case-sensitive.
    pub name: Arc<str>,
    /// What it is added to it, or set to.
    pub value: i32,
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

static INITIAL_LISTS: LazyLock<Lists> = LazyLock::new(Lists::default);

/// The background properties' initial values, which
/// [`ComputedStyle::background`] gives a style that sets none of them.
static INITIAL_BACKGROUND: LazyLock<Backgrounds> = LazyLock::new(Backgrounds::default);

/// The border image properties' initial values, which
/// [`ComputedStyle::border_image`] gives a style that sets none of them.
static INITIAL_BORDER_IMAGE: LazyLock<BorderImage> = LazyLock::new(BorderImage::default);

/// `border-radius`'s longhands (Backgrounds 3 section 5.1, gap batch E,
/// D50): each corner's horizontal and vertical radius, the top-left,
/// top-right, bottom-right and bottom-left corners in that order.
#[derive(Debug, Clone, PartialEq)]
pub struct Radii(pub [[LengthPercentage; 2]; 4]);

impl Default for Radii {
    fn default() -> Self {
        Radii(std::array::from_fn(|_| {
            [LengthPercentage::Px(0.0), LengthPercentage::Px(0.0)]
        }))
    }
}

static INITIAL_RADII: LazyLock<Radii> = LazyLock::new(Radii::default);

/// The image properties' initial values, which [`ComputedStyle::image`]
/// gives a style that sets none of them.
static INITIAL_IMAGE: LazyLock<ImageStyle> = LazyLock::new(ImageStyle::default);

/// The text decoration properties' initial values, which
/// [`ComputedStyle::decoration`] gives a style that sets none of them.
static INITIAL_DECORATION: LazyLock<Decoration> = LazyLock::new(Decoration::default);

/// The transition and animation properties' initial values, which
/// [`ComputedStyle::motion`] gives a style that sets none of them.
static INITIAL_MOTION: LazyLock<Motion> = LazyLock::new(Motion::default);

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
/// `::before`, `::after` and `::marker` (M21.6) read.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Content {
    /// `normal`, the initial value; on `::before` and `::after` it
    /// computes to `none`, and on `::marker` it is a marker made from
    /// the list item's `list-style-type` (Lists 3 §3.2).
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
#[derive(Debug, Clone, PartialEq)]
pub enum ContentItem {
    /// A string, or the value of an `attr()`.
    Text(String),
    /// `counter(<name>, <style>)` (M21.4): the innermost counter of the
    /// name the box sees, written in the counter style named, `decimal`
    /// when none is given; the box tree's builder writes it.
    Counter { name: Arc<str>, style: Arc<str> },
    /// `counters(<name>, <separator>, <style>)` (M21.4): every counter of
    /// the name the box sees, outermost first, each written in the style
    /// and joined by the separator.
    Counters {
        name: Arc<str>,
        separator: String,
        style: Arc<str>,
    },
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
    /// An image (M23.3): alone on an element, what the element shows in
    /// place of its content; in a pseudo-element's list, an image among
    /// its text.
    Image(Image),
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
    /// `text-align-last` (M21.7) — inherited.
    pub text_align_last: TextAlignLast,
    /// `text-justify` (M21.7) — inherited.
    pub text_justify: TextJustify,
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
    /// Which sides' `border-*-color` is `currentcolor`, resolved into
    /// `border_color`, a bit each, `1 << side` (M33): the reference
    /// browser draws an `inset`, `outset`, `groove` or `ridge` side of
    /// that colour in rgb(238, 238, 238) whatever `color` is. A byte in
    /// the padding; read and written by side through
    /// [`ComputedStyle::border_color_is_current`].
    pub border_current_color: u8,
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
    /// The outline properties (M26.4), behind one pointer likewise. Read
    /// through [`ComputedStyle::outline`].
    pub outline: Option<Arc<Outline>>,
    /// `cursor` (M26.4), inherited.
    pub cursor: Cursor,
    /// Whether the element uses the dark colour scheme (gap batch E,
    /// D49): what its system colours and `light-dark()` resolve by, its
    /// `color-scheme` and the user's preference deciding.
    pub dark: bool,
    /// `color-scheme` (D49), inherited: its words as written, `only`
    /// last; `None` is `normal`.
    pub color_scheme: Option<Arc<String>>,
    /// The break properties, `orphans`, `widows` and
    /// `print-color-adjust` (M20), behind one pointer likewise; `None` is
    /// every initial value. Read through [`ComputedStyle::fragmentation`].
    pub fragmentation: Option<Arc<Fragmentation>>,
    /// The custom properties (M21.1) — inherited, as every custom
    /// property is; `None` when none has a value. Shared with the parent
    /// unless the element declares one. Read through
    /// [`ComputedStyle::custom_property`].
    pub custom: Option<Arc<CustomProperties>>,
    /// The counter properties (M21.4) and the list properties (M21.6),
    /// behind one pointer as the grid's are, the inherited half copied to
    /// a child as `fragmentation`'s is; `None` is every initial value.
    /// Read through [`ComputedStyle::lists`].
    pub lists: Option<Arc<Lists>>,
    /// The text decoration properties (M21.7), behind one pointer as the
    /// list properties are; `None` is every initial value. Read through
    /// [`ComputedStyle::decoration`].
    pub decoration: Option<Arc<Decoration>>,
    /// Not a property: whether the element, or an ancestor, is inert
    /// (Display 4 §2.9, M22.1) — shown by a transition of `display` alone
    /// while its style without it has `display: none` — so that hit
    /// testing passes over it and over what is inside it. Inherited.
    pub inert: bool,
    /// `display: flow-root` (gap batch E): a block box that establishes
    /// a block formatting context, `display` being `Block` (or
    /// `ListItem`, `flow-root list-item`).
    pub flow_root: bool,
    /// Not a property: whether the style is the root element's, whose
    /// box establishes an independent formatting context (css-display-3
    /// section 2.8, CSS2 section 8.3.1: its margins do not collapse with
    /// its children's; G11).
    pub root: bool,
    /// The transition and animation properties (M22), behind one pointer
    /// as the effects are; `None` is every initial value. Read through
    /// [`ComputedStyle::motion`].
    pub motion: Option<Arc<Motion>>,
    /// `aspect-ratio` and the image properties (M23.2), behind one
    /// pointer, the inherited half copied to a child as the list
    /// properties are; `None` is every initial value. Read through
    /// [`ComputedStyle::image`].
    pub image: Option<Arc<ImageStyle>>,
    /// The background properties but `background-color` (M23.3), behind
    /// one pointer; `None` is every initial value. Read through
    /// [`ComputedStyle::background`].
    pub background: Option<Arc<Backgrounds>>,
    /// The border image properties (M23.3): `None` while all are their
    /// initial values.
    pub border_image: Option<Arc<BorderImage>>,
    /// The corners' radii (gap batch E, D50): `None` while all are 0.
    pub radii: Option<Arc<Radii>>,
    /// SVG's properties (M32): `None` while all are their initial
    /// values. Read through [`ComputedStyle::svg`] and
    /// [`ComputedStyle::svg_own`].
    pub svg: Option<Arc<Svg>>,
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
            text_align: TextAlign::Start,
            text_align_last: TextAlignLast::Auto,
            text_justify: TextJustify::Auto,
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
            border_current_color: 0b1111,
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
            outline: None,
            cursor: Cursor::Auto,
            dark: false,
            color_scheme: None,
            fragmentation: None,
            custom: None,
            lists: None,
            decoration: None,
            inert: false,
            flow_root: false,
            root: false,
            motion: None,
            image: None,
            background: None,
            border_image: None,
            radii: None,
            svg: None,
        }
    }
}

impl ComputedStyle {
    /// Whether side `side`'s `border-*-color` is `currentcolor` (M33).
    pub fn border_color_is_current(&self, side: usize) -> bool {
        self.border_current_color & (1 << side) != 0
    }

    /// Sets whether side `side`'s `border-*-color` is `currentcolor`.
    pub fn set_border_color_current(&mut self, side: usize, current: bool) {
        if current {
            self.border_current_color |= 1 << side;
        } else {
            self.border_current_color &= !(1 << side);
        }
    }

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

    /// The outline properties (M26.4): this style's, or their initial
    /// values.
    pub fn outline(&self) -> &Outline {
        self.outline.as_deref().unwrap_or(&INITIAL_OUTLINE)
    }

    /// The outline properties to change, this style's own copy made first
    /// if it shares them or has none.
    pub fn outline_mut(&mut self) -> &mut Outline {
        Arc::make_mut(self.outline.get_or_insert_with(Default::default))
    }

    /// The outline's colour, `currentcolor` resolved.
    pub fn outline_color(&self) -> Color {
        self.outline().color.unwrap_or(self.color)
    }

    /// The outline's width the box uses (M26.4): none for an outline
    /// style of `none`, else rounded as a border's is.
    pub fn used_outline_width(&self) -> f32 {
        let outline = self.outline();
        if outline.style == OutlineStyle::None || outline.width <= 0.0 {
            return 0.0;
        }
        outline.width.floor().max(1.0)
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

    /// The counter properties (M21.4): this style's, or their initial
    /// values.
    pub fn lists(&self) -> &Lists {
        self.lists.as_deref().unwrap_or(&INITIAL_LISTS)
    }

    /// The counter properties to change, this style's own copy made first
    /// if it shares them or has none.
    pub fn lists_mut(&mut self) -> &mut Lists {
        Arc::make_mut(self.lists.get_or_insert_with(Default::default))
    }

    /// The text decoration properties (M21.7), the initial values when
    /// the style has none of its own.
    pub fn decoration(&self) -> &Decoration {
        self.decoration.as_deref().unwrap_or(&INITIAL_DECORATION)
    }

    /// The text decoration properties to change, this style's own copy
    /// made first if it shares them or has none.
    pub fn decoration_mut(&mut self) -> &mut Decoration {
        Arc::make_mut(self.decoration.get_or_insert_with(Default::default))
    }

    /// The transition and animation properties (M22), the initial values
    /// when the style sets none of them.
    pub fn motion(&self) -> &Motion {
        self.motion.as_deref().unwrap_or(&INITIAL_MOTION)
    }

    /// The transition and animation properties to change, this style's
    /// own copy made first if it shares them or has none.
    pub fn motion_mut(&mut self) -> &mut Motion {
        Arc::make_mut(self.motion.get_or_insert_with(Default::default))
    }

    /// The background properties but `background-color` (M23.3), the
    /// initial values when the style sets none of them.
    pub fn background(&self) -> &Backgrounds {
        self.background.as_deref().unwrap_or(&INITIAL_BACKGROUND)
    }

    /// The background properties to change, this style's own copy made
    /// first if it shares them or has none.
    pub fn background_mut(&mut self) -> &mut Backgrounds {
        Arc::make_mut(self.background.get_or_insert_with(Default::default))
    }

    /// The border image properties (M23.3), the initial values when the
    /// style sets none of them.
    pub fn border_image(&self) -> &BorderImage {
        self.border_image
            .as_deref()
            .unwrap_or(&INITIAL_BORDER_IMAGE)
    }

    /// The border image properties to change, this style's own copy
    /// made first if it shares them or has none.
    pub fn border_image_mut(&mut self) -> &mut BorderImage {
        Arc::make_mut(self.border_image.get_or_insert_with(Default::default))
    }

    /// The corners' radii (gap batch E), all 0 when the style sets
    /// none.
    pub fn radii(&self) -> &Radii {
        self.radii.as_deref().unwrap_or(&INITIAL_RADII)
    }

    /// The radii to change, this style's own copy made first.
    pub fn radii_mut(&mut self) -> &mut Radii {
        Arc::make_mut(self.radii.get_or_insert_with(Default::default))
    }

    /// SVG's inherited properties (M32): this style's, or their initial
    /// values.
    pub fn svg(&self) -> &SvgInherited {
        self.svg.as_deref().map_or(&INITIAL_SVG, |s| &s.inherited)
    }

    /// SVG's inherited properties to change, this style's own copy made
    /// first if it shares them or has none.
    pub fn svg_mut(&mut self) -> &mut SvgInherited {
        &mut Arc::make_mut(self.svg.get_or_insert_with(Default::default)).inherited
    }

    /// SVG's properties an element does not inherit (M32): this style's,
    /// or their initial values.
    pub fn svg_own(&self) -> &SvgOwn {
        self.svg.as_deref().map_or(&INITIAL_SVG_OWN, |s| &s.own)
    }

    /// SVG's own properties to change, this style's copy made first if
    /// it shares them or has none.
    pub fn svg_own_mut(&mut self) -> &mut SvgOwn {
        &mut Arc::make_mut(self.svg.get_or_insert_with(Default::default)).own
    }

    /// `aspect-ratio` and the image properties (M23.2), the initial
    /// values when the style sets none of them.
    pub fn image(&self) -> &ImageStyle {
        self.image.as_deref().unwrap_or(&INITIAL_IMAGE)
    }

    /// The image properties to change, this style's own copy made first
    /// if it shares them or has none.
    pub fn image_mut(&mut self) -> &mut ImageStyle {
        Arc::make_mut(self.image.get_or_insert_with(Default::default))
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
            && (self.z_index.is_some()
                || self
                    .effects()
                    .will_change_in_effect()
                    .names(WillChange::Z_INDEX)))
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
            text_align_last: from.text_align_last,
            text_justify: from.text_justify,
            text_transform: from.text_transform,
            white_space: from.white_space,
            letter_spacing: from.letter_spacing,
            word_spacing: from.word_spacing,
            direction: from.direction,
            border_collapse: from.border_collapse,
            border_spacing: from.border_spacing,
            caption_side: from.caption_side,
            cursor: from.cursor,
            dark: from.dark,
            color_scheme: from.color_scheme.clone(),
            empty_cells: from.empty_cells,
            quotes: from.quotes.clone(),
            fragmentation: inherited_fragmentation(from),
            lists: inherited_lists(from),
            decoration: inherited_decoration(from),
            image: inherited_image(from),
            svg: inherited_svg(from),
            custom: from.custom.clone(),
            inert: from.inert,
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

/// SVG's properties a child of `from` inherits, as
/// [`inherited_fragmentation`] gives the fragmentation properties.
fn inherited_svg(from: &ComputedStyle) -> Option<Arc<Svg>> {
    let own = from.svg.as_ref()?;
    if own.inherited == SvgInherited::default() {
        None
    } else if own.own == SvgOwn::default() {
        Some(Arc::clone(own))
    } else {
        Some(Arc::new(Svg {
            inherited: own.inherited.clone(),
            own: SvgOwn::default(),
        }))
    }
}

/// The text decoration properties a child of `from` inherits, as
/// [`inherited_fragmentation`] gives the fragmentation properties.
fn inherited_decoration(from: &ComputedStyle) -> Option<Arc<Decoration>> {
    let own = from.decoration.as_ref()?;
    let inherited = own.inherited();
    if inherited == Decoration::default() {
        None
    } else if **own == inherited {
        Some(Arc::clone(own))
    } else {
        Some(Arc::new(inherited))
    }
}

/// The image properties a child of `from` inherits, as
/// [`inherited_fragmentation`] gives the fragmentation properties.
fn inherited_image(from: &ComputedStyle) -> Option<Arc<ImageStyle>> {
    let own = from.image.as_ref()?;
    let inherited = own.inherited();
    if inherited == ImageStyle::default() {
        None
    } else if **own == inherited {
        Some(Arc::clone(own))
    } else {
        Some(Arc::new(inherited))
    }
}

/// The list properties a child of `from` inherits, as
/// [`inherited_fragmentation`] gives the fragmentation properties.
fn inherited_lists(from: &ComputedStyle) -> Option<Arc<Lists>> {
    let own = from.lists.as_ref()?;
    let inherited = own.inherited();
    if inherited == Lists::default() {
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
        assert_eq!(s.text_align, TextAlign::Start);
        assert_eq!(s.text_align_last, TextAlignLast::Auto);
        assert_eq!(s.text_justify, TextJustify::Auto);
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
        // properties (M21.1) cost a pointer, 624 before them, the counter
        // properties (M21.4) another, 632 before them, and the text
        // decoration properties (M21.7) another, 640 before them. The
        // transition and animation properties (M22) another, 648 before
        // them. `aspect-ratio` and the image properties (M23.2) another,
        // 656 before them, and the background properties (M23.3) another,
        // 664 before them. The outline properties (M26.4) another, and
        // `cursor` a byte in the padding, 680 before them. SVG's
        // properties (M32) another, 688 before them. Which border colours
        // are `currentcolor` (M33), a byte that found no padding: 696
        // before it, and seven bytes free now. Colours that keep the
        // space they were written in (gap batch E, D48) grew each of the
        // six from 16 bytes to 32, to hold the components beside the
        // sRGB channels drawing reads: 704 before them. `color-scheme`
        // and the scheme it gives (D49), a pointer and a byte in the
        // padding: 800 before them. The corners' radii (D50) a pointer:
        // 808 before them. Growing this is a decision, not an accident;
        // say why where the number changes.
        assert_eq!(std::mem::size_of::<ComputedStyle>(), 816);
        assert_eq!(ComputedStyle::default().grid, None);
        assert_eq!(*ComputedStyle::default().grid(), GridStyle::default());
        assert_eq!(ComputedStyle::default().effects, None);
        assert_eq!(*ComputedStyle::default().effects(), Effects::default());
        assert_eq!(ComputedStyle::default().fragmentation, None);
        assert_eq!(
            *ComputedStyle::default().fragmentation(),
            Fragmentation::default()
        );
        assert_eq!(ComputedStyle::default().motion, None);
        assert_eq!(*ComputedStyle::default().motion(), Motion::default());
    }

    #[test]
    fn the_motion_properties_are_not_inherited_and_their_lists_coordinate() {
        let mut parent = ComputedStyle::default();
        parent.motion_mut().animation_name = vec![Some(Arc::from("a"))];
        assert_eq!(ComputedStyle::inherited_from(&parent).motion, None);
        // Values 4 Appendix A: the base list's length, the others
        // repeated or cut.
        let motion = Motion {
            animation_name: vec![Some(Arc::from("a")), None, Some(Arc::from("c"))],
            animation_duration: vec![1.0, 2.0],
            animation_delay: vec![0.5, 0.25, 0.125, 9.0],
            ..Motion::default()
        };
        let animations: Vec<_> = motion
            .animations()
            .map(|a| (a.name.map(|n| n.to_string()), a.duration, a.delay))
            .collect();
        assert_eq!(
            animations,
            vec![
                (Some("a".to_string()), 1.0, 0.5),
                (None, 2.0, 0.25),
                (Some("c".to_string()), 1.0, 0.125)
            ]
        );
        // `transition-property: none` asks for no transition at all.
        let none = Motion {
            transition_property: Vec::new(),
            ..Motion::default()
        };
        assert_eq!(none.transitions().count(), 0);
        let initial = Motion::default();
        let all: Vec<_> = initial.transitions().collect();
        assert_eq!(all.len(), 1);
        assert_eq!(*all[0].property, TransitionProperty::All);
        assert_eq!((all[0].duration, all[0].delay), (0.0, 0.0));
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
    fn the_list_properties_inherit_and_the_counter_properties_do_not() {
        let mut parent = ComputedStyle::default();
        assert_eq!(
            parent.lists().list_style_type,
            ListStyleType::Style(Arc::from("disc"))
        );
        // The counter properties alone: nothing to inherit.
        parent.lists_mut().counter_increment.push(CounterChange {
            name: Arc::from("c"),
            value: 1,
        });
        assert_eq!(ComputedStyle::inherited_from(&parent).lists, None);
        // A list property: a copy of those alone.
        parent.lists_mut().list_style_position = ListStylePosition::Inside;
        let child = ComputedStyle::inherited_from(&parent);
        assert_eq!(
            *child.lists(),
            Lists {
                list_style_position: ListStylePosition::Inside,
                ..Lists::default()
            }
        );
        // List properties alone: shared, not copied.
        let grandchild = ComputedStyle::inherited_from(&child);
        assert!(Arc::ptr_eq(
            grandchild.lists.as_ref().unwrap(),
            child.lists.as_ref().unwrap()
        ));
        assert_eq!(Display::ListItem.blockified(), Display::ListItem);
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
    fn a_percentage_resolves_to_a_length_in_range_whatever_its_size() {
        // A percentage written as large as an `f32` holds, or one a math
        // function made infinite or NaN (M21), is a length no larger
        // than the largest a declaration sets, and never NaN.
        for (p, basis, expected) in [
            (f32::MAX, 1280.0, MAX_LENGTH_PX),
            (-f32::MAX, 1280.0, -MAX_LENGTH_PX),
            (f32::INFINITY, 1.0, MAX_LENGTH_PX),
            (f32::NEG_INFINITY, 1.0, -MAX_LENGTH_PX),
            (f32::INFINITY, 0.0, 0.0),
            (f32::NAN, 1280.0, 0.0),
            (1e9, 1e9, MAX_LENGTH_PX),
            (50.0, 3.0, 1.5),
        ] {
            assert_eq!(
                LengthPercentage::Percent(p).resolve(basis),
                expected,
                "{p}% of {basis}"
            );
            assert_eq!(
                LengthPercentageAuto::Percent(p).resolve(basis, 7.0),
                expected,
                "{p}% of {basis}"
            );
        }
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
