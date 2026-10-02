//! Tables (CSS2 chapter 17, roadmap M12, docs/design/tables.md): a
//! [`BoxKind::Table`] box laid out as the grid its rows and cells form.
//!
//! The grid first (css-tables-3 §3.4): each cell takes the next free
//! slot in its row, after any slot a cell from a row above still spans,
//! and occupies `colspan` columns and `rowspan` rows, a `rowspan` past
//! the end of its row group cut at the group's end and 0 meaning that
//! end; the number of columns is the widest row's, or what the column
//! boxes declare, whichever is greater. Then the column widths: the
//! fixed algorithm of CSS2 §17.5.2.1 — the column boxes with a `width`,
//! then the cells of the first row with one, a spanning cell's split
//! equally across its columns, the columns left over sharing what
//! remains of the table's width equally — and, until the automatic
//! algorithm lands (step 4 of the note), a table whose `width` is `auto`
//! takes its containing block's width. Then the row heights: what the
//! cells need, each laid out as a block container at its column's width
//! and never shorter than its own `height`, a cell spanning rows giving
//! what it needs beyond the rows it spans to those rows in proportion
//! to their heights; the table's own `height` past the rows' sum is
//! given to the rows the same way. Every cell is stretched to the rows
//! it spans, its content at its top (`vertical-align` and the row
//! baselines are step 5's). The separated border model's
//! `border-spacing` lies between every two cells and between the cells
//! and the table's padding edge (§17.6.1).
//!
//! The fragments: the table's own is its wrapper box (CSS2 §17.4),
//! holding the captions and the table box — the border box the table's
//! background and border paint over, the grid inside it — so that a
//! caption lies outside the table's border and the wrapper is what the
//! flow places; the wrapper paints nothing and carries the table's
//! positioning, the table box inside it none. A column group, a column,
//! a row group and a row each have a fragment too, painting nothing of
//! their own: in the separated model their backgrounds show only under
//! the cells of their area, never in the spacing (§17.6.1), so each
//! paints its background as one fragment per cell, under the cell, in
//! §17.5.1's order — column group, column, row group, row, cell — which
//! is the tree's. A row's and a row group's fragments cover their rows'
//! area, so a point in the spacing between cells is the row's and one
//! between rows the group's; a column's fragment has no size, since a
//! column is never hit. The paint-order walk needs nothing new: these
//! are block-level boxes in flow.

use style::computed::{
    BOTTOM, BorderCollapse, BorderStyle, BoxSizing, CaptionSide, Color, ComputedStyle, Display,
    Float, LEFT, LengthPercentageAuto, Position, TOP,
};
use text::FontCollection;

use crate::block::{
    AdjoiningMargins, BlockLayout, Content, Deferred, Edges, Fragment, box_fragment, carried,
    definite_height, height_bounds, layout_block_contents, relative_offset,
    resolve_width_and_margins, settle_absolutes,
};
use crate::box_tree::{BoxKind, LayoutBox};

