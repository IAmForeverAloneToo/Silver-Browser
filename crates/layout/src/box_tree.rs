//! The box tree: a `style::StyledDom` in, a tree of [`LayoutBox`]es out
//! (roadmap M6.1) — one box per element that generates one (`display:
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

use style::computed::{CaptionSide, Display, Float};
use style::selector_matching::{FlatChild, FlatDom};
use style::{ComputedStyle, StyledDom};

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
    /// The element this box belongs to, as its `FlatDom` index (M9.4):
    /// the generating element for an element's box, the containing
    /// element for an anonymous text, block or table box. Hit testing
    /// answers with this, and the fragments the box produces carry it.
    pub node: usize,
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
    /// takes whatever's around it. A run of nothing but collapsible
    /// white space is kept where it separates inline-level content
    /// (M10.4: it is the gap between two words, or two inline-blocks)
    /// and dropped at the start and end of a block container's inline
    /// run, where a line would discard it anyway (css-text-3 §4.1.3).
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

/// Builds the box tree for all of `styled`'s top-level content (the
/// document's direct children — there's no single root box for them,
/// they're laid out as if they were children of the viewport itself,
/// see `block::layout`). The document node is the block container of
/// the top-level content: an inline run at the top goes into an
/// anonymous block inheriting from the document's (initial-valued)
/// style.
pub fn build_box_tree(styled: &StyledDom) -> Vec<LayoutBox> {
    build_block_container(styled, 0)
}

/// The block-level children of the block container `idx` generates
/// (CSS2 §9.2.1.1): its element children in document order, each
/// inline-level run wrapped in an anonymous block, each block found
/// inside an inline child hoisted out to this level.
fn build_block_container(styled: &StyledDom, idx: usize) -> Vec<LayoutBox> {
    build_block_container_of(styled, idx, &styled.flat.nodes[idx].children)
}

/// [`build_block_container`] for `children`, a slice of the children of
/// the element `idx`: what an anonymous cell holds (§17.2.1), built as
/// the block container it is, with `idx` as the element its anonymous
/// boxes name and inherit from.
fn build_block_container_of(
    styled: &StyledDom,
    idx: usize,
    children: &[FlatChild],
) -> Vec<LayoutBox> {
    let mut container = BlockContainer {
        styled,
        node: idx,
        out: Vec::new(),
        run: Vec::new(),
        open: Vec::new(),
    };
    container.add_children(idx, children);
    container.finish()
}

