//! The box tree: a `dom::Document` and its `style::Styles` in, a tree
//! of [`LayoutBox`]es out (roadmap M6.1, M13) — one box per element that generates one (`display:
//! none` generates none, and skips its whole subtree, per spec), plus an
//! anonymous box per non-blank text run, plus (M10.4) the anonymous
//! block boxes CSS2 §9.2.1.1 requires so that every block container's
//! children are block-level: a run of inline-level children is wrapped
//! in one, and a block-level box found inside an inline element is
//! hoisted up to the containing block with its inline ancestors split
//! around it. A float or an absolutely positioned box (M11) keeps its
//! place in the tree, where its element is: inline-level content when
//! it occurs in a run or an inline element, which it does not split, a
//! block-level child when it occurs between blocks. A table (M12,
//! docs/design/tables.md) is a box kind per `display` value, and the
//! structure a broken one lacks is supplied by CSS2 §17.2.1's anonymous
//! table objects: cells outside a row get an anonymous row, content
//! outside a cell an anonymous cell, and table parts outside a table an
//! anonymous table — an inline one inside an inline element.
//!
//! An element's `::before` and `::after` (M16,
//! docs/design/selectors-pseudo.md) generate a box as its first and
//! last child when their `content` is not `none` and their `display`
//! not `none`, unless the element is replaced: the box its `display`
//! makes, holding a text box with the content's text. The builder walks
//! [`Item`]s, which are nodes or these boxes or their text, so a
//! generated box is built by the code an element's is, anonymous boxes
//! and all. A list item's marker (M21.6) comes before its `::before`:
//! an inside one is built as a `::before` is, or as the shape a `disc`,
//! `circle` or `square` is drawn as; an outside one is a
//! [`BoxKind::Marker`], which goes into the run of the list item's first
//! line once the tree is built, or stays the list item's first child
//! where it has none. The text in one, its quote marks and counters (M21.4)
//! among it, is found ahead of the build by a walk of the document in
//! tree order ([`generated`]), the order both depend on. Once the tree
//! is built, the boxes of the run holding a block container's
//! first formatted line get their styles on it ([`first_line`]), and a
//! block container's `::first-letter` takes its first letter out of the
//! text box it is in ([`first_letter`]).

use std::borrow::Cow;
use std::cell::{OnceCell, RefCell};
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use dom::{Document, NodeId};
use style::computed::{
    BackfaceVisibility, CaptionSide, Content, Display, Float, ListStylePosition, ListStyleType,
    TransformStyle,
};
use style::selector_matching::PseudoElement;
use style::{ComputedStyle, Recascade, Styles};

mod first_letter;
mod first_line;
mod generated;

use crate::decoration::{self, Decorating};

/// The character a generated text holds where its `content` has an image
/// (M23.3): the builder makes the image's box there.
pub(crate) const OBJECT: char = '\u{fffc}';

use generated::GeneratedText;
pub use generated::MAX_GENERATED_TEXT;

/// One node in the box tree.
#[derive(Debug, Clone)]
pub struct LayoutBox {
    /// The generating element's computed style — owned, not `&'a
    /// ComputedStyle` as the roadmap's own sketch first put it: a
    /// borrowed field would thread a lifetime parameter through every
    /// consumer (`layout`, `paint`, tests) for what's currently a small,
    /// cheap-to-clone struct. Revisit with arenas if profiling ever
    /// shows it matters (that's what M35's "shared data arenas" line is
    /// for).
    pub style: ComputedStyle,
    /// What kind of box this is, and its own children (a [`BoxKind::Text`]
    /// never has children).
    pub kind: BoxKind,
    pub children: Vec<LayoutBox>,
    /// The node this box belongs to (M9.4, M13): the generating element
    /// for an element's box, the containing element for an anonymous
    /// text, block or table box — the document node for one at the top
    /// level — and the originating element for a generated box and the
    /// boxes in it (M16). Hit testing answers with this, and the
    /// fragments the box produces carry it.
    pub node: NodeId,
    /// Whether the box is anonymous: one generated for `node`'s content
    /// — a text box, an anonymous block, table, row or cell (CSS2
    /// §9.2.1.1, §17.2.1), or a `::before` or `::after` box (M16) —
    /// rather than one of the boxes `node` itself generates. Only the
    /// latter stand for the element: the layout cache keeps an
    /// element's results by its node (M13.2).
    pub anonymous: bool,
    /// The pseudo-element that generated the box (M16): `::before`,
    /// `::after` or a list item's `::marker` (M21.6) for the box one
    /// generates and for those inside it, its anonymous boxes and its
    /// text; `None` for the element's own boxes
    /// and the anonymous boxes in them. The fragments made from the box
    /// are marked as it is, so that what looks for an element's own
    /// boxes passes over them, and what tells two boxes of one element
    /// apart can.
    pub generated: Option<PseudoElement>,
    /// A fingerprint of the generated text in the box and the boxes in
    /// it (M21.4): of every text box a pseudo-element's box holds, or a
    /// `::first-letter` took from one, in order, and of the text
    /// decorations every text box in it has (M21.7); 0 when there is
    /// none. That text depends on the quote marks and counters before it
    /// in the document, and those decorations on the elements around the
    /// box, not only on the box's own subtree, and the layout cache keeps
    /// an element's results under it — where M16 kept them under the
    /// quote depth the box started at, which this covers. The builder
    /// sets it when it numbers the tree.
    pub generated_text: u64,
    /// For a text box, the text decorations in force over it (M21.7,
    /// [`crate::decoration`]), outermost element's first: the ones its
    /// element and the elements around it name, up to a float, an
    /// absolutely positioned box, an atomic inline or a replaced
    /// element. Text boxes the same decorations are over share the list.
    /// `None` for none, and for any other box.
    pub decorations: Option<Arc<[Decorating]>>,
    /// Whether the box's layout reads the style of an ancestor's
    /// pseudo-element (M16): a block between a block container and the
    /// text its `::first-letter` takes, or the run its `::first-line`
    /// styles; or holds an ancestor list item's outside marker, which
    /// the list item places (M21.6). A restyle of the ancestor does not
    /// reach the box, so the layout cache neither keeps nor takes it.
    pub styled_by_ancestor: bool,
    /// The box's style on the first formatted line, where it differs
    /// from its own (M16, [`first_line`]): set on the boxes in the run
    /// whose first line some `::first-line` styles, and on that run's
    /// anonymous block, which holds the `::first-line`s too.
    pub first_line: Option<Box<FirstLine>>,
    /// The box's place in tree order (CSS2 Appendix E's "tree order"),
    /// order-modified among a flex container's children (css-flexbox-1
    /// §5.4): its index in a walk of the box tree that visits a box
    /// before its children and a flex container's children by `order`,
    /// an absolutely positioned one counting as `order: 0`. The builder
    /// numbers the tree last ([`build_box_tree`]); the fragments made
    /// from the box carry it, and the paint order puts positioned boxes
    /// in it (M17).
    pub tree_order: u32,
    /// Whether the box is a flex or grid item (M17, M18): an in-flow
    /// child of a flex or grid container, whose `z-index` applies
    /// though it is not positioned (css-flexbox-1 §4.3, css-grid-2
    /// §6.5). The builder sets it when it numbers the tree.
    pub item: bool,
    /// The node of the box above this one in the tree (M19.5): the
    /// element whose box it is a child of, an anonymous box standing
    /// for the element it is in. What a parent's `perspective` and 3D
    /// rendering context reach: its children's boxes, wherever layout
    /// puts their fragments. `None` for a root. The builder sets it when
    /// it numbers the tree.
    pub parent: Option<NodeId>,
    /// Whether the box takes part in a 3D rendering context (M19.5,
    /// css-transforms-2 §9): the box above it, through any anonymous
    /// boxes, is transformable with a used `transform-style` of
    /// `preserve-3d`. A box in one whose `backface-visibility` is
    /// `hidden` is a stacking context and a containing block for all
    /// its descendants. The builder sets it when it numbers the tree.
    pub in_3d: bool,
    /// What the box shows when it is a replaced element's (M23.2): an
    /// `img`'s image, its icon, or nothing. The box has no children and
    /// its kind is what it is on the outside: an inline-block for one
    /// that is inline-level, a block, a float or an absolutely
    /// positioned box, its `style.display` the used one,
    /// `inline-block` or `block`. `None` for every other box.
    pub replaced: Option<Replaced>,
}

/// What a replaced box shows (M23.2, `docs/design/images.md`), as HTML
/// section 15.4.2 and the reference browser have it.
#[derive(Debug, Clone, PartialEq)]
pub enum Replaced {
    /// An image whose header is read: its number in the engine, which
    /// the fragments and display items name it by, and its header.
    Image { id: u64, header: images::Header },
    /// A gradient (M23.3): it has no natural size.
    Gradient(Arc<style::computed::Gradient>),
    /// The icon of an image that cannot be shown: 16 by 16, its box's
    /// `width` and `height` applying only when both are given, as in the
    /// reference browser.
    Icon,
    /// Nothing: an image still coming, or one that stands for nothing.
    /// It is 0 by 0, with no ratio, unless its style sizes it.
    Empty,
    /// A control's mark (M26.7), which paint draws: a checkbox or a
    /// radio button, 13 by 13 by nature, or a drop-down's arrow.
    Mark(Mark),
}

/// What a control's mark shows (M26.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// A checkbox: its box, with a tick when checked or a bar when
    /// indeterminate.
    Checkbox {
        checked: bool,
        indeterminate: bool,
        disabled: bool,
    },
    /// A radio button: its ring, with a dot when checked.
    Radio { checked: bool, disabled: bool },
    /// A drop-down select's arrow.
    Arrow,
}

/// The colour a placeholder is shown in (M26.7): the reference
/// browser's `#757575`.
const PLACEHOLDER_COLOUR: style::computed::Color = style::computed::Color {
    r: 117.0 / 255.0,
    g: 117.0 / 255.0,
    b: 117.0 / 255.0,
    a: 1.0,
};

/// The width of a drop-down select's arrow, in ems of its font (M26.7):
/// with the text's 4px before it, the reference browser's 20px at its
/// default size.
const ARROW_EMS: f32 = 1.2;

/// The space before a drop-down select's text, as the reference
/// browser leaves it, in px.
const SELECT_INDENT: f32 = 4.0;

/// What the engine knows of an `img` element's image (M23.1): what the
/// box tree builder asks of it.
pub trait Images {
    /// The image of the `img` element `node`, whose `src` is not empty.
    fn image(&self, node: NodeId) -> ImageState;

    /// The image a style's `url()` names (M23.3): a marker's, or
    /// `content`'s. Coming, unless the implementation knows it.
    fn url_image(&self, _url: &str) -> ImageState {
        ImageState::Coming
    }
}

/// An `img`'s image, as the engine has it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageState {
    /// Its header is read: its number and its header.
    Ready { id: u64, header: images::Header },
    /// Being fetched or read.
    Coming,
    /// Not fetched, not an image, or not one this engine reads.
    Broken,
}

/// No image is known: every one is still coming. What a tree built
/// without an engine shows.
impl Images for () {
    fn image(&self, _node: NodeId) -> ImageState {
        ImageState::Coming
    }
}

/// What a box is on the first formatted line (M16).
#[derive(Debug, Clone)]
pub struct FirstLine {
    /// The box's style there: for the run's anonymous block, the
    /// innermost `::first-line`'s, which the line's root inline box
    /// inherits — its font is the line's strut.
    pub style: ComputedStyle,
    /// For the run's anonymous block, the `::first-line`
    /// pseudo-elements around the line's content, outermost first: each
    /// an inline box naming its originating element, marked as the
    /// `::first-line`'s, with no children. Empty for any other box.
    pub chain: Vec<LayoutBox>,
}

/// A box's kind, per `display` (block, inline, inline-block, the table
/// values and the flex values, M17 — grid and so on generate real boxes
/// only from their own milestones on).
///
/// The builder's contract (CSS2 §9.2.1.1 and §17.2.1), which `block`
/// relies on: a block container's ([`BoxKind::Block`]'s,
/// [`BoxKind::InlineBlock`]'s, [`BoxKind::Float`]'s, [`BoxKind::Absolute`]'s,
/// [`BoxKind::Cell`]'s or [`BoxKind::Caption`]'s) children are all
/// block-level ([`BoxKind::Block`], [`BoxKind::AnonymousBlock`] or a
/// block-level [`BoxKind::Table`]) or out of flow ([`BoxKind::Float`],
/// [`BoxKind::Absolute`]); an [`BoxKind::AnonymousBlock`]'s and an
/// [`BoxKind::Inline`]'s children are all inline-level ([`BoxKind::Inline`],
/// [`BoxKind::InlineBlock`], an inline-level [`BoxKind::Table`] or
/// [`BoxKind::Text`]) or out of flow; a table's children are captions,
/// column groups, columns, row groups and rows; a row group's are rows;
/// a row's are cells; a column group's are columns; a column has none.
/// A flex container's children ([`BoxKind::Flex`]'s, or a float's or an
/// absolutely positioned box's whose `display` is `flex`) are its flex
/// items — block containers ([`BoxKind::Block`], an anonymous one for a
/// run of text), tables, flex containers and grid containers — and its
/// absolutely positioned children ([`BoxKind::Absolute`]), in document
/// order; a grid container's ([`BoxKind::Grid`]'s) are its grid items,
/// the same boxes.
#[derive(Debug, Clone, PartialEq)]
pub enum BoxKind {
    Block,
    Inline,
    /// An atomic inline-level box that is a block container inside
    /// (CSS2 §9.2.4, M10.4): an item on its parent's line, as wide as
    /// its margin box, its content laid out like a block's.
    InlineBlock,
    /// A floated box (CSS2 §9.5, M11): out of flow, a block container
    /// inside and a block formatting context root, sized like an
    /// inline-block and placed by the float context of the formatting
    /// context it is in. Its `style.float` says which side. It sits
    /// where its element is: in a run or an inline element when met
    /// there, placed when its line is laid out; among a block
    /// container's children when met between blocks, placed at the
    /// flow position it is met at. A floated table (M12) is this kind
    /// with `style.display` of `table`, and a table's children.
    Float,
    /// An absolutely positioned box (CSS2 §9.6, M11: `position:
    /// absolute` or `fixed`, which `style.position` says): out of
    /// flow, a block container inside and a block formatting context
    /// root, laid out against its containing block — the nearest
    /// positioned ancestor's padding box, or the initial containing
    /// block; the viewport for `fixed`, or the nearest ancestor with a
    /// transform or a `perspective` (M19) — once that is known, from
    /// the static position it is met at. As for a float, a table that
    /// is absolutely positioned is this kind with a table's children.
    Absolute,
    /// An anonymous block box (CSS2 §9.2.1.1, M10.4) wrapping one run of
    /// inline-level boxes inside a block container, so that the
    /// container's children are all block-level. Its `style` inherits
    /// from the containing block and takes the initial value for every
    /// other property (so no margin, padding, border, background or
    /// size of its own); it produces no fragment of its own — its
    /// content's fragments go into its parent's.
    AnonymousBlock,
    /// An anonymous box wrapping one text run. Its `style` (on the
    /// containing [`LayoutBox`]) is a copy of the *nearest element
    /// ancestor's* — exactly how CSS defines an anonymous box's style:
    /// text has no element of its own to be styled, so it visually
    /// takes whatever's around it; in a generated box, the
    /// pseudo-element's. An empty text makes none. A run of nothing but
    /// collapsible white space is kept where it separates inline-level
    /// content (M10.4: it is the gap between two words, or two
    /// inline-blocks) and dropped at the start and end of a block
    /// container's inline run, where a line would discard it anyway
    /// (css-text-3 §4.1.3).
    Text(String),
    /// A table (CSS2 §17.2, M12): the table wrapper box and the table
    /// box in one (docs/design/tables.md: the wrapper is implicit, the
    /// captions are among this box's children). Block-level, or an
    /// inline-level item on its line like an inline-block when its
    /// `style.display` is `inline-table`. Its children are kept in the
    /// order they are laid out and painted: the captions with
    /// `caption-side: top`, the column groups and columns, the header
    /// group, the other row groups and rows in document order, the
    /// footer group, then the bottom captions. An anonymous table (an
    /// element's table-internal children outside any table) carries
    /// the element's node and a style inherited from it.
    Table,
    /// A row group: `table-row-group`, or the header or footer group,
    /// which the builder moves first and last among the table's rows
    /// and records here; a second header or footer group is a body
    /// group (§17.2). Its children are rows.
    RowGroup(RowGroupKind),
    /// A row; its children are cells. An anonymous row wraps the cells
    /// found directly in a table or a row group.
    Row,
    /// A column group, `span` columns wide when it has no column
    /// children (the HTML attribute, 1 to 1000); its children, if any,
    /// are columns.
    ColumnGroup {
        span: u32,
    },
    /// A column of the grid, `span` columns wide (1 to 1000). It holds
    /// no content: its element's children generate no box (§17.2.1).
    Column {
        span: u32,
    },
    /// A cell: a block container inside and a block formatting context
    /// root, `colspan` columns wide (1 to 1000) and `rowspan` rows tall
    /// (0 to 65534, 0 meaning to the end of its row group). An
    /// anonymous cell wraps what is found in a row that is not a cell,
    /// spanning one column and one row.
    Cell {
        colspan: u32,
        rowspan: u32,
    },
    /// A caption: a block container inside, placed by its
    /// `style.caption_side` at the top or the bottom of its table.
    Caption,
    /// A flex container (css-flexbox-1, M17): block-level, or an
    /// inline-level item on its line like an inline-block when its
    /// `style.display` is `inline-flex`, as a table is by its display.
    /// Its children are its flex items and its absolutely positioned
    /// children; a floated or absolutely positioned flex container is
    /// the `Float` or `Absolute` kind with a flex container's children.
    Flex,
    /// A grid container (css-grid-2, M18): block-level, or an
    /// inline-level item on its line when its `style.display` is
    /// `inline-grid`. Its children are its grid items and its absolutely
    /// positioned children, as a flex container's are; a floated or
    /// absolutely positioned grid container is the `Float` or `Absolute`
    /// kind with a grid container's children.
    Grid,
    /// A list item's marker where it is not an inline box (Lists 3 §3,
    /// M21.6): one `outside` the list item, or one drawn as a `shape`.
    /// An outside marker is in the run of the list item's first line,
    /// as wide as nothing there, ending at the list item's border-box
    /// start edge when laid out; where the list item has no first line,
    /// it is the list item's first child, beside its content's first
    /// baseline or at its top. A marker drawn as a shape is the shape;
    /// any other holds its text, as a text box.
    Marker {
        outside: bool,
        shape: Option<MarkerShape>,
    },
}

/// The shapes `disc`, `circle` and `square` draw a marker as (Counter
/// Styles 3 §6.3, M21.6), where the reference browser draws them rather
/// than their characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerShape {
    /// A filled circle.
    Disc,
    /// A circle stroked one pixel wide.
    Circle,
    /// A filled square.
    Square,
}

/// Which of CSS2 §17.2's three row-group values a [`BoxKind::RowGroup`]
/// is laid out as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowGroupKind {
    /// Displayed before every other row and row group.
    Header,
    /// In document order.
    Body,
    /// Displayed after every other row and row group.
    Footer,
}

impl LayoutBox {
    /// Whether this box is a table, laid out as a grid of its table
    /// parts (M12): a [`BoxKind::Table`], or a float or an absolutely
    /// positioned box whose `display` is `table`, which the builder gives
    /// a table's children.
    pub fn is_table(&self) -> bool {
        match self.kind {
            BoxKind::Table => true,
            BoxKind::Float | BoxKind::Absolute => self.style.display == Display::Table,
            _ => false,
        }
    }

    /// Whether this box is a flex container, laid out by `crate::flex`
    /// (M17): a [`BoxKind::Flex`], or a float or an absolutely positioned
    /// box whose `display` is `flex`, which the builder gives a flex
    /// container's children.
    pub fn is_flex_container(&self) -> bool {
        match self.kind {
            BoxKind::Flex => true,
            BoxKind::Float | BoxKind::Absolute => self.style.display == Display::Flex,
            _ => false,
        }
    }

    /// Whether this box is a grid container, laid out by `crate::grid`
    /// (M18): a [`BoxKind::Grid`], or a float or an absolutely
    /// positioned box whose `display` is `grid`, which the builder gives
    /// a grid container's children.
    pub fn is_grid_container(&self) -> bool {
        match self.kind {
            BoxKind::Grid => true,
            BoxKind::Float | BoxKind::Absolute => self.style.display == Display::Grid,
            _ => false,
        }
    }

    /// Whether this box's in-flow children are items: it is a flex or a
    /// grid container.
    /// Whether the box is a transformable element's (css-transforms-1
    /// §3): a block-level or atomic inline-level box, a table part,
    /// anything but an inline box.
    pub fn is_transformable(&self) -> bool {
        !matches!(self.kind, BoxKind::Inline)
    }

    /// Whether the box is a stacking context (CSS2 Appendix E, M11,
    /// M19): [`ComputedStyle::is_stacking_context_of`] for its kind, or
    /// a box in a 3D rendering context whose `backface-visibility` is
    /// `hidden` (css-transforms-2 §9).
    pub fn is_stacking_context(&self) -> bool {
        self.style
            .is_stacking_context_of(self.item, self.is_transformable())
            || self.backface_hidden_in_3d()
    }

    /// Whether the box is a containing block for all its descendants,
    /// the fixed ones included (M19):
    /// [`Effects::contains_all_of`](style::computed::Effects::contains_all_of)
    /// for its kind, or a box in a 3D rendering context whose
    /// `backface-visibility` is `hidden`.
    pub fn contains_all_descendants(&self) -> bool {
        self.style
            .effects()
            .contains_all_of(self.is_transformable())
            || self.backface_hidden_in_3d()
    }

    /// Whether the box is a containing block for its absolutely
    /// positioned descendants by its effects (M19): as
    /// [`LayoutBox::contains_all_descendants`], or a `will-change`
    /// naming `position`.
    pub fn contains_absolutes(&self) -> bool {
        self.style
            .effects()
            .contains_absolutes_of(self.is_transformable())
            || self.backface_hidden_in_3d()
    }