/// Lays out the table box `b` whose content box is `content_width` wide
/// — the `width` given, or the containing block's width for `auto` —
/// with the border and padding `edges`, as [`layout_block_contents`]
/// does a block container: the result is relative to the wrapper box's
/// top-left, its `root` the wrapper's fragment and its `height` the
/// wrapper's, captions included. A table is a block formatting context
/// root: its margins are its own.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_table_contents<'a>(
    b: &'a LayoutBox,
    edges: &Edges,
    cb_width: f32,
    cb_height: Option<f32>,
    content_width: f32,
    margin_left: f32,
    fonts: &FontCollection,
) -> BlockLayout<'a> {
    let style = &b.style;
    let grid = Grid::of(b);
    let (hspacing, vspacing) = match style.border_collapse {
        BorderCollapse::Separate => (
            style.border_spacing.horizontal,
            style.border_spacing.vertical,
        ),
        BorderCollapse::Collapse => (0.0, 0.0),
    };
    let columns = grid.columns;

    // The column widths, and the content width they need: the fixed
    // algorithm's table is the wider of its `width` and its columns.
    let widths = fixed_column_widths(&grid, content_width, hspacing);
    let columns_width = if columns == 0 {
        0.0
    } else {
        widths.iter().sum::<f32>() + (columns + 1) as f32 * hspacing
    };
    let content_width = content_width.max(columns_width);

    // Every cell's content, laid out at its columns' width.
    let mut cells: Vec<CellLayout<'a>> = Vec::with_capacity(grid.cells.len());
    for cell in &grid.cells {
        let span_width = widths[cell.col..cell.col + cell.colspan]
            .iter()
            .sum::<f32>()
            + (cell.colspan - 1) as f32 * hspacing;
        cells.push(layout_cell(cell.b, span_width, content_width, fonts));
    }

    // The row heights: each row's own `height` and what its cells need,
    // a cell spanning rows giving what it needs beyond its rows to them.
    let mut heights: Vec<f32> = grid
        .rows
        .iter()
        .map(|row| match row.b.style.height {
            LengthPercentageAuto::Px(h) => h.max(0.0),
            LengthPercentageAuto::Auto | LengthPercentageAuto::Percent(_) => 0.0,
        })
        .collect();
    for (cell, layout) in grid.cells.iter().zip(&cells) {
        if cell.rowspan == 1 {
            heights[cell.row] = heights[cell.row].max(layout.natural_height);
        }
    }
    let mut spanning: Vec<(&GridCell<'a>, &CellLayout<'a>)> = grid
        .cells
        .iter()
        .zip(&cells)
        .filter(|(cell, _)| cell.rowspan > 1)
        .collect();
    spanning.sort_by_key(|(cell, _)| cell.row + cell.rowspan);
    for (cell, layout) in spanning {
        let rows = &mut heights[cell.row..cell.row + cell.rowspan];
        let have = rows.iter().sum::<f32>() + (cell.rowspan - 1) as f32 * vspacing;
        if layout.natural_height > have {
            distribute(layout.natural_height - have, rows);
        }
    }
    // The table's `height` is a minimum for the rows together.
    let rows_height = if heights.is_empty() {
        0.0
    } else {
        heights.iter().sum::<f32>() + (heights.len() + 1) as f32 * vspacing
    };
    let specified = definite_height(style, cb_height, edges.vertical());
    let (min_height, _) = height_bounds(style, cb_height, edges.vertical());
    let wanted = specified.unwrap_or(0.0).max(min_height);
    if wanted > rows_height && !heights.is_empty() {
        distribute(wanted - rows_height, &mut heights);
    }
    let content_height = rows_height.max(wanted);

    // Where the columns and rows are, from the table box's top-left.
    let Edges { border, padding } = edges;
    let mut col_x = Vec::with_capacity(columns);
    let mut x = border[LEFT] + padding[LEFT] + hspacing;
    for w in &widths {
        col_x.push(x);
        x += w + hspacing;
    }
    let mut row_y = Vec::with_capacity(heights.len());
    let mut y = border[TOP] + padding[TOP] + vspacing;
    for h in &heights {
        row_y.push(y);
        y += h + vspacing;
    }
    let grid_left = col_x
        .first()
        .copied()
        .unwrap_or(border[LEFT] + padding[LEFT]);
    let grid_width = if columns == 0 {
        0.0
    } else {
        widths.iter().sum::<f32>() + (columns - 1) as f32 * hspacing
    };
    let row_rect = |r: usize| Rect {
        x: grid_left,
        y: row_y[r],
        w: grid_width,
        h: heights[r],
    };
    let cell_rect = |cell: &GridCell<'_>| Rect {
        x: col_x[cell.col],
        y: row_y[cell.row],
        w: widths[cell.col..cell.col + cell.colspan]
            .iter()
            .sum::<f32>()
            + (cell.colspan - 1) as f32 * hspacing,
        h: heights[cell.row..cell.row + cell.rowspan]
            .iter()
            .sum::<f32>()
            + (cell.rowspan - 1) as f32 * vspacing,
    };

    // The table box's fragment, the grid's parts inside it.
    let table_width = content_width + edges.horizontal();
    let table_height = content_height + edges.vertical();
    let mut table_box = box_fragment(table_width, table_height, b);
    table_box.style = inner_style(style);
    // The absolutely positioned boxes met in the cells, relative to the
    // table box, and in the captions, relative to the wrapper.
    let mut cell_absolutes: Vec<Deferred<'a>> = Vec::new();
    let mut caption_absolutes: Vec<Deferred<'a>> = Vec::new();
    let mut cells: Vec<Option<CellLayout<'a>>> = cells.into_iter().map(Some).collect();

    // Column groups and columns: a fragment of no size each, holding its
    // background under the cells of its columns — the cells whose first
    // column is one of its own, gathered in one pass over the cells.
    let mut under_column: std::collections::HashMap<*const LayoutBox, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, cell) in grid.cells.iter().enumerate() {
        if let Some(cref) = grid.column_boxes.get(cell.col) {
            for part in [cref.column, cref.group].into_iter().flatten() {
                under_column
                    .entry(std::ptr::from_ref(part))
                    .or_default()
                    .push(i);
            }
        }
    }
    let cells_under = |part: &LayoutBox| {
        under_column
            .get(&std::ptr::from_ref(part))
            .map_or(&[][..], Vec::as_slice)
    };
    for part in &b.children {
        let is_column = matches!(
            part.kind,
            BoxKind::ColumnGroup { .. } | BoxKind::Column { .. }
        );
        if !is_column {
            continue;
        }
        let mut fragment = part_fragment(
            part,
            Rect::empty(grid_left, row_y.first().copied().unwrap_or(0.0)),
        );
        fragment.style.position = Position::Static;
        if part.style.background_color.a > 0.0 {
            for &i in cells_under(part) {
                let mut background = background_fragment(part, cell_rect(&grid.cells[i]));
                background.x -= fragment.x;
                background.y -= fragment.y;
                fragment.children.push(background);
            }
        }
        if let BoxKind::ColumnGroup { .. } = part.kind {
            for column in &part.children {
                let mut column_fragment =
                    part_fragment(column, Rect::empty(fragment.x, fragment.y));
                column_fragment.style.position = Position::Static;
                if column.style.background_color.a > 0.0 {
                    for &i in cells_under(column) {
                        let mut background = background_fragment(column, cell_rect(&grid.cells[i]));
                        background.x -= column_fragment.x;
                        background.y -= column_fragment.y;
                        column_fragment.children.push(background);
                    }
                }
                column_fragment.x -= fragment.x;
                column_fragment.y -= fragment.y;
                fragment.children.push(column_fragment);
            }
        }
        table_box.children.push(fragment);
    }

    // Row groups and rows, with their cells. Rows directly in the table
    // are its children as rows are a group's.
    let mut row_index = 0;
    for part in &b.children {
        match part.kind {
            BoxKind::RowGroup(_) => {
                let first = row_index;
                let count = part.children.len();
                let group_rect = if count == 0 {
                    Rect::empty(grid_left, row_y.get(first).copied().unwrap_or(0.0))
                } else {
                    let top = row_y[first];
                    let bottom = row_y[first + count - 1] + heights[first + count - 1];
                    Rect {
                        x: grid_left,
                        y: top,
                        w: grid_width,
                        h: bottom - top,
                    }
                };
                let mut group = part_fragment(part, group_rect);
                if part.style.background_color.a > 0.0 && count > 0 {
                    let range = grid.row_cells[first].start..grid.row_cells[first + count - 1].end;
                    for cell in &grid.cells[range] {
                        let mut background = background_fragment(part, cell_rect(cell));
                        background.x -= group.x;
                        background.y -= group.y;
                        group.children.push(background);
                    }
                }
                for row in &part.children {
                    let mut row_fragment = row_fragment(
                        row,
                        row_index,
                        &grid,
                        &mut cells,
                        &row_rect,
                        &cell_rect,
                        &mut cell_absolutes,
                    );
                    row_fragment.x -= group.x;
                    row_fragment.y -= group.y;
                    group.children.push(row_fragment);
                    row_index += 1;
                }
                table_box.children.push(group);
            }
            BoxKind::Row => {
                let fragment = row_fragment(
                    part,
                    row_index,
                    &grid,
                    &mut cells,
                    &row_rect,
                    &cell_rect,
                    &mut cell_absolutes,
                );
                table_box.children.push(fragment);
                row_index += 1;
            }
            _ => {}
        }
    }

    // The captions, blocks in the wrapper above and below the table box
    // (CSS2 §17.4): each at the wrapper's width, its margins its own.
    let mut top_captions = Vec::new();
    let mut bottom_captions = Vec::new();
    let mut top_height = 0.0f32;
    let mut bottom_height = 0.0f32;
    for part in &b.children {
        if part.kind != BoxKind::Caption {
            continue;
        }
        let (stack, height) = if part.style.caption_side == CaptionSide::Top {
            (&mut top_captions, &mut top_height)
        } else {
            (&mut bottom_captions, &mut bottom_height)
        };
        let layout = layout_caption(part, table_width, fonts);
        let margin_top = layout.margin_top.resolve();
        let margin_bottom = layout.margin_bottom.resolve();
        let y = *height + margin_top;
        let x = layout.margin_left;
        // A bottom caption's boxes are moved below the table box with it.
        let below = if part.style.caption_side == CaptionSide::Top {
            0.0
        } else {
            table_height
        };
        caption_absolutes.extend(carried(layout.absolutes, x, y + below));
        stack.push(layout.root.translated(x, y));
        *height = y + layout.height + margin_bottom;
    }

    // The wrapper: the captions and the table box stacked, the table's
    // margins around them all. The bottom captions' positions, and the
    // absolutely positioned boxes they carry, move below the table box.
    let wrapper_height = top_height + table_height + bottom_height;
    let mut root = box_fragment(table_width, wrapper_height, b);
    root.style = wrapper_style(style);
    root.children.extend(top_captions);
    root.children.push(table_box.translated(0.0, top_height));
    root.children.extend(
        bottom_captions
            .into_iter()
            .map(|f| f.translated(0.0, top_height + table_height)),
    );
    // A caption's boxes are relative to the wrapper, a top caption's as
    // they are and a bottom one's moved past the table box above; a
    // cell's are relative to the table box, which the top captions
    // precede.
    let mut table_absolutes = carried(cell_absolutes, 0.0, top_height);
    table_absolutes.extend(carried(caption_absolutes, 0.0, top_height));
    let absolutes = settle_absolutes(b, &mut root, border, table_absolutes, fonts);

    let own_top = AdjoiningMargins::of(style.margin[TOP].resolve(cb_width, 0.0));
    let own_bottom = AdjoiningMargins::of(style.margin[BOTTOM].resolve(cb_width, 0.0));
    BlockLayout {
        root,
        height: wrapper_height,
        margin_left,
        margin_top: own_top,
        margin_bottom: own_bottom,
        top_edge: own_top,
        bottom_edge: own_bottom,
        self_collapsing: false,
        last_baseline: None,
        absolutes,
        auto_content_height: content_height,
    }
}

