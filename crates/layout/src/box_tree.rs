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
//! and all. The quote marks of `open-quote` and `close-quote` are
//! chosen as the build reaches them, which is in document order.

use std::borrow::Cow;
use std::cell::Cell;

use dom::{Document, NodeId};
use style::computed::{CaptionSide, Content, ContentItem, Display, Float};
use style::selector_matching::PseudoElement;
use style::{ComputedStyle, Styles};

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
    /// The pseudo-element that generated the box (M16): `::before` or
    /// `::after` for the box one generates and for those inside it, its
    /// anonymous boxes and its text; `None` for the element's own boxes
    /// and the anonymous boxes in them. The fragments made from the box
    /// are marked as it is, so that what looks for an element's own
    /// boxes passes over them, and what tells two boxes of one element
    /// apart can.
    pub generated: Option<PseudoElement>,
    /// The quote depth (CSS 2.2 §12.3.2) where the box's content starts
    /// in document order: the `open-quote`s and `no-open-quote`s before
    /// it less the `close-quote`s and `no-close-quote`s, never below
    /// zero (M16). The quote marks in the box, and so its layout, depend
    /// on it as on the box's own subtree, and the layout cache keeps an
    /// element's results under it.
    pub quote_depth: u32,
}

/// A box's kind, per `display` (block, inline, inline-block and the
/// table values — flex, grid and so on generate real boxes only from
/// their own milestones on).
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
    /// block; the viewport for `fixed` — once that is known, from the
    /// static position it is met at. As for a float, a table that is
    /// absolutely positioned is this kind with a table's children.
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

    /// Whether this box is block-level: a child of a block container,
    /// stacked vertically by `block::layout_children`. A table is,
    /// unless it is an inline table; a float is neither block-level nor
    /// inline-level, it is out of flow; a table's internal boxes are
    /// children of table boxes, not of block containers.
    pub fn is_block_level(&self) -> bool {
        match self.kind {
            BoxKind::Block | BoxKind::AnonymousBlock => true,
            BoxKind::Table => self.style.display == Display::Table,
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
/// anything under it, as if it were `display: none`.
pub fn build_box_tree(document: &Document, styles: &Styles) -> Vec<LayoutBox> {
    let styled = Styled {
        document,
        styles,
        quote_depth: Cell::new(0),
    };
    build_block_container(&styled, Item::Node(document.root()))
}

/// What the builder walks (M16): a node of the document; or the box an
/// element's `::before` or `::after` generates, which stands where the
/// element's first or last child would; or the text in one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Node(NodeId),
    Generated(NodeId, Side),
    GeneratedText(NodeId, Side),
}

/// Which of an element's two generated boxes an [`Item`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Before,
    After,
}

impl Side {
    fn pseudo_element(self) -> PseudoElement {
        match self {
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
            Item::Node(node) | Item::Generated(node, _) | Item::GeneratedText(node, _) => node,
        }
    }

    /// The pseudo-element whose box the item is, or is the text in.
    fn generated(self) -> Option<PseudoElement> {
        match self {
            Item::Node(_) => None,
            Item::Generated(_, side) | Item::GeneratedText(_, side) => Some(side.pseudo_element()),
        }
    }
}

/// What the builder reads: the document, and its styles by node; and
/// the quote depth the build has reached.
struct Styled<'a> {
    document: &'a Document,
    styles: &'a Styles,
    /// The `open-quote`s and `no-open-quote`s in the generated text
    /// built so far, in document order, less the `close-quote`s and
    /// `no-close-quote`s, never below zero (CSS 2.2 §12.3.2).
    quote_depth: Cell<u32>,
}