    fn backface_hidden_in_3d(&self) -> bool {
        self.in_3d && self.style.effects().backface_visibility == BackfaceVisibility::Hidden
    }

    pub fn has_items(&self) -> bool {
        self.is_flex_container() || self.is_grid_container()
    }

    /// Whether this box is block-level: a child of a block container,
    /// stacked vertically by `block::layout_children`. A table is,
    /// unless it is an inline table; a float is neither block-level nor
    /// inline-level, it is out of flow; a table's internal boxes are
    /// children of table boxes, not of block containers.
    pub fn is_block_level(&self) -> bool {
        match self.kind {
            BoxKind::Block | BoxKind::AnonymousBlock => true,
            BoxKind::Table => self.style.display == Display::Table,
            BoxKind::Flex => self.style.display == Display::Flex,
            BoxKind::Grid => self.style.display == Display::Grid,
            _ => false,
        }
    }
}

/// Builds the box tree for all of `document`'s top-level content (the
/// document node's children — there's no single root box for them,
/// they're laid out as if they were children of the viewport itself,
/// see `block::layout`), from the computed `styles` a cascade of the
/// document gave. The document node is the block container of the
/// top-level content: an inline run at the top goes into an anonymous
/// block inheriting from the document's (initial-valued) style. An
/// element `styles` has no style for generates no box, nor does
/// anything under it, as if it were `display: none`. `recascade` gives
/// the styles that depend on where content falls (M16): a
/// `::first-letter`'s, which inherits from the element its text is in.
pub fn build_box_tree(
    document: &Document,
    styles: &Styles,
    recascade: &dyn Recascade,
) -> Vec<LayoutBox> {
    build_box_tree_showing(document, styles, recascade, &())
}

/// [`build_box_tree`] with each `img` showing what `images` says of its
/// image (M23.2).
pub fn build_box_tree_showing(
    document: &Document,
    styles: &Styles,
    recascade: &dyn Recascade,
    images: &dyn Images,
) -> Vec<LayoutBox> {
    let styled = Styled {
        document,
        styles,
        recascade,
        images,
        generated: OnceCell::new(),
        decorations: RefCell::new(vec![None; document.slot_count()]),
    };
    let mut boxes = build_block_container(&styled, Item::Node(document.root()));
    place_markers(&mut boxes);
    first_line::apply(&styled, &mut boxes);
    first_letter::apply(&styled, &mut boxes);
    number(
        &mut boxes,
        &mut 0,
        false,
        None,
        document,
        &mut HashSet::new(),
    );
    boxes
}

/// Numbers `boxes` and everything in them in tree order from `next`
/// ([`LayoutBox::tree_order`]): each box before its children, and the
/// children of a flex or a grid container, as `items` says `boxes` are,
/// by `order` (css-flexbox-1 §5.4, css-grid-2 §6.3), a stable sort
/// keeping document order among equals, an absolutely positioned child
/// counting as `order: 0`. Each box is given the fingerprint of its
/// generated text ([`LayoutBox::generated_text`]), its children's found
/// first, its parent and whether it
/// takes part in a 3D rendering context ([`LayoutBox::parent`],
/// [`LayoutBox::in_3d`]): an element's own box names the element's
/// parent element in the document, which a block hoisted out of an
/// inline element (CSS2 §9.2.1.1) keeps; a generated box its
/// originating element; an anonymous box the box above it, `parent`.
/// `holds` records the elements whose box holds a 3D rendering context,
/// which a parent's box is always numbered before its children's; few
/// do, and in most documents none, which a lookup then costs nothing.
fn number(
    boxes: &mut [LayoutBox],
    next: &mut u32,
    items: bool,
    parent: Option<(NodeId, bool)>,
    document: &Document,
    holds: &mut HashSet<NodeId>,
) {
    let held = |holds: &HashSet<NodeId>, node: NodeId| !holds.is_empty() && holds.contains(&node);
    let number_one = |b: &mut LayoutBox, next: &mut u32, holds: &mut HashSet<NodeId>| {
        b.tree_order = *next;
        b.item = items;
        let anonymous = b.anonymous && b.generated.is_none();
        let (above, in_3d) = if anonymous {
            (
                parent.map(|(node, _)| node),
                parent.is_some_and(|(_, in_3d)| in_3d),
            )
        } else if b.generated.is_some() {
            (Some(b.node), held(holds, b.node))
        } else {
            let element = document.parent_element(b.node);
            (element, element.is_some_and(|p| held(holds, p)))
        };
        b.parent = above;
        b.in_3d = in_3d;
        *next = next.saturating_add(1);
        let items = b.has_items();
        // An anonymous box standing for the box above it, a block
        // around a run of inline content say, passes its parent's
        // context on to the text in it; an element's box holds one
        // when it is transformable with a used `transform-style` of
        // `preserve-3d`.
        let holds_3d = if anonymous {
            in_3d && parent.is_some_and(|(node, _)| node == b.node)
        } else {
            let holds_3d = b.is_transformable()
                && b.style.effects().used_transform_style(b.style.overflow)
                    == TransformStyle::Preserve3d;
            // An element's later box, a table's beside its wrapper's,
            // says for the boxes after it.
            if holds_3d {
                holds.insert(b.node);
            } else if !holds.is_empty() {
                holds.remove(&b.node);
            }
            holds_3d
        };
        number(
            &mut b.children,
            next,
            items,
            Some((b.node, holds_3d)),
            document,
            holds,
        );
        b.generated_text = generated_text(b);
    };
    if !items {
        for b in boxes {
            number_one(b, next, holds);
        }
        return;
    }
    let mut visit: Vec<usize> = (0..boxes.len()).collect();
    visit.sort_by_key(|&i| match boxes[i].kind {
        BoxKind::Absolute => 0,
        _ => boxes[i].style.order,
    });
    for i in visit {
        number_one(&mut boxes[i], next, holds);
    }
}

/// The fingerprint of the generated text in `b`
/// ([`LayoutBox::generated_text`]), its children's known: of its own
/// text, if it is a text box of a pseudo-element, and its decorations,
/// then theirs.
fn generated_text(b: &LayoutBox) -> u64 {
    let own = match &b.kind {
        BoxKind::Text(text) if b.generated.is_some() => Some(text.as_str()),
        _ => None,
    };
    if own.is_none() && b.decorations.is_none() && b.children.iter().all(|c| c.generated_text == 0)
    {
        return 0;
    }
    let mut hasher = DefaultHasher::new();
    own.hash(&mut hasher);
    if let Some(decorations) = &b.decorations {
        for decoration in decorations.iter() {
            decoration.hash_into(&mut hasher);
        }
    }
    for child in &b.children {
        child.generated_text.hash(&mut hasher);
    }
    hasher.finish().max(1)
}

/// What the builder walks (M16): a node of the document; or the box an
/// element's `::before` or `::after` generates, which stands where the
/// element's first or last child would; or the text in one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Node(NodeId),
    Generated(NodeId, Side),
    GeneratedText(NodeId, Side),
    /// The text a control shows (M26.7): a field's value or placeholder,
    /// a button input's label, a drop-down's chosen option.
    ControlText(NodeId),
    /// A drop-down select's arrow (M26.7).
    ControlArrow(NodeId),
}

/// Which of an element's generated boxes an [`Item`] is: a list item's
/// marker (M21.6), or its `::before` or `::after`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Side {
    Marker,
    Before,
    After,
}

impl Side {
    fn pseudo_element(self) -> PseudoElement {
        match self {
            Side::Marker => PseudoElement::Marker,
            Side::Before => PseudoElement::Before,
            Side::After => PseudoElement::After,
        }
    }
}

impl Item {
    /// The node the item's boxes name: the node itself, or a generated
    /// box's originating element.
    fn node(self) -> NodeId {
        match self {
            Item::Node(node)
            | Item::Generated(node, _)
            | Item::GeneratedText(node, _)
            | Item::ControlText(node)
            | Item::ControlArrow(node) => node,
        }
    }

    /// The pseudo-element whose box the item is, or is the text in.
    fn generated(self) -> Option<PseudoElement> {
        match self {
            Item::Node(_) | Item::ControlText(_) | Item::ControlArrow(_) => None,
            Item::Generated(_, side) | Item::GeneratedText(_, side) => Some(side.pseudo_element()),
        }
    }
}

/// What the builder reads: the document, and its styles by node; and
/// the text of the boxes its pseudo-elements generate.
struct Styled<'a> {
    document: &'a Document,
    styles: &'a Styles,
    recascade: &'a dyn Recascade,
    /// What each `img` shows.
    images: &'a dyn Images,
    /// The generated texts, found by a walk of the document the first
    /// time the build asks for one: a document that generates none is
    /// not walked.
    generated: OnceCell<GeneratedText>,
    /// The text decorations in force over each element's text, found as
    /// the build asks, by the element's slot in the document.
    decorations: RefCell<Vec<Option<Decorations>>>,
}

/// The text decorations in force over some text, if any.
type Decorations = Option<Arc<[Decorating]>>;

impl<'a> Styled<'a> {
    /// The computed style of `item`: an element's, the document node's,
    /// or a generated box's pseudo-element's; `None` for a text, and for
    /// an element the cascade did not style.
    fn style(&self, item: Item) -> Option<&'a ComputedStyle> {
        match item {
            // While scripts run, a `noscript` makes no box (HTML section
            // 15.3.1): as an element with none.
            Item::Node(node)
                if self.styles.scripting && self.document.tag(node) == Some("noscript") =>
            {
                None
            }
            Item::Node(node) => self.styles.get(node),
            Item::Generated(element, side) => self.styles.pseudo(element, side.pseudo_element()),
            Item::GeneratedText(..) | Item::ControlText(_) | Item::ControlArrow(_) => None,
        }
    }

    /// The style of a text box in `item`: a copy of `item`'s own, or
    /// the initial values when it has none.
    fn style_or_initial(&self, item: Item) -> ComputedStyle {
        self.style(item).cloned().unwrap_or_default()
    }

    /// The style of an anonymous box in `item`: inherited from `item`'s
    /// own, every other property initial; the initial values throughout
    /// when `item` has no style.
    fn inherited_from(&self, item: Item) -> ComputedStyle {
        self.style(item)
            .map_or_else(ComputedStyle::default, ComputedStyle::inherited_from)
    }

    /// Whether `item` is built as an element is: an element, or a
    /// generated box.
    fn is_element(&self, item: Item) -> bool {
        match item {
            Item::Node(node) => self.document.is_element(node),
            Item::Generated(..) | Item::ControlArrow(_) => true,
            Item::GeneratedText(..) | Item::ControlText(_) => false,
        }
    }

    /// The items in `item`, in document order: a node's children, an
    /// element's after its marker and the box its `::before` generates
    /// and before the one its `::after` does, when they do; a generated
    /// box's text; nothing in a text.
    fn children(&self, item: Item) -> Vec<Item> {
        if let Item::Node(node) = item
            && let Some(items) = self.control_children(node)
        {
            return items;
        }
        match item {
            // A node none of whose pseudo-elements is styled, as most
            // are not, generates none.
            Item::Node(node) if self.styles.pseudo_styles(node).is_none() => {
                self.document.children(node).map(Item::Node).collect()
            }
            Item::Node(node) => {
                let generated = |side| {
                    self.generates(node, side)
                        .then_some(Item::Generated(node, side))
                };
                generated(Side::Marker)
                    .into_iter()
                    .chain(generated(Side::Before))
                    .chain(self.document.children(node).map(Item::Node))
                    .chain(generated(Side::After))
                    .collect()
            }
            Item::Generated(element, side) => vec![Item::GeneratedText(element, side)],
            Item::GeneratedText(..) | Item::ControlText(_) | Item::ControlArrow(_) => Vec::new(),
        }
    }

    /// What a control has inside its box in place of its children (M26.7,
    /// `dom::forms::shown_text`): a text field, a textarea, a button
    /// input or an option with a `label`, the text it shows; a drop-down
    /// select, its arrow and its chosen option's label. `None` for any
    /// other element, whose children are its own: a list box's options,
    /// a `button`'s content.
    fn control_children(&self, node: NodeId) -> Option<Vec<Item>> {
        let document = self.document;
        match document.tag(node)? {
            "select" if !dom::forms::is_list_box(document, node) => {
                Some(vec![Item::ControlArrow(node), Item::ControlText(node)])
            }
            "input" | "textarea" => {
                dom::forms::shown_text(document, node).map(|_| vec![Item::ControlText(node)])
            }
            "option" => document
                .attr(node, "label")
                .filter(|label| !label.is_empty())
                .map(|_| vec![Item::ControlText(node)]),
            _ => None,
        }
    }

    /// The text a control shows, and whether it is its placeholder.
    fn control_text(&self, node: NodeId) -> (String, bool) {
        if self.document.tag(node) == Some("option") {
            return (dom::forms::option_label(self.document, node), false);
        }
        dom::forms::shown_text(self.document, node).unwrap_or_default()
    }

    /// Whether the element `node` has a `side` item (Pseudo-Elements 4
    /// §4): the pseudo-element is styled, its `content` is a list (the
    /// cascade computes `normal` to `none` there), and the element is
    /// not replaced. One whose `display` is `none` makes no box, as an
    /// element's does not. A list item's marker (Lists 3 §3.2, M21.6),
    /// which the cascade styles for list items alone, is made from its
    /// `content` when that is a list, and from its `list-style-type`
    /// when that is `normal` and the type not `none`;
    /// `list-style-image` never gives one until images (M23).
    fn generates(&self, node: NodeId, side: Side) -> bool {
        if self.document.tag(node).is_none_or(is_replaced) {
            return false;
        }
        let Some(style) = self.styles.pseudo(node, side.pseudo_element()) else {
            return false;
        };
        match &style.content {
            Content::Items(_) => true,
            Content::Normal if side == Side::Marker => {
                style.lists().list_style_type != ListStyleType::None
                    || self.list_image(style).is_some()
            }
            Content::Normal | Content::None => false,
        }
    }

    /// What `image` shows, when it can be shown (M23.3): an image whose
    /// header is read, or a gradient; `None` for one coming or broken.
    fn shown(&self, image: &style::computed::Image) -> Option<Replaced> {
        match image {
            style::computed::Image::Url { url, .. } => match self.images.url_image(url) {
                ImageState::Ready { id, header } => Some(Replaced::Image { id, header }),
                ImageState::Coming | ImageState::Broken => None,
            },
            style::computed::Image::Gradient(gradient) => {
                Some(Replaced::Gradient(Arc::clone(gradient)))
            }
        }
    }

    /// The image a marker styled `style` is (Lists 3 section 3.3, M23.3):
    /// its `list-style-image`, its `content` being `normal`, when it can
    /// be shown; `list-style-type` stands in otherwise.
    fn list_image(&self, style: &ComputedStyle) -> Option<Replaced> {
        if style.content != Content::Normal {
            return None;
        }
        self.shown(style.lists().list_style_image.as_ref()?)
    }

    /// The image an element's `content` replaces its content with
    /// (M23.3): an image alone, the cascade's only `content` on an
    /// element; `None` when it has none.
    fn content_image<'s>(&self, style: &'s ComputedStyle) -> Option<&'s style::computed::Image> {
        match &style.content {
            Content::Items(items) => match &**items {
                [style::computed::ContentItem::Image(image)] => Some(image),
                _ => None,
            },
            Content::Normal | Content::None => None,
        }
    }

    /// The text of `item` when it is a text node or a generated box's
    /// text, and `None` otherwise. Every generated text the build
    /// reaches the walk ahead of it has written ([`GeneratedText`]).
    fn text(&self, item: Item) -> Option<Cow<'_, str>> {
        match item {
            Item::Node(node) => self.document.text(node).map(Cow::Borrowed),
            Item::Generated(..) => None,
            Item::GeneratedText(element, side) => Some(Cow::Borrowed(
                self.generated
                    .get_or_init(|| GeneratedText::of(self))
                    .get(element, side)
                    .unwrap_or_default(),
            )),
            Item::ControlText(node) => Some(Cow::Owned(self.control_text(node).0)),
            Item::ControlArrow(_) => None,
        }
    }

    /// The text decorations in force over the text in `item` (M21.7): an
    /// element's, or a generated box's, which is a child of its element;
    /// none in a marker.
    fn decorations(&self, item: Item) -> Option<Arc<[Decorating]>> {
        match item {
            Item::Node(node) | Item::ControlText(node) | Item::ControlArrow(node) => {
                self.element_decorations(node)
            }
            Item::Generated(_, Side::Marker) | Item::GeneratedText(_, Side::Marker) => None,
            Item::Generated(node, side) | Item::GeneratedText(node, side) => {
                let style = self.styles.pseudo(node, side.pseudo_element());
                let parent = self
                    .element_decorations(node)
                    .filter(|_| !style.is_some_and(decoration::stops_propagation));
                let parent = match style {
                    Some(style) => decoration::through(parent.as_ref(), style),
                    None => parent,
                };
                decoration::within(parent.as_ref(), style.and_then(Decorating::of))
            }
        }
    }

    /// The text decorations in force over the element `node`'s text: its
    /// own inside its parent's, unless its box stops theirs.
    fn element_decorations(&self, node: NodeId) -> Option<Arc<[Decorating]>> {
        if let Some(Some(found)) = self.decorations.borrow().get(node.index()) {
            return found.clone();
        }
        let found = self.styles.get(node).and_then(|style| {
            let stops = decoration::stops_propagation(style)
                || self.document.tag(node).is_some_and(is_replaced);
            let parent = match self.document.parent_element(node) {
                Some(parent) if !stops => self.element_decorations(parent),
                _ => None,
            };
            let parent = decoration::through(parent.as_ref(), style);
            decoration::within(parent.as_ref(), Decorating::of(style))
        });
        if let Some(slot) = self.decorations.borrow_mut().get_mut(node.index()) {
            *slot = Some(found.clone());
        }
        found
    }

    /// The box `item` generates with the computed `style`, of `kind`,
    /// its children the ones `build` makes: an element's own box, or a
    /// generated one, which is anonymous to the layout cache.
    fn own_box(
        &self,
        item: Item,
        style: &ComputedStyle,
        kind: BoxKind,
        build: impl FnOnce() -> Vec<LayoutBox>,
    ) -> LayoutBox {
        LayoutBox {
            style: self.control_style(item, style),
            kind,
            children: build(),
            node: item.node(),
            anonymous: item.generated().is_some(),
            generated: item.generated(),
            generated_text: 0,
            decorations: None,
            styled_by_ancestor: false,
            first_line: None,
            tree_order: 0,
            item: false,
            parent: None,
            in_3d: false,
            replaced: None,
        }
    }

    /// The style a control's own box is laid out in (M26.7): a
    /// drop-down's text starting [`SELECT_INDENT`] in, as the reference
    /// browser's does whatever its padding; the computed style scripts
    /// read is not changed. Any other box's style as it is.
    fn control_style(&self, item: Item, style: &ComputedStyle) -> ComputedStyle {
        let mut used = style.clone();
        if let Item::Node(node) = item
            && self.document.tag(node) == Some("select")
            && !dom::forms::is_list_box(self.document, node)
            && let style::computed::LengthPercentage::Px(left) = used.padding[style::computed::LEFT]
        {
            used.padding[style::computed::LEFT] =
                style::computed::LengthPercentage::Px(left + SELECT_INDENT);
        }
        used
    }

    /// The box the `img` element `node`, `item`, generates with the
    /// computed `style` (M23.2, HTML section 15.4.2, as the reference
    /// browser draws it): a replaced box showing its image when its
    /// header is read, nothing while it is coming; when it cannot be
    /// shown, or has no `src`, its alt text after the icon in a box that
    /// is not replaced, which its `width` and `height` do not size; with
    /// no `alt`, the icon, or nothing where there is no `src` either;
    /// with an empty `alt`, nothing. A replaced box's kind and used
    /// display are `inline-block` for an inline-level `display` and
    /// `block` for any other, a float's and an absolutely positioned
    /// one's `Float` and `Absolute`; the alt text's box is an inline box
    /// for `inline`, and a block container around a run of the icon and
    /// the text otherwise.
    fn img_box(&self, item: Item, node: NodeId, style: &ComputedStyle) -> LayoutBox {
        let src = self
            .document
            .attr(node, "src")
            .filter(|src| !src.is_empty());
        let alt = self.document.attr(node, "alt");
        let state = match src {
            Some(_) => self.images.image(node),
            None => ImageState::Broken,
        };
        let replaced = match (state, alt) {
            (ImageState::Ready { id, header }, _) => Some(Replaced::Image { id, header }),
            (ImageState::Coming, _) => Some(Replaced::Empty),
            (ImageState::Broken, Some("")) => Some(Replaced::Empty),
            (ImageState::Broken, Some(_)) => None,
            (ImageState::Broken, None) if src.is_some() => Some(Replaced::Icon),
            (ImageState::Broken, None) => Some(Replaced::Empty),
        };
        let out_of_flow = if style.position.is_out_of_flow() {
            Some(BoxKind::Absolute)
        } else if style.float != Float::None {
            Some(BoxKind::Float)
        } else {
            None
        };
        let inline_level = matches!(
            style.display,
            Display::Inline
                | Display::InlineBlock
                | Display::InlineTable
                | Display::InlineFlex
                | Display::InlineGrid
        );
        let mut used = style.clone();
        if let Some(replaced) = replaced {
            return self.replaced_box(item, style, replaced);
        }
        let text = self.alt_text(item, node);
        let (kind, display, children) = match out_of_flow {
            Some(kind) => (kind, Display::Block, vec![self.alt_block(item, text)]),
            None if style.display == Display::Inline => (BoxKind::Inline, Display::Inline, text),
            None if inline_level => (
                BoxKind::InlineBlock,
                Display::InlineBlock,
                vec![self.alt_block(item, text)],
            ),
            None => (
                BoxKind::Block,
                Display::Block,
                vec![self.alt_block(item, text)],
            ),
        };
        used.display = display;
        self.own_box(item, &used, kind, || children)
    }

    /// The replaced box `item`, styled `style`, showing `replaced`: its
    /// kind and used display `inline-block` for an inline-level
    /// `display`, `block` for any other, a float's and an absolutely
    /// positioned one's `Float` and `Absolute`.
    fn replaced_box(&self, item: Item, style: &ComputedStyle, replaced: Replaced) -> LayoutBox {
        let kind = if style.position.is_out_of_flow() {
            Some(BoxKind::Absolute)
        } else if style.float != Float::None {
            Some(BoxKind::Float)
        } else {
            None
        };
        let inline_level = matches!(
            style.display,
            Display::Inline
                | Display::InlineBlock
                | Display::InlineTable
                | Display::InlineFlex
                | Display::InlineGrid
        );
        let (kind, display) = match kind {
            Some(kind) => (kind, Display::Block),
            None if inline_level => (BoxKind::InlineBlock, Display::InlineBlock),
            None => (BoxKind::Block, Display::Block),
        };
        let mut used = style.clone();
        used.display = display;
        let mut b = self.own_box(item, &used, kind, Vec::new);
        b.replaced = Some(replaced);
        b
    }

    /// The replaced box of the element `item`, styled `style`, if it is
    /// one (M23): an element whose `content` is an image alone, an `img`
    /// too (Content 3 section 1) unless that image is broken, when the
    /// `img` shows its own, as the reference browser does; else an `img`.
    fn replaced_of(&self, item: Item, style: &ComputedStyle) -> Option<LayoutBox> {
        let Item::Node(node) = item else {
            return None;
        };
        if let Some(mark) = self.mark_of(node) {
            return Some(self.replaced_box(item, style, Replaced::Mark(mark)));
        }
        let img = self.document.tag(node) == Some("img");
        if let Some(image) = self.content_image(style) {
            let broken = match image {
                style::computed::Image::Url { url, .. } => {
                    self.images.url_image(url) == ImageState::Broken
                }
                style::computed::Image::Gradient(_) => false,
            };
            if !(img && broken) {
                return Some(self.content_box(item, style, image));
            }
        }
        img.then(|| self.img_box(item, node, style))
    }

    /// The mark a checkbox or a radio button shows (M26.7), as its state
    /// is; `None` for any other element.
    fn mark_of(&self, node: NodeId) -> Option<Mark> {
        let document = self.document;
        if document.tag(node) != Some("input") {
            return None;
        }
        let checked = dom::forms::checkedness(document, node);
        let disabled = dom::forms::actually_disabled(document, node);
        match dom::forms::input_type(document, node) {
            "checkbox" => Some(Mark::Checkbox {
                checked,
                indeterminate: dom::forms::indeterminate(document, node),
                disabled,
            }),
            "radio" => Some(Mark::Radio { checked, disabled }),
            _ => None,
        }
    }

    /// The box of the element `item`, styled `style`, whose `content` is
    /// `image` (M23.3): replaced, showing the image, or nothing while it
    /// is coming or when it cannot be shown.
    fn content_box(
        &self,
        item: Item,
        style: &ComputedStyle,
        image: &style::computed::Image,
    ) -> LayoutBox {
        let replaced = self.shown(image).unwrap_or(Replaced::Empty);
        self.replaced_box(item, style, replaced)
    }

    /// The inline-level content of an `img` shown by its alt text: the
    /// icon, sized by nothing of the element's, then the text.
    fn alt_text(&self, item: Item, node: NodeId) -> Vec<LayoutBox> {
        let mut icon =
            self.anonymous_box(item, Display::InlineBlock, BoxKind::InlineBlock, Vec::new());
        icon.replaced = Some(Replaced::Icon);
        let alt = self.document.attr(node, "alt").unwrap_or_default();
        let text = LayoutBox {
            style: self.style_or_initial(item),
            kind: BoxKind::Text(alt.to_owned()),
            children: Vec::new(),
            node,
            anonymous: true,
            generated: None,
            generated_text: 0,
            decorations: self.decorations(item),
            styled_by_ancestor: false,
            first_line: None,
            tree_order: 0,
            item: false,
            parent: None,
            in_3d: false,
            replaced: None,
        };
        vec![icon, text]
    }

    /// An anonymous block in `item` around the inline-level `content`.
    fn alt_block(&self, item: Item, content: Vec<LayoutBox>) -> LayoutBox {
        self.anonymous_box(item, Display::Block, BoxKind::AnonymousBlock, content)
    }

    /// An anonymous box of `kind` in `item`, with the style
    /// [`anonymous_style`] gives it, holding `children`.
    fn anonymous_box(
        &self,
        item: Item,
        display: Display,
        kind: BoxKind,
        children: Vec<LayoutBox>,
    ) -> LayoutBox {
        LayoutBox {
            style: anonymous_style(self, item, display),
            kind,
            children,
            node: item.node(),
            anonymous: true,
            generated: item.generated(),
            generated_text: 0,
            decorations: None,
            styled_by_ancestor: false,
            first_line: None,
            tree_order: 0,
            item: false,
            parent: None,
            in_3d: false,
            replaced: None,
        }
    }
}