/// A rectangle in the table box's coordinates.
#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn empty(x: f32, y: f32) -> Self {
        Rect {
            x,
            y,
            w: 0.0,
            h: 0.0,
        }
    }
}

/// The grid a table's rows and cells form (css-tables-3 §3.4).
struct Grid<'a> {
    columns: usize,
    rows: Vec<GridRow<'a>>,
    /// In row order, each row's in the order of its slots.
    cells: Vec<GridCell<'a>>,
    /// Per row, the range of `cells` that start in it, so that a row,
    /// a row group and a column find their cells without a search.
    row_cells: Vec<std::ops::Range<usize>>,
    /// Per column, the column box and column group box covering it,
    /// if any.
    column_boxes: Vec<ColumnRef<'a>>,
}

struct GridRow<'a> {
    b: &'a LayoutBox,
    group: usize,
}

/// A row group, or the run of consecutive rows directly in the table,
/// which act as one for a `rowspan` reaching past them.
struct GridGroup {
    first_row: usize,
    rows: usize,
}

struct GridCell<'a> {
    b: &'a LayoutBox,
    col: usize,
    row: usize,
    colspan: usize,
    rowspan: usize,
}

#[derive(Default, Clone, Copy)]
struct ColumnRef<'a> {
    group: Option<&'a LayoutBox>,
    column: Option<&'a LayoutBox>,
}