impl<'a> Styled<'a> {
    /// The computed style of `item`: an element's, the document node's,
    /// or a generated box's pseudo-element's; `None` for a text, and for
    /// an element the cascade did not style.
    fn style(&self, item: Item) -> Option<&'a ComputedStyle> {
        match item {
            Item::Node(node) => self.styles.get(node),
            Item::Generated(element, side) => self.styles.pseudo(element, side.pseudo_element()),
            Item::GeneratedText(..) => None,
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
            Item::Generated(..) => true,
            Item::GeneratedText(..) => false,
        }
    }

    /// The items in `item`, in document order: a node's children, an
    /// element's between the boxes its `::before` and `::after`
    /// generate, when they do; a generated box's text; nothing in a
    /// text.
    fn children(&self, item: Item) -> Vec<Item> {
        match item {
            Item::Node(node) => {
                let generated = |side| {
                    self.generates(node, side)
                        .then_some(Item::Generated(node, side))
                };
                generated(Side::Before)
                    .into_iter()
                    .chain(self.document.children(node).map(Item::Node))
                    .chain(generated(Side::After))
                    .collect()
            }
            Item::Generated(element, side) => vec![Item::GeneratedText(element, side)],
            Item::GeneratedText(..) => Vec::new(),
        }
    }

    /// Whether the element `node` has a `side` item (Pseudo-Elements 4
    /// §4): the pseudo-element is styled, its `content` is a list (the
    /// cascade computes `normal` to `none` there), and the element is
    /// not replaced. One whose `display` is `none` makes no box, as an
    /// element's does not.
    fn generates(&self, node: NodeId, side: Side) -> bool {
        if self.document.tag(node).is_none_or(is_replaced) {
            return false;
        }
        self.styles
            .pseudo(node, side.pseudo_element())
            .is_some_and(|style| matches!(style.content, Content::Items(_)))
    }

    /// The text of `item` when it is a text node or a generated box's
    /// text, and `None` otherwise. A generated text's quote marks are
    /// chosen at the depth the build has reached, and the depth then
    /// moves past them when `advance` says so: once per text, when the
    /// build reaches it, whether a box is made for it or not.
    fn text(&self, item: Item, advance: bool) -> Option<Cow<'a, str>> {
        match item {
            Item::Node(node) => self.document.text(node).map(Cow::Borrowed),
            Item::Generated(..) => None,
            Item::GeneratedText(element, side) => {
                let style = self.styles.pseudo(element, side.pseudo_element())?;
                let Content::Items(items) = &style.content else {
                    return None;
                };
                let mut depth = self.quote_depth.get();
                let mut text = String::new();
                for part in items.iter() {
                    match part {
                        ContentItem::Text(s) => text.push_str(s),
                        ContentItem::OpenQuote => {
                            if let Some((open, _)) = style.quotes.marks(depth as usize) {
                                text.push_str(open);
                            }
                            depth = depth.saturating_add(1);
                        }
                        // One that would take the depth below zero gives
                        // no mark, and the depth stays (§12.3.2).
                        ContentItem::CloseQuote => {
                            if depth > 0 {
                                depth -= 1;
                                if let Some((_, close)) = style.quotes.marks(depth as usize) {
                                    text.push_str(close);
                                }
                            }
                        }
                        ContentItem::NoOpenQuote => depth = depth.saturating_add(1),
                        ContentItem::NoCloseQuote => depth = depth.saturating_sub(1),
                    }
                }
                if advance {
                    self.quote_depth.set(depth);
                }
                Some(Cow::Owned(text))
            }
        }
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
        let quote_depth = self.quote_depth.get();
        LayoutBox {
            style: style.clone(),
            kind,
            children: build(),
            node: item.node(),
            anonymous: item.generated().is_some(),
            generated: item.generated(),
            quote_depth,
        }
    }

    /// An anonymous box of `kind` in `item`, with the style
    /// [`anonymous_style`] gives it, holding `children`, whose content
    /// starts at `quote_depth`.
    fn anonymous_box(
        &self,
        item: Item,
        display: Display,
        kind: BoxKind,
        children: Vec<LayoutBox>,
        quote_depth: u32,
    ) -> LayoutBox {
        LayoutBox {
            style: anonymous_style(self, item, display),
            kind,
            children,
            node: item.node(),
            anonymous: true,
            generated: item.generated(),
            quote_depth,
        }
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
            let quote_depth = styled.quote_depth.get();
            // The document node is no one's child; a stale id is no node
            // at all.
            let Some(text) = styled.text(child, true) else {
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
            self.add_inline_level(LayoutBox {
                style: styled.style_or_initial(parent),
                kind: BoxKind::Text(text.into_owned()),
                children: Vec::new(),
                node: parent.node(),
                anonymous: true,
                generated: parent.generated(),
                quote_depth,
            });
        }
        self.flush_misplaced(parent, &mut misplaced);
    }

    /// Adds the boxes of `child`, an element or a generated box among
    /// the children of `parent`, collecting it into `misplaced` when it
    /// is a table part out of place.
    fn add_element(&mut self, parent: Item, child: Item, misplaced: &mut Vec<Misplaced>) {
        let styled = self.styled;
        // No style, which the cascade gives every element in the tree,
        // or `display: none`: the element AND its subtree generate no
        // box at all.
        let Some(style) = styled.style(child) else {
            return;
        };
        if style.display == Display::None {
            return;
        }
        if needs_a_table(style.display) {
            self.collect_misplaced(misplaced, child, style);
            return;
        }
        self.flush_misplaced(parent, misplaced);
        match style.display {
            // A float or an absolutely positioned box computes to a
            // block-level `display` (CSS2 §9.7, the cascade's rule) and
            // is a block container inside, or a table when its display
            // is `table`; outside it is out of flow, and goes where it
            // is met: into the run or inline open around it, else among
            // the container's blocks.
            Display::Block | Display::Table
                if style.float != Float::None || style.position.is_out_of_flow() =>
            {
                let kind = if style.position.is_out_of_flow() {
                    BoxKind::Absolute
                } else {
                    BoxKind::Float
                };
                let out_of_flow = styled.own_box(child, style, kind, || {
                    if style.display == Display::Table {
                        build_table(styled, child)
                    } else {
                        build_block_container(styled, child)
                    }
                });
                if self.open.is_empty() && self.run.is_empty() {
                    self.out.push(out_of_flow);
                } else {
                    self.add_inline_level(out_of_flow);
                }
            }
            Display::Block => {
                let block = styled.own_box(child, style, BoxKind::Block, || {
                    build_block_container(styled, child)
                });
                self.add_block(block);
            }
            Display::Table => {
                let table =
                    styled.own_box(child, style, BoxKind::Table, || build_table(styled, child));
                self.add_block(table);
            }
            Display::Inline => {
                let inline = styled.own_box(child, style, BoxKind::Inline, Vec::new);
                self.open.push(inline);
                self.add_children(child, &styled.children(child));
                let inline = self.open.pop().expect("the inline just opened");
                self.add_inline_level(inline);
            }
            Display::InlineBlock => {
                let block = styled.own_box(child, style, BoxKind::InlineBlock, || {
                    build_block_container(styled, child)
                });
                self.add_inline_level(block);
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
        let quote_depth = parts[0].quote_depth;
        let table = styled.anonymous_box(
            parent,
            display,
            BoxKind::Table,
            order_table_children(parts),
            quote_depth,
        );
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
        let quote_depth = self.styled.quote_depth.get();
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
                quote_depth,
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
        let Some(first) = self.run.first() else {
            return;
        };
        let quote_depth = first.quote_depth;
        let children = std::mem::take(&mut self.run);
        self.out.push(LayoutBox {
            style: self.styled.inherited_from(self.item),
            kind: BoxKind::AnonymousBlock,
            children,
            node: self.item.node(),
            anonymous: true,
            generated: self.item.generated(),
            quote_depth,
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
    let quote_depth = cells[0].quote_depth;
    styled.anonymous_box(item, Display::TableRow, BoxKind::Row, cells, quote_depth)
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
/// space between parts dropped (§17.2.1's steps 1 and 2). Each is built
/// in document order, where the quote depth is the one it starts at.
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
                .text(child, false)
                .is_some_and(|text| text.chars().all(crate::inline::is_css_space))
        });
        if only_white_space {
            // No cell; a generated text's quotes still count.
            for &child in &content {
                let _ = styled.text(child, true);
            }
            return;
        }
        let quote_depth = styled.quote_depth.get();
        let children = build_block_container_of(styled, item, &content);
        cells.push(styled.anonymous_box(
            item,
            Display::TableCell,
            BoxKind::Cell {
                colspan: 1,
                rowspan: 1,
            },
            children,
            quote_depth,
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
            };
            // A generated box, or one in it, is marked with a `+`.
            let mark = if b.generated.is_some() { "+" } else { "" };
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
                    k.strip_prefix('+')
                        .unwrap_or(&k)
                        .strip_prefix("text ")
                        .map(str::to_string)
                })
                .collect()
        }

        /// [`Tree::shape_of`] without the text boxes.
        fn boxes_of(&self, boxes: &[LayoutBox]) -> Vec<(String, usize)> {
            self.shape_of(boxes)
                .into_iter()
                .filter(|(k, _)| !k.trim_start_matches('+').starts_with("text"))
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
    fn a_box_records_the_quote_depth_it_starts_at() {
        // Nodes: div 4, span 5, p 6, b 7.
        let tree = tree_of(
            "<div><span class=o></span><p><b>x</b></p></div>",
            ".o::before { content: open-quote }",
        );
        let div = &tree.body().children[0];
        assert_eq!(div.quote_depth, 0);
        let p = &div.children[1];
        assert_eq!(
            (
                tree.number(p.node),
                p.quote_depth,
                p.children[0].quote_depth
            ),
            (6, 1, 1)
        );
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
}