/// The shape a marker with the computed `style` is drawn as (M21.6): a
/// `disc`, `circle` or `square` from `list-style-type`, its `content`
/// being `normal`, as the reference browser draws them; `None` for any
/// other, which is text.
fn marker_shape(style: &ComputedStyle) -> Option<MarkerShape> {
    if style.content != Content::Normal {
        return None;
    }
    match &style.lists().list_style_type {
        ListStyleType::Style(name) => match name.as_ref() {
            "disc" => Some(MarkerShape::Disc),
            "circle" => Some(MarkerShape::Circle),
            "square" => Some(MarkerShape::Square),
            _ => None,
        },
        ListStyleType::String(_) | ListStyleType::None => None,
    }
}

/// Moves each outside marker in `boxes` and their descendants that its
/// list item holds first into the run of the list item's first line,
/// where it has one, as its first box (M21.6): found as `::first-line`
/// finds its line ([`first_line`]). The blocks on the way down are
/// marked as styled by an ancestor: the marker's place on the line is
/// the list item's to give, which the layout cache does not see from
/// theirs. Where there is no such line, the marker stays the list
/// item's first child.
fn place_markers(boxes: &mut [LayoutBox]) {
    for b in boxes {
        let outside = matches!(
            b.children.first(),
            Some(LayoutBox {
                kind: BoxKind::Marker { outside: true, .. },
                ..
            })
        );
        let mut path = Vec::new();
        if outside
            && let first_line::Found::Line = first_line::in_blocks(&b.children[1..], &mut path)
        {
            let marker = b.children.remove(0);
            let (&last, way) = path.split_last().expect("a path to a run");
            let mut holder = &mut *b;
            for &i in way {
                holder = &mut holder.children[i];
                if holder.kind == BoxKind::Block {
                    holder.styled_by_ancestor = true;
                }
            }
            holder.children[last].children.insert(0, marker);
        }
        place_markers(&mut b.children);
    }
}

/// Whether the element named `tag` is replaced, so that it has no
/// `::before` or `::after` box (Pseudo-Elements 4 §4): in this engine
/// the HTML elements whose content is not a box tree's — `br` and `wbr`
/// among them, which make no box of their own.
fn is_replaced(tag: &str) -> bool {
    matches!(
        tag,
        "img"
            | "input"
            | "textarea"
            | "select"
            | "iframe"
            | "video"
            | "audio"
            | "canvas"
            | "embed"
            | "object"
            | "br"
            | "wbr"
    )
}

/// The block-level children of the block container `item` generates
/// (CSS2 §9.2.1.1): its children in document order, each inline-level
/// run wrapped in an anonymous block, each block found inside an inline
/// child hoisted out to this level.
fn build_block_container(styled: &Styled<'_>, item: Item) -> Vec<LayoutBox> {
    build_block_container_of(styled, item, &styled.children(item))
}

/// [`build_block_container`] for `children`, some of the children of
/// `item`: what an anonymous cell holds (§17.2.1), built as the block
/// container it is, with `item` as what its anonymous boxes name and
/// inherit from.
fn build_block_container_of(styled: &Styled<'_>, item: Item, children: &[Item]) -> Vec<LayoutBox> {
    let mut container = BlockContainer {
        styled,
        item,
        out: Vec::new(),
        run: Vec::new(),
        open: Vec::new(),
    };
    container.add_children(item, children);
    container.finish()
}

/// The children of a box whose inside is what its block-level
/// `display` says: a table's parts, a flex or a grid container's items
/// (M17, M18), or a block container's children.
fn build_inside(styled: &Styled<'_>, item: Item, style: &ComputedStyle) -> Vec<LayoutBox> {
    match style.display {
        Display::Table => build_table(styled, item),
        Display::Flex | Display::Grid => build_items(styled, item),
        _ => build_block_container(styled, item),
    }
}

/// The children of the flex or grid container `item` generates
/// (css-flexbox-1 §4, M17; css-grid-2 §6, M18, which says the same), in
/// document order: each in-flow child element, `::before` and `::after`
/// is an item, the box its `display`, blockified by the cascade,
/// generates — a floated one as if it did not float; an absolutely
/// positioned child is the out-of-flow box it is, not an item; and each
/// run of consecutive text children is an anonymous item, an anonymous
/// block container around the run, unless the run is nothing but white
/// space, which is not rendered. An element with no box does not end a
/// run; an absolutely positioned one does, as in the reference browsers.
/// Layout takes the items in `order` (css-flexbox-1 §5.4, css-grid-2
/// §6.3).
fn build_items(styled: &Styled<'_>, item: Item) -> Vec<LayoutBox> {
    let mut out = Vec::new();
    let mut run = Vec::new();
    for child in styled.children(item) {
        if !styled.is_element(child) {
            run.push(child);
            continue;
        }
        let Some(style) = styled.style(child).filter(|s| s.display != Display::None) else {
            continue;
        };
        flush_text_run(styled, item, &mut run, &mut out);
        let replaced = styled.replaced_of(child, style);
        if let Some(mut img) = replaced {
            if img.kind != BoxKind::Absolute {
                // An item is blockified (css-flexbox-1 section 4).
                img.kind = BoxKind::Block;
                img.style.display = Display::Block;
                if img.replaced.is_none() {
                    img.children = vec![styled.alt_block(child, img.children)];
                }
            }
            out.push(img);
            continue;
        }
        let kind = match style.display {
            _ if style.position.is_out_of_flow() => BoxKind::Absolute,
            Display::Table => BoxKind::Table,
            Display::Flex => BoxKind::Flex,
            Display::Grid => BoxKind::Grid,
            _ => BoxKind::Block,
        };
        out.push(styled.own_box(child, style, kind, || build_inside(styled, child, style)));
    }
    flush_text_run(styled, item, &mut run, &mut out);
    out
}

/// The anonymous item around the text children in `run`, if
/// there is anything but white space in them, added to `out`.
fn flush_text_run(styled: &Styled<'_>, item: Item, run: &mut Vec<Item>, out: &mut Vec<LayoutBox>) {
    if run.is_empty() {
        return;
    }
    let children = build_block_container_of(styled, item, run);
    run.clear();
    if !children.is_empty() {
        out.push(styled.anonymous_box(item, Display::Block, BoxKind::Block, children));
    }
}

/// The style of an anonymous box: inherited from `item`, the element
/// or generated box it belongs to, every other property initial (so no
/// margin, padding, border, background or size of its own), and the
/// `display` it is generated with.
fn anonymous_style(styled: &Styled<'_>, item: Item, display: Display) -> ComputedStyle {
    let mut style = styled.inherited_from(item);
    style.display = display;
    style
}

/// Whether a box with this `display` is a table-internal box or a
/// caption (CSS2 §17.2.1's "internal table element" and `table-caption`):
/// what needs a table above it, and gets an anonymous one when its
/// parent is not a table part.
fn needs_a_table(display: Display) -> bool {
    matches!(
        display,
        Display::TableRowGroup
            | Display::TableHeaderGroup
            | Display::TableFooterGroup
            | Display::TableRow
            | Display::TableColumnGroup
            | Display::TableColumn
            | Display::TableCell
            | Display::TableCaption
    )
}

/// The children of one block container while they are built: the
/// block-level boxes so far, the inline-level run being collected for
/// the next anonymous block, and the chain of inline elements currently
/// open around the position being built — which is what a block found
/// inside them is hoisted out of, and what is reopened after it.
struct BlockContainer<'a> {
    styled: &'a Styled<'a>,
    /// The container's element or generated box: what its anonymous
    /// blocks name and inherit from.
    item: Item,
    out: Vec<LayoutBox>,
    run: Vec<LayoutBox>,
    open: Vec<LayoutBox>,
}

/// One of a run of consecutive table-internal children met outside a
/// table (§17.2.1's misparented boxes), on their way into an anonymous
/// table: a proper table child as its box, or the cells that share an
/// anonymous row.
enum Misplaced {
    Proper(Box<LayoutBox>),
    Cells(Vec<LayoutBox>),
}