impl<'a> Grid<'a> {
    fn of(table: &'a LayoutBox) -> Self {
        let mut groups: Vec<GridGroup> = Vec::new();
        let mut rows: Vec<GridRow<'a>> = Vec::new();
        let mut implicit: Option<usize> = None;
        for part in &table.children {
            match part.kind {
                BoxKind::RowGroup(_) => {
                    implicit = None;
                    let g = groups.len();
                    groups.push(GridGroup {
                        first_row: rows.len(),
                        rows: part.children.len(),
                    });
                    rows.extend(part.children.iter().map(|row| GridRow { b: row, group: g }));
                }
                BoxKind::Row => {
                    let g = *implicit.get_or_insert_with(|| {
                        groups.push(GridGroup {
                            first_row: rows.len(),
                            rows: 0,
                        });
                        groups.len() - 1
                    });
                    groups[g].rows += 1;
                    rows.push(GridRow { b: part, group: g });
                }
                _ => {}
            }
        }

        // The columns the column boxes declare: a column group's columns,
        // or `span` columns of its own when it has none.
        let mut column_boxes: Vec<ColumnRef<'a>> = Vec::new();
        for part in &table.children {
            match part.kind {
                BoxKind::ColumnGroup { span } => {
                    if part.children.is_empty() {
                        for _ in 0..span {
                            column_boxes.push(ColumnRef {
                                group: Some(part),
                                column: None,
                            });
                        }
                    } else {
                        for column in &part.children {
                            if let BoxKind::Column { span } = column.kind {
                                for _ in 0..span {
                                    column_boxes.push(ColumnRef {
                                        group: Some(part),
                                        column: Some(column),
                                    });
                                }
                            }
                        }
                    }
                }
                BoxKind::Column { span } => {
                    for _ in 0..span {
                        column_boxes.push(ColumnRef {
                            group: None,
                            column: Some(part),
                        });
                    }
                }
                _ => {}
            }
        }

        // The slots: each cell after the slots the rows above still span.
        let mut cells = Vec::new();
        let mut occupied: Vec<Vec<bool>> = vec![Vec::new(); rows.len()];
        let mut columns = column_boxes.len();
        let mut row_cells = Vec::with_capacity(rows.len());
        for r in 0..rows.len() {
            let start = cells.len();
            let group = &groups[rows[r].group];
            let group_end = group.first_row + group.rows;
            let mut c = 0;
            for cell in &rows[r].b.children {
                let BoxKind::Cell { colspan, rowspan } = cell.kind else {
                    continue;
                };
                while occupied[r].get(c).copied().unwrap_or(false) {
                    c += 1;
                }
                let colspan = (colspan as usize).max(1);
                let rowspan = if rowspan == 0 {
                    group_end - r
                } else {
                    (rowspan as usize).min(group_end - r)
                }
                .max(1);
                cells.push(GridCell {
                    b: cell,
                    col: c,
                    row: r,
                    colspan,
                    rowspan,
                });
                for row_slots in &mut occupied[r..r + rowspan] {
                    if row_slots.len() < c + colspan {
                        row_slots.resize(c + colspan, false);
                    }
                    for slot in &mut row_slots[c..c + colspan] {
                        *slot = true;
                    }
                }
                c += colspan;
                columns = columns.max(c);
            }
            row_cells.push(start..cells.len());
        }
        column_boxes.resize(columns, ColumnRef::default());
        Grid {
            columns,
            rows,
            cells,
            row_cells,
            column_boxes,
        }
    }
}

/// CSS2 §17.5.2.1, the fixed table layout algorithm, for a table whose
/// content box is `table_width` wide with `hspacing` between columns:
/// a column box's `width` sets its columns' width (each of them, for a
/// `span`); a first-row cell's `width` — its border box, a percentage
/// of the table's width — sets its column's, or, spanning, is split
/// equally across its columns less the spacing between them; the
/// columns left over share what remains of the width equally, and
/// nothing when nothing remains. Width left over with no column to
/// take it goes to every column in proportion to its width, equally
/// when all are zero (§17.5.2.1: "the extra space should be
/// distributed over the columns").
fn fixed_column_widths(grid: &Grid<'_>, table_width: f32, hspacing: f32) -> Vec<f32> {
    let n = grid.columns;
    let mut widths: Vec<Option<f32>> = vec![None; n];
    for (c, cref) in grid.column_boxes.iter().enumerate() {
        let source = cref.column.or(cref.group);
        if let Some(part) = source
            && let Some(w) = given_width(&part.style, table_width)
        {
            widths[c] = Some(w.max(0.0));
        }
    }
    for cell in grid.cells.iter().filter(|cell| cell.row == 0) {
        let Some(w) = given_width(&cell.b.style, table_width) else {
            continue;
        };
        let border_box = match cell.b.style.box_sizing {
            BoxSizing::BorderBox => w,
            BoxSizing::ContentBox => w + Edges::of(&cell.b.style, table_width).horizontal(),
        };
        let each =
            ((border_box - (cell.colspan - 1) as f32 * hspacing) / cell.colspan as f32).max(0.0);
        for slot in &mut widths[cell.col..cell.col + cell.colspan] {
            if slot.is_none() {
                *slot = Some(each);
            }
        }
    }
    let set: f32 = widths.iter().flatten().sum();
    let remaining = (table_width - set - (n + 1) as f32 * hspacing).max(0.0);
    let auto = widths.iter().filter(|w| w.is_none()).count();
    if auto > 0 {
        let each = remaining / auto as f32;
        for slot in &mut widths {
            if slot.is_none() {
                *slot = Some(each);
            }
        }
    }
    let mut widths: Vec<f32> = widths.into_iter().map(|w| w.unwrap_or(0.0)).collect();
    if auto == 0 && remaining > 0.0 && n > 0 {
        distribute(remaining, &mut widths);
    }
    widths
}

/// A `width` as a length, a percentage of `of`, or `None` for `auto`.
fn given_width(style: &ComputedStyle, of: f32) -> Option<f32> {
    match style.width {
        LengthPercentageAuto::Auto => None,
        LengthPercentageAuto::Px(w) => Some(w),
        LengthPercentageAuto::Percent(p) => Some(of * p / 100.0),
    }
}

/// Gives `extra` to `parts` in proportion to their sizes, equally when
/// all are zero.
fn distribute(extra: f32, parts: &mut [f32]) {
    if parts.is_empty() || extra <= 0.0 {
        return;
    }
    let total: f32 = parts.iter().sum();
    if total > 0.0 {
        for part in parts.iter_mut() {
            *part += extra * *part / total;
        }
    } else {
        let each = extra / parts.len() as f32;
        for part in parts.iter_mut() {
            *part += each;
        }
    }
}

/// A cell's content laid out at its columns' width.
struct CellLayout<'a> {
    layout: BlockLayout<'a>,
    /// The border-box height the cell needs: its content's, or its own
    /// `height` and `min-height` when taller (CSS2 §17.5.3: a cell's
    /// height is a minimum), plus its padding and border.
    natural_height: f32,
    /// The move `position: relative` gives the cell (CSS2 §9.4.3), its
    /// containing block the table box, applied where the cell is placed.
    relative: (f32, f32),
}