/// The style of an anonymous box: inherited from the element `node`
/// it belongs to, every other property initial (so no margin, padding,
/// border, background or size of its own), and the `display` it is
/// generated with.
fn anonymous_style(styled: &StyledDom, node: usize, display: Display) -> ComputedStyle {
    let mut style = ComputedStyle::inherited_from(&styled.styles[node]);
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
    styled: &'a StyledDom,
    /// The container's element: what its anonymous boxes name.
    node: usize,
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
    /// Adds the boxes of `children`, the children of the element
    /// `parent`, in document order, to whatever is innermost: the open
    /// inline chain, else the run.
    fn add_children(&mut self, parent: usize, children: &[FlatChild]) {
        let styled = self.styled;
        // Table-internal boxes and captions met here, whose parent is a
        // block container or an inline element, are collected into an
        // anonymous table (§17.2.1); white space between them is
        // dropped, as the browsers do, and anything else ends the run.
        let mut misplaced: Vec<Misplaced> = Vec::new();
        for child in children {
            match child {
                FlatChild::Element(child_idx) => {
                    let style = &styled.styles[*child_idx];
                    if style.display == Display::None {
                        continue; // the element AND its subtree: no box at all
                    }
                    if needs_a_table(style.display) {
                        self.collect_misplaced(&mut misplaced, *child_idx);
                        continue;
                    }
                    self.flush_misplaced(parent, &mut misplaced);
                    match style.display {
                        // A float or an absolutely positioned box computes
                        // to a block-level `display` (CSS2 §9.7, the
                        // cascade's rule) and is a block container inside,
                        // or a table when its display is `table`; outside
                        // it is out of flow, and goes where it is met:
                        // into the run or inline open around it, else
                        // among the container's blocks.
                        Display::Block | Display::Table
                            if style.float != Float::None || style.position.is_out_of_flow() =>
                        {
                            let kind = if style.position.is_out_of_flow() {
                                BoxKind::Absolute
                            } else {
                                BoxKind::Float
                            };
                            let children = if style.display == Display::Table {
                                build_table(styled, *child_idx)
                            } else {
                                build_block_container(styled, *child_idx)
                            };
                            let out_of_flow = LayoutBox {
                                style: style.clone(),
                                kind,
                                children,
                                node: *child_idx,
                            };
                            if self.open.is_empty() && self.run.is_empty() {
                                self.out.push(out_of_flow);
                            } else {
                                self.add_inline_level(out_of_flow);
                            }
                        }
                        Display::Block => {
                            let block = LayoutBox {
                                style: style.clone(),
                                kind: BoxKind::Block,
                                children: build_block_container(styled, *child_idx),
                                node: *child_idx,
                            };
                            self.add_block(block);
                        }
                        Display::Table => {
                            let table = LayoutBox {
                                style: style.clone(),
                                kind: BoxKind::Table,
                                children: build_table(styled, *child_idx),
                                node: *child_idx,
                            };
                            self.add_block(table);
                        }
                        Display::Inline => {
                            self.open.push(LayoutBox {
                                style: style.clone(),
                                kind: BoxKind::Inline,
                                children: Vec::new(),
                                node: *child_idx,
                            });
                            self.add_children(*child_idx, &styled.flat.nodes[*child_idx].children);
                            let inline = self.open.pop().expect("the inline just opened");
                            self.add_inline_level(inline);
                        }
                        Display::InlineBlock => {
                            let block = LayoutBox {
                                style: style.clone(),
                                kind: BoxKind::InlineBlock,
                                children: build_block_container(styled, *child_idx),
                                node: *child_idx,
                            };
                            self.add_inline_level(block);
                        }
                        Display::InlineTable => {
                            let table = LayoutBox {
                                style: style.clone(),
                                kind: BoxKind::Table,
                                children: build_table(styled, *child_idx),
                                node: *child_idx,
                            };
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
                FlatChild::Text(text) => {
                    let white_space = text.chars().all(crate::inline::is_css_space);
                    if white_space && !misplaced.is_empty() {
                        continue; // between two table parts: no content
                    }
                    self.flush_misplaced(parent, &mut misplaced);
                    // White space at the start of the run (css-text-3
                    // §4.1: space, tab, newline, carriage return, form
                    // feed — a no-break space is content) is dropped:
                    // a line discards it. Elsewhere it is kept, as the
                    // gap it is; `flush_run` drops it at the end.
                    if white_space && self.open.is_empty() && self.run.is_empty() {
                        continue;
                    }
                    self.add_inline_level(LayoutBox {
                        style: styled.styles[parent].clone(),
                        kind: BoxKind::Text(text.clone()),
                        children: Vec::new(),
                        node: parent,
                    });
                }
            }
        }
        self.flush_misplaced(parent, &mut misplaced);
    }

    /// Collects the table-internal element `idx`, met where its parent
    /// is not a table part, for the anonymous table that will wrap it
    /// and its consecutive table-internal siblings (§17.2.1): a cell
    /// joins the anonymous row of the cells just before it, or starts
    /// one; a row, a row group, a column, a column group or a caption is
    /// built as itself.
    fn collect_misplaced(&mut self, misplaced: &mut Vec<Misplaced>, idx: usize) {
        let styled = self.styled;
        let style = &styled.styles[idx];
        if style.display == Display::TableCell {
            let cell = build_cell(styled, idx);
            match misplaced.last_mut() {
                Some(Misplaced::Cells(cells)) => cells.push(cell),
                _ => misplaced.push(Misplaced::Cells(vec![cell])),
            }
        } else {
            misplaced.push(Misplaced::Proper(Box::new(build_table_part(styled, idx))));
        }
    }

    /// Wraps the table-internal boxes collected so far, if any, in an
    /// anonymous table and adds it: an inline table, inline-level
    /// content of the inline element `parent` when one is open, else a
    /// block-level table among the container's blocks (§17.2.1: "if the
    /// parent is an inline box, the anonymous box is an inline-table").
    fn flush_misplaced(&mut self, parent: usize, misplaced: &mut Vec<Misplaced>) {
        if misplaced.is_empty() {
            return;
        }
        let styled = self.styled;
        let parts = std::mem::take(misplaced)
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
        let table = LayoutBox {
            style: anonymous_style(styled, parent, display),
            kind: BoxKind::Table,
            children: order_table_children(parts),
            node: parent,
        };
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
    /// keep the inline elements' style and node, and either may end up
    /// empty.
    fn add_block(&mut self, block: LayoutBox) {
        let chain: Vec<(ComputedStyle, usize)> = self
            .open
            .iter()
            .map(|b| (b.style.clone(), b.node))
            .collect();
        while let Some(inline) = self.open.pop() {
            self.add_inline_level(inline);
        }
        self.flush_run();
        self.out.push(block);
        for (style, node) in chain {
            self.open.push(LayoutBox {
                style,
                kind: BoxKind::Inline,
                children: Vec::new(),
                node,
            });
        }
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
            style: ComputedStyle::inherited_from(&self.styled.styles[self.node]),
            kind: BoxKind::AnonymousBlock,
            children,
            node: self.node,
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

/// The box of the element `idx`, whose `display` is one of the table
/// values other than a cell: a row, a row group, a column, a column
/// group or a caption, with its children built as the kind requires.
fn build_table_part(styled: &StyledDom, idx: usize) -> LayoutBox {
    let style = &styled.styles[idx];
    let (kind, children) = match style.display {
        Display::TableRow => (BoxKind::Row, build_row(styled, idx)),
        Display::TableRowGroup => (
            BoxKind::RowGroup(RowGroupKind::Body),
            build_row_group(styled, idx),
        ),
        Display::TableHeaderGroup => (
            BoxKind::RowGroup(RowGroupKind::Header),
            build_row_group(styled, idx),
        ),
        Display::TableFooterGroup => (
            BoxKind::RowGroup(RowGroupKind::Footer),
            build_row_group(styled, idx),
        ),
        Display::TableColumnGroup => {
            // Its column children, and nothing else (§17.2.1: any other
            // child of a column group is treated as `display: none`).
            let columns = styled.flat.nodes[idx]
                .children
                .iter()
                .filter_map(|child| match child {
                    FlatChild::Element(c) if styled.styles[*c].display == Display::TableColumn => {
                        Some(build_column(styled, *c))
                    }
                    _ => None,
                })
                .collect();
            (
                BoxKind::ColumnGroup {
                    span: column_span(&styled.flat, idx),
                },
                columns,
            )
        }
        Display::TableColumn => return build_column(styled, idx),
        Display::TableCaption => (BoxKind::Caption, build_block_container(styled, idx)),
        _ => unreachable!("a table part's display"),
    };
    LayoutBox {
        style: style.clone(),
        kind,
        children,
        node: idx,
    }
}

/// A column box: `span` columns of the grid, no children whatever the
/// element holds (§17.2.1).
fn build_column(styled: &StyledDom, idx: usize) -> LayoutBox {
    LayoutBox {
        style: styled.styles[idx].clone(),
        kind: BoxKind::Column {
            span: column_span(&styled.flat, idx),
        },
        children: Vec::new(),
        node: idx,
    }
}

/// A cell box: a block container inside, its spans from the HTML
/// attributes.
fn build_cell(styled: &StyledDom, idx: usize) -> LayoutBox {
    let (colspan, rowspan) = cell_spans(&styled.flat, idx);
    LayoutBox {
        style: styled.styles[idx].clone(),
        kind: BoxKind::Cell { colspan, rowspan },
        children: build_block_container(styled, idx),
        node: idx,
    }
}

/// An anonymous row holding `cells`, in the element `node`.
fn anonymous_row(styled: &StyledDom, node: usize, cells: Vec<LayoutBox>) -> LayoutBox {
    LayoutBox {
        style: anonymous_style(styled, node, Display::TableRow),
        kind: BoxKind::Row,
        children: cells,
        node,
    }
}

/// The children of a table box (CSS2 §17.2.1), ordered for layout and
/// paint: the element's captions, column groups, columns, row groups
/// and rows as themselves; its cells, and any other content, in
/// anonymous rows — consecutive ones in the same row, a cell as itself
/// in it and the rest in anonymous cells.
fn build_table(styled: &StyledDom, idx: usize) -> Vec<LayoutBox> {
    order_table_children(rows_and_parts(styled, idx, |display| {
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

/// The rows of a row group: the element's rows as themselves, and
/// anything else in anonymous rows as in a table.
fn build_row_group(styled: &StyledDom, idx: usize) -> Vec<LayoutBox> {
    rows_and_parts(styled, idx, |display| display == Display::TableRow)
}

/// The children of the table or row group `idx`: an element child
/// whose `display` passes `proper` is built as the part it is; every
/// other child — a cell, an element of any other display, text — goes
/// into an anonymous row, consecutive ones into the same row, white
/// space between parts dropped (§17.2.1's steps 1 and 2).
fn rows_and_parts(
    styled: &StyledDom,
    idx: usize,
    proper: impl Fn(Display) -> bool,
) -> Vec<LayoutBox> {
    let mut parts = Vec::new();
    let mut row_run: Vec<RowItem<'_>> = Vec::new();
    let flush = |parts: &mut Vec<LayoutBox>, row_run: &mut Vec<RowItem<'_>>| {
        let cells = cells_of(styled, idx, std::mem::take(row_run));
        if !cells.is_empty() {
            parts.push(anonymous_row(styled, idx, cells));
        }
    };
    for child in &styled.flat.nodes[idx].children {
        match child {
            FlatChild::Element(c) => {
                let display = styled.styles[*c].display;
                if display == Display::None {
                    continue;
                }
                if proper(display) {
                    flush(&mut parts, &mut row_run);
                    parts.push(build_table_part(styled, *c));
                } else if display == Display::TableCell {
                    row_run.push(RowItem::Cell(Box::new(build_cell(styled, *c))));
                } else {
                    row_run.push(RowItem::Content(child));
                }
            }
            FlatChild::Text(_) => row_run.push(RowItem::Content(child)),
        }
    }
    flush(&mut parts, &mut row_run);
    parts
}

/// The cells of the row `idx`: the element's cells as themselves, and
/// anything else in anonymous cells, consecutive ones in the same cell.
fn build_row(styled: &StyledDom, idx: usize) -> Vec<LayoutBox> {
    let items = styled.flat.nodes[idx]
        .children
        .iter()
        .filter_map(|child| match child {
            FlatChild::Element(c) => match styled.styles[*c].display {
                Display::None => None,
                Display::TableCell => Some(RowItem::Cell(Box::new(build_cell(styled, *c)))),
                _ => Some(RowItem::Content(child)),
            },
            FlatChild::Text(_) => Some(RowItem::Content(child)),
        })
        .collect();
    cells_of(styled, idx, items)
}

/// One child of a row, or of the anonymous row a table's or row
/// group's stray children get: a cell, or content for an anonymous
/// cell.
enum RowItem<'a> {
    Cell(Box<LayoutBox>),
    Content(&'a FlatChild),
}

/// The cells `items` make, in the element `node`: a cell as itself; a
/// run of consecutive content in one anonymous cell, built as the
/// block container a cell is; a run of nothing but white space in none
/// (§17.2.1 step 1: white space between table parts is no content).
fn cells_of(styled: &StyledDom, node: usize, items: Vec<RowItem<'_>>) -> Vec<LayoutBox> {
    let mut cells = Vec::new();
    let mut content: Vec<FlatChild> = Vec::new();
    let flush = |cells: &mut Vec<LayoutBox>, content: &mut Vec<FlatChild>| {
        let only_white_space = content.iter().all(|child| match child {
            FlatChild::Text(text) => text.chars().all(crate::inline::is_css_space),
            FlatChild::Element(_) => false,
        });
        let content = std::mem::take(content);
        if only_white_space {
            return;
        }
        cells.push(LayoutBox {
            style: anonymous_style(styled, node, Display::TableCell),
            kind: BoxKind::Cell {
                colspan: 1,
                rowspan: 1,
            },
            children: build_block_container_of(styled, node, &content),
            node,
        });
    };
    for item in items {
        match item {
            RowItem::Cell(cell) => {
                flush(&mut cells, &mut content);
                cells.push(*cell);
            }
            RowItem::Content(child) => content.push(child.clone()),
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

/// A `colspan`, `rowspan` or `span` attribute's value by the HTML
/// standard's rules for parsing non-negative integers: leading ASCII
/// white space and a `+` are skipped, the digits that follow are the
/// number and whatever follows them is ignored; no digit, or a
/// negative number, is an error (`None`). A number too large for a
/// `u64` saturates, which every clamp below treats alike.
fn html_non_negative_integer(value: &str) -> Option<u64> {
    let s = value.trim_start_matches([' ', '\t', '\n', '\r', '\x0c']);
    let (negative, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let value = s[..digits].parse::<u64>().unwrap_or(u64::MAX);
    if negative && value != 0 {
        return None;
    }
    Some(value)
}

/// A `td` or `th` element's `colspan` and `rowspan` (HTML's processing
/// model for tables): `colspan` is 1 when absent, unparsable or zero
/// and at most 1000; `rowspan` is 1 when absent or unparsable, zero
/// meaning to the end of the row group, and at most 65534. An element
/// of any other name is one cell of the grid whatever it says.
fn cell_spans(flat: &FlatDom, idx: usize) -> (u32, u32) {
    if !matches!(flat.nodes[idx].tag.as_str(), "td" | "th") {
        return (1, 1);
    }
    let colspan = flat
        .attr(idx, "colspan")
        .and_then(html_non_negative_integer)
        .filter(|v| *v != 0)
        .unwrap_or(1)
        .min(1000);
    let rowspan = flat
        .attr(idx, "rowspan")
        .and_then(html_non_negative_integer)
        .unwrap_or(1)
        .min(65534);
    (colspan as u32, rowspan as u32)
}

/// A `col` or `colgroup` element's `span`: 1 when absent, unparsable
/// or zero, at most 1000; 1 for an element of any other name.
fn column_span(flat: &FlatDom, idx: usize) -> u32 {
    if !matches!(flat.nodes[idx].tag.as_str(), "col" | "colgroup") {
        return 1;
    }
    flat.attr(idx, "span")
        .and_then(html_non_negative_integer)
        .filter(|v| *v != 0)
        .unwrap_or(1)
        .min(1000) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use dom::Element;

    fn styled(root: Element, css_src: &str) -> StyledDom {
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(css_src).expect("test css is valid");
        style::style_document(&doc, &[sheet])
    }

    /// The box tree of the document `html` parses to: the parser implies
    /// `html`, `head` and `body`, so the tree is the html box, whose one
    /// child is the body's (the head generates no box). Nodes: html 1,
    /// head 2, body 3, then the markup's elements from 4.
    fn tree_of(html: &str, css_src: &str) -> Vec<LayoutBox> {
        let doc = html::parse_document(html).expect("html parsing never fails");
        let sheet = css::parse_stylesheet(css_src).expect("test css is valid");
        build_box_tree(&style::style_document(&doc, &[sheet]))
    }

    /// The body's box in a [`tree_of`] tree.
    fn body_of(tree: &[LayoutBox]) -> &LayoutBox {
        let body = &tree[0].children[0];
        assert_eq!((body.kind.clone(), body.node), (BoxKind::Block, 3));
        body
    }

    /// `b`'s subtree as `(kind, node)` pairs in preorder, a text box's
    /// kind spelled as its text, for assertions on structure.
    fn shape(b: &LayoutBox, out: &mut Vec<(String, usize)>) {
        let kind = match &b.kind {
            BoxKind::Block => "block".to_string(),
            BoxKind::Inline => "inline".to_string(),
            BoxKind::InlineBlock => "inline-block".to_string(),
            BoxKind::Float => "float".to_string(),
            BoxKind::Absolute => "absolute".to_string(),
            BoxKind::AnonymousBlock => "anon".to_string(),
            BoxKind::Text(t) => format!("text {t}"),
            BoxKind::Table if b.style.display == Display::InlineTable => "inline-table".to_string(),
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
        out.push((kind, b.node));
        for c in &b.children {
            shape(c, out);
        }
    }

    fn shape_of(tree: &[LayoutBox]) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        for b in tree {
            shape(b, &mut out);
        }
        out
    }

    fn pairs(items: &[(&str, usize)]) -> Vec<(String, usize)> {
        items.iter().map(|(k, n)| (k.to_string(), *n)).collect()
    }

    /// [`shape_of`] without the text boxes.
    fn boxes_of(tree: &[LayoutBox]) -> Vec<(String, usize)> {
        shape_of(tree)
            .into_iter()
            .filter(|(k, _)| !k.starts_with("text"))
            .collect()
    }

    #[test]
    fn block_and_inline_elements_get_matching_box_kinds() {
        let root = Element::new("div").push(Element::new("span").text("x"));
        let tree = build_box_tree(&styled(
            root,
            "div { display: block; } span { display: inline; }",
        ));
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].kind, BoxKind::Block);
        // The span is inline-level content of the div, so it sits in an
        // anonymous block (M10.4) that names the div, its container.
        let anon = &tree[0].children[0];
        assert_eq!(anon.kind, BoxKind::AnonymousBlock);
        assert_eq!(anon.children[0].kind, BoxKind::Inline);
        // Element boxes name their element; the text box inside the
        // span names the span, its containing element.
        assert_eq!(tree[0].node, 1);
        assert_eq!(anon.node, 1);
        assert_eq!(anon.children[0].node, 2);
        assert_eq!(anon.children[0].children[0].node, 2);
    }

    #[test]
    fn display_none_produces_no_box_at_all() {
        let root = Element::new("div")
            .push(Element::new("p").text("hidden").attr("class", "gone"))
            .push(Element::new("span").text("shown"));
        let tree = build_box_tree(&styled(root, "p { display: none; }"));
        assert_eq!(tree[0].children.len(), 1); // the p is gone entirely, not just empty
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[("anon", 1), ("inline", 3), ("text shown", 3)])
        );
    }

    #[test]
    fn text_runs_become_anonymous_boxes_carrying_the_parent_style() {
        let root = Element::new("p").text("hello");
        let tree = build_box_tree(&styled(root, "p { color: #ff0000; }"));
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
        let tree = build_box_tree(&styled(root, ""));
        assert!(tree[0].children.is_empty(), "not even an anonymous block");
        // A no-break space is content, not white space.
        let root = Element::new("div").text("\u{a0}");
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
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
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
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
        let tree = build_box_tree(&styled(
            root,
            "div { color: #ff0000; text-align: center; padding: 10px; margin: 5px;
                   border: 2px solid #000000; background-color: #00ff00; width: 50px; }",
        ));
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
        let tree = build_box_tree(&styled(root, "span { color: #0000ff; }"));
        assert_eq!(
            shape_of(&tree[0].children),
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
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
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
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
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
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            boxes_of(&tree[0].children),
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
        let tree = build_box_tree(&styled(root, ""));
        let p = &tree[0].children[1];
        assert_eq!((p.kind.clone(), p.node), (BoxKind::Block, 3));
        assert_eq!(
            shape_of(&p.children),
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
        let tree = build_box_tree(&styled(root, "p { display: none; }"));
        assert_eq!(
            shape_of(&tree[0].children),
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
        let tree = build_box_tree(&styled(root, "span { display: inline-block; }"));
        assert_eq!(
            shape_of(&tree[0].children),
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
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
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
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[("anon", 1), ("inline", 2), ("text  ", 2)])
        );
        // White space before a block, at the end of a run, is dropped
        // with the run when nothing else is in it.
        let root = Element::new("div")
            .text(" ")
            .push(Element::new("p").text("x"))
            .text(" ");
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
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
        let tree = build_box_tree(&styled(root, ".f { float: left; }"));
        assert_eq!(
            boxes_of(&tree[0].children),
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
        let tree = build_box_tree(&styled(root, ".f { float: right; }"));
        assert_eq!(
            shape_of(&tree[0].children),
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
        let tree = build_box_tree(&styled(root, ".f { float: left; }"));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[("float", 2), ("anon", 1), ("text x", 1)])
        );
        // An absolutely positioned box the same way, a float no more
        // (CSS2 §9.7). Nodes: div 1, span 2, div.a 3.
        let root = Element::new("div").text("a").push(
            Element::new("span")
                .text("b")
                .push(Element::new("div").attr("class", "a").text("f")),
        );
        let tree = build_box_tree(&styled(root, ".a { position: absolute; float: left; }"));
        assert_eq!(
            shape_of(&tree[0].children),
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
        let tree = build_box_tree(&styled(root, ".a { position: fixed; }"));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[("absolute", 2), ("block", 3), ("anon", 3), ("text x", 3)])
        );
        assert!(!tree[0].children[0].is_block_level());
    }

    #[test]
    fn top_level_inline_content_goes_into_an_anonymous_block_of_the_document() {
        // A hand-built document whose root element is inline: the
        // document node (0) is the block container.
        let tree = build_box_tree(&styled(Element::new("span").text("x"), ""));
        assert_eq!(
            shape_of(&tree),
            pairs(&[("anon", 0), ("inline", 1), ("text x", 1)])
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
        let body = body_of(&tree);
        assert_eq!((body.kind.clone(), body.node), (BoxKind::Block, 3));
        assert_eq!(
            boxes_of(&body.children),
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
            shape_of(&body.children)
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
        let table = &body_of(&tree).children[0];
        let parts: Vec<(String, usize)> = table
            .children
            .iter()
            .map(|c| (shape_of(std::slice::from_ref(c))[0].0.clone(), c.node))
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
        let table = &body_of(&tree).children[0];
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
            table.children[0].node, 8,
            "the second footer, as a body group"
        );
        assert_eq!(table.children[2].node, 5, "the first footer, last");
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
        let table = &body_of(&tree).children[0];
        let parts: Vec<(String, usize)> = table
            .children
            .iter()
            .map(|c| (shape_of(std::slice::from_ref(c))[0].0.clone(), c.node))
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
        let table = &body_of(&tree).children[0];
        assert_eq!(
            boxes_of(&table.children),
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
        let body = body_of(&tree);
        assert_eq!(
            shape_of(&body.children),
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
        let table = &body_of(&tree).children[0];
        assert_eq!(
            shape_of(&table.children),
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
        let div = &body_of(&tree).children[0];
        assert_eq!(
            boxes_of(&div.children),
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
        let cell = &body_of(&tree).children[0].children[0].children[0].children[0];
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
        let div = &body_of(&tree).children[0];
        assert_eq!(
            shape_of(&div.children),
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
        let div = &body_of(&tree).children[0];
        assert_eq!(
            shape_of(&div.children),
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
        let div = &body_of(&tree).children[0];
        assert_eq!(
            boxes_of(&div.children),
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
        let table = &body_of(&tree).children[0].children[0];
        assert_eq!(
            boxes_of(&table.children),
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
        let table = &body_of(&tree).children[0];
        assert_eq!(
            boxes_of(&table.children),
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
            let cell = &body_of(&tree).children[0].children[0].children[0];
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
            let col = &body_of(&tree).children[0].children[0];
            assert_eq!(col.kind, BoxKind::Column { span: expected }, "{attr}");
        }
        assert_eq!(html_non_negative_integer("  12abc"), Some(12));
        assert_eq!(html_non_negative_integer("abc"), None);
        assert_eq!(html_non_negative_integer("-3"), None);
        assert_eq!(html_non_negative_integer("+"), None);
    }
}