impl BlockContainer<'_> {
    /// Adds the boxes of `children`, the children of `parent`, in
    /// document order, to whatever is innermost: the open inline chain,
    /// else the run.
    fn add_children(&mut self, parent: Item, children: &[Item]) {
        let styled = self.styled;
        // Table-internal boxes and captions met here, whose parent is a
        // block container or an inline element, are collected into an
        // anonymous table (§17.2.1); white space between them is
        // dropped, as the browsers do, and anything else ends the run.
        let mut misplaced: Vec<Misplaced> = Vec::new();
        for &child in children {
            if styled.is_element(child) {
                self.add_element(parent, child, &mut misplaced);
                continue;
            }
            // The document node is no one's child; a stale id is no node
            // at all.
            let Some(text) = styled.text(child) else {
                continue;
            };
            let white_space = text.chars().all(crate::inline::is_css_space);
            if white_space && !misplaced.is_empty() {
                continue; // between two table parts: no content
            }
            self.flush_misplaced(parent, &mut misplaced);
            // White space at the start of the run (css-text-3 §4.1:
            // space, tab, newline, carriage return, form feed — a
            // no-break space is content) is dropped: a line discards
            // it. Elsewhere it is kept, as the gap it is; `flush_run`
            // drops it at the end. An empty text is nothing.
            if text.is_empty() || (white_space && self.open.is_empty() && self.run.is_empty()) {
                continue;
            }
            if let Item::ControlText(node) = child {
                self.add_control_text(parent, node, text.into_owned());
                continue;
            }
            let decorations = styled.decorations(parent);
            if text.contains(OBJECT)
                && let Item::GeneratedText(element, side) = child
            {
                self.add_generated_images(element, side, &text, decorations);
                continue;
            }
            self.add_inline_level(LayoutBox {
                style: styled.style_or_initial(parent),
                kind: BoxKind::Text(text.into_owned()),
                children: Vec::new(),
                node: parent.node(),
                anonymous: true,
                generated: parent.generated(),
                generated_text: 0,
                decorations,
                styled_by_ancestor: false,
                first_line: None,
                tree_order: 0,
                item: false,
                parent: None,
                in_3d: false,
                replaced: None,
            });
        }
        self.flush_misplaced(parent, &mut misplaced);
    }

    /// Adds the text box of the text the control `node` shows (M26.7):
    /// in its style, but kept on one line in a single-line control, and
    /// in the placeholder's colour, the reference browser's, when it is
    /// the placeholder. A field scrolled to keep its caret in view
    /// (M26.8) has its text in an inline box moved left as far.
    fn add_control_text(&mut self, parent: Item, node: NodeId, text: String) {
        let styled = self.styled;
        let mut style = styled.style_or_initial(parent);
        if styled.document.tag(node) != Some("textarea") {
            style.white_space = style::computed::WhiteSpace::Pre;
        }
        if styled.control_text(node).1 {
            style.color = PLACEHOLDER_COLOUR;
        }
        let scroll = styled.document.control(node).map_or(0, |c| c.scroll);
        if scroll > 0 && styled.document.tag(node) == Some("input") {
            let mut moved = styled.inherited_from(parent);
            moved.position = style::computed::Position::Relative;
            moved.inset[style::computed::LEFT] =
                style::computed::LengthPercentageAuto::Px(-(scroll as f32));
            let mut inline =
                self.styled
                    .own_box(Item::ControlText(node), &moved, BoxKind::Inline, Vec::new);
            inline.anonymous = true;
            self.open.push(inline);
            self.add_control_text_box(parent, node, text, style);
            let inline = self.open.pop().expect("just opened");
            self.add_inline_level(inline);
            return;
        }
        self.add_control_text_box(parent, node, text, style);
    }

    /// [`BlockContainer::add_control_text`]'s text box.
    fn add_control_text_box(
        &mut self,
        parent: Item,
        node: NodeId,
        text: String,
        style: ComputedStyle,
    ) {
        let styled = self.styled;
        self.add_inline_level(LayoutBox {
            style,
            kind: BoxKind::Text(text),
            children: Vec::new(),
            node,
            anonymous: true,
            generated: None,
            generated_text: 0,
            decorations: styled.decorations(parent),
            styled_by_ancestor: false,
            first_line: None,
            tree_order: 0,
            item: false,
            parent: None,
            in_3d: false,
            replaced: None,
        });
    }

    /// Adds a drop-down select's arrow (M26.7): a float at the right of
    /// its content, as wide as [`ARROW_EMS`] and as tall as a line,
    /// showing the arrow.
    fn add_control_arrow(&mut self, select: NodeId) {
        let styled = self.styled;
        let Some(style) = styled.styles.get(select) else {
            return;
        };
        let mut used = ComputedStyle::inherited_from(style);
        used.display = Display::Block;
        used.float = Float::Right;
        used.width = style::computed::Size::Px(style.font_size_px * ARROW_EMS);
        used.height = style::computed::Size::Px(style.font_size_px * 1.15);
        let mut arrow = styled.own_box(Item::ControlArrow(select), &used, BoxKind::Float, Vec::new);
        arrow.anonymous = true;
        arrow.replaced = Some(Replaced::Mark(Mark::Arrow));
        if self.open.is_empty() && self.run.is_empty() {
            self.out.push(arrow);
        } else {
            self.add_inline_level(arrow);
        }
    }

    /// Adds the boxes of `child`, an element or a generated box among
    /// the children of `parent`, collecting it into `misplaced` when it
    /// is a table part out of place.
    fn add_element(&mut self, parent: Item, child: Item, misplaced: &mut Vec<Misplaced>) {
        let styled = self.styled;
        if let Item::ControlArrow(select) = child {
            self.add_control_arrow(select);
            return;
        }
        // No style, which the cascade gives every element in the tree,
        // or `display: none`: the element AND its subtree generate no
        // box at all.
        let Some(style) = styled.style(child) else {
            return;
        };
        if style.display == Display::None {
            return;
        }
        // An `img` is replaced, or its alt text: a table-internal
        // `display` is a block's for it (M23.2).
        let replaced = styled.replaced_of(child, style);
        if needs_a_table(style.display) && replaced.is_none() {
            self.collect_misplaced(misplaced, child, style);
            return;
        }
        self.flush_misplaced(parent, misplaced);
        if let Item::Generated(element, Side::Marker) = child {
            self.add_marker(child, element, style);
            return;
        }
        if let Some(replaced) = replaced {
            self.add_img(replaced);
            return;
        }
        match style.display {
            // A float or an absolutely positioned box computes to a
            // block-level `display` (CSS2 §9.7, the cascade's rule) and
            // is a block container inside, or a table when its display
            // is `table`; outside it is out of flow, and goes where it
            // is met: into the run or inline open around it, else among
            // the container's blocks.
            Display::Block | Display::ListItem | Display::Table | Display::Flex | Display::Grid
                if style.float != Float::None || style.position.is_out_of_flow() =>
            {
                let kind = if style.position.is_out_of_flow() {
                    BoxKind::Absolute
                } else {
                    BoxKind::Float
                };
                let out_of_flow =
                    styled.own_box(child, style, kind, || build_inside(styled, child, style));
                if self.open.is_empty() && self.run.is_empty() {
                    self.out.push(out_of_flow);
                } else {
                    self.add_inline_level(out_of_flow);
                }
            }
            Display::Block | Display::ListItem => {
                let block = styled.own_box(child, style, BoxKind::Block, || {
                    build_block_container(styled, child)
                });
                self.add_block(block);
            }
            Display::Flex => {
                let flex =
                    styled.own_box(child, style, BoxKind::Flex, || build_items(styled, child));
                self.add_block(flex);
            }
            Display::Grid => {
                let grid =
                    styled.own_box(child, style, BoxKind::Grid, || build_items(styled, child));
                self.add_block(grid);
            }
            Display::Table => {
                let table =
                    styled.own_box(child, style, BoxKind::Table, || build_table(styled, child));
                self.add_block(table);
            }
            Display::Inline => self.add_inline(child, style),
            Display::InlineBlock => {
                let block = styled.own_box(child, style, BoxKind::InlineBlock, || {
                    build_block_container(styled, child)
                });
                self.add_inline_level(block);
            }
            Display::InlineFlex => {
                let flex =
                    styled.own_box(child, style, BoxKind::Flex, || build_items(styled, child));
                self.add_inline_level(flex);
            }
            Display::InlineGrid => {
                let grid =
                    styled.own_box(child, style, BoxKind::Grid, || build_items(styled, child));
                self.add_inline_level(grid);
            }
            Display::InlineTable => {
                let table =
                    styled.own_box(child, style, BoxKind::Table, || build_table(styled, child));
                self.add_inline_level(table);
            }
            Display::None
            | Display::TableRowGroup
            | Display::TableHeaderGroup
            | Display::TableFooterGroup
            | Display::TableRow
            | Display::TableColumnGroup
            | Display::TableColumn
            | Display::TableCell
            | Display::TableCaption => unreachable!("handled above"),
        }
    }

    /// Adds the text of the `side` pseudo-element of `element`, `text`,
    /// which holds images (M23.3): its runs of text as text boxes with
    /// `decorations`, and in place of each [`OBJECT`] the next image its
    /// `content` names, an inline-block showing it, or nothing where it
    /// cannot be shown (Content 3 allows that or an icon).
    fn add_generated_images(
        &mut self,
        element: NodeId,
        side: Side,
        text: &str,
        decorations: Option<Arc<[Decorating]>>,
    ) {
        let styled = self.styled;
        let item = Item::Generated(element, side);
        let style = styled.style_or_initial(item);
        let mut images = match &style.content {
            Content::Items(items) => items
                .iter()
                .filter_map(|i| match i {
                    style::computed::ContentItem::Image(image) => Some(image.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .into_iter(),
            Content::Normal | Content::None => Vec::new().into_iter(),
        };
        for (i, run) in text.split(OBJECT).enumerate() {
            if i > 0
                && let Some(image) = images.next()
                && let Some(replaced) = styled.shown(&image)
            {
                let mut b = styled.anonymous_box(
                    item,
                    Display::InlineBlock,
                    BoxKind::InlineBlock,
                    Vec::new(),
                );
                b.replaced = Some(replaced);
                self.add_inline_level(b);
            }
            if run.is_empty() {
                continue;
            }
            self.add_inline_level(LayoutBox {
                style: style.clone(),
                kind: BoxKind::Text(run.to_owned()),
                children: Vec::new(),
                node: element,
                anonymous: true,
                generated: Some(side.pseudo_element()),
                generated_text: 0,
                decorations: decorations.clone(),
                styled_by_ancestor: false,
                first_line: None,
                tree_order: 0,
                item: false,
                parent: None,
                in_3d: false,
                replaced: None,
            });
        }
    }

    /// Adds an `img`'s box (M23.2) where its kind puts it: a float or an
    /// absolutely positioned one where it is met, as any is.
    fn add_img(&mut self, img: LayoutBox) {
        match img.kind {
            BoxKind::Float | BoxKind::Absolute if self.open.is_empty() && self.run.is_empty() => {
                self.out.push(img)
            }
            BoxKind::Block => self.add_block(img),
            _ => self.add_inline_level(img),
        }
    }

    /// Adds the inline element or generated box `item`, with the computed
    /// `style`, and its content inside it.
    fn add_inline(&mut self, item: Item, style: &ComputedStyle) {
        let styled = self.styled;
        let inline = styled.own_box(item, style, BoxKind::Inline, Vec::new);
        self.open.push(inline);
        self.add_children(item, &styled.children(item));
        let inline = self.open.pop().expect("the inline just opened");
        self.add_inline_level(inline);
    }

    /// Adds the marker `item` of the list item `element`, with the
    /// computed `style` (Lists 3 §3, M21.6), which is the first of the
    /// list item's children. Inside, it is an inline box as a `::before`
    /// is, or the shape it is drawn as, at the start of the content.
    /// Outside, it is a [`BoxKind::Marker`] the container holds first,
    /// whatever comes after it, which [`place_markers`] then moves into
    /// the run of the list item's first line, where there is one.
    fn add_marker(&mut self, item: Item, element: NodeId, style: &ComputedStyle) {
        let styled = self.styled;
        let outside = styled.styles.get(element).is_none_or(|list_item| {
            list_item.lists().list_style_position == ListStylePosition::Outside
        });
        // An image marker (M23.3) is a marker box showing it.
        if let Some(image) = styled.list_image(style) {
            let kind = BoxKind::Marker {
                outside,
                shape: None,
            };
            let mut marker = styled.own_box(item, style, kind, Vec::new);
            marker.replaced = Some(image);
            if outside {
                self.out.push(marker);
            } else {
                self.add_inline_level(marker);
            }
            return;
        }
        let shape = marker_shape(style);
        if !outside && shape.is_none() {
            self.add_inline(item, style);
            return;
        }
        let text = match (shape, item) {
            (None, Item::Generated(element, side)) => styled
                .text(Item::GeneratedText(element, side))
                // A marker's text shows no image (M23.3).
                .map(|text| match text.contains(OBJECT) {
                    true => Cow::Owned(text.replace(OBJECT, "")),
                    false => text,
                })
                .filter(|text| !text.is_empty())
                .map(|text| LayoutBox {
                    style: style.clone(),
                    kind: BoxKind::Text(text.into_owned()),
                    children: Vec::new(),
                    node: element,
                    anonymous: true,
                    generated: Some(PseudoElement::Marker),
                    generated_text: 0,
                    decorations: None,
                    styled_by_ancestor: false,
                    first_line: None,
                    tree_order: 0,
                    item: false,
                    parent: None,
                    in_3d: false,
                    replaced: None,
                }),
            _ => None,
        };
        let marker = styled.own_box(item, style, BoxKind::Marker { outside, shape }, || {
            text.into_iter().collect()
        });
        if outside {
            debug_assert!(self.out.is_empty() && self.run.is_empty() && self.open.is_empty());
            self.out.push(marker);
        } else {
            self.add_inline_level(marker);
        }
    }

    /// Collects the table-internal `item`, with the computed `style`,
    /// met where its parent is not a table part, for the anonymous
    /// table that will wrap it and its consecutive table-internal
    /// siblings (§17.2.1): a cell joins the anonymous row of the cells
    /// just before it, or starts one; a row, a row group, a column, a
    /// column group or a caption is built as itself.
    fn collect_misplaced(
        &mut self,
        misplaced: &mut Vec<Misplaced>,
        item: Item,
        style: &ComputedStyle,
    ) {
        let styled = self.styled;
        if style.display == Display::TableCell {
            let cell = build_cell(styled, item, style);
            match misplaced.last_mut() {
                Some(Misplaced::Cells(cells)) => cells.push(cell),
                _ => misplaced.push(Misplaced::Cells(vec![cell])),
            }
        } else {
            misplaced.push(Misplaced::Proper(Box::new(build_table_part(
                styled, item, style,
            ))));
        }
    }

    /// Wraps the table-internal boxes collected so far, if any, in an
    /// anonymous table and adds it: an inline table, inline-level
    /// content of the inline element `parent` when one is open, else a
    /// block-level table among the container's blocks (§17.2.1: "if the
    /// parent is an inline box, the anonymous box is an inline-table").
    fn flush_misplaced(&mut self, parent: Item, misplaced: &mut Vec<Misplaced>) {
        if misplaced.is_empty() {
            return;
        }
        let styled = self.styled;
        let parts: Vec<LayoutBox> = std::mem::take(misplaced)
            .into_iter()
            .map(|m| match m {
                Misplaced::Proper(part) => *part,
                Misplaced::Cells(cells) => anonymous_row(styled, parent, cells),
            })
            .collect();
        let inline_level = !self.open.is_empty();
        let display = if inline_level {
            Display::InlineTable
        } else {
            Display::Table
        };
        let table =
            styled.anonymous_box(parent, display, BoxKind::Table, order_table_children(parts));
        if inline_level {
            self.add_inline_level(table);
        } else {
            self.add_block(table);
        }
    }

    /// An inline-level box goes to the innermost open inline, or to the
    /// run when none is open.
    fn add_inline_level(&mut self, b: LayoutBox) {
        match self.open.last_mut() {
            Some(inline) => inline.children.push(b),
            None => self.run.push(b),
        }
    }

    /// A block-level box becomes a child of the container itself,
    /// wherever it was found (CSS2 §9.2.1.1): the inline chain open
    /// around it is closed, the run so far wrapped into an anonymous
    /// block before it, and the chain reopened after it — the halves
    /// keep the inline elements' style, node and marks, and either may
    /// end up empty.
    fn add_block(&mut self, block: LayoutBox) {
        let chain: Vec<LayoutBox> = self
            .open
            .iter()
            .map(|b| LayoutBox {
                style: b.style.clone(),
                kind: BoxKind::Inline,
                children: Vec::new(),
                node: b.node,
                anonymous: b.anonymous,
                generated: b.generated,
                generated_text: 0,
                decorations: None,
                styled_by_ancestor: false,
                first_line: None,
                tree_order: 0,
                item: false,
                parent: None,
                in_3d: false,
                replaced: None,
            })
            .collect();
        while let Some(inline) = self.open.pop() {
            self.add_inline_level(inline);
        }
        self.flush_run();
        self.out.push(block);
        self.open = chain;
    }

    /// Wraps the run so far, if any, in an anonymous block. White space
    /// at its end is dropped first: a line discards it, and a run of
    /// nothing else is no content at all.
    fn flush_run(&mut self) {
        while self.run.last().is_some_and(
            |b| matches!(&b.kind, BoxKind::Text(t) if t.chars().all(crate::inline::is_css_space)),
        ) {
            self.run.pop();
        }
        if self.run.is_empty() {
            return;
        }
        let children = std::mem::take(&mut self.run);
        self.out.push(LayoutBox {
            style: self.styled.inherited_from(self.item),
            kind: BoxKind::AnonymousBlock,
            children,
            node: self.item.node(),
            anonymous: true,
            generated: self.item.generated(),
            generated_text: 0,
            decorations: None,
            styled_by_ancestor: false,
            first_line: None,
            tree_order: 0,
            item: false,
            parent: None,
            in_3d: false,
            replaced: None,
        });
    }

    fn finish(mut self) -> Vec<LayoutBox> {
        debug_assert!(
            self.open.is_empty(),
            "every inline is closed by its own element"
        );
        self.flush_run();
        self.out
    }
}

/// The box of `item` with the computed `style`, whose `display` is one
/// of the table values other than a cell: a row, a row group, a column,
/// a column group or a caption, with its children built as the kind
/// requires.
fn build_table_part(styled: &Styled<'_>, item: Item, style: &ComputedStyle) -> LayoutBox {
    type Build = fn(&Styled<'_>, Item) -> Vec<LayoutBox>;
    let (kind, build): (BoxKind, Build) = match style.display {
        Display::TableRow => (BoxKind::Row, build_row),
        Display::TableRowGroup => (BoxKind::RowGroup(RowGroupKind::Body), build_row_group),
        Display::TableHeaderGroup => (BoxKind::RowGroup(RowGroupKind::Header), build_row_group),
        Display::TableFooterGroup => (BoxKind::RowGroup(RowGroupKind::Footer), build_row_group),
        Display::TableColumnGroup => (
            BoxKind::ColumnGroup {
                span: column_span(styled.document, item),
            },
            build_columns,
        ),
        Display::TableColumn => return build_column(styled, item, style),
        Display::TableCaption => (BoxKind::Caption, build_block_container),
        _ => unreachable!("a table part's display"),
    };
    styled.own_box(item, style, kind, || build(styled, item))
}

/// The columns of the column group `item`: its column children, and
/// nothing else (§17.2.1: any other child of a column group is treated
/// as `display: none`).
fn build_columns(styled: &Styled<'_>, item: Item) -> Vec<LayoutBox> {
    styled
        .children(item)
        .into_iter()
        .filter_map(|child| {
            let style = styled
                .style(child)
                .filter(|s| s.display == Display::TableColumn)?;
            Some(build_column(styled, child, style))
        })
        .collect()
}

/// A column box: `span` columns of the grid, no children whatever the
/// element holds (§17.2.1).
fn build_column(styled: &Styled<'_>, item: Item, style: &ComputedStyle) -> LayoutBox {
    let span = column_span(styled.document, item);
    styled.own_box(item, style, BoxKind::Column { span }, Vec::new)
}

/// A cell box: a block container inside, its spans from the HTML
/// attributes.
fn build_cell(styled: &Styled<'_>, item: Item, style: &ComputedStyle) -> LayoutBox {
    let (colspan, rowspan) = cell_spans(styled.document, item);
    styled.own_box(item, style, BoxKind::Cell { colspan, rowspan }, || {
        build_block_container(styled, item)
    })
}

/// An anonymous row holding `cells`, at least one, in `item`.
fn anonymous_row(styled: &Styled<'_>, item: Item, cells: Vec<LayoutBox>) -> LayoutBox {
    styled.anonymous_box(item, Display::TableRow, BoxKind::Row, cells)
}

/// The children of a table box (CSS2 §17.2.1), ordered for layout and
/// paint: the item's captions, column groups, columns, row groups and
/// rows as themselves; its cells, and any other content, in anonymous
/// rows — consecutive ones in the same row, a cell as itself in it and
/// the rest in anonymous cells.
fn build_table(styled: &Styled<'_>, item: Item) -> Vec<LayoutBox> {
    order_table_children(rows_and_parts(styled, item, |display| {
        matches!(
            display,
            Display::TableCaption
                | Display::TableColumnGroup
                | Display::TableColumn
                | Display::TableRowGroup
                | Display::TableHeaderGroup
                | Display::TableFooterGroup
                | Display::TableRow
        )
    }))
}

/// The rows of a row group: the item's rows as themselves, and anything
/// else in anonymous rows as in a table.
fn build_row_group(styled: &Styled<'_>, item: Item) -> Vec<LayoutBox> {
    rows_and_parts(styled, item, |display| display == Display::TableRow)
}

/// The children of the table or row group `item`: an element child
/// whose `display` passes `proper` is built as the part it is; every
/// other child — a cell, an element of any other display, text — goes
/// into an anonymous row, consecutive ones into the same row, white
/// space between parts dropped (§17.2.1's steps 1 and 2).
fn rows_and_parts(
    styled: &Styled<'_>,
    item: Item,
    proper: impl Fn(Display) -> bool,
) -> Vec<LayoutBox> {
    let mut parts = Vec::new();
    let mut row_run: Vec<RowItem<'_>> = Vec::new();
    let flush = |parts: &mut Vec<LayoutBox>, row_run: &mut Vec<RowItem<'_>>| {
        let cells = cells_of(styled, item, std::mem::take(row_run));
        if !cells.is_empty() {
            parts.push(anonymous_row(styled, item, cells));
        }
    };
    for child in styled.children(item) {
        match row_item(styled, child) {
            Some(RowItem::Element(child, style)) if proper(style.display) => {
                flush(&mut parts, &mut row_run);
                parts.push(build_table_part(styled, child, style));
            }
            Some(row_item) => row_run.push(row_item),
            None => {}
        }
    }
    flush(&mut parts, &mut row_run);
    parts
}

/// The cells of the row `item`: its cells as themselves, and anything
/// else in anonymous cells, consecutive ones in the same cell.
fn build_row(styled: &Styled<'_>, item: Item) -> Vec<LayoutBox> {
    let items = styled
        .children(item)
        .into_iter()
        .filter_map(|child| row_item(styled, child))
        .collect();
    cells_of(styled, item, items)
}

/// One child of a table, a row group or a row, before it is built,
/// which is in document order: an element or a generated box, with
/// the style it is shown with, or a text.
enum RowItem<'a> {
    Element(Item, &'a ComputedStyle),
    Text(Item),
}

/// `child` as a [`RowItem`]; `None` for an element shown with
/// `display: none` or not styled, which makes no box.
fn row_item<'a>(styled: &Styled<'a>, child: Item) -> Option<RowItem<'a>> {
    if !styled.is_element(child) {
        return Some(RowItem::Text(child));
    }
    let style = styled.style(child)?;
    (style.display != Display::None).then_some(RowItem::Element(child, style))
}

/// The cells `items` make, in `item`: a cell as itself; a run of
/// consecutive content, which is an element of any display other than
/// a cell's or a text, in one anonymous cell, built as the block
/// container a cell is; a run of nothing but white space in none
/// (§17.2.1 step 1: white space between table parts is no content).
fn cells_of(styled: &Styled<'_>, item: Item, items: Vec<RowItem<'_>>) -> Vec<LayoutBox> {
    let mut cells = Vec::new();
    let mut content: Vec<Item> = Vec::new();
    let flush = |cells: &mut Vec<LayoutBox>, content: &mut Vec<Item>| {
        let content = std::mem::take(content);
        let only_white_space = content.iter().all(|&child| {
            styled
                .text(child)
                .is_some_and(|text| text.chars().all(crate::inline::is_css_space))
        });
        if only_white_space {
            return;
        }
        let children = build_block_container_of(styled, item, &content);
        cells.push(styled.anonymous_box(
            item,
            Display::TableCell,
            BoxKind::Cell {
                colspan: 1,
                rowspan: 1,
            },
            children,
        ));
    };
    for row_item in items {
        match row_item {
            RowItem::Element(child, style) if style.display == Display::TableCell => {
                flush(&mut cells, &mut content);
                cells.push(build_cell(styled, child, style));
            }
            RowItem::Element(child, _) | RowItem::Text(child) => content.push(child),
        }
    }
    flush(&mut cells, &mut content);
    cells
}

/// A table's children in the order they are laid out and painted (CSS2
/// §17.2, docs/design/tables.md): the captions with `caption-side:
/// top`, the column groups and columns, the first header group, the
/// other row groups and rows in document order — a second header or
/// footer group among them as a body group — the first footer group,
/// then the bottom captions.
fn order_table_children(parts: Vec<LayoutBox>) -> Vec<LayoutBox> {
    let mut top = Vec::new();
    let mut columns = Vec::new();
    let mut header = None;
    let mut rows = Vec::new();
    let mut footer = None;
    let mut bottom = Vec::new();
    for mut part in parts {
        match part.kind {
            BoxKind::Caption => {
                if part.style.caption_side == CaptionSide::Top {
                    top.push(part);
                } else {
                    bottom.push(part);
                }
            }
            BoxKind::ColumnGroup { .. } | BoxKind::Column { .. } => columns.push(part),
            BoxKind::RowGroup(RowGroupKind::Header) if header.is_none() => header = Some(part),
            BoxKind::RowGroup(RowGroupKind::Footer) if footer.is_none() => footer = Some(part),
            BoxKind::RowGroup(_) => {
                part.kind = BoxKind::RowGroup(RowGroupKind::Body);
                rows.push(part);
            }
            BoxKind::Row => rows.push(part),
            _ => unreachable!("a table's children are table parts"),
        }
    }
    top.into_iter()
        .chain(columns)
        .chain(header)
        .chain(rows)
        .chain(footer)
        .chain(bottom)
        .collect()
}

/// A `td` or `th` element's `colspan` and `rowspan` (HTML's processing
/// model for tables): `colspan` is 1 when absent, unparsable or zero
/// and at most 1000; `rowspan` is 1 when absent or unparsable, zero
/// meaning to the end of the row group, and at most 65534. An element
/// of any other name, and a generated box, is one cell of the grid
/// whatever it says.
fn cell_spans(document: &Document, item: Item) -> (u32, u32) {
    let Item::Node(idx) = item else {
        return (1, 1);
    };
    if !matches!(document.tag(idx), Some("td" | "th")) {
        return (1, 1);
    }
    let colspan = document
        .attr(idx, "colspan")
        .and_then(dom::parse_non_negative_integer)
        .filter(|v| *v != 0)
        .unwrap_or(1)
        .min(1000);
    let rowspan = document
        .attr(idx, "rowspan")
        .and_then(dom::parse_non_negative_integer)
        .unwrap_or(1)
        .min(65534);
    (colspan as u32, rowspan as u32)
}