/// Lays out the cell `b`'s content as the block formatting context root
/// it is, `span_width` wide over its border box (a cell has no margins),
/// percentages of `table_width`.
fn layout_cell<'a>(
    b: &'a LayoutBox,
    span_width: f32,
    table_width: f32,
    fonts: &FontCollection,
) -> CellLayout<'a> {
    let edges = Edges::of(&b.style, table_width);
    let content_width = (span_width - edges.horizontal()).max(0.0);
    let layout = layout_block_contents(
        b,
        &edges,
        table_width,
        None,
        content_width,
        0.0,
        Content::Root,
        fonts,
    );
    let specified = definite_height(&b.style, None, edges.vertical()).unwrap_or(0.0);
    let (min_height, _) = height_bounds(&b.style, None, edges.vertical());
    let content_height = layout.auto_content_height.max(specified).max(min_height);
    CellLayout {
        natural_height: content_height + edges.vertical(),
        relative: relative_offset(&b.style, table_width, None),
        layout,
    }
}

/// A caption laid out as a block in the wrapper box, whose width is the
/// table box's (CSS2 §17.4).
fn layout_caption<'a>(
    b: &'a LayoutBox,
    wrapper_width: f32,
    fonts: &FontCollection,
) -> BlockLayout<'a> {
    let edges = Edges::of(&b.style, wrapper_width);
    let (content_width, margin_left, _) =
        resolve_width_and_margins(&b.style, wrapper_width, wrapper_width, edges.horizontal());
    layout_block_contents(
        b,
        &edges,
        wrapper_width,
        None,
        content_width,
        margin_left,
        Content::Root,
        fonts,
    )
}

