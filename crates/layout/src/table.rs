//! Tables (CSS2 chapter 17, roadmap M12, docs/design/tables.md): a
//! [`BoxKind::Table`] box laid out as the grid its rows and cells form.
//!
//! The grid first (css-tables-3 §3.4): each cell takes the next free
//! slot in its row, after any slot a cell from a row above still spans,
//! and occupies `colspan` columns and `rowspan` rows, a `rowspan` past
//! the end of its row group cut at the group's end and 0 meaning that
//! end; the number of columns is the widest row's, or what the column
//! boxes declare, whichever is greater. Then the column widths: for a
//! `table-layout: fixed` table with a `width`, the fixed algorithm of
//! CSS2 §17.5.2.1 — the column boxes with a `width`, then the cells of
//! the first row with one, a spanning cell's split equally across its
//! columns, the columns left over sharing what remains equally — and for
//! every other table the automatic one, as css-tables-3 §3.9 writes the
//! browsers' common practice down: each column's minimum, maximum,
//! percentage and whether a length `width` constrains it, from its
//! cells and its column box, cells spanning several columns giving
//! their excess to them; the table's minimum and maximum widths (MIN,
//! MAX) from those; and the width the table gets shared out by the
//! sizing guesses of §3.9.3. The cells are measured twice, once for the
//! table's own width by whoever places it and once here for its
//! columns: a linear cost, nested tables included, which a cache of
//! measures (the performance milestone's) would halve. Then the row
//! heights: what the
//! cells need, each laid out as a block container at its column's width
//! and never shorter than its own `height` (CSS2 §17.5.3). The cells
//! whose `vertical-align` is `baseline` — every value but `top`,
//! `middle` and `bottom` counts as it — share their row's baseline, the
//! lowest of their own (a cell's first line box's, or the first row's of
//! a table in it, else its content's bottom edge), so the row holds each
//! of them moved down to it; a cell spanning rows aligns to its first
//! row's and gives what it needs beyond the rows it spans to those rows
//! in proportion to their heights; the table's own `height` past the
//! rows' sum is given to the rows the same way. Every cell is stretched
//! to the rows it spans, and its content placed in it by its alignment:
//! at its top, centred, at its bottom, or on the row's baseline. The
//! table's baseline is its first row's, the lowest cell content's bottom
//! for a row with no cell on its baseline, and an inline table aligns
//! by it. The separated border model's
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
    Float, LEFT, LengthPercentage, LengthPercentageAuto, Position, RIGHT, TOP, TableLayout,
    VerticalAlign,
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
    let (hspacing, vspacing) = spacing(style);
    let columns = grid.columns;

    // The column widths, and the content width they need: the table is
    // never narrower than its columns' minimums and its captions'
    // (MIN and CAPMIN), nor, laid out by the fixed algorithm, than the
    // widths its columns are given. The widths are shared out of the
    // assignable width, the content width less the spacing.
    let spacing = columns_spacing(columns, hspacing);
    let (widths, content_width) = if uses_fixed_layout(style) {
        let widths = fixed_column_widths(&grid, (content_width - spacing).max(0.0), hspacing);
        let needed = widths.iter().sum::<f32>() + spacing;
        (widths, content_width.max(needed))
    } else {
        let measures = measure_columns(&grid, hspacing, fonts);
        let (grid_min, _) = grid_widths(&measures, hspacing);
        let content_width =
            content_width
                .max(grid_min)
                .max(caption_min(b, edges.horizontal(), fonts));
        let widths = auto_column_widths(&measures, (content_width - spacing).max(0.0));
        (widths, content_width)
    };

    // Every cell's content, laid out at its columns' width.
    let mut cells: Vec<CellLayout<'a>> = Vec::with_capacity(grid.cells.len());
    for cell in &grid.cells {
        let span_width = widths[cell.col..cell.col + cell.colspan]
            .iter()
            .sum::<f32>()
            + (cell.colspan - 1) as f32 * hspacing;
        cells.push(layout_cell(cell.b, span_width, content_width, fonts));
    }

    // The row baselines: the lowest baseline among the cells aligned to
    // it, a cell spanning rows to its first row's.
    let mut row_baselines: Vec<Option<f32>> = vec![None; grid.rows.len()];
    for (cell, layout) in grid.cells.iter().zip(&cells) {
        if layout.align == CellAlign::Baseline {
            let baseline = &mut row_baselines[cell.row];
            *baseline = Some(baseline.map_or(layout.baseline, |b| b.max(layout.baseline)));
        }
    }
    // What a cell needs of the rows it spans: its natural height, and,
    // aligned to the baseline, its content moved down to the row's.
    let needed = |cell: &GridCell<'_>, layout: &CellLayout<'_>| match row_baselines[cell.row] {
        Some(row_baseline) if layout.align == CellAlign::Baseline => layout
            .natural_height
            .max(row_baseline - layout.baseline + layout.extent),
        _ => layout.natural_height,
    };

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
            heights[cell.row] = heights[cell.row].max(needed(cell, layout));
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
        let wants = needed(cell, layout);
        if wants > have {
            distribute(wants - have, rows);
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

    // Where each cell's content goes in its height: at the top, centred,
    // at the bottom, or on its row's baseline; never above its top.
    let offsets: Vec<f32> = grid
        .cells
        .iter()
        .zip(&cells)
        .map(|(cell, layout)| {
            let height = cell_rect(cell).h;
            let offset = match layout.align {
                CellAlign::Top => 0.0,
                CellAlign::Middle => (height - layout.extent) / 2.0,
                CellAlign::Bottom => height - layout.extent,
                CellAlign::Baseline => {
                    row_baselines[cell.row].map_or(0.0, |row| row - layout.baseline)
                }
            };
            offset.max(0.0)
        })
        .collect();
    // A row's baseline from the table box's top: its cells' shared one,
    // or, with no cell on it, the lowest content bottom among its cells,
    // or its bottom with no cell at all.
    let row_baseline = |r: usize| -> f32 {
        let own = row_baselines[r].or_else(|| {
            grid.row_cells[r]
                .clone()
                .map(|i| offsets[i] + cells[i].content_bottom)
                .reduce(f32::max)
        });
        row_y[r] + own.unwrap_or(heights[r])
    };
    let first_row_baseline = (!heights.is_empty()).then(|| row_baseline(0));
    let last_row_baseline = (!heights.is_empty()).then(|| row_baseline(heights.len() - 1));

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
                        &offsets,
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
                    &offsets,
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
        // The first row's baseline is the table's own (an inline table
        // aligns by it); the last row's is what a box ending in the table
        // takes as its last.
        first_baseline: first_row_baseline.map(|b| top_height + b),
        last_baseline: last_row_baseline.map(|b| top_height + b),
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

/// CSS2 §17.5.2.1, the fixed table layout algorithm, sharing out the
/// assignable width `assignable` — the table's content width less the
/// spacing around and between its columns, `hspacing` each: a column
/// box's `width` sets its columns' width (each of them, for a `span`;
/// a column's own, else its group's); a first-row cell's `width` — its
/// border box — sets its column's, or, spanning, is split equally across
/// its columns less the spacing between them; a percentage is of the
/// assignable width; the columns left over share what remains equally,
/// and nothing when nothing remains. Width left over with no column to
/// take it goes to every column in proportion to its width, equally
/// when all are zero (§17.5.2.1: "the extra space should be
/// distributed over the columns").
fn fixed_column_widths(grid: &Grid<'_>, assignable: f32, hspacing: f32) -> Vec<f32> {
    let n = grid.columns;
    let mut widths: Vec<Option<f32>> = vec![None; n];
    for (c, cref) in grid.column_boxes.iter().enumerate() {
        if let Some(w) = column_box_width(cref).and_then(|w| resolve_width(w, assignable)) {
            widths[c] = Some(w.max(0.0));
        }
    }
    for cell in grid.cells.iter().filter(|cell| cell.row == 0) {
        let Some(w) = resolve_width(cell.b.style.width, assignable) else {
            continue;
        };
        let border_box = match cell.b.style.box_sizing {
            BoxSizing::BorderBox => w,
            BoxSizing::ContentBox => w + Edges::of(&cell.b.style, assignable).horizontal(),
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
    let remaining = (assignable - set).max(0.0);
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
fn resolve_width(width: LengthPercentageAuto, of: f32) -> Option<f32> {
    match width {
        LengthPercentageAuto::Auto => None,
        LengthPercentageAuto::Px(w) => Some(w),
        LengthPercentageAuto::Percent(p) => Some(of * p / 100.0),
    }
}

/// The `width` a column's boxes give it: its column box's, else its
/// column group's when the column box gives none (or there is none);
/// `None` when neither does.
fn column_box_width(cref: &ColumnRef<'_>) -> Option<LengthPercentageAuto> {
    [cref.column, cref.group]
        .into_iter()
        .flatten()
        .map(|part| part.style.width)
        .find(|w| !w.is_auto())
}

/// The spacing around and between `columns` columns in the separated
/// model, `hspacing` each: none without a column.
fn columns_spacing(columns: usize, hspacing: f32) -> f32 {
    if columns == 0 {
        0.0
    } else {
        (columns + 1) as f32 * hspacing
    }
}

/// Whether the table is laid out by the fixed algorithm: `table-layout:
/// fixed` and a `width` other than `auto` (CSS2 §17.5.2.1; with `auto`,
/// the automatic algorithm is used).
fn uses_fixed_layout(style: &ComputedStyle) -> bool {
    style.table_layout == TableLayout::Fixed && !style.width.is_auto()
}

/// The table's horizontal border spacing, and its vertical: the
/// separated model's `border-spacing`, none in the collapsed model.
fn spacing(style: &ComputedStyle) -> (f32, f32) {
    match style.border_collapse {
        BorderCollapse::Separate => (
            style.border_spacing.horizontal,
            style.border_spacing.vertical,
        ),
        BorderCollapse::Collapse => (0.0, 0.0),
    }
}

/// What a column needs (css-tables-3 §3.9.1): its minimum and maximum
/// widths, the percentage of the table's assignable width it asks for,
/// and whether a length `width` — a cell's or its column box's —
/// constrains it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct ColumnMeasure {
    min: f32,
    max: f32,
    percent: Option<f32>,
    constrained: bool,
}

/// A cell's outer widths (css-tables-3 §3.9.1): its minimum is its
/// content's min-content width, or its `min-width` when larger; its
/// maximum is its `width` when that is a length — which constrains its
/// column, and is not a minimum — else its content's max-content width,
/// never below the minimum; both over its border box, padding and border
/// included, a percentage padding counting as zero as in every intrinsic
/// width. A percentage `width` is the share of the table it asks for.
fn measure_cell(b: &LayoutBox, fonts: &FontCollection) -> ColumnMeasure {
    let style = &b.style;
    let (content_min, content_max) = crate::intrinsic::content_widths(b, fonts);
    let fixed = |v: LengthPercentage| match v {
        LengthPercentage::Px(px) => px,
        LengthPercentage::Percent(_) => 0.0,
    };
    let border = style.used_border_width();
    let edges =
        border[LEFT] + border[RIGHT] + fixed(style.padding[LEFT]) + fixed(style.padding[RIGHT]);
    let border_box = |v: f32| match style.box_sizing {
        BoxSizing::ContentBox => v.max(0.0) + edges,
        BoxSizing::BorderBox => v.max(edges),
    };
    let mut min = content_min + edges;
    if let LengthPercentage::Px(floor) = style.min_width {
        min = min.max(border_box(floor));
    }
    match style.width {
        LengthPercentageAuto::Px(w) => ColumnMeasure {
            min,
            max: min.max(border_box(w)),
            percent: None,
            constrained: true,
        },
        LengthPercentageAuto::Percent(p) => ColumnMeasure {
            min,
            max: min.max(content_max + edges),
            percent: Some(p.max(0.0)),
            constrained: false,
        },
        LengthPercentageAuto::Auto => ColumnMeasure {
            min,
            max: min.max(content_max + edges),
            percent: None,
            constrained: false,
        },
    }
}

/// Every column's measure (css-tables-3 §3.9.1): a column box's length
/// `width` raises the column's maximum and constrains it, a percentage
/// one is its percentage; the cells spanning one column raise its
/// minimum and maximum to theirs, a percentage cell raises its
/// percentage, a cell with a length `width` constrains it. Then the
/// cells spanning several columns, fewest columns first: the part of a
/// cell's minimum beyond its columns' minimums and the spacing between
/// them is given to those columns in proportion to their maximums,
/// equally when all are zero, and its maximum the same way; its
/// percentage, less the percentages its columns already have, is split
/// equally among those that have none. Last, the percentages are cut,
/// in column order, so that together they ask for no more than the
/// whole.
fn measure_columns(grid: &Grid<'_>, hspacing: f32, fonts: &FontCollection) -> Vec<ColumnMeasure> {
    let mut columns = vec![ColumnMeasure::default(); grid.columns];
    for (column, cref) in columns.iter_mut().zip(&grid.column_boxes) {
        match column_box_width(cref) {
            Some(LengthPercentageAuto::Px(w)) => {
                column.max = column.max.max(w.max(0.0));
                column.constrained = true;
            }
            Some(LengthPercentageAuto::Percent(p)) => column.percent = Some(p.max(0.0)),
            Some(LengthPercentageAuto::Auto) | None => {}
        }
    }
    let measures: Vec<ColumnMeasure> = grid
        .cells
        .iter()
        .map(|cell| measure_cell(cell.b, fonts))
        .collect();
    let raise_percent = |column: &mut ColumnMeasure, p: f32| {
        column.percent = Some(column.percent.map_or(p, |q| q.max(p)));
    };
    for (cell, m) in grid.cells.iter().zip(&measures) {
        if cell.colspan != 1 {
            continue;
        }
        let column = &mut columns[cell.col];
        column.min = column.min.max(m.min);
        column.max = column.max.max(m.max);
        column.constrained |= m.constrained;
        if let Some(p) = m.percent {
            raise_percent(column, p);
        }
    }
    for column in &mut columns {
        column.max = column.max.max(column.min);
    }
    let mut spanning: Vec<usize> = (0..grid.cells.len())
        .filter(|&i| grid.cells[i].colspan > 1)
        .collect();
    spanning.sort_by_key(|&i| grid.cells[i].colspan);
    for i in spanning {
        let cell = &grid.cells[i];
        let m = &measures[i];
        let spanned = &mut columns[cell.col..cell.col + cell.colspan];
        let between = (cell.colspan - 1) as f32 * hspacing;
        if let Some(p) = m.percent {
            let have: f32 = spanned.iter().filter_map(|c| c.percent).sum();
            let without = spanned.iter().filter(|c| c.percent.is_none()).count();
            if p > have && without > 0 {
                let each = (p - have) / without as f32;
                for column in spanned.iter_mut().filter(|c| c.percent.is_none()) {
                    column.percent = Some(each);
                }
            }
        }
        let weights: Vec<f32> = spanned.iter().map(|c| c.max).collect();
        let min_have = spanned.iter().map(|c| c.min).sum::<f32>() + between;
        if m.min > min_have {
            for (column, share) in spanned.iter_mut().zip(shares(m.min - min_have, &weights)) {
                column.min += share;
            }
        }
        let max_have = spanned.iter().map(|c| c.max).sum::<f32>() + between;
        if m.max > max_have {
            for (column, share) in spanned.iter_mut().zip(shares(m.max - max_have, &weights)) {
                column.max += share;
            }
        }
        for column in spanned.iter_mut() {
            column.max = column.max.max(column.min);
        }
    }
    let mut total = 0.0f32;
    for column in &mut columns {
        if let Some(p) = column.percent {
            let p = p.min(100.0 - total).max(0.0);
            column.percent = Some(p);
            total += p;
        }
    }
    columns
}

/// `extra` shared in proportion to `weights`, equally when all are zero.
fn shares(extra: f32, weights: &[f32]) -> Vec<f32> {
    let total: f32 = weights.iter().sum();
    if total > 0.0 {
        weights.iter().map(|w| extra * w / total).collect()
    } else if weights.is_empty() {
        Vec::new()
    } else {
        vec![extra / weights.len() as f32; weights.len()]
    }
}

/// The widest a table's assignable width can need to be: what a table
/// whose percentages ask for the whole and whose other columns still
/// want room asks for (Blink's own ceiling on a table's width).
const UNBOUNDED_TABLE_WIDTH: f32 = 1_000_000.0;

/// The grid's minimum and maximum widths, spacing included (css-tables-3
/// §3.9.2): the columns' minimums, and their maximums — raised so that,
/// where it can, each percentage column's maximum is its percentage of
/// the whole, and the other columns' maximums fill what the percentages
/// leave; when the percentages leave nothing and other columns want
/// room, as wide as a table gets.
fn grid_widths(columns: &[ColumnMeasure], hspacing: f32) -> (f32, f32) {
    let spacing = columns_spacing(columns.len(), hspacing);
    let min = columns.iter().map(|c| c.min).sum::<f32>();
    let max = columns.iter().map(|c| c.max).sum::<f32>();
    let total_percent: f32 = columns.iter().filter_map(|c| c.percent).sum();
    let mut needed = 0.0f32;
    let mut others = 0.0f32;
    for column in columns {
        match column.percent {
            Some(p) if p > 0.0 => needed = needed.max(column.max * 100.0 / p),
            Some(_) => {}
            None => others += column.max,
        }
    }
    if others > 0.0 {
        needed = if total_percent >= 100.0 {
            UNBOUNDED_TABLE_WIDTH
        } else {
            needed.max(others * 100.0 / (100.0 - total_percent))
        };
    }
    (min + spacing, max.max(needed).max(min) + spacing)
}

/// The automatic algorithm's column widths for the assignable width
/// `width` (css-tables-3 §3.9.3). Four sizing guesses, each no narrower
/// than the one before: every column at its minimum; then the
/// percentage columns at their percentage of `width`, never below their
/// minimum; then the columns a length constrains at their maximum too;
/// then the other columns at their maximum. A width between two guesses
/// is their linear combination — every column moving the same fraction
/// of the way from one to the next — and a width past the last is that
/// guess with the excess given out: to the unconstrained columns in
/// proportion to their maximums, else to the constrained ones the same
/// way, else to the percentage columns in proportion to their
/// percentages, else equally to the unconstrained columns, to the
/// constrained ones, to all.
fn auto_column_widths(columns: &[ColumnMeasure], width: f32) -> Vec<f32> {
    let percent_width = |c: &ColumnMeasure, p: f32| c.min.max(width * p / 100.0);
    let guesses: [Vec<f32>; 4] = [
        columns.iter().map(|c| c.min).collect(),
        columns
            .iter()
            .map(|c| c.percent.map_or(c.min, |p| percent_width(c, p)))
            .collect(),
        columns
            .iter()
            .map(|c| match c.percent {
                Some(p) => percent_width(c, p),
                None if c.constrained => c.max,
                None => c.min,
            })
            .collect(),
        columns
            .iter()
            .map(|c| c.percent.map_or(c.max, |p| percent_width(c, p)))
            .collect(),
    ];
    let sums: Vec<f32> = guesses.iter().map(|g| g.iter().sum()).collect();
    if width <= sums[0] {
        return guesses[0].clone();
    }
    for k in 0..3 {
        if width <= sums[k + 1] {
            let span = sums[k + 1] - sums[k];
            let t = if span > 0.0 {
                (width - sums[k]) / span
            } else {
                0.0
            };
            return guesses[k]
                .iter()
                .zip(&guesses[k + 1])
                .map(|(a, b)| a + t * (b - a))
                .collect();
        }
    }
    let mut widths = guesses[3].clone();
    let excess = width - sums[3];
    let unconstrained = |c: &ColumnMeasure| c.percent.is_none() && !c.constrained;
    let constrained = |c: &ColumnMeasure| c.percent.is_none() && c.constrained;
    type Pick<'p> = (
        &'p dyn Fn(&ColumnMeasure) -> bool,
        &'p dyn Fn(&ColumnMeasure) -> f32,
    );
    let by_max = |c: &ColumnMeasure| c.max;
    let by_percent = |c: &ColumnMeasure| c.percent.unwrap_or(0.0);
    let equally = |_: &ColumnMeasure| 1.0;
    let any = |_: &ColumnMeasure| true;
    let order: [Pick<'_>; 6] = [
        (&unconstrained, &by_max),
        (&constrained, &by_max),
        (&|c: &ColumnMeasure| c.percent.is_some(), &by_percent),
        (&unconstrained, &equally),
        (&constrained, &equally),
        (&any, &equally),
    ];
    for (pick, weight) in order {
        let weights: Vec<f32> = columns
            .iter()
            .map(|c| if pick(c) { weight(c) } else { 0.0 })
            .collect();
        if weights.iter().sum::<f32>() > 0.0 {
            for (w, share) in widths.iter_mut().zip(shares(excess, &weights)) {
                *w += share;
            }
            break;
        }
    }
    widths
}

/// The widest minimum among the table's captions, over their margin
/// boxes, less the table's own horizontal border and padding
/// `table_edges`: the content width the table needs so that a caption,
/// as wide as the table, holds its content (CAPMIN, CSS2 §17.5.2.2).
fn caption_min(b: &LayoutBox, table_edges: f32, fonts: &FontCollection) -> f32 {
    b.children
        .iter()
        .filter(|part| part.kind == BoxKind::Caption)
        .map(|caption| crate::intrinsic::contribution(caption, fonts).0 - table_edges)
        .fold(0.0, f32::max)
}

/// A table's min-content and max-content widths over its content box —
/// what is inside its border and padding — for the intrinsic sizes of
/// the boxes around it and for its own shrink-to-fit width: the grid's
/// MIN and MAX, never narrower than its captions' minimum (CAPMIN). A
/// table the fixed algorithm lays out measures no cell: it needs the
/// widths its columns are given.
pub(crate) fn content_widths(b: &LayoutBox, fonts: &FontCollection) -> (f32, f32) {
    let style = &b.style;
    let grid = Grid::of(b);
    let (hspacing, _) = spacing(style);
    let (min, max) = if uses_fixed_layout(style) {
        let widths = fixed_column_widths(&grid, 0.0, hspacing);
        let width = widths.iter().sum::<f32>() + columns_spacing(grid.columns, hspacing);
        (width, width)
    } else {
        grid_widths(&measure_columns(&grid, hspacing, fonts), hspacing)
    };
    let fixed = |v: LengthPercentage| match v {
        LengthPercentage::Px(px) => px,
        LengthPercentage::Percent(_) => 0.0,
    };
    let border = style.used_border_width();
    let edges =
        border[LEFT] + border[RIGHT] + fixed(style.padding[LEFT]) + fixed(style.padding[RIGHT]);
    let min = min.max(caption_min(b, edges, fonts));
    (min, max.max(min))
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

/// How a cell's content is placed in it (CSS2 §17.5.3): `top`,
/// `middle` and `bottom` as they say, every other `vertical-align` on
/// the row's baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellAlign {
    Top,
    Middle,
    Bottom,
    Baseline,
}

impl CellAlign {
    fn of(style: &ComputedStyle) -> Self {
        match style.vertical_align {
            VerticalAlign::Top => CellAlign::Top,
            VerticalAlign::Middle => CellAlign::Middle,
            VerticalAlign::Bottom => CellAlign::Bottom,
            _ => CellAlign::Baseline,
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
    /// The border-box height of its content as laid out, padding and
    /// border included: what its alignment places inside its height.
    extent: f32,
    /// From its border box's top to its content's bottom edge.
    content_bottom: f32,
    /// From its border box's top to its baseline: its first line box's,
    /// or a table's in it, else its content's bottom edge (§17.5.3).
    baseline: f32,
    align: CellAlign,
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
    let content_bottom = edges.border[TOP] + edges.padding[TOP] + layout.auto_content_height;
    CellLayout {
        natural_height: content_height + edges.vertical(),
        extent: layout.auto_content_height + edges.vertical(),
        content_bottom,
        baseline: layout.first_baseline.unwrap_or(content_bottom),
        align: CellAlign::of(&b.style),
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
    offsets: &[f32],
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
        // The content moves by the cell's alignment; the absolutely
        // positioned boxes the cell itself places against its padding box
        // do not, and those carried up keep their static positions in
        // the content.
        let offset = offsets[i];
        for child in &mut cell_fragment.children {
            if !child.style.position.is_out_of_flow() {
                child.y += offset;
            }
        }
        absolutes.extend(carried(
            layout.layout.absolutes,
            r.x + dx,
            r.y + dy + offset,
        ));
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
        // a float like any root, shrinking to its content in the room
        // beside it. Nodes: div.f 4, table 5, tr 6, td 7, p 8.
        let layout = document_of(
            "<div class=f></div><table><tr><td><p></p></td></tr></table>",
            "body { margin: 0; } td { padding: 0; } .f { float: left; width: 100px; height: 50px; }
             table { border-spacing: 0; margin-top: 10px; }
             p { margin: 20px 0; height: 10px; width: 50px; }",
        );
        let table = fragments_of(&layout, 5);
        assert_eq!(rect(table[0]), (100.0, 10.0, 50.0, 50.0));
        // The cell is a root too: the p's margins are inside it.
        assert_eq!(rect(fragment_of(&layout, 8)), (100.0, 30.0, 50.0, 10.0));
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

    /// Two cells whose widths are known without text: the first's floats
    /// make its minimum 50 and its maximum 80, the second's block makes
    /// both 40. No spacing, no padding.
    const TWO_CELLS: &str = "<table><tr><td><div class=f50></div><div class=f30></div></td>\
                             <td><div class=b40></div></td></tr></table>";
    const MEASURED: &str = "body { margin: 0; } table { border-spacing: 0; } td { padding: 0; }
        .f50 { float: left; width: 50px; height: 10px; }
        .f30 { float: left; width: 30px; height: 10px; }
        .b40 { width: 40px; height: 10px; }";

    /// The x and the width of the fragments of `nodes`.
    fn columns_of(layout: &DocumentLayout, nodes: &[usize]) -> Vec<(f32, f32)> {
        nodes
            .iter()
            .map(|&n| {
                let f = fragment_of(layout, n);
                (f.x, f.w)
            })
            .collect()
    }

    #[test]
    fn the_automatic_algorithm_fits_the_columns_to_the_width_the_table_gets() {
        // Nodes: table 4, tr 5, td 6, div 7, div 8, td 9, div 10. An
        // `auto` table is as wide as its maximum, 120, each column at
        // its maximum. Given 100px, between the minimums (90) and the
        // maximums (120): every column a third of the way from its
        // minimum to its maximum. Given 60px, below MIN: the table is
        // MIN, each column at its minimum. Given 300px, past MAX: the
        // excess goes in proportion to the maximums, 180 as 120 to 60.
        for (css, table, cells) in [
            ("", 120.0, [(0.0, 80.0), (80.0, 40.0)]),
            (
                "table { width: 100px; }",
                100.0,
                [(0.0, 60.0), (60.0, 40.0)],
            ),
            ("table { width: 60px; }", 90.0, [(0.0, 50.0), (50.0, 40.0)]),
            (
                "table { width: 300px; }",
                300.0,
                [(0.0, 200.0), (200.0, 100.0)],
            ),
        ] {
            let layout = document_of(TWO_CELLS, &format!("{MEASURED} {css}"));
            assert_eq!(fragment_of(&layout, 4).w, table, "{css}");
            assert_eq!(columns_of(&layout, &[6, 9]), cells, "{css}");
        }
    }

    #[test]
    fn percentage_columns_take_their_share_and_raise_the_tables_maximum() {
        // The first cell asks for 25%. In a 400px table: 100px, the rest
        // going to the other column. With an `auto` width the table is
        // as wide as makes the first column's maximum, 80, its 25%: 320.
        // Nodes: table 4, tr 5, td 6, td 9.
        let css = format!("{MEASURED} td:first-child {{ width: 25%; }}");
        let layout = document_of(TWO_CELLS, &format!("{css} table {{ width: 400px; }}"));
        assert_eq!(columns_of(&layout, &[6, 9]), [(0.0, 100.0), (100.0, 300.0)]);
        let layout = document_of(TWO_CELLS, &css);
        assert_eq!(fragment_of(&layout, 4).w, 320.0);
        assert_eq!(columns_of(&layout, &[6, 9]), [(0.0, 80.0), (80.0, 240.0)]);
        // A column asking for the whole while another wants room: the
        // table is as wide as it can be, the room it is given; the other
        // column keeps its minimum.
        let css = format!("{MEASURED} td:first-child {{ width: 100%; }}");
        let layout = document_of(TWO_CELLS, &css);
        assert_eq!(fragment_of(&layout, 4).w, VIEWPORT);
        assert_eq!(columns_of(&layout, &[6, 9]), [(0.0, 760.0), (760.0, 40.0)]);
        // Percentages past the whole are cut in column order: 70% and
        // 60% are 70% and 30%. Nodes: table 4, tr 5, td 6, td 7.
        let layout = document_of(
            "<table><tr><td class=a></td><td class=b></td></tr></table>",
            "body { margin: 0; } table { border-spacing: 0; width: 100px; } td { padding: 0; }
             .a { width: 70%; } .b { width: 60%; }",
        );
        assert_eq!(columns_of(&layout, &[6, 7]), [(0.0, 70.0), (70.0, 30.0)]);
    }

    #[test]
    fn a_cells_length_width_constrains_its_column_without_being_a_minimum() {
        // An empty cell 100px wide and the 40px block's cell: the table
        // is 140 when it can be, and given 120px the constrained column
        // gives way first, down to its minimum, 0, given 30px.
        // Nodes: table 4, tr 5, td 6, td 7.
        let html = "<table><tr><td class=w></td><td><div class=b40></div></td></tr></table>";
        let css = format!("{MEASURED} .w {{ width: 100px; }}");
        for (extra, table, cells) in [
            ("", 140.0, [(0.0, 100.0), (100.0, 40.0)]),
            (
                "table { width: 120px; }",
                120.0,
                [(0.0, 80.0), (80.0, 40.0)],
            ),
            ("table { width: 30px; }", 40.0, [(0.0, 0.0), (0.0, 40.0)]),
        ] {
            let layout = document_of(html, &format!("{css} {extra}"));
            assert_eq!(fragment_of(&layout, 4).w, table, "{extra}");
            assert_eq!(columns_of(&layout, &[6, 7]), cells, "{extra}");
        }
    }

    #[test]
    fn a_spanning_cell_gives_its_excess_to_its_columns_by_their_maximums() {
        // A 200px block spanning two columns over cells of 30 and 10: the
        // 160 beyond them goes 120 and 40, so the columns are 150 and 50.
        // Nodes: table 4, tr 5, td 6, div 7, tr 8, td 9, div 10, td 11.
        let layout = document_of(
            "<table><tr><td colspan=2><div class=b200></div></td></tr>\
             <tr><td><div class=b30></div></td><td><div class=b10></div></td></tr></table>",
            "body { margin: 0; } table { border-spacing: 0; } td { padding: 0; }
             .b200 { width: 200px; } .b30 { width: 30px; } .b10 { width: 10px; }",
        );
        assert_eq!(fragment_of(&layout, 4).w, 200.0);
        assert_eq!(
            columns_of(&layout, &[6, 9, 11]),
            [(0.0, 200.0), (0.0, 150.0), (150.0, 50.0)]
        );
    }

    #[test]
    fn column_boxes_constrain_their_columns_and_a_group_its_columns_without_a_width() {
        // A 60px col over an empty cell; a 70px colgroup over two cols
        // without a width. Nodes: table 4, col 5, tr 6, td 7, td 8, div
        // 9; table 10, colgroup 11, col 12, col 13, tr 14, td 15, td 16.
        let layout = document_of(
            "<table><col class=c60><tr><td></td><td><div class=b40></div></td></tr></table>\
             <table><colgroup class=c70><col><col></colgroup><tr><td></td><td></td></tr></table>",
            &format!("{MEASURED} .c60 {{ width: 60px; }} .c70 {{ width: 70px; }}"),
        );
        assert_eq!(columns_of(&layout, &[7, 8]), [(0.0, 60.0), (60.0, 40.0)]);
        assert_eq!(fragment_of(&layout, 10).w, 140.0);
        assert_eq!(columns_of(&layout, &[15, 16]), [(0.0, 70.0), (70.0, 70.0)]);
    }

    #[test]
    fn auto_margins_centre_a_table_and_a_caption_widens_it() {
        // Nodes: table 4, tr 5, td 6, td 9.
        let layout = document_of(
            TWO_CELLS,
            &format!("{MEASURED} table {{ margin: 0 auto; }}"),
        );
        assert_eq!(rect(fragment_of(&layout, 4)), (340.0, 0.0, 120.0, 10.0));
        // A caption with a 300px block: the table is 300 wide, the 180
        // past its maximum going as 120 to 60. Nodes: table 4, caption
        // 5, div 6, tr 7, td 8, td 11.
        let layout = document_of(
            "<table><caption><div class=b300></div></caption><tr>\
             <td><div class=f50></div><div class=f30></div></td>\
             <td><div class=b40></div></td></tr></table>",
            &format!("{MEASURED} .b300 {{ width: 300px; }}"),
        );
        assert_eq!(fragment_of(&layout, 4).w, 300.0);
        assert_eq!(
            columns_of(&layout, &[8, 11]),
            [(0.0, 200.0), (200.0, 100.0)]
        );
    }

    #[test]
    fn floated_positioned_and_contained_tables_are_grids_at_their_shrink_to_fit_width() {
        // A floated table and an absolutely positioned one are laid out
        // as grids, as wide as their maximum; a float holding a table
        // shrinks to the table's maximum. Nodes: table 4, tr 5, td 6,
        // td 9.
        for css in [
            "table { float: left; }",
            "table { position: absolute; top: 0; }",
        ] {
            let layout = document_of(TWO_CELLS, &format!("{MEASURED} {css}"));
            assert_eq!(fragment_of(&layout, 4).w, 120.0, "{css}");
            assert_eq!(
                columns_of(&layout, &[6, 9]),
                [(0.0, 80.0), (80.0, 40.0)],
                "{css}"
            );
        }
        // Nodes: div 4, table 5, tr 6, td 7, td 10.
        let layout = document_of(
            &format!("<div class=fl>{TWO_CELLS}</div>"),
            &format!("{MEASURED} .fl {{ float: left; }}"),
        );
        assert_eq!(fragment_of(&layout, 4).w, 120.0);
        assert_eq!(columns_of(&layout, &[7, 10]), [(0.0, 80.0), (80.0, 40.0)]);
    }

    #[test]
    fn the_fixed_algorithm_takes_percentages_of_the_width_less_the_spacing() {
        // 210px with 10px spacing leaves 180 for the columns: 50% is 90.
        // Nodes: table 4, tr 5, td 6, td 7.
        let layout = document_of(
            "<table><tr><td class=h></td><td></td></tr></table>",
            "body { margin: 0; } td { padding: 0; }
             table { table-layout: fixed; width: 210px; border-spacing: 10px; }
             .h { width: 50%; }",
        );
        assert_eq!(columns_of(&layout, &[6, 7]), [(10.0, 90.0), (110.0, 90.0)]);
    }

    #[test]
    fn the_sizing_guesses_move_every_column_the_same_fraction() {
        let column = |min: f32, max: f32, percent: Option<f32>, constrained: bool| ColumnMeasure {
            min,
            max,
            percent,
            constrained,
        };
        // A percentage column, a constrained one and a free one. At a
        // width of 100 the guesses sum to 30, 60, 90 and 130, so 100 is a
        // quarter of the way from the third to the fourth; at 45 they sum
        // to 30, 38, 68 and 108 (the percentage is of the width), so 45
        // is 7/30 of the way from the second to the third.
        let columns = [
            column(10.0, 10.0, Some(40.0), false),
            column(10.0, 40.0, None, true),
            column(10.0, 50.0, None, false),
        ];
        assert_eq!(
            auto_column_widths(&columns, 75.0 / 0.75),
            [40.0, 40.0, 20.0]
        );
        assert_eq!(auto_column_widths(&columns, 45.0), [18.0, 17.0, 10.0]);
        assert_eq!(auto_column_widths(&columns, 30.0), [10.0, 10.0, 10.0]);
        // Past the maximum guess: the free column takes the excess.
        assert_eq!(auto_column_widths(&columns, 200.0), [80.0, 40.0, 80.0]);
        // Without a free column, the constrained one; without either,
        // the percentage columns by percentage; without any, all equally.
        let only = [
            column(0.0, 20.0, None, true),
            column(0.0, 0.0, Some(0.0), false),
        ];
        assert_eq!(auto_column_widths(&only, 40.0), [40.0, 0.0]);
        let empty = [column(0.0, 0.0, None, false), column(0.0, 0.0, None, false)];
        assert_eq!(auto_column_widths(&empty, 40.0), [20.0, 20.0]);
        assert_eq!(auto_column_widths(&[], 40.0), Vec::<f32>::new());
    }

    /// Text-free baselines: cells of font size zero, whose lines are as
    /// tall as the inline-blocks on them and whose baseline is those
    /// inline-blocks' bottom.
    const BASELINES: &str = "body { margin: 0; } table { border-spacing: 0; }
        td, .c { padding: 0; font-size: 0; }
        .ib { display: inline-block; width: 10px; }
        .i10 { height: 10px; } .i20 { height: 20px; } .i30 { height: 30px; }
        .i50 { height: 50px; }
        .b5 { height: 5px; } .b10 { height: 10px; } .b25 { height: 25px; } .b60 { height: 60px; }
        .base { vertical-align: baseline; } .top { vertical-align: top; }
        .mid { vertical-align: middle; } .bot { vertical-align: bottom; }";

    #[test]
    fn cells_place_their_content_at_the_top_middle_bottom_or_on_the_rows_baseline() {
        // A 60px row: two cells on its baseline (30 and 10 from their
        // tops, so the second's content moves down 20), and cells at
        // the top, the middle and the bottom. Nodes: table 4, tr 5, td 6,
        // span 7, td 8, span 9, td 10, div 11, td 12, div 13, td 14, div
        // 15, td 16, div 17.
        let layout = document_of(
            "<table><tr><td class=base><span class='ib i30'></span></td>\
             <td class=base><span class='ib i10'></span></td>\
             <td class=top><div class=b5></div></td><td class=mid><div class=b10></div></td>\
             <td class=top><div class=b60></div></td><td class=bot><div class=b10></div></td>\
             </tr></table>",
            BASELINES,
        );
        assert_eq!(fragment_of(&layout, 4).h, 60.0);
        for (node, y) in [
            (7, 0.0),
            (9, 20.0),
            (11, 0.0),
            (13, 25.0),
            (15, 0.0),
            (17, 50.0),
        ] {
            assert_eq!(fragment_of(&layout, node).y, y, "node {node}");
        }
        // Every cell is stretched to the row whatever its alignment.
        for cell in [6, 8, 10, 12, 14, 16] {
            assert_eq!(fragment_of(&layout, cell).h, 60.0, "cell {cell}");
        }
    }

    #[test]
    fn a_cell_moved_down_to_the_baseline_makes_its_row_taller() {
        // The second cell's baseline is 10 and its content 35 tall: on a
        // baseline of 30 it needs 55. Nodes: table 4, tr 5, td 6, span 7,
        // td 8, span 9, div 10.
        let layout = document_of(
            "<table><tr><td class=base><span class='ib i30'></span></td>\
             <td class=base><span class='ib i10'></span><div class=b25></div></td></tr></table>",
            BASELINES,
        );
        assert_eq!(fragment_of(&layout, 4).h, 55.0);
        assert_eq!(fragment_of(&layout, 9).y, 20.0);
        assert_eq!(fragment_of(&layout, 10).y, 30.0);
    }

    #[test]
    fn an_html_cell_is_centred_and_a_css_cell_sits_on_the_baseline() {
        // The UA sheet: 2px spacing, 1px padding, a row group's middle
        // inherited. The short cell's 12px is centred in 42: its content
        // is 15 below the cell's top, at 8 + 2 + 15 + 1. Nodes: table 4,
        // tr 5, td 6, div 7, td 8, div 9.
        let layout = document_of(
            "<table><tr><td><div class=h40></div></td><td><div class=h10></div></td></tr></table>",
            ".h40 { height: 40px; width: 5px; } .h10 { height: 10px; width: 5px; }",
        );
        assert_eq!(fragment_of(&layout, 9).y, 26.0);
        // `display: table-cell` on plain elements: on the baseline, the
        // initial value. Nodes: div 4, div 5, span 6, div 7, span 8.
        let layout = document_of(
            "<div><div class=c><span class='ib i30'></span></div>\
             <div class=c><span class='ib i10'></span></div></div>",
            &format!("{BASELINES} .c {{ display: table-cell; }}"),
        );
        assert_eq!(fragment_of(&layout, 6).y, 0.0);
        assert_eq!(fragment_of(&layout, 8).y, 20.0);
    }

    #[test]
    fn an_inline_table_sits_on_its_line_by_its_first_rows_baseline() {
        // A 50px inline-block and an inline table whose first row's
        // baseline is 20 below its top: the table's top is 30 down the
        // line. One with no row sits by its bottom. Nodes: div 4, span 5,
        // table 6, tr 7, td 8, span 9, tr 10, td 11, span 12; table 13.
        let layout = document_of(
            "<div class=c><span class='ib i50'></span><table class=it>\
             <tr><td><span class='ib i20'></span></td></tr>\
             <tr><td><span class='ib i30'></span></td></tr></table>\
             <table class='it empty'></table></div>",
            &format!(
                "{BASELINES} .c {{ display: block; }} .it {{ display: inline-table; }}
                 .empty {{ width: 10px; height: 20px; }}"
            ),
        );
        let table = fragment_of(&layout, 6);
        assert_eq!((table.y, table.h), (30.0, 50.0));
        assert_eq!(fragment_of(&layout, 13).y, 30.0);
    }

    #[test]
    fn a_table_first_in_a_cell_gives_the_cell_its_first_rows_baseline() {
        // The first cell's baseline is the nested table's first row's,
        // 30; the second cell's content moves down 20 to it. Nodes: table
        // 4, tr 5, td 6, table 7, tr 8, td 9, span 10, td 11, span 12.
        let layout = document_of(
            "<table><tr><td class=base><table><tr><td><span class='ib i30'></span></td></tr></table></td>\
             <td class=base><span class='ib i10'></span></td></tr></table>",
            BASELINES,
        );
        assert_eq!(fragment_of(&layout, 12).y, 20.0);
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