/// A `col` or `colgroup` element's `span`: 1 when absent, unparsable
/// or zero, at most 1000; 1 for an element of any other name, and for a
/// generated box.
fn column_span(document: &Document, item: Item) -> u32 {
    let Item::Node(idx) = item else {
        return 1;
    };
    if !matches!(document.tag(idx), Some("col" | "colgroup")) {
        return 1;
    }
    document
        .attr(idx, "span")
        .and_then(dom::parse_non_negative_integer)
        .filter(|v| *v != 0)
        .unwrap_or(1)
        .min(1000) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{boxes_of, number};
    use dom::Element;

    /// A box tree and the document it was built from, whose nodes the
    /// tests name by their [`number`].
    struct Tree {
        document: dom::Document,
        boxes: Vec<LayoutBox>,
    }

    impl std::ops::Deref for Tree {
        type Target = [LayoutBox];

        fn deref(&self) -> &[LayoutBox] {
            &self.boxes
        }
    }

    impl Tree {
        /// `node`'s number in the tree's document.
        fn number(&self, node: NodeId) -> usize {
            number(&self.document, node)
        }

        /// `boxes`' subtrees as `(kind, node)` pairs in preorder, a text
        /// box's kind spelled as its text, for assertions on structure.
        fn shape_of(&self, boxes: &[LayoutBox]) -> Vec<(String, usize)> {
            let mut out = Vec::new();
            for b in boxes {
                self.shape(b, &mut out);
            }
            out
        }

        fn shape(&self, b: &LayoutBox, out: &mut Vec<(String, usize)>) {
            let kind = match &b.kind {
                BoxKind::Block => "block".to_string(),
                BoxKind::Inline => "inline".to_string(),
                BoxKind::InlineBlock => "inline-block".to_string(),
                BoxKind::Float => "float".to_string(),
                BoxKind::Absolute => "absolute".to_string(),
                BoxKind::AnonymousBlock => "anon".to_string(),
                BoxKind::Text(t) => format!("text {t}"),
                BoxKind::Table if b.style.display == Display::InlineTable => {
                    "inline-table".to_string()
                }
                BoxKind::Table => "table".to_string(),
                BoxKind::RowGroup(RowGroupKind::Header) => "header".to_string(),
                BoxKind::RowGroup(RowGroupKind::Body) => "body".to_string(),
                BoxKind::RowGroup(RowGroupKind::Footer) => "footer".to_string(),
                BoxKind::Row => "row".to_string(),
                BoxKind::ColumnGroup { span } => format!("colgroup {span}"),
                BoxKind::Column { span } => format!("col {span}"),
                BoxKind::Cell { colspan, rowspan } => format!("cell {colspan}x{rowspan}"),
                BoxKind::Caption => "caption".to_string(),
                BoxKind::Flex if b.style.display == Display::InlineFlex => {
                    "inline-flex".to_string()
                }
                BoxKind::Flex => "flex".to_string(),
                BoxKind::Grid if b.style.display == Display::InlineGrid => {
                    "inline-grid".to_string()
                }
                BoxKind::Grid => "grid".to_string(),
                BoxKind::Marker { outside, shape } => {
                    let place = if *outside { "outside" } else { "inside" };
                    match shape {
                        Some(shape) => format!("marker {place} {shape:?}"),
                        None => format!("marker {place}"),
                    }
                }
            };
            // A `::before` or `::after` box, or one in it, is marked
            // with a `+`, a `::first-letter` one with a `^`, a marker or
            // one in it with a `*`.
            let mark = match b.generated {
                None => "",
                Some(PseudoElement::FirstLetter) => "^",
                Some(PseudoElement::Marker) => "*",
                Some(_) => "+",
            };
            out.push((format!("{mark}{kind}"), self.number(b.node)));
            for c in &b.children {
                self.shape(c, out);
            }
        }

        /// The text boxes' texts in `boxes`' subtrees, in preorder.
        fn texts(&self, boxes: &[LayoutBox]) -> Vec<String> {
            self.shape_of(boxes)
                .into_iter()
                .filter_map(|(k, _)| {
                    k.trim_start_matches(['+', '^', '*'])
                        .strip_prefix("text ")
                        .map(str::to_string)
                })
                .collect()
        }

        /// [`Tree::shape_of`] without the text boxes.
        fn boxes_of(&self, boxes: &[LayoutBox]) -> Vec<(String, usize)> {
            self.shape_of(boxes)
                .into_iter()
                .filter(|(k, _)| !k.trim_start_matches(['+', '^', '*']).starts_with("text"))
                .collect()
        }

        /// The body's box in a [`tree_of`] tree.
        fn body(&self) -> &LayoutBox {
            let body = &self.boxes[0].children[0];
            assert_eq!(
                (body.kind.clone(), self.number(body.node)),
                (BoxKind::Block, 3)
            );
            body
        }
    }

    /// The box tree of a document holding `root`, under `css_src`.
    fn built(root: Element, css_src: &str) -> Tree {
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(css_src).expect("test css is valid");
        let (document, boxes) = boxes_of(&doc, &[sheet]);
        Tree { document, boxes }
    }

    /// The box tree of the document `html` parses to: the parser implies
    /// `html`, `head` and `body`, so the tree is the html box, whose one
    /// child is the body's (the head generates no box). Nodes: html 1,
    /// head 2, body 3, then the markup's elements from 4.
    fn tree_of(html: &str, css_src: &str) -> Tree {
        let doc = html::parse_document(html).expect("html parsing never fails");
        let sheet = css::parse_stylesheet(css_src).expect("test css is valid");
        let (document, boxes) = boxes_of(&doc, &[sheet]);
        Tree { document, boxes }
    }

    fn pairs(items: &[(&str, usize)]) -> Vec<(String, usize)> {
        items.iter().map(|(k, n)| (k.to_string(), *n)).collect()
    }

    #[test]
    fn block_and_inline_elements_get_matching_box_kinds() {
        let root = Element::new("div").push(Element::new("span").text("x"));
        let tree = built(root, "div { display: block; } span { display: inline; }");
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].kind, BoxKind::Block);
        // The span is inline-level content of the div, so it sits in an
        // anonymous block (M10.4) that names the div, its container.
        let anon = &tree[0].children[0];
        assert_eq!(anon.kind, BoxKind::AnonymousBlock);
        assert_eq!(anon.children[0].kind, BoxKind::Inline);
        // Element boxes name their element; the text box inside the
        // span names the span, its containing element.
        assert_eq!(tree.number(tree[0].node), 1);
        assert_eq!(tree.number(anon.node), 1);
        assert_eq!(tree.number(anon.children[0].node), 2);
        assert_eq!(tree.number(anon.children[0].children[0].node), 2);
    }

    #[test]
    fn display_none_produces_no_box_at_all() {
        let root = Element::new("div")
            .push(Element::new("p").text("hidden").attr("class", "gone"))
            .push(Element::new("span").text("shown"));
        let tree = built(root, "p { display: none; }");
        assert_eq!(tree[0].children.len(), 1); // the p is gone entirely, not just empty
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[("anon", 1), ("inline", 3), ("text shown", 3)])
        );
    }

    #[test]
    fn text_runs_become_anonymous_boxes_carrying_the_parent_style() {
        let root = Element::new("p").text("hello");
        let tree = built(root, "p { color: #ff0000; }");
        let text = &tree[0].children[0].children[0];
        match &text.kind {
            BoxKind::Text(t) => assert_eq!(t, "hello"),
            other => panic!("expected a text box, got {other:?}"),
        }
        assert_eq!(text.style.color, style::computed::Color::rgb8(255, 0, 0));
    }

    #[test]
    fn whitespace_only_text_produces_no_box() {
        let root = Element::new("div").text("   \n  ");
        let tree = built(root, "");
        assert!(tree[0].children.is_empty(), "not even an anonymous block");
        // A no-break space is content, not white space.
        let root = Element::new("div").text("\u{a0}");
        let tree = built(root, "");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[("anon", 1), ("text \u{a0}", 1)])
        );
    }

    #[test]
    fn inline_runs_between_blocks_get_one_anonymous_block_each() {
        // <div>a<p>b</p>c<span>d</span><p>e</p></div>: CSS2 §9.2.1.1's
        // own example. Nodes: div 1, p 2, span 3, p 4.
        let root = Element::new("div")
            .text("a")
            .push(Element::new("p").text("b"))
            .text("c")
            .push(Element::new("span").text("d"))
            .push(Element::new("p").text("e"));
        let tree = built(root, "");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("text a", 1),
                ("block", 2),
                ("anon", 2),
                ("text b", 2),
                ("anon", 1),
                ("text c", 1),
                ("inline", 3),
                ("text d", 3),
                ("block", 4),
                ("anon", 4),
                ("text e", 4),
            ])
        );
        assert!(tree[0].children.iter().all(|c| c.is_block_level()));
    }

    #[test]
    fn an_anonymous_block_inherits_from_its_container_and_nothing_else() {
        let root = Element::new("div").text("x");
        let tree = built(
            root,
            "div { color: #ff0000; text-align: center; padding: 10px; margin: 5px;
                   border: 2px solid #000000; background-color: #00ff00; width: 50px; }",
        );
        let anon = &tree[0].children[0];
        assert_eq!(anon.kind, BoxKind::AnonymousBlock);
        let s = &anon.style;
        assert_eq!(s.color, style::computed::Color::rgb8(255, 0, 0));
        assert_eq!(s.text_align, style::computed::TextAlign::Center);
        let initial = ComputedStyle::default();
        assert_eq!(s.padding, initial.padding);
        assert_eq!(s.margin, initial.margin);
        assert_eq!(s.used_border_width(), [0.0; 4]);
        assert_eq!(s.background_color, initial.background_color);
        assert_eq!(s.width, initial.width);
    }

    #[test]
    fn a_block_inside_an_inline_is_hoisted_and_the_inline_split() {
        // <div><span>a<p>b</p>c</span></div>: the p becomes a child of
        // the div between two anonymous blocks, each holding one half
        // of the span (CSS2 §9.2.1.1). Nodes: div 1, span 2, p 3.
        let root = Element::new("div").push(
            Element::new("span")
                .text("a")
                .push(Element::new("p").text("b"))
                .text("c"),
        );
        let tree = built(root, "span { color: #0000ff; }");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("inline", 2),
                ("text a", 2),
                ("block", 3),
                ("anon", 3),
                ("text b", 3),
                ("anon", 1),
                ("inline", 2),
                ("text c", 2),
            ])
        );
        // Both halves keep the span's style.
        let blue = style::computed::Color::rgb8(0, 0, 255);
        assert_eq!(tree[0].children[0].children[0].style.color, blue);
        assert_eq!(tree[0].children[2].children[0].style.color, blue);
    }

    #[test]
    fn a_block_inside_nested_inlines_splits_the_whole_chain() {
        // <div><span><em>a<p>b</p>c</em>d</span></div>. Nodes: div 1,
        // span 2, em 3, p 4. The em half after the p sits inside the
        // span half after the p, as it did before; "d" follows it in
        // the span.
        let root = Element::new("div").push(
            Element::new("span")
                .push(
                    Element::new("em")
                        .text("a")
                        .push(Element::new("p").text("b"))
                        .text("c"),
                )
                .text("d"),
        );
        let tree = built(root, "");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("inline", 2),
                ("inline", 3),
                ("text a", 3),
                ("block", 4),
                ("anon", 4),
                ("text b", 4),
                ("anon", 1),
                ("inline", 2),
                ("inline", 3),
                ("text c", 3),
                ("text d", 2),
            ])
        );
    }

    #[test]
    fn a_block_at_the_start_or_end_of_an_inline_leaves_an_empty_half() {
        // <div><span><p>b</p></span></div>: CSS2 §9.2.1.1 splits "even
        // if either side is empty". The empty halves are kept — layout
        // treats a line box with nothing in it as not existing (CSS2
        // §9.4.2), so they cost nothing and the structure stays
        // uniform. Nodes: div 1, span 2, p 3.
        let root = Element::new("div").push(Element::new("span").push(Element::new("p").text("b")));
        let tree = built(root, "");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("inline", 2),
                ("block", 3),
                ("anon", 3),
                ("text b", 3),
                ("anon", 1),
                ("inline", 2),
            ])
        );
    }

    #[test]
    fn two_blocks_inside_one_inline_split_it_three_ways() {
        // <div><span>a<p>b</p>c<p>d</p>e</span></div>. Nodes: div 1,
        // span 2, p 3, p 4.
        let root = Element::new("div").push(
            Element::new("span")
                .text("a")
                .push(Element::new("p").text("b"))
                .text("c")
                .push(Element::new("p").text("d"))
                .text("e"),
        );
        let tree = built(root, "");
        assert_eq!(
            tree.boxes_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("inline", 2),
                ("block", 3),
                ("anon", 3),
                ("anon", 1),
                ("inline", 2),
                ("block", 4),
                ("anon", 4),
                ("anon", 1),
                ("inline", 2),
            ])
        );
    }

    #[test]
    fn a_block_inside_an_inline_is_itself_a_block_container() {
        // The hoisted block's own children get the same treatment: a
        // span inside it goes into an anonymous block of the p's.
        // Nodes: div 1, span 2, p 3, em 4, p 5.
        let root = Element::new("div").push(
            Element::new("span").push(
                Element::new("p")
                    .push(Element::new("em").text("x"))
                    .push(Element::new("p").text("y")),
            ),
        );
        let tree = built(root, "");
        let p = &tree[0].children[1];
        assert_eq!((p.kind.clone(), tree.number(p.node)), (BoxKind::Block, 3));
        assert_eq!(
            tree.shape_of(&p.children),
            pairs(&[
                ("anon", 3),
                ("inline", 4),
                ("text x", 4),
                ("block", 5),
                ("anon", 5),
                ("text y", 5),
            ])
        );
    }

    #[test]
    fn a_hidden_block_inside_an_inline_splits_nothing() {
        // display: none generates no box, so the span is not split
        // around it. Nodes: div 1, span 2, p 3.
        let root = Element::new("div").push(
            Element::new("span")
                .text("a")
                .push(Element::new("p").text("b"))
                .text("c"),
        );
        let tree = built(root, "p { display: none; }");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[("anon", 1), ("inline", 2), ("text a", 2), ("text c", 2)])
        );
    }

    #[test]
    fn an_inline_block_is_inline_level_outside_and_a_block_container_inside() {
        // <div>a<span class=ib>b<p>c</p></span>d</div>: the span sits in
        // the div's one anonymous block like any inline-level box; its
        // own children are block-level, "b" in an anonymous block of
        // the span's. Nodes: div 1, span 2, p 3.
        let root = Element::new("div")
            .text("a")
            .push(
                Element::new("span")
                    .text("b")
                    .push(Element::new("p").text("c")),
            )
            .text("d");
        let tree = built(root, "span { display: inline-block; }");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("text a", 1),
                ("inline-block", 2),
                ("anon", 2),
                ("text b", 2),
                ("block", 3),
                ("anon", 3),
                ("text c", 3),
                ("text d", 1),
            ])
        );
        assert!(!tree[0].children[0].children[1].is_block_level());
    }

    #[test]
    fn white_space_between_inline_level_boxes_is_kept_and_at_the_ends_dropped() {
        // <div> <span>a</span> <span>b</span> </div>: the space between
        // the spans is the gap between two words and stays as a text
        // box; the spaces at the start and the end would be discarded
        // by any line and are not built. Nodes: div 1, spans 2 and 3.
        let root = Element::new("div")
            .text(" ")
            .push(Element::new("span").text("a"))
            .text(" ")
            .push(Element::new("span").text("b"))
            .text(" ");
        let tree = built(root, "");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("inline", 2),
                ("text a", 2),
                ("text  ", 1),
                ("inline", 3),
                ("text b", 3),
            ])
        );
        // Inside an inline element white space is kept wherever it is:
        // `<span> </span>` between two inline-blocks is their gap.
        let root = Element::new("div").push(Element::new("span").text(" "));
        let tree = built(root, "");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[("anon", 1), ("inline", 2), ("text  ", 2)])
        );
        // White space before a block, at the end of a run, is dropped
        // with the run when nothing else is in it.
        let root = Element::new("div")
            .text(" ")
            .push(Element::new("p").text("x"))
            .text(" ");
        let tree = built(root, "");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[("block", 2), ("anon", 2), ("text x", 2)])
        );
    }

    #[test]
    fn a_float_stays_where_it_is_met_and_splits_nothing() {
        // <div><p>a</p><div class=f>f</div><p>b</p></div>: between
        // blocks, the float is a child of the div like them. Nodes:
        // div 1, p 2, div.f 3, p 4.
        let root = Element::new("div")
            .push(Element::new("p").text("a"))
            .push(Element::new("div").attr("class", "f").text("f"))
            .push(Element::new("p").text("b"));
        let tree = built(root, ".f { float: left; }");
        assert_eq!(
            tree.boxes_of(&tree[0].children),
            pairs(&[
                ("block", 2),
                ("anon", 2),
                ("float", 3),
                ("anon", 3),
                ("block", 4),
                ("anon", 4),
            ])
        );
        assert!(!tree[0].children[1].is_block_level());
        // <div>a<span>b<div class=f>f</div>c</span>d</div>: in the run,
        // inside the span, the float is inline-level content of the span
        // and does not split it (only an in-flow block does, CSS2
        // §9.2.1.1). Nodes: div 1, span 2, div.f 3.
        let root = Element::new("div").text("a").push(
            Element::new("span")
                .text("b")
                .push(Element::new("div").attr("class", "f").text("f"))
                .text("c"),
        );
        let tree = built(root, ".f { float: right; }");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("text a", 1),
                ("inline", 2),
                ("text b", 2),
                ("float", 3),
                ("anon", 3),
                ("text f", 3),
                ("text c", 2),
            ])
        );
        // A float first, then text: the float is met with the run still
        // empty, so it is a child of the div and the text's run follows.
        let root = Element::new("div")
            .push(Element::new("div").attr("class", "f"))
            .text("x");
        let tree = built(root, ".f { float: left; }");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[("float", 2), ("anon", 1), ("text x", 1)])
        );
        // An absolutely positioned box the same way, a float no more
        // (CSS2 §9.7). Nodes: div 1, span 2, div.a 3.
        let root = Element::new("div").text("a").push(
            Element::new("span")
                .text("b")
                .push(Element::new("div").attr("class", "a").text("f")),
        );
        let tree = built(root, ".a { position: absolute; float: left; }");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("text a", 1),
                ("inline", 2),
                ("text b", 2),
                ("absolute", 3),
                ("anon", 3),
                ("text f", 3),
            ])
        );
        let root = Element::new("div")
            .push(Element::new("div").attr("class", "a"))
            .push(Element::new("p").text("x"));
        let tree = built(root, ".a { position: fixed; }");
        assert_eq!(
            tree.shape_of(&tree[0].children),
            pairs(&[("absolute", 2), ("block", 3), ("anon", 3), ("text x", 3)])
        );
        assert!(!tree[0].children[0].is_block_level());
    }

    #[test]
    fn top_level_inline_content_goes_into_an_anonymous_block_of_the_document() {
        // A hand-built document with an inline element after the root
        // element, which is a block whatever its `display` (the
        // cascade blockifies it): the document node (0) is the block
        // container. Nodes: span 1, b 2.
        let doc = dom::Node::document(vec![
            Element::new("span").into(),
            Element::new("b").text("x").into(),
        ]);
        let (document, boxes) = boxes_of(&doc, &[]);
        let tree = Tree { document, boxes };
        assert_eq!(
            tree.shape_of(&tree),
            pairs(&[("block", 1), ("anon", 0), ("inline", 2), ("text x", 2)])
        );
    }

    #[test]
    fn table_elements_get_the_table_box_kinds() {
        // The UA sheet's display values, each its own kind. Nodes: html
        // 1, head 2, body 3, table 4, caption 5, colgroup 6, col 7,
        // thead 8, tr 9, th 10, tbody 11, tr 12, td 13, td 14, tfoot 15,
        // tr 16, td 17.
        let tree = tree_of(
            "<table><caption>c</caption><colgroup><col span=2></colgroup>\
             <thead><tr><th>h</th></tr></thead>\
             <tbody><tr><td colspan=2 rowspan=3>a</td><td>b</td></tr></tbody>\
             <tfoot><tr><td>f</td></tr></tfoot></table>",
            "",
        );
        let body = tree.body();
        assert_eq!(
            (body.kind.clone(), tree.number(body.node)),
            (BoxKind::Block, 3)
        );
        assert_eq!(
            tree.boxes_of(&body.children),
            pairs(&[
                ("table", 4),
                ("caption", 5),
                ("anon", 5),
                ("colgroup 1", 6),
                ("col 2", 7),
                ("header", 8),
                ("row", 9),
                ("cell 1x1", 10),
                ("anon", 10),
                ("body", 11),
                ("row", 12),
                ("cell 2x3", 13),
                ("anon", 13),
                ("cell 1x1", 14),
                ("anon", 14),
                ("footer", 15),
                ("row", 16),
                ("cell 1x1", 17),
                ("anon", 17),
            ])
        );
        let table = &body.children[0];
        assert!(table.is_block_level());
        assert!(
            !table.children[0].is_block_level(),
            "a caption is a table's child"
        );
        // The white space between the parts made no box anywhere.
        assert!(
            tree.shape_of(&body.children)
                .iter()
                .all(|(k, _)| !k.starts_with("text ") || k.trim() != "text")
        );
    }

    #[test]
    fn the_header_group_is_first_and_the_footer_group_last_whatever_their_place() {
        // tfoot, tbody, thead, a second thead and a row in source order:
        // laid out as header, body, thead-as-body, row, footer (CSS2
        // §17.2: only the first header and the first footer are such).
        // Nodes: table 4, tfoot 5, tbody 8, thead 11, thead 14, tr 17.
        let tree = tree_of(
            "<table><tfoot><tr><td>f</td></tr></tfoot><tbody><tr><td>b</td></tr></tbody>\
             <thead><tr><td>h</td></tr></thead><thead><tr><td>h2</td></tr></thead>\
             <tr><td>r</td></tr></table>",
            "",
        );
        let table = &tree.body().children[0];
        let parts: Vec<(String, usize)> = table
            .children
            .iter()
            .map(|c| {
                (
                    tree.shape_of(std::slice::from_ref(c))[0].0.clone(),
                    tree.number(c.node),
                )
            })
            .collect();
        assert_eq!(
            parts,
            pairs(&[
                ("header", 11),
                ("body", 8),
                ("body", 14),
                ("row", 17),
                ("footer", 5)
            ])
        );
        // A second footer is a body group too, in its place.
        let tree = tree_of(
            "<table><tfoot><tr><td>1</td></tr></tfoot><tfoot><tr><td>2</td></tr></tfoot>\
             <tbody><tr><td>3</td></tr></tbody></table>",
            "",
        );
        let table = &tree.body().children[0];
        let kinds: Vec<BoxKind> = table.children.iter().map(|c| c.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                BoxKind::RowGroup(RowGroupKind::Body),
                BoxKind::RowGroup(RowGroupKind::Body),
                BoxKind::RowGroup(RowGroupKind::Footer),
            ]
        );
        assert_eq!(
            tree.number(table.children[0].node),
            8,
            "the second footer, as a body group"
        );
        assert_eq!(
            tree.number(table.children[2].node),
            5,
            "the first footer, last"
        );
    }

    #[test]
    fn captions_go_to_their_side_and_columns_before_the_rows() {
        // Two captions, the first at the bottom by its own caption-side
        // and the second at the top by the table's inherited one, with
        // a column group after the rows: the order is top caption,
        // columns, rows, bottom caption. Nodes: table 4, caption 5,
        // caption 6, tr 7, td 8, colgroup 9.
        let tree = tree_of(
            "<table><caption class=b>b</caption><caption>t</caption>\
             <tr><td>x</td></tr><colgroup></colgroup></table>",
            "table { caption-side: top; } .b { caption-side: bottom; }",
        );
        let table = &tree.body().children[0];
        let parts: Vec<(String, usize)> = table
            .children
            .iter()
            .map(|c| {
                (
                    tree.shape_of(std::slice::from_ref(c))[0].0.clone(),
                    tree.number(c.node),
                )
            })
            .collect();
        assert_eq!(
            parts,
            pairs(&[
                ("caption", 6),
                ("colgroup 1", 9),
                ("row", 7),
                ("caption", 5)
            ])
        );
    }

    #[test]
    fn cells_outside_a_row_get_an_anonymous_row() {
        // Cells directly in the table share one anonymous row until a
        // row interrupts them; the same in a row group. The anonymous
        // row names the table (or the group) and inherits its style.
        // Nodes: table 4, td 5, td 6, tr 7, td 8, td 9, tbody 10, td 11.
        let tree = tree_of(
            "<table><td>a</td><td>b</td><tr><td>c</td></tr><td>d</td>\
             <tbody><td>e</td></tbody></table>",
            "table { color: #ff0000; }",
        );
        let table = &tree.body().children[0];
        assert_eq!(
            tree.boxes_of(&table.children),
            pairs(&[
                ("row", 4),
                ("cell 1x1", 5),
                ("anon", 5),
                ("cell 1x1", 6),
                ("anon", 6),
                ("row", 7),
                ("cell 1x1", 8),
                ("anon", 8),
                ("row", 4),
                ("cell 1x1", 9),
                ("anon", 9),
                ("body", 10),
                ("row", 10),
                ("cell 1x1", 11),
                ("anon", 11),
            ])
        );
        let anonymous_row = &table.children[0];
        assert_eq!(anonymous_row.style.display, Display::TableRow);
        assert_eq!(
            anonymous_row.style.color,
            style::computed::Color::rgb8(255, 0, 0)
        );
        assert_eq!(
            anonymous_row.style.padding,
            ComputedStyle::default().padding
        );
    }

    #[test]
    fn content_that_is_not_a_cell_gets_an_anonymous_cell() {
        // In a row: text and inline elements around a cell share an
        // anonymous cell, consecutive ones in the same cell, each cell
        // a block container with the content in an anonymous block;
        // the anonymous cell names the row, spans one column and one
        // row, and sits on the baseline (vertical-align is not
        // inherited). In a table: an anonymous row around an anonymous
        // cell. Nodes: table 4, tr 5, td 6, span 7, b 8, table 9, p 10.
        let tree = tree_of(
            "<table><tr>x<td>a</td><span>y</span><b>z</b></tr></table>\
             <table><p>para</p></table>",
            "",
        );
        let body = tree.body();
        assert_eq!(
            tree.shape_of(&body.children),
            pairs(&[
                ("table", 4),
                ("row", 5),
                ("cell 1x1", 5),
                ("anon", 5),
                ("text x", 5),
                ("cell 1x1", 6),
                ("anon", 6),
                ("text a", 6),
                ("cell 1x1", 5),
                ("anon", 5),
                ("inline", 7),
                ("text y", 7),
                ("inline", 8),
                ("text z", 8),
                ("table", 9),
                ("row", 9),
                ("cell 1x1", 9),
                ("block", 10),
                ("anon", 10),
                ("text para", 10),
            ])
        );
        let anonymous_cell = &body.children[0].children[0].children[0];
        assert_eq!(anonymous_cell.style.display, Display::TableCell);
        assert_eq!(
            anonymous_cell.style.vertical_align,
            style::computed::VerticalAlign::Baseline
        );
        assert_eq!(
            anonymous_cell.style.padding,
            ComputedStyle::default().padding
        );
    }

    #[test]
    fn white_space_between_table_parts_is_no_content() {
        // Spaces and newlines between rows, cells and groups make
        // neither cells nor rows; inside an anonymous cell's content
        // they are the gaps they are, dropped at its ends. Nodes: table
        // 4, tbody 5, tr 6, td 7, td 8, tr 9, span 10, span 11.
        let tree = tree_of(
            "<table>\n  <tbody>\n    <tr>\n      <td>a</td> <td>b</td>\n    </tr>\n  </tbody>\n\
             <tr> <span>x</span> <span>y</span> </tr> </table>",
            "",
        );
        let table = &tree.body().children[0];
        assert_eq!(
            tree.shape_of(&table.children),
            pairs(&[
                ("body", 5),
                ("row", 6),
                ("cell 1x1", 7),
                ("anon", 7),
                ("text a", 7),
                ("cell 1x1", 8),
                ("anon", 8),
                ("text b", 8),
                ("row", 9),
                ("cell 1x1", 9),
                ("anon", 9),
                ("inline", 10),
                ("text x", 10),
                ("text  ", 9),
                ("inline", 11),
                ("text y", 11),
            ])
        );
    }

    #[test]
    fn table_parts_outside_a_table_get_an_anonymous_table() {
        // CSS2 §17.2.1's missing parents: cells in a div share an
        // anonymous row in an anonymous table that names the div and
        // inherits from it; white space between them is dropped; a row
        // and a row group join the same table; other content ends it.
        // Nodes: div 4, div.c 5, div.c 6, div.r 7, div.c 8, div.g 9,
        // div.c 10, p 11, div.c 12.
        let tree = tree_of(
            "<div><div class=c>a</div> <div class=c>b</div><div class=r><div class=c>c</div></div>\
             <div class=g><div class=c>d</div></div><p>x</p><div class=c>e</div></div>",
            ".c { display: table-cell; } .r { display: table-row; }
             .g { display: table-row-group; } div { color: #0000ff; }",
        );
        let div = &tree.body().children[0];
        assert_eq!(
            tree.boxes_of(&div.children),
            pairs(&[
                ("table", 4),
                ("row", 4),
                ("cell 1x1", 5),
                ("anon", 5),
                ("cell 1x1", 6),
                ("anon", 6),
                ("row", 7),
                ("cell 1x1", 8),
                ("anon", 8),
                ("body", 9),
                ("row", 9),
                ("cell 1x1", 10),
                ("anon", 10),
                ("block", 11),
                ("anon", 11),
                ("table", 4),
                ("row", 4),
                ("cell 1x1", 12),
                ("anon", 12),
            ])
        );
        let anonymous_table = &div.children[0];
        assert!(anonymous_table.is_block_level());
        assert_eq!(anonymous_table.style.display, Display::Table);
        assert_eq!(
            anonymous_table.style.color,
            style::computed::Color::rgb8(0, 0, 255)
        );
        assert_eq!(
            anonymous_table.style.margin,
            ComputedStyle::default().margin
        );
        // A cell whose element is a `div` spans one column and one row
        // whatever its attributes say: the HTML attributes are td's and
        // th's.
        let tree = tree_of(
            "<div><div class=c colspan=3 rowspan=2>a</div></div>",
            ".c { display: table-cell; }",
        );
        let cell = &tree.body().children[0].children[0].children[0].children[0];
        assert_eq!(
            cell.kind,
            BoxKind::Cell {
                colspan: 1,
                rowspan: 1
            }
        );
    }

    #[test]
    fn table_parts_inside_an_inline_get_an_anonymous_inline_table() {
        // <div>a<span>b<em class=c>c</em> <em class=c>d</em>e</span></div>:
        // the anonymous table's parent is the span, so it is an inline
        // table, inline-level content of the span, which it does not
        // split. Nodes: div 4, span 5, em 6, em 7.
        let tree = tree_of(
            "<div>a<span>b<em class=c>c</em> <em class=c>d</em>e</span></div>",
            ".c { display: table-cell; }",
        );
        let div = &tree.body().children[0];
        assert_eq!(
            tree.shape_of(&div.children),
            pairs(&[
                ("anon", 4),
                ("text a", 4),
                ("inline", 5),
                ("text b", 5),
                ("inline-table", 5),
                ("row", 5),
                ("cell 1x1", 6),
                ("anon", 6),
                ("text c", 6),
                ("cell 1x1", 7),
                ("anon", 7),
                ("text d", 7),
                ("text e", 5),
            ])
        );
        let table = &div.children[0].children[1].children[1];
        assert!(!table.is_block_level());
        assert_eq!(table.style.display, Display::InlineTable);
    }

    #[test]
    fn an_inline_table_is_inline_level_content() {
        // Nodes: div 4, table 5, tr 6, td 7.
        let tree = tree_of(
            "<div>a<table class=i><tr><td>x</td></tr></table>b</div>",
            ".i { display: inline-table; }",
        );
        let div = &tree.body().children[0];
        assert_eq!(
            tree.shape_of(&div.children),
            pairs(&[
                ("anon", 4),
                ("text a", 4),
                ("inline-table", 5),
                ("row", 6),
                ("cell 1x1", 7),
                ("anon", 7),
                ("text x", 7),
                ("text b", 4),
            ])
        );
    }

    #[test]
    fn a_floated_or_positioned_table_is_out_of_flow_with_a_tables_children() {
        // The float is the box, with `display: table` and the table's
        // parts inside it; so for an absolutely positioned table and a
        // floated inline table, which CSS2 §9.7 makes a table. Nodes:
        // div 4, table 5, tr 6, td 7, table 8, tr 9, td 10, table 11,
        // tr 12, td 13.
        let tree = tree_of(
            "<div><table class=f><tr><td>x</td></tr></table>\
             <table class=a><tr><td>y</td></tr></table>\
             <table class=i><tr><td>z</td></tr></table></div>",
            ".f { float: left; } .a { position: absolute; }
             .i { display: inline-table; float: right; }",
        );
        let div = &tree.body().children[0];
        assert_eq!(
            tree.boxes_of(&div.children),
            pairs(&[
                ("float", 5),
                ("row", 6),
                ("cell 1x1", 7),
                ("anon", 7),
                ("absolute", 8),
                ("row", 9),
                ("cell 1x1", 10),
                ("anon", 10),
                ("float", 11),
                ("row", 12),
                ("cell 1x1", 13),
                ("anon", 13),
            ])
        );
        assert!(
            div.children
                .iter()
                .all(|c| c.style.display == Display::Table)
        );
        // A float inside a row is content for an anonymous cell, a
        // float inside it. Nodes: table 5, tr 6, div.f 7, td 8.
        let tree = tree_of(
            "<div><table><tr><div class=f>f</div><td>x</td></tr></table></div>",
            ".f { float: left; }",
        );
        let table = &tree.body().children[0].children[0];
        assert_eq!(
            tree.boxes_of(&table.children),
            pairs(&[
                ("row", 6),
                ("cell 1x1", 6),
                ("float", 7),
                ("anon", 7),
                ("cell 1x1", 8),
                ("anon", 8),
            ])
        );
    }

    #[test]
    fn column_boxes_hold_no_content_and_a_column_group_only_columns() {
        // A col's children generate nothing; a colgroup's children that
        // are not columns are dropped; a colgroup's span counts when it
        // has no columns. Nodes: table 4, colgroup 5, col 6, span 8,
        // colgroup 9, col 10, col 11, tr 12.
        let tree = tree_of(
            "<table><colgroup span=4><col>text<span>s</span></col><span>x</span></colgroup>\
             <colgroup><col><col span=3></colgroup><tr><td>c</td></tr></table>",
            "",
        );
        let table = &tree.body().children[0];
        assert_eq!(
            tree.boxes_of(&table.children),
            pairs(&[
                ("colgroup 4", 5),
                ("col 1", 6),
                ("colgroup 1", 9),
                ("col 1", 10),
                ("col 3", 11),
                ("row", 12),
                ("cell 1x1", 13),
                ("anon", 13),
            ])
        );
        assert!(table.children[0].children[0].children.is_empty());
    }

    #[test]
    fn the_span_attributes_are_read_as_the_html_standard_parses_them() {
        for (attrs, expected) in [
            ("", (1, 1)),
            ("colspan=3 rowspan=2", (3, 2)),
            ("colspan=0 rowspan=0", (1, 0)),
            ("colspan=5000 rowspan=70000", (1000, 65534)),
            ("colspan=x rowspan=' 7px'", (1, 7)),
            ("colspan='+2' rowspan=-1", (2, 1)),
            ("colspan=2.5 rowspan=99999999999999999999999", (2, 65534)),
            ("colspan=-0", (1, 1)),
        ] {
            let tree = tree_of(&format!("<table><tr><td {attrs}>x</td></tr></table>"), "");
            let cell = &tree.body().children[0].children[0].children[0];
            assert_eq!(
                cell.kind,
                BoxKind::Cell {
                    colspan: expected.0,
                    rowspan: expected.1
                },
                "{attrs}"
            );
        }
        for (attr, expected) in [("", 1), ("span=2", 2), ("span=0", 1), ("span=1001", 1000)] {
            let tree = tree_of(
                &format!("<table><col {attr}><tr><td>x</td></tr></table>"),
                "",
            );
            let col = &tree.body().children[0].children[0];
            assert_eq!(col.kind, BoxKind::Column { span: expected }, "{attr}");
        }
    }

    #[test]
    fn before_and_after_generate_the_first_and_last_child_boxes() {
        // Nodes: p 4, b 5.
        let tree = tree_of(
            "<p>x<b>y</b></p>",
            "p::before { content: 'a'; color: #ff0000 } p::after { content: 'c' }",
        );
        assert_eq!(
            tree.shape_of(&tree.body().children),
            pairs(&[
                ("block", 4),
                ("anon", 4),
                ("+inline", 4),
                ("+text a", 4),
                ("text x", 4),
                ("inline", 5),
                ("text y", 5),
                ("+inline", 4),
                ("+text c", 4),
            ])
        );
        // The generated box names its element and is anonymous to the
        // cache; it and its text take the pseudo-element's style.
        let before = &tree.body().children[0].children[0].children[0];
        assert_eq!(before.generated, Some(PseudoElement::Before));
        assert!(before.anonymous);
        assert_eq!(before.style.color, style::computed::Color::rgb8(255, 0, 0));
        assert_eq!(
            before.children[0].style.color,
            style::computed::Color::rgb8(255, 0, 0)
        );
        let p = &tree.body().children[0];
        assert!(p.generated.is_none() && !p.anonymous);
    }

    #[test]
    fn a_generated_box_is_built_as_its_display_makes_it() {
        // Nodes: div 4.
        for (css, expected) in [
            (
                "display: block",
                vec![("+block", 4), ("+anon", 4), ("anon", 4)],
            ),
            (
                "display: inline-block",
                vec![("anon", 4), ("+inline-block", 4), ("+anon", 4)],
            ),
            (
                "float: left",
                vec![("+float", 4), ("+anon", 4), ("anon", 4)],
            ),
            (
                "position: absolute",
                vec![("+absolute", 4), ("+anon", 4), ("anon", 4)],
            ),
            (
                "display: table",
                vec![
                    ("+table", 4),
                    ("+row", 4),
                    ("+cell 1x1", 4),
                    ("+anon", 4),
                    ("anon", 4),
                ],
            ),
            (
                "display: inline-table",
                vec![
                    ("anon", 4),
                    ("+inline-table", 4),
                    ("+row", 4),
                    ("+cell 1x1", 4),
                    ("+anon", 4),
                ],
            ),
            // A table part outside a table gets an anonymous table and
            // row, which belong to the element.
            (
                "display: table-cell",
                vec![
                    ("table", 4),
                    ("row", 4),
                    ("+cell 1x1", 4),
                    ("+anon", 4),
                    ("anon", 4),
                ],
            ),
            (
                "display: table-row",
                vec![
                    ("table", 4),
                    ("+row", 4),
                    ("+cell 1x1", 4),
                    ("+anon", 4),
                    ("anon", 4),
                ],
            ),
            (
                "display: table-caption",
                vec![("table", 4), ("+caption", 4), ("+anon", 4), ("anon", 4)],
            ),
        ] {
            let tree = tree_of(
                "<div>x</div>",
                &format!("div::before {{ content: 'a'; {css} }}"),
            );
            let mut expected = expected;
            expected.insert(0, ("block", 4));
            assert_eq!(
                tree.boxes_of(&tree.body().children),
                pairs(&expected),
                "{css}"
            );
            assert_eq!(tree.texts(&tree.body().children), ["a", "x"], "{css}");
        }
    }

    #[test]
    fn a_generated_block_in_an_inline_is_hoisted_and_splits_it() {
        // Nodes: div 4, span 5.
        let tree = tree_of(
            "<div><span>x</span></div>",
            "span::before { display: block; content: 'a' }",
        );
        assert_eq!(
            tree.shape_of(&tree.body().children[0].children),
            pairs(&[
                ("anon", 4),
                ("inline", 5),
                ("+block", 5),
                ("+anon", 5),
                ("+text a", 5),
                ("anon", 4),
                ("inline", 5),
                ("text x", 5),
            ])
        );
    }

    #[test]
    fn no_box_without_content_or_with_display_none() {
        for css in [
            "p::before { content: none }",
            "p::before { content: normal }",
            "p::before { color: #ff0000 }",
            "p::before { content: 'a'; display: none }",
            "p::after { content: 'a' } p { display: none }",
        ] {
            let tree = tree_of("<p>x</p><div></div>", css);
            assert!(
                tree.shape_of(&tree.body().children)
                    .iter()
                    .all(|(k, _)| !k.starts_with('+')),
                "{css}"
            );
        }
        // An empty string makes a box with nothing in it.
        let tree = tree_of("<p>x</p>", "p::before { content: '' }");
        assert_eq!(
            tree.shape_of(&tree.body().children),
            pairs(&[("block", 4), ("anon", 4), ("+inline", 4), ("text x", 4)])
        );
    }

    #[test]
    fn replaced_elements_have_no_generated_boxes() {
        for tag in [
            "img", "input", "br", "wbr", "embed", "textarea", "select", "iframe", "video", "audio",
            "canvas", "object",
        ] {
            let tree = tree_of(
                &format!("<p><{tag}></{tag}><span></span></p>"),
                &format!(
                    "{tag}::before, {tag}::after, span::after {{ content: 'r'; display: block }}"
                ),
            );
            // The span's alone.
            assert_eq!(tree.texts(&tree.body().children), ["r"], "{tag}");
        }
    }

    #[test]
    fn generated_white_space_is_text_like_any_other() {
        // At the start of a block container's run, and between table
        // parts, it is no content.
        let tree = tree_of(
            "<p>x</p><div>y</div>",
            "p::before { display: block; content: ' ' } div::before { display: table; content: ' ' }",
        );
        assert_eq!(
            tree.boxes_of(&tree.body().children),
            pairs(&[
                ("block", 4),
                ("+block", 4),
                ("anon", 4),
                ("block", 5),
                ("+table", 5),
                ("anon", 5)
            ])
        );
        // Between two words it is the gap.
        let tree = tree_of("<p>x<b>y</b></p>", "b::before { content: ' ' }");
        assert_eq!(tree.texts(&tree.body().children), ["x", " ", "y"]);
    }

    #[test]
    fn content_attr_reads_the_originating_elements_attribute() {
        let tree = tree_of(
            "<a href=/x>link</a><a>bare</a>",
            "a::after { content: ' (' attr(href) ')' }",
        );
        assert_eq!(
            tree.texts(&tree.body().children),
            ["link", " (/x)", "bare", " ()"]
        );
    }

    #[test]
    fn quote_marks_follow_the_depth_in_document_order() {
        // The UA sheet's `q` rules, and `quotes: auto`.
        let tree = tree_of("<p><q>a<q>b</q>c</q><q>d</q></p>", "");
        assert_eq!(
            tree.texts(&tree.body().children),
            [
                "\u{201C}", "a", "\u{2018}", "b", "\u{2019}", "c", "\u{201D}", "\u{201C}", "d",
                "\u{201D}"
            ]
        );
        // A close-quote at depth 0 gives no mark and leaves the depth;
        // the no- keywords move it without one; a depth past the pairs
        // takes the last.
        let tree = tree_of(
            "<p><span class=x></span><q>a</q><span class=n></span><q>b<q>c<q>d</q></q></q>\
             <span class=m></span><q>e</q></p>",
            "p { quotes: '<' '>' '{' '}' } .x::before { content: close-quote 'X' }
             .n::before { content: no-open-quote } .m::before { content: no-close-quote }",
        );
        assert_eq!(
            tree.texts(&tree.body().children),
            [
                "X", "<", "a", ">", "{", "b", "{", "c", "{", "d", "}", "}", "}", "<", "e", ">"
            ]
        );
        // `quotes: none` gives no marks.
        let tree = tree_of("<p><q>a</q></p>", "p { quotes: none }");
        assert_eq!(tree.texts(&tree.body().children), ["a"]);
    }

    #[test]
    fn a_quote_moves_the_depth_where_its_box_shows_nothing() {
        // The open quotes are white space, which makes no text box, in
        // a block and in a table; the depth moves all the same.
        for display in ["block", "table"] {
            let tree = tree_of(
                "<div class=s></div><q>a</q>",
                &format!(
                    ".s {{ quotes: ' ' ' ' }} .s::before {{ content: open-quote; display: {display} }}"
                ),
            );
            assert_eq!(
                tree.texts(&tree.body().children),
                ["\u{2018}", "a", "\u{2019}"],
                "{display}"
            );
        }
    }

    #[test]
    fn quote_depth_follows_document_order_through_table_parts() {
        // The span before the cell goes into an anonymous cell, which
        // is built before the cell, as it comes first.
        for html in [
            "<div class=row><span class=o></span><div class=cell><span class=o></span></div></div>",
            "<div class=table><span class=o></span><div class=cell><span class=o></span></div></div>",
            "<div class=group><span class=o></span><div class=row><span class=o></span></div></div>",
        ] {
            let tree = tree_of(
                html,
                ".o::before { content: open-quote } .table { display: table }
                 .group { display: table-row-group } .row { display: table-row }
                 .cell { display: table-cell }",
            );
            assert_eq!(
                tree.texts(&tree.body().children),
                ["\u{201C}", "\u{2018}"],
                "{html}"
            );
        }
    }

    #[test]
    fn a_box_carries_a_fingerprint_of_the_generated_text_in_it() {
        // Nodes: div 4, span 5, p 6, b 7.
        let tree = tree_of(
            "<div><span class=o></span><p><b>x</b></p></div>",
            ".o::before { content: open-quote }",
        );
        let div = &tree.body().children[0];
        let span = &div.children[0].children[0];
        let p = &div.children[1];
        assert_eq!(tree.number(span.node), 5);
        assert_ne!(span.generated_text, 0);
        assert_ne!(div.generated_text, 0);
        assert_ne!(tree.body().generated_text, 0);
        assert_eq!((tree.number(p.node), p.generated_text), (6, 0));
        assert_eq!(p.children[0].generated_text, 0);
        // The same box with another counter value in it.
        let counted = |reset: &str| {
            let tree = tree_of(
                "<p class=c></p>",
                &format!(
                    "body {{ counter-reset: c {reset} }} .c::before {{ content: counter(c) }}"
                ),
            );
            tree.body().children[0].generated_text
        };
        assert_ne!(counted("1"), counted("2"));
        assert_eq!(counted("1"), counted("1"));
    }

    /// The texts in the body of the document `html` parses to, under
    /// `css_src`.
    fn texts_of(html: &str, css_src: &str) -> Vec<String> {
        let tree = tree_of(html, css_src);
        tree.texts(&tree.body().children)
    }

    #[test]
    fn counters_are_scoped_as_the_reference_browser_and_the_tests_have_it() {
        // css/css-lists/counters-scope-001: a counter a sibling
        // instantiates inside an ancestor's ends with the sibling.
        assert_eq!(
            texts_of(
                "<span class=scope><span class='one c'><span class=c></span></span>\
                 <span class=c></span></span><span class=c></span>\
                 <span class=scope><span class='two c'><span class=c></span></span>\
                 <span class=c></span></span><span class=c></span>",
                ".scope { counter-reset: c 1 }
                 .scope::before, .scope::after, .c::before { content: counter(c) }
                 .one::before { counter-reset: c 2 } .two { counter-reset: c 3 }",
            ),
            ["1", "2", "1", "1", "1", "1", "1", "3", "3", "1", "1", "1"]
        );
        // counters-scope-002.
        assert_eq!(
            texts_of(
                "<div id=scope><span><span><span id=reset><span></span><span></span></span>\
                 <span><span></span></span></span></span></div>",
                "#scope, span#reset { counter-reset: c 0 }
                 span::before { counter-increment: c 1; content: 'B' counters(c, '.') '-' }
                 span::after { counter-increment: c 1; content: 'A' counters(c, '.') '-' }",
            )
            .concat(),
            "B1-B2-B2.1-B2.2-A2.3-B2.4-A2.5-A2.6-B3-B4-A5-A6-A7-A8-"
        );
        // counters-scope-003.
        assert_eq!(
            texts_of(
                "<div><span><span id=reset><span></span></span></span></div>",
                "body, span#reset::before { counter-reset: c 0 }
                 span::before { counter-increment: c 1; content: 'B' counters(c, '.') '-' }
                 span::after { counter-increment: c 1; content: 'A' counters(c, '.') '-' }",
            )
            .concat(),
            "B1-B1.1-B2-A3-A4-A5-"
        );
        // counters-scope-004, which asks for "1 1 R 2 3": the counter
        // the `::before` instantiates is inside one a sibling of the
        // span did, not an ancestor, and passes on, as in the reference
        // browser and the draft.
        assert_eq!(
            texts_of(
                "<div><span class=reset></span><span class=use></span>\
                 <span class=reset></span><span class=use></span><span class=rb>\
                 <span class=use></span><span class=reset></span><span class=use></span>\
                 </span></div>",
                ".reset { counter-reset: c } .rb::before { counter-reset: c; content: 'R' }
                 .use { counter-increment: c } .use::before { content: counters(c, '.') }",
            ),
            ["1", "1", "R", "1.1", "1.1"]
        );
    }

    #[test]
    fn a_counter_a_sibling_nests_passes_on_unless_an_ancestor_has_the_name() {
        let use_ = ".s::before { content: counters(c, '.') }";
        // The outer counter is a preceding sibling's of the parent: the
        // nested one passes on.
        assert_eq!(
            texts_of(
                "<div><div style='counter-reset: c 10'></div><div>\
                 <div style='counter-reset: c 1'></div><div class=s></div></div></div>",
                use_,
            ),
            ["10.1"]
        );
        // The grandparent's: it does not.
        assert_eq!(
            texts_of(
                "<div style='counter-reset: c 10'><div>\
                 <div style='counter-reset: c 1'></div><div class=s></div></div></div>",
                use_,
            ),
            ["10"]
        );
        // An increment that instantiates the counter instantiates it
        // where it is, which a reset inside then nests in.
        assert_eq!(
            texts_of(
                "<div style='counter-increment: c 5'><div style='counter-reset: c 3'></div>\
                 <div class=s></div></div><div class=s></div>",
                use_,
            ),
            ["5", "5"]
        );
        // A `counter()` that finds none instantiates one at 0 on its
        // pseudo-element, as the drafts say; the reference browser
        // writes "7" here.
        assert_eq!(
            texts_of(
                "<div class=s><div><div style='counter-reset: c 7'></div>\
                 <div class=s></div></div></div>",
                use_,
            ),
            ["0", "0.7"]
        );
    }

    #[test]
    fn counters_reset_increment_and_set_in_that_order() {
        assert_eq!(
            texts_of(
                "<p id=a></p><p id=b></p><p id=c></p>",
                "body { counter-reset: n 5 m }
                 #a { counter-increment: n 2 n } #a::before { content: counter(n) }
                 #b { counter-reset: n 1; counter-increment: n; counter-set: n 9 }
                 #b::before { content: counter(n) }
                 #c { counter-set: n 1 n 4; counter-increment: m -3 }
                 #c::before { content: counter(n) ',' counter(m) ',' counter(new) }",
            ),
            ["8", "9", "4,-3,0"]
        );
    }

    #[test]
    fn what_generates_no_box_touches_no_counter() {
        // css/css-lists/counter-reset-increment-set-display-none, and a
        // pseudo-element whose content is `none`.
        assert_eq!(
            texts_of(
                "<p class='i none'></p><p class=i></p><p class=u></p><p class=k></p><p class=u></p>",
                "body { counter-reset: n } .i { counter-increment: n 10 } .none { display: none }
                 .k::before { counter-increment: n 100; content: none }
                 .u::before { content: counter(n) }",
            ),
            ["10", "10"]
        );
    }

    #[test]
    fn an_increment_past_32_bits_is_not_made() {
        // css/css-lists/counter-reset-increment-overflow-underflow, as the
        // reference browser has it: the value stays where it was.
        assert_eq!(
            texts_of(
                "<div class=o><h3></h3><h3></h3><h3></h3></div>\
                 <div class=u><h3></h3><h3></h3><h3></h3></div>",
                ".o { counter-reset: n 2000000000 } .u { counter-reset: n -2000000000 }
                 .o h3::before { counter-increment: n 50000000; content: counter(n) }
                 .u h3::before { counter-increment: n -50000000; content: counter(n) }",
            ),
            [
                "2050000000",
                "2100000000",
                "2100000000",
                "-2050000000",
                "-2100000000",
                "-2100000000"
            ]
        );
    }

    #[test]
    fn counters_are_written_in_their_counter_styles() {
        assert_eq!(
            texts_of(
                "<p></p>",
                "p { counter-reset: n 4 }
                 p::before { content: counter(n, upper-roman) '|' counter(n, LOWER-ALPHA) '|'
                     counter(n, none) '|' counters(n, '.', disc) '|' counter(n, my-own) }",
            ),
            ["IV|d||\u{2022}|4"]
        );
    }

    #[test]
    fn a_reversed_counter_starts_where_its_increments_count_down_from() {
        // The web-platform-tests' counter-reset-reversed-*: siblings-001a,
        // nested, pseudo-001, pseudo-003, multiple, with-regular and
        // display-none.
        let cases: [(&str, &str, &[&str]); 7] = [
            (
                "<div style='counter-reset: reversed(foo)'>\
                 <div class=u style='counter-increment: foo -1'></div></div>\
                 <div class=u style='counter-increment: foo -2'></div>",
                ".u::before { content: counter(foo) }",
                &["4", "2"],
            ),
            (
                "<ol><li>Three</li><li>Two<ol><li>Eleven</li><li class=set>Nine</li>\
                 <li>Eight</li></ol></li><li>One</li></ol>",
                "ol { counter-reset: reversed(foo) } li { display: block }
                 li::before { counter-increment: foo -1; content: counter(foo) '. ' }
                 .set { counter-set: foo 10 }",
                &[
                    "3. ", "Three", "2. ", "Two", "11. ", "Eleven", "9. ", "Nine", "8. ", "Eight",
                    "1. ", "One",
                ],
            ),
            (
                "<div class=list><div class=item></div><div class=item></div></div>",
                ".list { counter-reset: reversed(foo) }
                 .item::before { counter-increment: foo -1; content: 'B' counter(foo) }
                 .item::after { counter-increment: foo -2; content: 'A' counter(foo) }",
                &["B7", "A5", "B4", "A2"],
            ),
            (
                "<div class=item></div>",
                ".item { counter-reset: reversed(foo); counter-increment: foo -1 }
                 .item::before { counter-increment: foo -2; content: 'B' counter(foo) }
                 .item::after { counter-increment: foo -3; content: 'A' counter(foo) }",
                &["B6", "A3"],
            ),
            (
                "<div class=multi><div class=item></div><div class=item></div></div>",
                ".multi { counter-reset: reversed(c1) reversed(c2) }
                 .item { counter-increment: c1 -1 c2 -2 }
                 .item::before { content: counter(c1) ',' counter(c2) }",
                &["2,4", "1,2"],
            ),
            (
                "<div class=r><div class=item><div class=g><div class=out></div></div></div>\
                 <div class=item></div></div>",
                ".r { counter-reset: reversed(foo) } .item { counter-increment: foo -1 }
                 .g { counter-reset: foo 2 } .out::before { content: counters(foo, '.') }",
                &["2.2"],
            ),
            (
                "<div style='counter-reset: reversed(foo)'><div class=u></div>\
                 <div class='u hidden'></div><div class='u none'></div><div class=u></div></div>",
                ".u { counter-increment: foo -1 } .u::before { content: counter(foo) }
                 .none { display: none } .hidden { visibility: hidden }",
                &["3", "2", "1"],
            ),
        ];
        for (html, css_src, expected) in cases {
            assert_eq!(texts_of(html, css_src), expected, "{html}");
        }
        // counter-reset-reversed-siblings-003 asks for "4" and "2", the
        // second element seeing the reversed counter its sibling nests
        // in its parent's, which the scoping here ends with the sibling.
        assert_eq!(
            texts_of(
                "<div style='counter-reset: foo 10'><div style='counter-reset: reversed(foo)'>\
                 <div class=u style='counter-increment: foo -1'></div></div>\
                 <div class=u style='counter-increment: foo -2'></div></div>",
                ".u::before { content: counter(foo) }",
            ),
            ["1", "8"]
        );
    }

    #[test]
    fn a_list_items_marker_goes_where_its_position_puts_it() {
        // Nodes: ol 4, li 5, p 6. Outside, in the run of the first line,
        // as its first box, whichever block that run is in, the blocks
        // on the way marked; inside, an inline box at the start.
        let tree = tree_of("<ol><li>a</li></ol>", "");
        assert_eq!(
            tree.shape_of(&tree.body().children),
            pairs(&[
                ("block", 4),
                ("block", 5),
                ("anon", 5),
                ("*marker outside", 5),
                ("*text 1. ", 5),
                ("text a", 5),
            ])
        );
        let tree = tree_of("<ol><li><p>a</p></li></ol>", "");
        let li = &tree.body().children[0].children[0];
        let p = &li.children[0];
        assert_eq!(tree.number(p.node), 6);
        assert!(p.styled_by_ancestor && !li.styled_by_ancestor);
        assert_eq!(
            tree.shape_of(&p.children),
            pairs(&[
                ("anon", 6),
                ("*marker outside", 5),
                ("*text 1. ", 5),
                ("text a", 6)
            ])
        );
        let tree = tree_of("<ol><li>a</li></ol>", "li { list-style-position: inside }");
        assert_eq!(
            tree.shape_of(&tree.body().children[0].children[0].children),
            pairs(&[("anon", 5), ("*inline", 5), ("*text 1. ", 5), ("text a", 5)])
        );
        // With no first line, the list item's first child.
        let tree = tree_of("<ul><li><table><tr><td>a</table></li></ul>", "");
        let li = &tree.body().children[0].children[0];
        assert_eq!(
            tree.boxes_of(&li.children)[..2],
            pairs(&[("*marker outside Disc", 5), ("table", 6)])
        );
        // Nested lists' markers share the first line, the outer one's
        // first.
        let tree = tree_of("<ul><li><ol><li>a</li></ol></li></ul>", "");
        assert_eq!(
            tree.texts(&tree.body().children),
            ["1. ", "a"],
            "the disc is drawn, not text"
        );
        let markers: Vec<_> = tree
            .boxes_of(&tree.body().children)
            .into_iter()
            .filter(|(kind, _)| kind.starts_with("*marker"))
            .collect();
        assert_eq!(
            markers,
            pairs(&[("*marker outside", 7), ("*marker outside Disc", 5)])
        );
    }

    #[test]
    fn a_marker_says_the_list_item_counter_in_the_list_style_type() {
        let markers = |html: &str, css_src: &str| -> Vec<String> {
            let tree = tree_of(html, css_src);
            let mut out = Vec::new();
            let mut stack: Vec<&LayoutBox> = tree.boxes.iter().collect();
            while let Some(b) = stack.pop() {
                if let (BoxKind::Text(text), Some(PseudoElement::Marker)) = (&b.kind, b.generated) {
                    out.push(text.clone());
                }
                stack.extend(b.children.iter().rev());
            }
            out
        };
        assert_eq!(
            markers("<ol><li>a<li>b<li>c</ol>", ""),
            ["1. ", "2. ", "3. "]
        );
        // `start`, `reversed`, `value` and `type`.
        assert_eq!(markers("<ol start=9><li>a<li>b</ol>", ""), ["9. ", "10. "]);
        assert_eq!(
            markers("<ol reversed><li>a<li>b<li>c</ol>", ""),
            ["3. ", "2. ", "1. "]
        );
        assert_eq!(
            markers("<ol reversed start=10><li>a<li>b</ol>", ""),
            ["10. ", "9. "]
        );
        assert_eq!(
            markers("<ol><li>a<li value=7>b<li>c</ol>", ""),
            ["1. ", "7. ", "8. "]
        );
        assert_eq!(
            markers("<ol type=I><li>a<li type=a>b</ol>", ""),
            ["I. ", "b. "]
        );
        // A string, `content`, none; an explicit increment of the
        // counter replaces a list item's own.
        assert_eq!(
            markers(
                "<ol><li class=s>a<li class=c>b<li class=n>c<li class=x>d<li>e</ol>",
                ".s { list-style-type: '-> ' } .c::marker { content: '(' counter(list-item) ')' }
                 .n { list-style-type: none } .x { counter-increment: list-item 5 }",
            ),
            // The third, with no marker, counts all the same: 3, then 8.
            ["-> ", "(2)", "8. ", "9. "]
        );
        assert_eq!(
            markers(
                "<ol><li>a<li>b</ol>",
                "li::marker { content: none } li:first-child::marker { content: normal }"
            ),
            ["1. "]
        );
        // Nested, each list its own counter.
        assert_eq!(
            markers(
                "<ol><li>a<ol><li>b<li>c</ol><li>d</ol>",
                "ol ol li::marker { content: counters(list-item, '.') ' ' }"
            ),
            ["1. ", "1.1 ", "1.2 ", "2. "]
        );
    }

    #[test]
    fn a_list_items_first_letter_is_after_its_marker() {
        for position in ["outside", "inside"] {
            let tree = tree_of(
                "<ol><li>ab</li></ol>",
                &format!(
                    "li {{ list-style-position: {position} }} li::first-letter {{ color: #f00 }}"
                ),
            );
            let shape = tree.shape_of(&tree.body().children);
            let letter = shape
                .iter()
                .position(|(kind, _)| kind == "^text a")
                .expect("the first letter is the content's");
            let marker = shape
                .iter()
                .position(|(kind, _)| kind == "*text 1. ")
                .expect("the marker keeps its text");
            assert!(marker < letter, "{position}");
        }
    }

    #[test]
    fn a_set_and_a_walk_hold_only_so_many_counters() {
        // The 1025th name of a reset is not instantiated, and `counter()`
        // cannot instantiate it either.
        let names: Vec<String> = (0..1100).map(|i| format!("c{i}")).collect();
        let css_src = format!(
            "p {{ counter-reset: {} }} p::before {{ content: counter(c1023, decimal) ',' counter(c1024) }}",
            names
                .iter()
                .map(|n| format!("{n} 7"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert_eq!(texts_of("<p></p>", &css_src), ["7,0"]);
    }

    #[test]
    fn generated_table_parts_take_no_spans_and_go_where_their_display_puts_them() {
        // Nodes: table 4, tr 5, td 6.
        let tree = tree_of(
            "<table><tr><td colspan=3>x</td></tr></table>",
            "tr::before { display: table-cell; content: 'a' }
             td::before { display: table-cell; content: 'b' }",
        );
        let table = &tree.body().children[0];
        assert_eq!(
            tree.boxes_of(&table.children),
            pairs(&[
                ("row", 5),
                ("+cell 1x1", 5),
                ("+anon", 5),
                ("cell 3x1", 6),
                ("table", 6),
                ("row", 6),
                ("+cell 1x1", 6),
                ("+anon", 6),
                ("anon", 6),
            ])
        );
        // A generated column is a column of one, in its group.
        let tree = tree_of(
            "<table><colgroup span=3></colgroup><tr><td>x</td></tr></table>",
            "colgroup::before { display: table-column; content: '' }",
        );
        let table = &tree.body().children[0];
        assert_eq!(
            tree.boxes_of(&table.children[..1]),
            pairs(&[("colgroup 3", 5), ("+col 1", 5)])
        );
    }

    #[test]
    fn the_first_letter_goes_into_a_box_of_its_own() {
        // Nodes: p 4.
        let tree = tree_of(
            "<p>The end</p>",
            "p { color: #00f } p::first-letter { font-size: 30px }",
        );
        assert_eq!(
            tree.shape_of(&tree.body().children),
            pairs(&[
                ("block", 4),
                ("anon", 4),
                ("^inline", 4),
                ("^text T", 4),
                ("text he end", 4),
            ])
        );
        let letter = &tree.body().children[0].children[0].children[0];
        assert!(letter.anonymous);
        // Its style is the `::first-letter`'s, inheriting from the p's.
        assert_eq!(
            (letter.style.font_size_px, letter.style.color),
            (30.0, style::computed::Color::rgb8(0, 0, 255))
        );
        assert_eq!(letter.style.display, Display::Inline);
        assert_eq!(letter.children[0].style.font_size_px, 30.0);
    }

    #[test]
    fn the_first_letter_inherits_from_the_element_its_text_is_in() {
        // Nodes: p 4, span 5. The box is the span's, inside it.
        let tree = tree_of(
            "<p><span>The</span></p>",
            "span { color: #00f; font-size: 10px } p::first-letter { font-size: 2em }",
        );
        assert_eq!(
            tree.shape_of(&tree.body().children[0].children),
            pairs(&[
                ("anon", 4),
                ("inline", 5),
                ("^inline", 5),
                ("^text T", 5),
                ("text he", 5),
            ])
        );
        let letter = &tree.body().children[0].children[0].children[0].children[0];
        assert_eq!(
            (letter.style.font_size_px, letter.style.color),
            (20.0, style::computed::Color::rgb8(0, 0, 255))
        );
    }

    #[test]
    fn the_first_letter_takes_its_punctuation_and_leaves_white_space_before_it() {
        let tree = tree_of(
            "<p>  \u{201C}Hi\u{201D} said</p>",
            "p::first-letter { color: #f00 }",
        );
        assert_eq!(
            tree.texts(&tree.body().children),
            ["  ", "\u{201C}H", "i\u{201D} said"]
        );
        // A `::before`'s text counts: "N" of "Note: ".
        let tree = tree_of(
            "<p>x</p>",
            "p::before { content: 'Note: ' } p::first-letter { color: #f00 }",
        );
        assert_eq!(
            tree.shape_of(&tree.body().children[0].children),
            pairs(&[
                ("anon", 4),
                ("+inline", 4),
                ("^inline", 4),
                ("^text N", 4),
                ("+text ote: ", 4),
                ("text x", 4),
            ])
        );
    }

    #[test]
    fn the_first_formatted_line_may_be_a_block_childs() {
        // Nodes: div 4, p 5, span 6, p 7. An empty block and a float are
        // passed over; the p holding the letter is marked, the div not.
        let tree = tree_of(
            "<div><p></p><span class=f>F</span><p>Text</p></div>",
            ".f { float: left } div::first-letter { color: #f00 }",
        );
        assert_eq!(tree.texts(&tree.body().children), ["F", "T", "ext"]);
        let div = &tree.body().children[0];
        let p = &div.children[2];
        assert_eq!(tree.number(p.node), 7);
        assert!(p.styled_by_ancestor && !div.styled_by_ancestor);
        assert_eq!(
            p.children[0].children[0].generated,
            Some(PseudoElement::FirstLetter)
        );
        assert_eq!(tree.number(p.children[0].children[0].node), 7);
        // A floated `::before` is passed over too: it is out of flow, and
        // no block container of the div's own.
        let tree = tree_of(
            "<div>Cd</div>",
            "div::before { content: 'Ab'; float: left } div::first-letter { color: #f00 }",
        );
        assert_eq!(tree.texts(&tree.body().children), ["Ab", "C", "d"]);
    }

    #[test]
    fn out_of_flow_boxes_and_white_space_alone_are_passed_over() {
        // Among blocks, and on the line (in an inline element, where
        // they are met in a run): floats, absolutely positioned boxes,
        // and a text of white space alone kept in an inline.
        for html in [
            "<div><p class=a>A</p><p>Text</p></div>",
            "<div><span class=f>F</span><span class=a>A</span><b> </b>Text</div>",
            "<div><b><span class=f>F</span>Text</b></div>",
            "<div><b><span class=a>A</span>Text</b></div>",
        ] {
            let tree = tree_of(
                html,
                ".f { float: left } .a { position: absolute }
                 div::first-letter { color: #f00 }",
            );
            let letters: Vec<_> = tree
                .shape_of(&tree.body().children)
                .into_iter()
                .filter(|(k, _)| k.starts_with("^text"))
                .map(|(k, _)| k)
                .collect();
            assert_eq!(letters, ["^text T"], "{html}");
        }
    }

    #[test]
    fn there_is_no_first_letter_after_other_content_or_an_empty_first_line() {
        for (html, css) in [
            // An inline-block, an inline table, a table: content first.
            (
                "<div><span class=x>X</span>y</div>",
                ".x { display: inline-block }",
            ),
            (
                "<div><span class=x>X</span>y</div>",
                ".x { display: inline-table }",
            ),
            ("<div><table><tr><td>X</td></tr></table>y</div>", ""),
            // A kept line break first: the first line is empty.
            ("<div class=pre>\nText</div>", ".pre { white-space: pre }"),
            // The punctuation's element is not the letter's.
            ("<div>\u{201C}<b>T</b>he</div>", ""),
            // An inline element has none of its own, nor a table row,
            // whose text is in an anonymous cell.
            (
                "<p><span>The</span></p>",
                "span::first-letter { color: #f00 }",
            ),
            ("<div class=row>Text</div>", ".row { display: table-row }"),
        ] {
            let tree = tree_of(html, &format!("{css} div::first-letter {{ color: #f00 }}"));
            assert!(
                tree.shape_of(&tree.body().children)
                    .iter()
                    .all(|(k, _)| !k.starts_with('^')),
                "{html}"
            );
        }
        // But a cell, a caption, an inline-block and a float have theirs.
        for (html, tag) in [
            ("<table><tr><td>Cell</td></tr></table>", "td"),
            ("<table><caption>Cap</caption></table>", "caption"),
            ("<p><span class=x>Ib</span></p>", "span"),
            ("<p><span class=f>Fl</span></p>", "span"),
        ] {
            let tree = tree_of(
                html,
                &format!(
                    ".x {{ display: inline-block }} .f {{ float: left }}
                     {tag}::first-letter {{ color: #f00 }}"
                ),
            );
            assert_eq!(
                tree.shape_of(&tree.body().children)
                    .iter()
                    .filter(|(k, _)| k.starts_with("^text"))
                    .count(),
                1,
                "{tag}"
            );
        }
    }

    #[test]
    fn a_floated_first_letter_is_a_float_holding_its_line() {
        let tree = tree_of(
            "<p>The</p>",
            "p::first-letter { float: left; font-size: 3em }",
        );
        assert_eq!(
            tree.shape_of(&tree.body().children[0].children),
            pairs(&[
                ("anon", 4),
                ("^float", 4),
                ("^anon", 4),
                ("^text T", 4),
                ("text he", 4),
            ])
        );
    }

    #[test]
    fn several_first_letters_nest_outermost_first() {
        // Nodes: div 4, p 5. The p's inherits from the div's.
        for (outer, shape) in [
            (
                "color: #f00",
                vec![("^inline", 5), ("^inline", 5), ("^text T", 5)],
            ),
            (
                "color: #f00; float: left",
                vec![("^float", 5), ("^anon", 5), ("^inline", 5), ("^text T", 5)],
            ),
        ] {
            let tree = tree_of(
                "<div><p>The</p></div>",
                &format!("div::first-letter {{ {outer} }} p::first-letter {{ font-size: 20px }}"),
            );
            let p = &tree.body().children[0].children[0];
            let line = &p.children[0];
            let mut expected = shape.clone();
            expected.push(("text he", 5));
            assert_eq!(tree.shape_of(&line.children), pairs(&expected), "{outer}");
            let mut inner = &line.children[0];
            while inner.kind != BoxKind::Inline || inner.style.font_size_px != 20.0 {
                inner = &inner.children[0];
            }
            assert_eq!(
                inner.style.color,
                style::computed::Color::rgb8(255, 0, 0),
                "{outer}"
            );
        }
    }

    /// Every box under `boxes` with a style on the first line, as
    /// `(kind, node number, color, font size)` there, in tree order.
    fn first_line_styles(tree: &Tree, boxes: &[LayoutBox]) -> Vec<(String, usize, [u8; 3], f32)> {
        fn walk(tree: &Tree, b: &LayoutBox, out: &mut Vec<(String, usize, [u8; 3], f32)>) {
            if let Some(line) = &b.first_line {
                let kind = match &b.kind {
                    BoxKind::Text(t) => format!("text {t}"),
                    BoxKind::AnonymousBlock => "anon".to_string(),
                    BoxKind::Inline => "inline".to_string(),
                    other => format!("{other:?}"),
                };
                let c = line.style.color;
                let rgb = [c.r, c.g, c.b].map(|v| (v * 255.0).round() as u8);
                out.push((kind, tree.number(b.node), rgb, line.style.font_size_px));
            }
            for c in &b.children {
                walk(tree, c, out);
            }
        }
        let mut out = Vec::new();
        for b in boxes {
            walk(tree, b, &mut out);
        }
        out
    }

    #[test]
    fn the_first_lines_run_holds_the_first_line_and_its_boxes_their_styles_there() {
        // Nodes: p 4, b 5.
        let tree = tree_of(
            "<p>Some <b>bold</b> text</p>",
            "p { color: #00f; font-size: 10px; background-color: #0f0 }
             b { font-size: 2em; background-color: inherit }
             p::first-line { color: #f00; font-size: 20px; margin-left: 9px;
                 background-color: #ff0 }",
        );
        let red = [255, 0, 0];
        // The run, the text in the p and the b's, and the b, whose 2em
        // are of the line's 20px.
        assert_eq!(
            first_line_styles(&tree, &tree.body().children),
            [
                ("anon".to_string(), 4, red, 20.0),
                ("text Some ".to_string(), 4, red, 20.0),
                ("inline".to_string(), 5, red, 40.0),
                ("text bold".to_string(), 5, red, 40.0),
                ("text  text".to_string(), 4, red, 20.0),
            ]
        );
        let p = &tree.body().children[0];
        let line = p.children[0].first_line.as_ref().unwrap();
        // The `::first-line` is the p's; margins do not apply to it.
        assert_eq!(line.chain.len(), 1);
        let pseudo = &line.chain[0];
        assert_eq!(
            (tree.number(pseudo.node), pseudo.generated, &pseudo.kind),
            (4, Some(PseudoElement::FirstLine), &BoxKind::Inline)
        );
        assert_eq!(line.style.margin, ComputedStyle::default().margin);
        // The boxes' own styles are as they were.
        let b = &p.children[0].children[1];
        assert_eq!(
            (b.style.color, b.style.font_size_px),
            (style::computed::Color::rgb8(0, 0, 255), 20.0)
        );
        // What the b inherits but is not inherited, it inherits from the
        // p, not the `::first-line` (Pseudo-Elements 4 §2.1.3).
        let line = b.first_line.as_ref().unwrap();
        assert_eq!(
            line.style.background_color,
            style::computed::Color::rgb8(0, 255, 0)
        );
    }

    #[test]
    fn several_first_lines_nest_and_the_outermost_inherits_from_the_block_whose_line_it_is() {
        // Nodes: div 4, p 5, span 6, p 7. The empty p and the float are
        // passed over; the div's `::first-line` stands inside the p whose
        // line it is, the p's inside it.
        let tree = tree_of(
            "<div><p></p><span class=f>F</span><p class=x>Text</p></div>",
            ".f { float: left } .x { font-size: 10px }
             div::first-line { font-size: 2em } p::first-line { color: #f00 }",
        );
        let div = &tree.body().children[0];
        let p = &div.children[2];
        assert_eq!(tree.number(p.node), 7);
        assert!(p.styled_by_ancestor && !div.styled_by_ancestor);
        let line = p.children[0].first_line.as_ref().expect("the p's run");
        let chain: Vec<_> = line.chain.iter().map(|b| tree.number(b.node)).collect();
        assert_eq!(chain, [4, 7]);
        // 2em of the p's 10px; the p's inherits it, and is red.
        assert_eq!(line.chain[0].style.font_size_px, 20.0);
        assert_eq!(
            (line.style.font_size_px, line.style.color),
            (20.0, style::computed::Color::rgb8(255, 0, 0))
        );
        // Only that run is styled.
        assert_eq!(
            first_line_styles(&tree, &tree.body().children).len(),
            2,
            "the run and its text"
        );
    }

    #[test]
    fn a_run_has_a_line_when_it_holds_content_or_an_edge() {
        for (html, first) in [
            // An empty inline element with an edge makes a line, whether
            // of margin, border or padding, a percentage among them; one
            // without makes none, nor does an `auto` margin, nor white
            // space alone.
            ("<div><p><span class=e></span></p><p>Text</p></div>", 0),
            ("<div><p><span class=m></span></p><p>Text</p></div>", 0),
            ("<div><p><span class=b></span></p><p>Text</p></div>", 0),
            ("<div><p><span class=pc></span></p><p>Text</p></div>", 0),
            ("<div><p><span></span></p><p>Text</p></div>", 1),
            ("<div><p><span class=auto></span></p><p>Text</p></div>", 1),
            ("<div><p><span> </span></p><p>Text</p></div>", 1),
            // Text in an inline element is content, an inline-block is,
            // and a line break kept is a line, though empty — between
            // two elements, as white space alone at a run's start or end
            // makes no box, whatever `white-space` says.
            ("<div><p><b>X</b></p><p>Text</p></div>", 0),
            ("<div><p><span class=ib></span></p><p>Text</p></div>", 0),
            (
                "<div><p class=pre><span></span>\n<span></span></p><p>Text</p></div>",
                0,
            ),
            (
                "<div><p><span></span>\n<span></span></p><p>Text</p></div>",
                1,
            ),
            // An absolutely positioned block is out of flow.
            ("<div><p class=abs>A</p><p>Text</p></div>", 1),
        ] {
            let tree = tree_of(
                html,
                ".e { padding-left: 2px } .m { margin-right: 2px }
                 .b { border-left: 1px solid } .pc { padding-right: 5% }
                 .auto { margin: 0 auto } .ib { display: inline-block }
                 .pre { white-space: pre } .abs { position: absolute }
                 div::first-line { color: #f00 }",
            );
            let div = &tree.body().children[0];
            let runs: Vec<bool> = div
                .children
                .iter()
                .map(|p| {
                    p.children
                        .first()
                        .is_some_and(|run| run.first_line.is_some())
                })
                .collect();
            let mut expected = vec![false, false];
            expected[first] = true;
            assert_eq!(runs, expected, "{html}");
        }
    }

    #[test]
    fn there_is_no_first_line_in_a_table_after_one_or_on_an_inline() {
        for (html, css) in [
            ("<div><table><tr><td>X</td></tr></table>y</div>", ""),
            (
                "<p><span>The</span></p>",
                "span::first-line { color: #f00 }",
            ),
            ("<div class=row>Text</div>", ".row { display: table-row }"),
        ] {
            let tree = tree_of(html, &format!("{css} div::first-line {{ color: #f00 }}"));
            assert_eq!(
                first_line_styles(&tree, &tree.body().children),
                [],
                "{html}"
            );
        }
        // But a cell has its own.
        let tree = tree_of(
            "<table><tr><td>Cell</td></tr></table>",
            "td::first-line { color: #f00 }",
        );
        assert_eq!(first_line_styles(&tree, &tree.body().children).len(), 2);
    }

    #[test]
    fn what_a_first_line_inherits_nothing_from_costs_no_cascade() {
        use std::cell::Cell;

        /// A [`Recascade`] counting the elements it is asked for.
        struct Counting<'a>(&'a dyn Recascade, Cell<usize>);
        impl Recascade for Counting<'_> {
            fn element(&self, element: NodeId, parent: &ComputedStyle) -> ComputedStyle {
                self.1.set(self.1.get() + 1);
                self.0.element(element, parent)
            }
            fn pseudo_element(
                &self,
                element: NodeId,
                pseudo: PseudoElement,
                parent: &ComputedStyle,
            ) -> Option<ComputedStyle> {
                self.0.pseudo_element(element, pseudo, parent)
            }
        }
        for (css, cascades, styled) in [
            // A background is not inherited: only the run is styled.
            ("p::first-line { background-color: #0f0 }", 0, 1),
            // A colour is: the b is cascaded again, and it and the texts
            // styled.
            ("p::first-line { color: #f00 }", 1, 5),
            // Unless the b sets its own: cascaded again, it is as it was.
            ("p::first-line { color: #f00 } b { color: #00f }", 1, 3),
        ] {
            let node = html::parse_document("<p>a <b>b</b> c</p>").unwrap();
            let document = Document::from_node(&node);
            let sheets = style::Stylesheets::new(vec![css::parse_stylesheet(css).unwrap()]);
            let context = style::CascadeContext::default();
            let styles = sheets.cascade(&document, &context);
            let recascader = sheets.recascader(&document, &styles, &context);
            let counting = Counting(&recascader, Cell::new(0));
            let boxes = build_box_tree(&document, &styles, &counting);
            assert_eq!(counting.1.get(), cascades, "{css}");
            let tree = Tree { document, boxes };
            assert_eq!(first_line_styles(&tree, &tree.boxes).len(), styled, "{css}");
        }
    }

    #[test]
    fn generated_inline_boxes_and_the_first_letter_are_styled_on_the_first_line() {
        // Nodes: p 4. A `::before` on the line is cascaded again from it,
        // and the first letter inherits from it: 2em of its 20px.
        let tree = tree_of(
            "<p>The</p>",
            "p { font-size: 10px } p::first-line { font-size: 20px; color: #f00 }
             p::before { content: 'x' } p::first-letter { font-size: 2em }",
        );
        let run = &tree.body().children[0].children[0];
        assert_eq!(
            tree.shape_of(&run.children),
            pairs(&[
                ("+inline", 4),
                ("^inline", 4),
                ("^text x", 4),
                ("text The", 4),
            ])
        );
        let before = &run.children[0];
        assert_eq!(
            before
                .first_line
                .as_ref()
                .map(|line| line.style.font_size_px),
            Some(20.0)
        );
        let letter = &before.children[0];
        assert_eq!(letter.style.font_size_px, 40.0);
        assert_eq!(letter.style.color, style::computed::Color::rgb8(255, 0, 0));
        // The rest of the text keeps its style on the line.
        assert_eq!(
            run.children[1]
                .first_line
                .as_ref()
                .map(|line| line.style.font_size_px),
            Some(20.0)
        );
        // So does the rest of a text the letter is split from.
        let tree = tree_of(
            "<p>The</p>",
            "p::first-line { font-size: 20px } p::first-letter { font-size: 2em }",
        );
        let run = &tree.body().children[0].children[0];
        assert_eq!(tree.texts(&run.children), ["T", "he"]);
        assert_eq!(
            run.children[1]
                .first_line
                .as_ref()
                .map(|line| line.style.font_size_px),
            Some(20.0)
        );
    }

    #[test]
    fn an_atomic_inline_and_an_out_of_flow_box_keep_their_styles_on_the_first_line() {
        let tree = tree_of(
            "<p><span class=ib>x</span><span class=f>y</span> z</p>",
            ".ib { display: inline-block } .f { float: left }
             p::first-line { color: #f00 }",
        );
        let run = &tree.body().children[0].children[0];
        assert!(run.first_line.is_some());
        for b in &run.children[..2] {
            assert!(
                first_line_styles(&tree, std::slice::from_ref(b)).is_empty(),
                "{:?}",
                b.kind
            );
        }
    }

    #[test]
    fn a_flex_containers_children_are_its_items() {
        // M17, css-flexbox-1 §4. Nodes: div 4, span 5, p 6, i 7, b 8, em
        // 9. Each child element is an item, its display blockified; each
        // run of text an anonymous item, a block around the run's
        // anonymous block, unless the run is nothing but white space; an
        // absolutely positioned child is no item; a floated one is an
        // item as if it did not float; a `::before` is an item too.
        let tree = tree_of(
            "<div> a <span>b</span> <p>c</p>d<i>e</i> <b class=abs></b> <em class=f>f</em> </div>",
            "div { display: flex } .abs { position: absolute } .f { float: left }
             div::before { content: 'g' }",
        );
        assert_eq!(
            tree.shape_of(&tree.body().children),
            pairs(&[
                ("flex", 4),
                ("+block", 4),
                ("+anon", 4),
                ("+text g", 4),
                ("block", 4),
                ("anon", 4),
                ("text  a ", 4),
                ("block", 5),
                ("anon", 5),
                ("text b", 5),
                ("block", 6),
                ("anon", 6),
                ("text c", 6),
                ("block", 4),
                ("anon", 4),
                ("text d", 4),
                ("block", 7),
                ("anon", 7),
                ("text e", 7),
                ("absolute", 8),
                ("block", 9),
                ("anon", 9),
                ("text f", 9),
            ])
        );
        let flex = &tree.body().children[0];
        assert!(flex.is_flex_container() && flex.is_block_level());
        // The anonymous item is anonymous, and inherits from the
        // container as an anonymous block does.
        assert!(flex.children[1].anonymous);
        assert_eq!(flex.children[1].style.display, Display::Block);
        // A floated or absolutely positioned flex container is a flex
        // container too, and its children are items.
        for css in [".x { float: left }", ".x { position: absolute }"] {
            let tree = tree_of(
                "<p><span class=x><i>y</i></span></p>",
                &format!(".x {{ display: flex }} {css}"),
            );
            let x = &tree.body().children[0].children[0];
            assert!(x.is_flex_container(), "{css}");
            assert!(!x.is_block_level(), "{css}");
            assert_eq!(x.children[0].kind, BoxKind::Block, "{css}");
        }
    }

    #[test]
    fn an_element_with_no_box_does_not_end_a_flex_text_run() {
        // As in the reference browsers (css-flexbox WPT
        // anonymous-flex-item-002): the text either side of a `display:
        // none` element is one item, one line of text; an absolutely
        // positioned element ends the run. Nodes: div 4, span 5, b 6.
        let tree = tree_of(
            "<div>two <span class=none></span>words <b class=abs></b>more</div>",
            "div { display: flex } .none { display: none } .abs { position: absolute }",
        );
        assert_eq!(
            tree.shape_of(&tree.body().children),
            pairs(&[
                ("flex", 4),
                ("block", 4),
                ("anon", 4),
                ("text two ", 4),
                ("text words ", 4),
                ("absolute", 6),
                ("block", 4),
                ("anon", 4),
                ("text more", 4),
            ])
        );
    }

    #[test]
    fn an_inline_flex_container_is_an_atomic_item_on_its_line() {
        // Nodes: p 4, span 5, i 6.
        let tree = tree_of(
            "<p>x<span>y<i>z</i></span>w</p>",
            "span { display: inline-flex }",
        );
        assert_eq!(
            tree.shape_of(&tree.body().children),
            pairs(&[
                ("block", 4),
                ("anon", 4),
                ("text x", 4),
                ("inline-flex", 5),
                ("block", 5),
                ("anon", 5),
                ("text y", 5),
                ("block", 6),
                ("anon", 6),
                ("text z", 6),
                ("text w", 4),
            ])
        );
        let span = &tree.body().children[0].children[0].children[1];
        assert!(span.is_flex_container() && !span.is_block_level());
    }

    #[test]
    fn a_grid_containers_children_are_items_as_a_flex_containers_are() {
        // M18, css-grid-2 §6, whose rules are css-flexbox-1 §4's word
        // for word: the same page makes the same boxes under a grid.
        let html =
            "<div> a <span>b</span> <p>c</p>d<i>e</i> <b class=abs></b> <em class=f>f</em> </div>";
        let css = ".abs { position: absolute } .f { float: left } div::before { content: 'g' }";
        let flex = tree_of(html, &format!("div {{ display: flex }} {css}"));
        let grid = tree_of(html, &format!("div {{ display: grid }} {css}"));
        let mut expected = flex.shape_of(&flex.body().children);
        assert_eq!(expected[0], ("flex".to_string(), 4));
        expected[0].0 = "grid".to_string();
        assert_eq!(grid.shape_of(&grid.body().children), expected);
        let container = &grid.body().children[0];
        assert!(container.is_grid_container() && container.is_block_level());
        // An item that is a grid container itself is one.
        let nested = tree_of("<div><p>x</p></div>", "div, p { display: grid }");
        let item = &nested.body().children[0].children[0];
        assert_eq!(item.kind, BoxKind::Grid);
        assert!(!container.is_flex_container() && container.has_items());
        assert!(container.children[1].anonymous);
        // A floated or absolutely positioned grid container is a grid
        // container too, and its children are items.
        for extra in [".x { float: left }", ".x { position: absolute }"] {
            let tree = tree_of(
                "<p><span class=x><i>y</i></span></p>",
                &format!(".x {{ display: inline-grid }} {extra}"),
            );
            let x = &tree.body().children[0].children[0];
            assert!(x.is_grid_container() && !x.is_block_level(), "{extra}");
            assert_eq!(x.style.display, Display::Grid, "{extra}");
            assert_eq!(x.children[0].kind, BoxKind::Block, "{extra}");
        }
    }

    #[test]
    fn an_inline_grid_container_is_an_atomic_item_on_its_line() {
        // Nodes: p 4, span 5, i 6.
        let tree = tree_of(
            "<p>x<span>y<i>z</i></span>w</p>",
            "span { display: inline-grid }",
        );
        assert_eq!(
            tree.shape_of(&tree.body().children),
            pairs(&[
                ("block", 4),
                ("anon", 4),
                ("text x", 4),
                ("inline-grid", 5),
                ("block", 5),
                ("anon", 5),
                ("text y", 5),
                ("block", 6),
                ("anon", 6),
                ("text z", 6),
                ("text w", 4),
            ])
        );
        let span = &tree.body().children[0].children[0].children[1];
        assert!(span.is_grid_container() && !span.is_block_level());
    }

    #[test]
    fn a_grid_containers_children_are_numbered_in_order_modified_document_order() {
        // css-grid-2 §6.3: `order` decides the paint order, as in a flex
        // container; an absolutely positioned child counts as `order: 0`.
        // Nodes: div 4, i 5, b 6, em 7, u 8.
        let tree = tree_of(
            "<div><i></i><b></b><em></em><u></u></div>",
            "div { display: grid } i { order: 2 } b { order: -1 } u { position: absolute; order: -5 }",
        );
        let grid = &tree.body().children[0];
        let order: Vec<(usize, u32)> = grid
            .children
            .iter()
            .map(|b| (b.node.index(), b.tree_order))
            .collect();
        let rank = |node: usize| order.iter().find(|(n, _)| *n == node).expect("a child").1;
        assert!(
            rank(6) < rank(7) && rank(7) < rank(8) && rank(8) < rank(5),
            "{order:?}"
        );
        assert!(grid.tree_order < rank(6));
    }

    #[test]
    fn a_flex_container_has_no_first_line_or_first_letter_but_its_items_do() {
        // css-flexbox-1 §3: neither applies to a flex container, nor
        // reaches through one from a block around it; an item, a block
        // container, has its own. Nodes: div 4, p 5.
        for (html, css) in [
            ("<div>Text</div>", "div { display: flex }"),
            ("<div><p>Text</p></div>", "div { display: flex }"),
            (
                "<div><p>Text</p></div>",
                "p { display: flex } div { display: block }",
            ),
            ("<div>Text</div>", "div { display: inline-flex }"),
            // Nor to a grid container (css-grid-2 §5.1, M18).
            ("<div>Text</div>", "div { display: grid }"),
            (
                "<div><p>Text</p></div>",
                "p { display: grid } div { display: block }",
            ),
            ("<div>Text</div>", "div { display: inline-grid }"),
        ] {
            let tree = tree_of(
                html,
                &format!(
                    "{css} div::first-letter {{ color: #f00 }} div::first-line {{ color: #f00 }}"
                ),
            );
            let shape = tree.shape_of(&tree.body().children);
            assert!(
                shape.iter().all(|(k, _)| !k.starts_with('^')),
                "{html} {css}"
            );
            assert!(
                first_line_styles(&tree, &tree.body().children).is_empty(),
                "{html} {css}"
            );
        }
        let tree = tree_of(
            "<div><p>Text</p></div>",
            "div { display: flex } p::first-letter { color: #f00 } p::first-line { color: #f00 }",
        );
        let shape = tree.shape_of(&tree.body().children);
        assert_eq!(shape.iter().filter(|(k, _)| k == "^text T").count(), 1);
        assert!(!first_line_styles(&tree, &tree.body().children).is_empty());
    }
}