/// The row `row`'s fragment over `row_rect(index)`: its background under
/// each of its cells, then the cells, each stretched to the rows it
/// spans, relative to the row.
#[allow(clippy::too_many_arguments)]
fn row_fragment<'a>(
    row: &'a LayoutBox,
    index: usize,
    grid: &Grid<'a>,
    cells: &mut [Option<CellLayout<'a>>],
    row_rect: &dyn Fn(usize) -> Rect,
    cell_rect: &dyn Fn(&GridCell<'_>) -> Rect,
    absolutes: &mut Vec<Deferred<'a>>,
) -> Fragment {
    let rect = row_rect(index);
    let mut fragment = part_fragment(row, rect);
    let own = grid.row_cells[index].clone();
    if row.style.background_color.a > 0.0 {
        for i in own.clone() {
            let mut background = background_fragment(row, cell_rect(&grid.cells[i]));
            background.x -= rect.x;
            background.y -= rect.y;
            fragment.children.push(background);
        }
    }
    for i in own {
        let cell = &grid.cells[i];
        let Some(layout) = cells[i].take() else {
            continue;
        };
        let r = cell_rect(cell);
        let mut cell_fragment = layout.layout.root;
        cell_fragment.w = r.w;
        cell_fragment.h = r.h;
        let (dx, dy) = layout.relative;
        cell_fragment.x = r.x - rect.x + dx;
        cell_fragment.y = r.y - rect.y + dy;
        absolutes.extend(carried(layout.layout.absolutes, r.x + dx, r.y + dy));
        fragment.children.push(cell_fragment);
    }
    fragment
}

/// The fragment of a table part that paints nothing of its own — a
/// row, a row group, a column, a column group, the table box's wrapper:
/// its style with no background and no border, over `rect`.
fn part_fragment(part: &LayoutBox, rect: Rect) -> Fragment {
    let mut fragment = box_fragment(rect.w, rect.h, part);
    fragment.x = rect.x;
    fragment.y = rect.y;
    fragment.style.background_color = Color::TRANSPARENT;
    fragment.style.border_style = [BorderStyle::None; 4];
    fragment
}

/// A table part's background under one cell: a fragment of the part's
/// style with no border, over the cell's border box, naming the part.
fn background_fragment(part: &LayoutBox, rect: Rect) -> Fragment {
    let mut fragment = box_fragment(rect.w, rect.h, part);
    fragment.x = rect.x;
    fragment.y = rect.y;
    fragment.style.border_style = [BorderStyle::None; 4];
    fragment.style.position = Position::Static;
    fragment.style.float = Float::None;
    fragment.style.z_index = None;
    fragment
}

/// The wrapper box's style: the table's, painting nothing of its own
/// (the table box paints the background and border) and carrying the
/// table's positioning for the walk.
fn wrapper_style(style: &ComputedStyle) -> ComputedStyle {
    let mut wrapper = style.clone();
    wrapper.background_color = Color::TRANSPARENT;
    wrapper.border_style = [BorderStyle::None; 4];
    wrapper
}

/// The table box's style inside the wrapper: the table's, painting its
/// background and border, in flow and block-level whatever the table
/// is, since the wrapper carries its positioning and its place on a
/// line.
fn inner_style(style: &ComputedStyle) -> ComputedStyle {
    let mut inner = style.clone();
    inner.display = Display::Table;
    inner.position = Position::Static;
    inner.float = Float::None;
    inner.z_index = None;
    inner.inset = [LengthPercentageAuto::Auto; 4];
    inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{DocumentLayout, FragmentKind, layout_document};
    use style::computed::{LengthPercentage, VerticalAlign};

    const VIEWPORT: f32 = 800.0;

    fn test_fonts() -> FontCollection {
        FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
    }

    /// The document `html` parses to, styled under `css` and laid out
    /// against an 800px viewport. The parser implies html, head and
    /// body: nodes html 1, head 2, body 3, the markup's from 4.
    fn document_of(html: &str, css: &str) -> DocumentLayout {
        let fonts = test_fonts();
        let doc = html::parse_document(html).expect("html parsing never fails");
        let sheet = css::parse_stylesheet(css).expect("test css is valid");
        let styled = style::style_document(&doc, &[sheet]);
        let boxes = crate::build_box_tree(&styled);
        layout_document(&boxes, VIEWPORT, &fonts)
    }

    /// The first fragment in tree order for the node `node`.
    fn fragment_of(layout: &DocumentLayout, node: usize) -> &Fragment {
        layout
            .iter()
            .find(|f| f.node == node && matches!(f.kind, FragmentKind::Box))
            .unwrap_or_else(|| panic!("a box fragment for node {node}"))
    }

    /// Every box fragment of `node`, in tree order.
    fn fragments_of(layout: &DocumentLayout, node: usize) -> Vec<&Fragment> {
        layout
            .iter()
            .filter(|f| f.node == node && matches!(f.kind, FragmentKind::Box))
            .collect()
    }

    fn rect(f: &Fragment) -> (f32, f32, f32, f32) {
        (f.x, f.y, f.w, f.h)
    }

    #[test]
    fn the_fixed_algorithm_shares_the_width_among_the_columns() {
        // Two columns in a 200px table with 10px spacing: each column is
        // (200 - 3 * 10) / 2 = 85px; the cells sit after the spacing and
        // the rows after it too. Nodes: table 4, tr 5, td 6, td 7, tr 8,
        // td 9, td 10.
        let layout = document_of(
            "<table><tr><td></td><td></td></tr><tr><td></td><td></td></tr></table>",
            "body { margin: 0; } td { padding: 0; height: 20px; }
             table { table-layout: fixed; width: 200px; border-spacing: 10px; }",
        );
        let table = fragments_of(&layout, 4);
        assert_eq!(table.len(), 2, "the wrapper and the table box");
        assert_eq!(rect(table[0]), (0.0, 0.0, 200.0, 70.0), "the wrapper");
        assert_eq!(rect(table[1]), (0.0, 0.0, 200.0, 70.0), "the table box");
        assert_eq!(rect(fragment_of(&layout, 6)), (10.0, 10.0, 85.0, 20.0));
        assert_eq!(rect(fragment_of(&layout, 7)), (105.0, 10.0, 85.0, 20.0));
        assert_eq!(rect(fragment_of(&layout, 9)), (10.0, 40.0, 85.0, 20.0));
        assert_eq!(rect(fragment_of(&layout, 10)), (105.0, 40.0, 85.0, 20.0));
        // The rows cover their cells' area, spacing between the cells
        // included, the spacing around the grid excluded.
        assert_eq!(rect(fragment_of(&layout, 5)), (10.0, 10.0, 180.0, 20.0));
        assert_eq!(rect(fragment_of(&layout, 8)), (10.0, 40.0, 180.0, 20.0));
        // The wrapper paints nothing; the table box keeps the style.
        assert_eq!(table[0].style.background_color, Color::TRANSPARENT);
        assert_eq!(table[1].style.display, Display::Table);
    }

    #[test]
    fn column_boxes_and_first_row_cells_set_their_columns_and_the_rest_share() {
        // A 300px table, no spacing: a col of 50px, a first-row cell of
        // 100px (content-box, plus 10px of padding each side = 120px
        // border box), a percentage cell of 10% (30px, 50 with its
        // padding), and one column left to take the rest: 300 - 50 - 120
        // - 50 = 80. The second row's widths change nothing. Nodes:
        // table 4, col 5, tr 6, td 7, td 8, td 9, td 10, tr 11, td 12.
        let layout = document_of(
            "<table><col><tr><td></td><td class=w></td><td class=p></td><td></td></tr>\
             <tr><td class=wide></td></tr></table>",
            "body { margin: 0; } td { padding: 0 10px; height: 10px; }
             table { table-layout: fixed; width: 300px; border-spacing: 0; }
             col { width: 50px; } .w { width: 100px; } .p { width: 10%; }
             .wide { width: 500px; }",
        );
        assert_eq!(rect(fragment_of(&layout, 7)), (0.0, 0.0, 50.0, 10.0));
        assert_eq!(rect(fragment_of(&layout, 8)), (50.0, 0.0, 120.0, 10.0));
        assert_eq!(rect(fragment_of(&layout, 9)), (170.0, 0.0, 50.0, 10.0));
        assert_eq!(rect(fragment_of(&layout, 10)), (220.0, 0.0, 80.0, 10.0));
        assert_eq!(rect(fragment_of(&layout, 12)), (0.0, 10.0, 50.0, 10.0));
        assert_eq!(fragment_of(&layout, 4).w, 300.0);
    }

    #[test]
    fn columns_wider_than_the_table_widen_it_and_a_span_splits_a_cells_width() {
        // Two cols of 200px in a 100px table: the table is 400px wide.
        // A cell spanning both with a width of 300px splits it 150/150,
        // which the cols already set; a `span=2` col gives each of its
        // columns the width. Nodes: table 4, col 5, tr 6, td 7, table 8,
        // col 9, tr 10, td 11, td 12.
        let layout = document_of(
            "<table><col span=2><tr><td colspan=2></td></tr></table>\
             <table class=b><col span=2><tr><td></td><td></td></tr></table>",
            "body { margin: 0; } td { padding: 0; height: 10px; }
             table { table-layout: fixed; width: 100px; border-spacing: 0; }
             col { width: 200px; }",
        );
        assert_eq!(fragment_of(&layout, 4).w, 400.0);
        assert_eq!(rect(fragment_of(&layout, 7)), (0.0, 0.0, 400.0, 10.0));
        assert_eq!(rect(fragment_of(&layout, 11)), (0.0, 10.0, 200.0, 10.0));
        assert_eq!(rect(fragment_of(&layout, 12)), (200.0, 10.0, 200.0, 10.0));
    }

    #[test]
    fn spanning_cells_take_their_slots_and_push_the_cells_after_them() {
        // Row 1: a cell spanning two rows, then one spanning two columns.
        // Row 2: its first cell goes to the second column, after the slot
        // the rowspan holds. A rowspan of 0 reaches the end of the row
        // group, and one past the end is cut there. Nodes: table 4,
        // tbody 5, tr 6, td 7, td 8, tr 9, td 10, td 11, tr 12, td 13,
        // tbody 14, tr 15, td 16.
        let layout = document_of(
            "<table><tbody><tr><td rowspan=2></td><td colspan=2></td></tr>\
             <tr><td></td><td rowspan=0></td></tr><tr><td></td></tr></tbody>\
             <tbody><tr><td rowspan=5></td></tr></tbody></table>",
            "body { margin: 0; } td { padding: 0; height: 10px; }
             table { table-layout: fixed; width: 300px; border-spacing: 0; }",
        );
        // Three columns of 100px.
        assert_eq!(rect(fragment_of(&layout, 7)), (0.0, 0.0, 100.0, 20.0));
        assert_eq!(rect(fragment_of(&layout, 8)), (100.0, 0.0, 200.0, 10.0));
        assert_eq!(rect(fragment_of(&layout, 10)), (100.0, 10.0, 100.0, 10.0));
        // rowspan=0: rows 2 and 3 of the group.
        assert_eq!(rect(fragment_of(&layout, 11)), (200.0, 10.0, 100.0, 20.0));
        assert_eq!(rect(fragment_of(&layout, 13)), (0.0, 20.0, 100.0, 10.0));
        // The second group's cell spans five rows but the group has one.
        assert_eq!(rect(fragment_of(&layout, 16)), (0.0, 30.0, 100.0, 10.0));
        assert_eq!(fragment_of(&layout, 4).h, 40.0);
        // The groups cover their rows.
        assert_eq!(rect(fragment_of(&layout, 5)), (0.0, 0.0, 300.0, 30.0));
        assert_eq!(rect(fragment_of(&layout, 14)), (0.0, 30.0, 300.0, 10.0));
    }

    #[test]
    fn rows_are_as_tall_as_their_cells_need_and_a_rowspan_shares_its_excess() {
        // Row 1 has a 30px cell; row 2 a 10px one; a cell spanning both
        // needs 100px: the 60px beyond the two rows' 40px go to them in
        // proportion, 45 and 15. A row's own height is a minimum, and a
        // cell's height is one too, under taller content. Nodes: table
        // 4, tr 5, td 6, td 7, tr 8, td 9, table 10, tr 11, td 12, tr 13,
        // td 14, div 15.
        let layout = document_of(
            "<table><tr><td class=h30></td><td rowspan=2 class=h100></td></tr>\
             <tr><td class=h10></td></tr></table>\
             <table><tr class=r40><td></td></tr><tr><td class=h10><div class=h50></div></td></tr></table>",
            "body { margin: 0; } td { padding: 0; }
             table { table-layout: fixed; width: 200px; border-spacing: 0; }
             .h10 { height: 10px; } .h30 { height: 30px; } .h50 { height: 50px; }
             .h100 { height: 100px; } .r40 { height: 40px; }",
        );
        assert_eq!(rect(fragment_of(&layout, 6)), (0.0, 0.0, 100.0, 75.0));
        assert_eq!(rect(fragment_of(&layout, 7)), (100.0, 0.0, 100.0, 100.0));
        assert_eq!(rect(fragment_of(&layout, 9)), (0.0, 75.0, 100.0, 25.0));
        assert_eq!(fragment_of(&layout, 4).h, 100.0);
        assert_eq!(rect(fragment_of(&layout, 12)), (0.0, 100.0, 200.0, 40.0));
        assert_eq!(rect(fragment_of(&layout, 14)), (0.0, 140.0, 200.0, 50.0));
    }

    #[test]
    fn the_tables_height_is_shared_among_the_rows_and_spacing_counts() {
        // Two 10px rows with 5px spacing in a table 100px tall (border-
        // box, the UA sheet's sizing, with a 2px border): the content is
        // 96px, the rows and spacing 35px, and the 61px extra goes to the
        // rows equally since their heights are equal. Nodes: table 4, tr
        // 5, td 6, tr 7, td 8.
        let layout = document_of(
            "<table><tr><td></td></tr><tr><td></td></tr></table>",
            "body { margin: 0; } td { padding: 0; height: 10px; }
             table { table-layout: fixed; width: 100px; height: 100px; border-spacing: 5px;
                     border: 2px solid #000; box-sizing: border-box; }",
        );
        let table = fragments_of(&layout, 4);
        assert_eq!(rect(table[1]), (0.0, 0.0, 100.0, 100.0));
        assert_eq!(rect(fragment_of(&layout, 6)), (7.0, 7.0, 86.0, 40.5));
        assert_eq!(rect(fragment_of(&layout, 8)), (7.0, 52.5, 86.0, 40.5));
    }

    #[test]
    fn backgrounds_of_rows_groups_and_columns_lie_under_their_cells_only() {
        // A row, a row group and a column with backgrounds: each gets a
        // background fragment per cell of its area, painting nothing of
        // its own; a row without one has only its cells. Nodes: table 4,
        // col 5, col 6, tbody 7, tr 8, td 9, td 10, tr 11, td 12, td 13.
        let layout = document_of(
            "<table><col class=bg><col><tbody class=bg><tr class=bg><td></td><td></td></tr>\
             <tr><td></td><td></td></tr></tbody></table>",
            "body { margin: 0; } td { padding: 0; height: 10px; }
             table { table-layout: fixed; width: 100px; border-spacing: 10px; }
             .bg { background-color: #ff0000; }",
        );
        let red = Color::rgb8(255, 0, 0);
        // The column: a fragment of no size, then its cells' backgrounds.
        let col = fragments_of(&layout, 5);
        assert_eq!(col.len(), 3);
        assert_eq!((col[0].w, col[0].h), (0.0, 0.0));
        assert_eq!(col[0].style.background_color, Color::TRANSPARENT);
        assert_eq!(rect(col[1]), (10.0, 10.0, 35.0, 10.0));
        assert_eq!(rect(col[2]), (10.0, 30.0, 35.0, 10.0));
        assert_eq!(col[1].style.background_color, red);
        assert_eq!(col[1].style.used_border_width(), [0.0; 4]);
        assert_eq!(
            fragments_of(&layout, 6).len(),
            1,
            "no background, no extra fragment"
        );
        // The group: its area, then four backgrounds, then its rows.
        let group = fragments_of(&layout, 7);
        assert_eq!(group.len(), 5);
        assert_eq!(rect(group[0]), (10.0, 10.0, 80.0, 30.0));
        assert_eq!(group[0].style.background_color, Color::TRANSPARENT);
        assert_eq!(rect(group[1]), (10.0, 10.0, 35.0, 10.0));
        assert_eq!(rect(group[4]), (55.0, 30.0, 35.0, 10.0));
        // The row with a background: two; the other: none.
        assert_eq!(fragments_of(&layout, 8).len(), 3);
        assert_eq!(fragments_of(&layout, 11).len(), 1);
        // In tree order the column's backgrounds come before the group's,
        // the group's before the row's, and the row's before its cells.
        let order: Vec<usize> = layout
            .iter()
            .filter(|f| matches!(f.kind, FragmentKind::Box))
            .map(|f| f.node)
            .collect();
        let first = |node: usize| order.iter().position(|n| *n == node).unwrap();
        let last = |node: usize| order.iter().rposition(|n| *n == node).unwrap();
        assert!(last(5) < first(7), "column backgrounds before the group");
        assert!(last(7) < first(8), "group backgrounds before the first row");
        assert!(last(8) < first(9), "row backgrounds before the cells");
    }

    #[test]
    fn captions_stack_above_and_below_the_table_box_in_the_wrapper() {
        // A top caption of 20px with a 5px margin and a bottom one of
        // 10px: the wrapper is 25 + 30 + 10 tall, the table box at y =
        // 25 in it, the captions as wide as the table. Nodes: table 4,
        // caption 5, caption 6, tr 7, td 8, p 9.
        let layout = document_of(
            "<table><caption class=t></caption><caption class=b></caption>\
             <tr><td></td></tr></table><p>after</p>",
            "body { margin: 0; } td { padding: 0; height: 30px; }
             table { table-layout: fixed; width: 100px; border-spacing: 0; }
             caption { margin: 0; } .t { height: 20px; margin-top: 5px; }
             .b { caption-side: bottom; height: 10px; } p { margin: 0; }",
        );
        let table = fragments_of(&layout, 4);
        assert_eq!(rect(table[0]), (0.0, 0.0, 100.0, 65.0), "the wrapper");
        assert_eq!(rect(table[1]), (0.0, 25.0, 100.0, 30.0), "the table box");
        assert_eq!(rect(fragment_of(&layout, 5)), (0.0, 5.0, 100.0, 20.0));
        assert_eq!(rect(fragment_of(&layout, 6)), (0.0, 55.0, 100.0, 10.0));
        assert_eq!(rect(fragment_of(&layout, 8)), (0.0, 25.0, 100.0, 30.0));
        assert_eq!(
            fragment_of(&layout, 9).y,
            65.0,
            "the flow goes on below the wrapper"
        );
    }

    #[test]
    fn a_table_is_a_formatting_context_root_in_its_flow() {
        // Its margins do not collapse with its cells' content; it dodges
        // a float like any root, its `auto` width the room beside it
        // until the automatic algorithm lands. Nodes: div.f 4, table 5,
        // tr 6, td 7, p 8.
        let layout = document_of(
            "<div class=f></div><table><tr><td><p>x</p></td></tr></table>",
            "body { margin: 0; } td { padding: 0; } .f { float: left; width: 100px; height: 50px; }
             table { table-layout: fixed; border-spacing: 0; margin-top: 10px; }
             p { margin: 20px 0; height: 10px; }",
        );
        let table = fragments_of(&layout, 5);
        assert_eq!(rect(table[0]), (100.0, 10.0, 700.0, 50.0));
        // The cell is a root too: the p's margins are inside it.
        assert_eq!(rect(fragment_of(&layout, 8)), (100.0, 30.0, 700.0, 10.0));
    }

    #[test]
    fn an_empty_table_and_one_without_rows_have_their_edges_alone() {
        // Nodes: table 4, table 5, col 6.
        let layout = document_of(
            "<table></table><table><col></table>",
            "body { margin: 0; } table { width: 50px; border: 3px solid #000; padding: 2px;
                                           border-spacing: 10px; box-sizing: content-box; }",
        );
        assert_eq!(rect(fragment_of(&layout, 4)), (0.0, 0.0, 60.0, 10.0));
        assert_eq!(rect(fragment_of(&layout, 5)), (0.0, 10.0, 60.0, 10.0));
    }

    #[test]
    fn the_ua_sheet_gives_a_plain_table_its_spacing_and_padding() {
        // 2px spacing and 1px cell padding from the UA sheet; the body's
        // 8px margin; a cell 20px tall inside: the cell's border box is
        // 22px. Nodes: table 4, tr 5, td 6, div 7.
        let layout = document_of(
            "<table><tr><td><div class=c></div></td></tr></table>",
            "table { table-layout: fixed; width: 100px; } .c { height: 20px; width: 30px; }",
        );
        assert_eq!(rect(fragment_of(&layout, 4)), (8.0, 8.0, 100.0, 26.0));
        assert_eq!(rect(fragment_of(&layout, 6)), (10.0, 10.0, 96.0, 22.0));
        assert_eq!(rect(fragment_of(&layout, 7)), (11.0, 11.0, 30.0, 20.0));
        assert_eq!(
            fragment_of(&layout, 6).style.vertical_align,
            VerticalAlign::Middle
        );
        assert_eq!(
            fragment_of(&layout, 6).style.padding,
            [LengthPercentage::Px(1.0); 4]
        );
    }

    #[test]
    fn distribute_shares_in_proportion_or_equally() {
        let mut parts = [10.0, 30.0];
        distribute(20.0, &mut parts);
        assert_eq!(parts, [15.0, 45.0]);
        let mut parts = [0.0, 0.0, 0.0];
        distribute(30.0, &mut parts);
        assert_eq!(parts, [10.0; 3]);
        let mut none: [f32; 0] = [];
        distribute(5.0, &mut none);
        let mut parts = [4.0];
        distribute(-1.0, &mut parts);
        assert_eq!(parts, [4.0]);
    }
}
