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
//! row's and gives what it needs beyond the rows it spans to them, as
//! browsers do where the specifications say nothing: those ending in an
//! earlier row first, and of those ending together the shorter, each to
//! the rows after its first where another spanning cell starts, equally,
//! else to the unconstrained rows (no length `height` on the row or a
//! cell in it alone) in proportion to their heights, else to the
//! constrained ones, else to the last row. A row group's length `height`
//! is a minimum for its rows, and the table's own `height` past the
//! rows' sum is given to the body groups (to the header and footer
//! groups when there is none) and in each to its rows: the unconstrained
//! ones, or all when every one is constrained, in proportion to their
//! heights, equally when those are all zero. Every cell is stretched
//! to the rows it spans, and its content placed in it by its alignment:
//! at its top, centred, at its bottom, or on the row's baseline. The
//! table's baseline is its first row's, the lowest cell content's bottom
//! for a row with no cell on its baseline, and an inline table aligns
//! by it.
//!
//! The border models (CSS2 §17.6). In the separated model,
//! `border-spacing` lies between every two cells and between the cells
//! and the table's padding edge, every cell paints its own border, and
//! under `empty-cells: hide` a cell with no content paints neither its
//! border nor its background. In the collapsed model there is no
//! spacing and `empty-cells` does nothing: every segment of every grid
//! line — per column of a horizontal line, per row of a vertical one —
//! is resolved to one border by §17.6.2.1's conflict rules among the
//! cells on either side and, where the line is their edge, the rows,
//! row groups, columns, column groups and the table; each cell holds
//! half of each line on its side, exactly (css-tables-3 §3.7.1.1), which
//! is its border in layout and in its measures; the table has no
//! padding, and its border is half the widest border along each of its
//! outer lines, so that its border box holds them whole; and the
//! resolved borders are painted by fragments of their own, one per
//! segment, the table's, after the cells' backgrounds.
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

use std::sync::Arc;

use style::computed::{
    BOTTOM, BorderCollapse, BorderStyle, BoxSizing, CaptionSide, Color, ComputedStyle, Display,
    EmptyCells, Float, LEFT, LengthPercentage, LengthPercentageAuto, Overflow, Position, RIGHT,
    Size, TOP, TableLayout, VerticalAlign,
};

use crate::block::{
    AdjoiningMargins, BlockLayout, Content, Deferred, Edges, Fragment, Imposed, box_fragment,
    carried, definite_height, height_bounds, layout_block_contents, relative_offset,
    resolve_width_and_margins, settle_absolutes,
};
use crate::box_tree::{BoxKind, LayoutBox, RowGroupKind};
use crate::pass::Pass;

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
    pass: &Pass<'a>,
    imposed: Option<Imposed>,
) -> BlockLayout<'a> {
    let style = &b.style;
    let grid = Grid::of(b);
    let (hspacing, vspacing) = spacing(style);
    let columns = grid.columns;
    // In the collapsed border model, the borders every grid line resolves
    // to, and the part of them each cell holds (CSS2 §17.6.2).
    let collapsed = collapses(style).then(|| collapse_borders(b, &grid));
    let cell_borders = collapsed.as_ref().map(|c| c.cells.as_slice());

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
        let measures = measure_columns(&grid, hspacing, cell_borders, pass);
        let (grid_min, _) = grid_widths(&measures, hspacing);
        let content_width =
            content_width
                .max(grid_min)
                .max(caption_min(b, edges.horizontal(), pass));
        let widths = auto_column_widths(&measures, (content_width - spacing).max(0.0));
        (widths, content_width)
    };

    // Every cell's content, laid out at its columns' width.
    let mut cells: Vec<CellLayout<'a>> = Vec::with_capacity(grid.cells.len());
    for (i, cell) in grid.cells.iter().enumerate() {
        let span_width = widths[cell.col..cell.col + cell.colspan]
            .iter()
            .sum::<f32>()
            + (cell.colspan - 1) as f32 * hspacing;
        let border = cell_borders.map(|borders| borders[i]);
        cells.push(layout_cell(cell.b, span_width, content_width, border, pass));
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
    // A row is constrained when its own `height`, or that of a cell in
    // it alone, is a length; the rows where a cell spanning rows starts
    // take another spanning cell's excess first.
    let mut heights: Vec<f32> = grid
        .rows
        .iter()
        .map(|row| match row.b.style.height {
            Size::Px(h) => h.max(0.0),
            _ => 0.0,
        })
        .collect();
    let constrained: Vec<bool> = grid
        .rows
        .iter()
        .zip(&grid.row_cells)
        .map(|(row, own)| {
            is_length(&row.b.style.height)
                || own.clone().any(|i| {
                    let cell = &grid.cells[i];
                    cell.rowspan == 1 && is_length(&cell.b.style.height)
                })
        })
        .collect();
    let starts_span: Vec<bool> = grid
        .row_cells
        .iter()
        .map(|own| own.clone().any(|i| grid.cells[i].rowspan > 1))
        .collect();
    for (cell, layout) in grid.cells.iter().zip(&cells) {
        if cell.rowspan == 1 {
            heights[cell.row] = heights[cell.row].max(needed(cell, layout));
        }
    }
    // The spanning cells: those ending in an earlier row first and, of
    // those ending in the same row, those spanning fewer rows.
    let mut spanning: Vec<(&GridCell<'a>, &CellLayout<'a>)> = grid
        .cells
        .iter()
        .zip(&cells)
        .filter(|(cell, _)| cell.rowspan > 1)
        .collect();
    spanning.sort_by_key(|(cell, _)| (cell.row + cell.rowspan, cell.rowspan));
    for (cell, layout) in spanning {
        let span = cell.row..cell.row + cell.rowspan;
        let have = heights[span.clone()].iter().sum::<f32>() + (cell.rowspan - 1) as f32 * vspacing;
        let wants = needed(cell, layout);
        if wants > have {
            distribute_rowspan(
                wants - have,
                &mut heights[span.clone()],
                &constrained[span.clone()],
                &starts_span[span],
            );
        }
    }
    // A row group's own `height` is a minimum for its rows together.
    let group_extent = |heights: &[f32], group: &GridGroup<'_>| {
        heights[group.span()].iter().sum::<f32>() + group.rows.saturating_sub(1) as f32 * vspacing
    };
    for group in grid.groups.iter().filter(|g| g.rows > 0) {
        let Some(&Size::Px(h)) = group.b.map(|g| &g.style.height) else {
            continue;
        };
        let have = group_extent(&heights, group);
        if h > have {
            distribute_height(
                h - have,
                &mut heights[group.span()],
                &constrained[group.span()],
            );
        }
    }
    // The table's `height` is a minimum for the rows together. What it
    // adds goes to the body groups, the rows directly in the table being
    // one, or to the header and footer groups when there is none; among
    // the groups as among rows, a group with a length `height` being
    // constrained; and in each group to its rows.
    let rows_height = if heights.is_empty() {
        0.0
    } else {
        heights.iter().sum::<f32>() + (heights.len() + 1) as f32 * vspacing
    };
    // A height imposed on the table (M17: a stretched or flexed flex
    // item's) is a minimum in place of its own `height` and
    // `min-height`, which the flex layout has already applied: they are
    // no second minimum (css-flexbox-1 §9.8, as the reference browsers
    // have it). A keyword in `min-height` is the rows' height, the
    // table's automatic height.
    let wanted = match imposed {
        Some(imposed) => imposed.height - edges.vertical(),
        None => {
            let specified = definite_height(style, cb_height, edges.vertical());
            let bounds = height_bounds(style, cb_height, edges.vertical(), Some(rows_height));
            specified.unwrap_or(0.0).max(bounds.0)
        }
    };
    if wanted > rows_height && !heights.is_empty() {
        let with_rows = || grid.groups.iter().filter(|g| g.rows > 0);
        let mut groups: Vec<&GridGroup<'_>> = with_rows().filter(|g| g.is_body()).collect();
        if groups.is_empty() {
            groups = with_rows().collect();
        }
        let before: Vec<f32> = groups.iter().map(|g| group_extent(&heights, g)).collect();
        let groups_constrained: Vec<bool> = groups
            .iter()
            .map(|g| g.b.is_some_and(|b| is_length(&b.style.height)))
            .collect();
        let mut after = before.clone();
        distribute_height(wanted - rows_height, &mut after, &groups_constrained);
        for ((group, before), after) in groups.iter().zip(&before).zip(&after) {
            distribute_height(
                after - before,
                &mut heights[group.span()],
                &constrained[group.span()],
            );
        }
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
    table_box.style = inner_style(style).into();
    if collapsed.is_some() {
        // Its border is the grid's outer lines', painted as segments.
        Arc::make_mut(&mut table_box.style).border_style = [BorderStyle::None; 4];
    }
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
        Arc::make_mut(&mut fragment.style).position = Position::Static;
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
                Arc::make_mut(&mut column_fragment.style).position = Position::Static;
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
                // The rows' positioned boxes, in the table box's
                // coordinates, settled at the group when it is a
                // containing block for them, a transformed one (M19).
                let mut group_absolutes = Vec::new();
                for row in &part.children {
                    let mut row_fragment = row_fragment(
                        row,
                        row_index,
                        &grid,
                        &mut cells,
                        &offsets,
                        collapsed.is_some(),
                        &row_rect,
                        &cell_rect,
                        &mut group_absolutes,
                        pass,
                    );
                    row_fragment.x -= group.x;
                    row_fragment.y -= group.y;
                    group.children.push(row_fragment);
                    row_index += 1;
                }
                let mine = carried(group_absolutes, -group.x, -group.y);
                let up = settle_absolutes(part, &mut group, &[0.0; 4], mine, pass);
                cell_absolutes.extend(carried(up, group.x, group.y));
                table_box.children.push(group);
            }
            BoxKind::Row => {
                let fragment = row_fragment(
                    part,
                    row_index,
                    &grid,
                    &mut cells,
                    &offsets,
                    collapsed.is_some(),
                    &row_rect,
                    &cell_rect,
                    &mut cell_absolutes,
                    pass,
                );
                table_box.children.push(fragment);
                row_index += 1;
            }
            _ => {}
        }
    }

    // The collapsed borders, last, over the cells' backgrounds: one
    // fragment per segment, the table's own.
    if let Some(collapsed) = collapsed
        .as_ref()
        .filter(|_| columns > 0 && !heights.is_empty())
    {
        let line_x: Vec<f32> = (0..=columns)
            .map(|v| match col_x.get(v) {
                Some(x) => *x,
                None => col_x[columns - 1] + widths[columns - 1],
            })
            .collect();
        let line_y: Vec<f32> = (0..=heights.len())
            .map(|l| match row_y.get(l) {
                Some(y) => *y,
                None => row_y[l - 1] + heights[l - 1],
            })
            .collect();
        table_box
            .children
            .extend(collapsed.segments(b, &line_x, &line_y));
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
        let mut layout = layout_caption(part, table_width, pass);
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
        // A positioned or a transformed caption is a containing block
        // for the positioned boxes in it (M19).
        let caption_border = Edges::of(&part.style, table_width).border;
        let mine = std::mem::take(&mut layout.absolutes);
        let up = settle_absolutes(part, &mut layout.root, &caption_border, mine, pass);
        caption_absolutes.extend(carried(up, x, y + below));
        stack.push(layout.root.translated(x, y));
        *height = y + layout.height + margin_bottom;
    }

    // The wrapper: the captions and the table box stacked, the table's
    // margins around them all. The bottom captions' positions, and the
    // absolutely positioned boxes they carry, move below the table box.
    let wrapper_height = top_height + table_height + bottom_height;
    let mut root = box_fragment(table_width, wrapper_height, b);
    root.style = wrapper_style(style).into();
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
    let absolutes = settle_absolutes(b, &mut root, border, table_absolutes, pass);

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
        // takes as its last, an inline-block's included.
        first_baseline: first_row_baseline.map(|b| top_height + b),
        first_line: None,
        last_baseline: last_row_baseline.map(|b| top_height + b),
        inline_block_baseline: last_row_baseline.map(|b| top_height + b),
        absolutes,
        // As with `height: auto`: the captions and the rows, the table's
        // own `height` and anything imposed left out — what a flex item
        // or a keyword limit takes as the table's content (M17).
        auto_content_height: top_height + rows_height + bottom_height,
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
    groups: Vec<GridGroup<'a>>,
}

struct GridRow<'a> {
    b: &'a LayoutBox,
    group: usize,
}

/// A row group, or the run of consecutive rows directly in the table,
/// which act as one for a `rowspan` reaching past them and have no box.
struct GridGroup<'a> {
    b: Option<&'a LayoutBox>,
    first_row: usize,
    rows: usize,
}

impl GridGroup<'_> {
    /// Its rows' indices.
    fn span(&self) -> std::ops::Range<usize> {
        self.first_row..self.first_row + self.rows
    }

    /// Whether it is a body group: a `tbody`, or rows directly in the
    /// table, as the HTML parser's implied `tbody` would hold them.
    fn is_body(&self) -> bool {
        self.b
            .is_none_or(|b| b.kind == BoxKind::RowGroup(RowGroupKind::Body))
    }
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
        let mut groups: Vec<GridGroup<'a>> = Vec::new();
        let mut rows: Vec<GridRow<'a>> = Vec::new();
        let mut implicit: Option<usize> = None;
        for part in &table.children {
            match part.kind {
                BoxKind::RowGroup(_) => {
                    implicit = None;
                    let g = groups.len();
                    groups.push(GridGroup {
                        b: Some(part),
                        first_row: rows.len(),
                        rows: part.children.len(),
                    });
                    rows.extend(part.children.iter().map(|row| GridRow { b: row, group: g }));
                }
                BoxKind::Row => {
                    let g = *implicit.get_or_insert_with(|| {
                        groups.push(GridGroup {
                            b: None,
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
            groups,
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
        if let Some(w) = column_box_width(cref).and_then(|w| resolve_width(&w, assignable)) {
            widths[c] = Some(w.max(0.0));
        }
    }
    for cell in grid.cells.iter().filter(|cell| cell.row == 0) {
        let Some(w) = resolve_width(&cell.b.style.width, assignable) else {
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

/// A `width` as a length, a percentage of `of`, or `None` for `auto`, an
/// intrinsic keyword or a `calc()` with a percentage in it, which the
/// table algorithms take as `auto` (M21, as `measure_cell` does and the
/// reference browser in either algorithm: calc-width-table-fixed-1).
fn resolve_width(width: &Size, of: f32) -> Option<f32> {
    match width {
        Size::Calc(_) => None,
        width => width.length_percentage().map(|w| w.resolve(of)),
    }
}

/// The `width` a column's boxes give it: its column box's, else its
/// column group's when the column box gives none (or there is none);
/// `None` when neither does.
fn column_box_width(cref: &ColumnRef<'_>) -> Option<Size> {
    [cref.column, cref.group]
        .into_iter()
        .flatten()
        .map(|part| &part.style.width)
        .find(|w| resolve_width(w, 0.0).is_some())
        .cloned()
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
    style.table_layout == TableLayout::Fixed && style.width.length_percentage().is_some()
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
/// In the collapsed border model its border is its share of the grid
/// lines, which a column counts where the separated model counts
/// spacing.
fn measure_cell<'a>(
    b: &'a LayoutBox,
    collapsed_border: Option<[f32; 4]>,
    pass: &Pass<'a>,
) -> ColumnMeasure {
    let style = &b.style;
    let (content_min, content_max) = crate::intrinsic::content_widths(b, pass);
    // A percentage padding is a percentage of zero in a measure, as in
    // any contribution (`intrinsic::fixed_edges`).
    let fixed = |v: &LengthPercentage| v.resolve(0.0);
    let border = collapsed_border.unwrap_or_else(|| style.used_border_width());
    let edges =
        border[LEFT] + border[RIGHT] + fixed(&style.padding[LEFT]) + fixed(&style.padding[RIGHT]);
    let border_box = |v: f32| match style.box_sizing {
        BoxSizing::ContentBox => v.max(0.0) + edges,
        BoxSizing::BorderBox => v.max(edges),
    };
    let mut min = content_min + edges;
    if let Size::Px(floor) = style.min_width {
        min = min.max(border_box(floor));
    }
    match &style.width {
        Size::Px(w) => ColumnMeasure {
            min,
            max: min.max(border_box(*w)),
            percent: None,
            constrained: true,
        },
        Size::Percent(p) => ColumnMeasure {
            min,
            max: min.max(content_max + edges),
            percent: Some(p.max(0.0)),
            constrained: false,
        },
        // A `calc()` with a percentage in it is no width a column can
        // take: the column measures as for `auto`.
        Size::Auto | Size::Calc(_) | Size::MinContent | Size::MaxContent | Size::FitContent => {
            ColumnMeasure {
                min,
                max: min.max(content_max + edges),
                percent: None,
                constrained: false,
            }
        }
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
fn measure_columns<'a>(
    grid: &Grid<'a>,
    hspacing: f32,
    cell_borders: Option<&[[f32; 4]]>,
    pass: &Pass<'a>,
) -> Vec<ColumnMeasure> {
    let mut columns = vec![ColumnMeasure::default(); grid.columns];
    for (column, cref) in columns.iter_mut().zip(&grid.column_boxes) {
        match column_box_width(cref) {
            Some(Size::Px(w)) => {
                column.max = column.max.max(w.max(0.0));
                column.constrained = true;
            }
            Some(Size::Percent(p)) => column.percent = Some(p.max(0.0)),
            Some(_) | None => {}
        }
    }
    let measures: Vec<ColumnMeasure> = grid
        .cells
        .iter()
        .enumerate()
        .map(|(i, cell)| measure_cell(cell.b, cell_borders.map(|b| b[i]), pass))
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
fn caption_min<'a>(b: &'a LayoutBox, table_edges: f32, pass: &Pass<'a>) -> f32 {
    b.children
        .iter()
        .filter(|part| part.kind == BoxKind::Caption)
        .map(|caption| crate::intrinsic::contribution(caption, pass).0 - table_edges)
        .fold(0.0, f32::max)
}

/// A table's min-content and max-content widths over its content box —
/// what is inside its border and padding — for the intrinsic sizes of
/// the boxes around it and for its own shrink-to-fit width: the grid's
/// MIN and MAX, never narrower than its captions' minimum (CAPMIN). A
/// table the fixed algorithm lays out measures no cell: it needs the
/// widths its columns are given.
pub(crate) fn content_widths<'a>(b: &'a LayoutBox, pass: &Pass<'a>) -> (f32, f32) {
    let style = &b.style;
    let grid = Grid::of(b);
    let (hspacing, _) = spacing(style);
    let (min, max) = if uses_fixed_layout(style) {
        let widths = fixed_column_widths(&grid, 0.0, hspacing);
        let width = widths.iter().sum::<f32>() + columns_spacing(grid.columns, hspacing);
        (width, width)
    } else {
        let collapsed = collapses(style).then(|| collapse_borders(b, &grid));
        let cell_borders = collapsed.as_ref().map(|c| c.cells.as_slice());
        grid_widths(
            &measure_columns(&grid, hspacing, cell_borders, pass),
            hspacing,
        )
    };
    let fixed = |v: &LengthPercentage| v.resolve(0.0);
    let border = style.used_border_width();
    let edges = if collapses(style) {
        let border = collapsed_table_border(b);
        border[LEFT] + border[RIGHT]
    } else {
        border[LEFT] + border[RIGHT] + fixed(&style.padding[LEFT]) + fixed(&style.padding[RIGHT])
    };
    let min = min.max(caption_min(b, edges, pass));
    (min, max.max(min))
}

/// Whether a size is a length, which makes a row or a row group
/// constrained.
fn is_length(size: &Size) -> bool {
    matches!(size, Size::Px(_))
}

/// Gives `extra`, what a cell spanning `rows` needs beyond their heights
/// and the spacing between them, to those rows as browsers do, which
/// css-tables-3 leaves open: to the rows after the first where another
/// cell spanning rows starts (`starts_span`), equally; else to the rows
/// not `constrained` in proportion to their heights; else to the
/// constrained ones the same way; and, every row being of no height, to
/// the last.
fn distribute_rowspan(extra: f32, rows: &mut [f32], constrained: &[bool], starts_span: &[bool]) {
    if extra <= 0.0 || rows.is_empty() {
        return;
    }
    let starting = starts_span.iter().skip(1).filter(|&&s| s).count();
    if starting > 0 {
        let each = extra / starting as f32;
        for (row, _) in rows
            .iter_mut()
            .zip(starts_span)
            .skip(1)
            .filter(|(_, s)| **s)
        {
            *row += each;
        }
        return;
    }
    for taking in [false, true] {
        let total: f32 = rows
            .iter()
            .zip(constrained)
            .filter(|(_, c)| **c == taking)
            .map(|(h, _)| *h)
            .sum();
        if total > 0.0 {
            for (row, _) in rows
                .iter_mut()
                .zip(constrained)
                .filter(|(_, c)| **c == taking)
            {
                *row += extra * *row / total;
            }
            return;
        }
    }
    if let Some(last) = rows.last_mut() {
        *last += extra;
    }
}

/// Gives `extra` to `parts`, rows or row groups, as browsers share out a
/// table's or a row group's height, which is not css-tables-3 §3.10.5's
/// equal increments: to the parts not `constrained`, or to all of them
/// when every one is, in proportion to their sizes, equally when those
/// are all zero.
fn distribute_height(extra: f32, parts: &mut [f32], constrained: &[bool]) {
    if extra <= 0.0 || parts.is_empty() {
        return;
    }
    let all = constrained.iter().all(|&c| c);
    let takes = |c: &bool| all || !*c;
    let (total, count) = parts
        .iter()
        .zip(constrained)
        .filter(|(_, c)| takes(c))
        .fold((0.0, 0usize), |(total, count), (p, _)| {
            (total + p, count + 1)
        });
    for (part, _) in parts.iter_mut().zip(constrained).filter(|(_, c)| takes(c)) {
        *part += if total > 0.0 {
            extra * *part / total
        } else {
            extra / count as f32
        };
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
    /// The cell's border widths, the collapsed ones in that model: what
    /// its padding box is inset by, against which it settles the
    /// positioned boxes it contains (M19).
    border: [f32; 4],
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
/// percentages of `table_width`; in the collapsed border model its
/// border is `collapsed_border`, its share of the grid lines around it.
fn layout_cell<'a>(
    b: &'a LayoutBox,
    span_width: f32,
    table_width: f32,
    collapsed_border: Option<[f32; 4]>,
    pass: &Pass<'a>,
) -> CellLayout<'a> {
    let mut edges = Edges::of(&b.style, table_width);
    if let Some(border) = collapsed_border {
        edges.border = border;
    }
    let content_width = (span_width - edges.horizontal()).max(0.0);
    let layout = layout_block_contents(
        b,
        &edges,
        table_width,
        None,
        content_width,
        0.0,
        Content::Root,
        pass,
        None,
        None,
    );
    let specified = definite_height(&b.style, None, edges.vertical()).unwrap_or(0.0);
    let (min_height, _) = height_bounds(
        &b.style,
        None,
        edges.vertical(),
        Some(layout.auto_content_height),
    );
    let content_height = layout.auto_content_height.max(specified).max(min_height);
    let content_bottom = edges.border[TOP] + edges.padding[TOP] + layout.auto_content_height;
    CellLayout {
        border: edges.border,
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
fn layout_caption<'a>(b: &'a LayoutBox, wrapper_width: f32, pass: &Pass<'a>) -> BlockLayout<'a> {
    let edges = Edges::of(&b.style, wrapper_width);
    let (content_width, margin_left, _) =
        resolve_width_and_margins(b, wrapper_width, wrapper_width, edges.horizontal(), pass);
    layout_block_contents(
        b,
        &edges,
        wrapper_width,
        None,
        content_width,
        margin_left,
        Content::Root,
        pass,
        None,
        None,
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
    collapsed: bool,
    row_rect: &dyn Fn(usize) -> Rect,
    cell_rect: &dyn Fn(&GridCell<'_>) -> Rect,
    absolutes: &mut Vec<Deferred<'a>>,
    pass: &Pass<'a>,
) -> Fragment {
    let rect = row_rect(index);
    let mut fragment = part_fragment(row, rect);
    // The positioned boxes the cells hand up, relative to the row.
    let mut row_absolutes: Vec<Deferred<'a>> = Vec::new();
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
        if collapsed {
            // Its borders are the grid lines', painted as segments.
            Arc::make_mut(&mut cell_fragment.style).border_style = [BorderStyle::None; 4];
        } else if cell.b.style.empty_cells == EmptyCells::Hide && is_empty_cell(cell.b) {
            // `empty-cells: hide` (CSS2 §17.6.1.1): an empty cell paints
            // neither its border nor its background.
            let style = Arc::make_mut(&mut cell_fragment.style);
            style.border_style = [BorderStyle::None; 4];
            style.background_color = Color::TRANSPARENT;
        }
        let (dx, dy) = layout.relative;
        cell_fragment.x = r.x - rect.x + dx;
        cell_fragment.y = r.y - rect.y + dy;
        // The content moves by the cell's alignment, and so do the
        // static positions of the positioned boxes in it; a cell that
        // is a containing block for them, a positioned or a transformed
        // one (M19), settles them against its padding box, at its full
        // height, and the rest go up through the row.
        let offset = offsets[i];
        for child in &mut cell_fragment.children {
            if !child.style.position.is_out_of_flow() {
                child.y += offset;
            }
        }
        let mine = carried(layout.layout.absolutes, 0.0, offset);
        let up = settle_absolutes(cell.b, &mut cell_fragment, &layout.border, mine, pass);
        row_absolutes.extend(carried(up, r.x - rect.x + dx, r.y - rect.y + dy));
        fragment.children.push(cell_fragment);
    }
    // A transformed row is a containing block too (M19); it has no
    // border of its own.
    let up = settle_absolutes(row, &mut fragment, &[0.0; 4], row_absolutes, pass);
    absolutes.extend(carried(up, rect.x, rect.y));
    fragment
}

/// The fragment of a table part that paints nothing of its own — a
/// row, a row group, a column, a column group, the table box's wrapper:
/// its style with no background and no border, over `rect`.
fn part_fragment(part: &LayoutBox, rect: Rect) -> Fragment {
    let mut fragment = box_fragment(rect.w, rect.h, part);
    fragment.x = rect.x;
    fragment.y = rect.y;
    Arc::make_mut(&mut fragment.style).background_color = Color::TRANSPARENT;
    Arc::make_mut(&mut fragment.style).border_style = [BorderStyle::None; 4];
    fragment
}

/// A table part's background under one cell: a fragment of the part's
/// style with no border, over the cell's border box, naming the part.
fn background_fragment(part: &LayoutBox, rect: Rect) -> Fragment {
    let mut fragment = box_fragment(rect.w, rect.h, part);
    fragment.x = rect.x;
    fragment.y = rect.y;
    Arc::make_mut(&mut fragment.style).border_style = [BorderStyle::None; 4];
    Arc::make_mut(&mut fragment.style).position = Position::Static;
    Arc::make_mut(&mut fragment.style).float = Float::None;
    Arc::make_mut(&mut fragment.style).z_index = None;
    fragment
}

/// Whether a cell is empty for `empty-cells` (CSS2 §17.6.1.1): it has
/// no in-flow or floating content, white space collapsed away being
/// none; an absolutely positioned box alone leaves it empty.
fn is_empty_cell(b: &LayoutBox) -> bool {
    b.children.iter().all(|c| c.kind == BoxKind::Absolute)
}

/// Whether a table's borders collapse (CSS2 §17.6.2): `border-collapse:
/// collapse` on a table. Such a table has no padding, and its border in
/// layout is [`collapsed_table_border`]'s: half of each outer line's
/// widest border, the other half its cells', so its border box holds its
/// outer lines whole.
pub(crate) fn collapses(style: &ComputedStyle) -> bool {
    matches!(style.display, Display::Table | Display::InlineTable)
        && style.border_collapse == BorderCollapse::Collapse
}

/// One border taking part in the conflict on a segment of a grid line
/// (CSS2 §17.6.2.1), and, once it has won, the border the segment is.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Candidate {
    /// The used width: zero for `none`, whole pixels otherwise.
    width: f32,
    style: BorderStyle,
    color: Color,
    /// Whether `color` is the owner's `currentcolor` (M33).
    current: bool,
    /// What the border belongs to, the most specific first: a cell 0, a
    /// row 1, a row group 2, a column 3, a column group 4, the table 5.
    owner: u8,
    /// Between two borders of the same kind, 0 for the one further left
    /// or further up.
    order: u8,
}

impl Candidate {
    fn of(b: &LayoutBox, side: usize, owner: u8, order: u8) -> Self {
        Candidate {
            width: b.style.used_border_width()[side],
            style: b.style.border_style[side],
            color: b.style.border_color[side],
            current: b.style.border_color_is_current(side),
            owner,
            order,
        }
    }

    /// How a border ranks among those of the same width (§17.6.2.1:
    /// `double`, `solid`, `dashed`, `dotted`, `ridge`, `outset`,
    /// `groove`, `inset`).
    fn style_rank(&self) -> u8 {
        match self.style {
            BorderStyle::Double => 8,
            BorderStyle::Solid => 7,
            BorderStyle::Dashed => 6,
            BorderStyle::Dotted => 5,
            BorderStyle::Ridge => 4,
            BorderStyle::Outset => 3,
            BorderStyle::Groove => 2,
            BorderStyle::Inset => 1,
            BorderStyle::None | BorderStyle::Hidden => 0,
        }
    }

    /// Whether `self` loses to `other`: the wider wins, then the style
    /// higher in the list, then the more specific owner, then the one
    /// further left or up.
    fn precedence(&self, other: &Self) -> std::cmp::Ordering {
        self.width
            .total_cmp(&other.width)
            .then(self.style_rank().cmp(&other.style_rank()))
            .then(other.owner.cmp(&self.owner))
            .then(other.order.cmp(&self.order))
    }
}

/// CSS2 §17.6.2.1's conflict resolution: a `hidden` border wins over
/// everything and leaves no border; `none` loses to everything, and no
/// border is left when every border is `none` (or of no width); among
/// the others, [`Candidate::precedence`] decides.
fn resolve(candidates: &[Candidate]) -> Option<Candidate> {
    if candidates.iter().any(|c| c.style == BorderStyle::Hidden) {
        return None;
    }
    candidates
        .iter()
        .filter(|c| c.style != BorderStyle::None && c.width > 0.0)
        .copied()
        .max_by(Candidate::precedence)
}

/// The part of a grid line's border on either side of it: half of it,
/// exactly, as css-tables-3 §3.7.1.1 divides every collapsed border by
/// two; the odd pixel of a line falls where painting snaps it.
fn half(width: f32) -> f32 {
    width / 2.0
}

/// The borders of a table in the collapsed model.
struct CollapsedBorders {
    /// Per horizontal grid line, top to bottom (rows + 1), per column:
    /// its border, `None` for no border or inside a cell spanning rows.
    horizontal: Vec<Vec<Option<Candidate>>>,
    /// Per vertical grid line, left to right (columns + 1), per row.
    vertical: Vec<Vec<Option<Candidate>>>,
    /// Per cell, in the grid's order, the border widths its border box
    /// holds, top, right, bottom and left: on each side the part of the
    /// line on the cell's side, the widest along the side where the line
    /// changes under a spanning cell.
    cells: Vec<[f32; 4]>,
    /// The table's own border widths, top, right, bottom and left: half
    /// the widest border along each of its outer lines (css-tables-3
    /// §3.7.1.1), so that every outer line lies inside its border box;
    /// none for a grid without rows or columns, which has no lines.
    table: [f32; 4],
}

/// Resolves every segment of every grid line of `table` (CSS2 §17.6.2):
/// the borders of the cells on either side of it, and, where the line
/// is their edge, of the rows, the row groups, the columns, the column
/// groups and the table.
fn collapse_borders(table: &LayoutBox, grid: &Grid<'_>) -> CollapsedBorders {
    let (rows, columns) = (grid.rows.len(), grid.columns);
    let mut slots = vec![vec![u32::MAX; columns]; rows];
    for (i, cell) in grid.cells.iter().enumerate() {
        for row in &mut slots[cell.row..cell.row + cell.rowspan] {
            for slot in &mut row[cell.col..cell.col + cell.colspan] {
                *slot = i as u32;
            }
        }
    }
    let at = |r: usize, c: usize| -> Option<usize> {
        let i = slots[r][c];
        (i != u32::MAX).then_some(i as usize)
    };
    // Whether the column box (or group) over column `c` starts or ends
    // there: a box spanning columns has its sides at its outer ones.
    let same = |a: Option<&LayoutBox>, b: Option<&LayoutBox>| match (a, b) {
        (Some(a), Some(b)) => std::ptr::eq(a, b),
        _ => false,
    };
    let column_part = |c: usize, group: bool| {
        let cref = &grid.column_boxes[c];
        if group { cref.group } else { cref.column }
    };
    let starts =
        |c: usize, group: bool| c == 0 || !same(column_part(c - 1, group), column_part(c, group));
    let ends = |c: usize, group: bool| {
        c + 1 == columns || !same(column_part(c + 1, group), column_part(c, group))
    };

    let mut horizontal = vec![vec![None; columns]; rows + 1];
    let mut vertical = vec![vec![None; rows]; columns + 1];
    let mut candidates = Vec::with_capacity(8);
    for (l, line) in horizontal.iter_mut().enumerate() {
        for (c, segment) in line.iter_mut().enumerate() {
            let above = if l > 0 { at(l - 1, c) } else { None };
            let below = if l < rows { at(l, c) } else { None };
            if above.is_some() && above == below {
                continue; // inside a cell spanning rows
            }
            candidates.clear();
            if let Some(i) = above {
                candidates.push(Candidate::of(grid.cells[i].b, BOTTOM, 0, 0));
            }
            if let Some(i) = below {
                candidates.push(Candidate::of(grid.cells[i].b, TOP, 0, 1));
            }
            if l > 0 {
                let row = &grid.rows[l - 1];
                candidates.push(Candidate::of(row.b, BOTTOM, 1, 0));
                let group = &grid.groups[row.group];
                if let Some(g) = group.b.filter(|_| l == group.first_row + group.rows) {
                    candidates.push(Candidate::of(g, BOTTOM, 2, 0));
                }
            }
            if l < rows {
                let row = &grid.rows[l];
                candidates.push(Candidate::of(row.b, TOP, 1, 1));
                let group = &grid.groups[row.group];
                if let Some(g) = group.b.filter(|_| l == group.first_row) {
                    candidates.push(Candidate::of(g, TOP, 2, 1));
                }
            }
            if l == 0 || l == rows {
                let side = if l == 0 { TOP } else { BOTTOM };
                if let Some(column) = column_part(c, false) {
                    candidates.push(Candidate::of(column, side, 3, 0));
                }
                if let Some(group) = column_part(c, true) {
                    candidates.push(Candidate::of(group, side, 4, 0));
                }
                candidates.push(Candidate::of(table, side, 5, 0));
            }
            *segment = resolve(&candidates);
        }
    }
    for (v, line) in vertical.iter_mut().enumerate() {
        for (r, segment) in line.iter_mut().enumerate() {
            let left = if v > 0 { at(r, v - 1) } else { None };
            let right = if v < columns { at(r, v) } else { None };
            if left.is_some() && left == right {
                continue; // inside a cell spanning columns
            }
            candidates.clear();
            if let Some(i) = left {
                candidates.push(Candidate::of(grid.cells[i].b, RIGHT, 0, 0));
            }
            if let Some(i) = right {
                candidates.push(Candidate::of(grid.cells[i].b, LEFT, 0, 1));
            }
            if v == 0 || v == columns {
                let side = if v == 0 { LEFT } else { RIGHT };
                let row = &grid.rows[r];
                candidates.push(Candidate::of(row.b, side, 1, 0));
                if let Some(g) = grid.groups[row.group].b {
                    candidates.push(Candidate::of(g, side, 2, 0));
                }
            }
            for (group, owner) in [(false, 3), (true, 4)] {
                if v > 0
                    && ends(v - 1, group)
                    && let Some(part) = column_part(v - 1, group)
                {
                    candidates.push(Candidate::of(part, RIGHT, owner, 0));
                }
                if v < columns
                    && starts(v, group)
                    && let Some(part) = column_part(v, group)
                {
                    candidates.push(Candidate::of(part, LEFT, owner, 1));
                }
            }
            if v == 0 || v == columns {
                let side = if v == 0 { LEFT } else { RIGHT };
                candidates.push(Candidate::of(table, side, 5, 0));
            }
            *segment = resolve(&candidates);
        }
    }

    let width = |b: &Option<Candidate>| b.map_or(0.0, |b| b.width);
    let widest_half =
        |line: &[Option<Candidate>]| line.iter().map(|b| half(width(b))).fold(0.0, f32::max);
    let table = if rows == 0 || columns == 0 {
        [0.0; 4]
    } else {
        [
            widest_half(&horizontal[0]),
            widest_half(&vertical[columns]),
            widest_half(&horizontal[rows]),
            widest_half(&vertical[0]),
        ]
    };
    let cells = grid
        .cells
        .iter()
        .map(|cell| {
            let cols = cell.col..cell.col + cell.colspan;
            let span_rows = cell.row..cell.row + cell.rowspan;
            let widest = |values: &mut dyn Iterator<Item = f32>| values.fold(0.0, f32::max);
            [
                widest(
                    &mut horizontal[cell.row][cols.clone()]
                        .iter()
                        .map(|b| half(width(b))),
                ),
                widest(
                    &mut vertical[cell.col + cell.colspan][span_rows.clone()]
                        .iter()
                        .map(|b| half(width(b))),
                ),
                widest(
                    &mut horizontal[cell.row + cell.rowspan][cols]
                        .iter()
                        .map(|b| half(width(b))),
                ),
                widest(&mut vertical[cell.col][span_rows].iter().map(|b| half(width(b)))),
            ]
        })
        .collect();
    CollapsedBorders {
        horizontal,
        vertical,
        cells,
        table,
    }
}

/// The border widths of a table whose borders collapse, top, right,
/// bottom and left: half the widest border along each of its grid's
/// outer lines (css-tables-3 §3.7.1.1), which only its grid can tell;
/// it has no padding.
pub(crate) fn collapsed_table_border(table: &LayoutBox) -> [f32; 4] {
    collapse_borders(table, &Grid::of(table)).table
}

impl CollapsedBorders {
    /// The fragments that paint the resolved borders, the table's own:
    /// one per segment of a grid line per column or row it crosses, a
    /// box with just the one side set, lying across its line as the line
    /// is split. A joint, where lines cross, is its winner's: a segment
    /// reaches over the joint at its end by half the widest border
    /// crossing it there when no border meeting there wins over it, and
    /// otherwise stops at the joint's edge, as browsers paint it, so that
    /// a losing border never shows through a winning `double`'s gap and
    /// neighbouring segments meet square. They come lowest precedence
    /// first, so where borders that tie overlap at a joint the later one
    /// is painted over the others. `line_x` and `line_y` are the grid
    /// lines' positions in the table box. `inset` and `outset` are
    /// painted as `ridge` and `groove`, as the collapsed model has them
    /// (§17.6.3).
    fn segments(&self, table: &LayoutBox, line_x: &[f32], line_y: &[f32]) -> Vec<Fragment> {
        let width = |b: Option<&Option<Candidate>>| b.copied().flatten().map_or(0.0, |b| b.width);
        // The borders meeting at the joint of horizontal line `l` and
        // vertical line `v`, and how far a segment ending there reaches
        // past the line it crosses: over the joint by half the widest
        // crossing border, `crossing`, when none of them wins over it,
        // short of it by as much otherwise.
        let meeting = |l: usize, v: usize| {
            let at = |line: &Vec<Option<Candidate>>, i: Option<usize>| {
                i.and_then(|i| line.get(i).copied().flatten())
            };
            [
                at(&self.horizontal[l], v.checked_sub(1)),
                at(&self.horizontal[l], Some(v)),
                at(&self.vertical[v], l.checked_sub(1)),
                at(&self.vertical[v], Some(l)),
            ]
        };
        let reach = |border: &Candidate, l: usize, v: usize, crossing: f32| {
            let beaten = meeting(l, v)
                .iter()
                .flatten()
                .any(|other| other.precedence(border) == std::cmp::Ordering::Greater);
            if beaten {
                -half(crossing)
            } else {
                half(crossing)
            }
        };
        // The widest vertical border at the joint of horizontal line `l`
        // and vertical line `v`, and the widest horizontal one.
        let vertical_at = |l: usize, v: usize| {
            let line = &self.vertical[v];
            let above = if l > 0 { width(line.get(l - 1)) } else { 0.0 };
            above.max(width(line.get(l)))
        };
        let horizontal_at = |l: usize, v: usize| {
            let line = &self.horizontal[l];
            let left = if v > 0 { width(line.get(v - 1)) } else { 0.0 };
            left.max(width(line.get(v)))
        };
        let mut segments: Vec<(Candidate, Fragment)> = Vec::new();
        for (l, line) in self.horizontal.iter().enumerate() {
            for (c, border) in line.iter().enumerate() {
                let Some(border) = *border else { continue };
                let x0 = line_x[c] - reach(&border, l, c, vertical_at(l, c));
                let x1 = line_x[c + 1] + reach(&border, l, c + 1, vertical_at(l, c + 1));
                let y0 = line_y[l] - half(border.width);
                let rect = Rect {
                    x: x0,
                    y: y0,
                    w: (x1 - x0).max(0.0),
                    h: border.width,
                };
                segments.push((border, segment_fragment(table, border, TOP, rect)));
            }
        }
        for (v, line) in self.vertical.iter().enumerate() {
            for (r, border) in line.iter().enumerate() {
                let Some(border) = *border else { continue };
                let y0 = line_y[r] - reach(&border, r, v, horizontal_at(r, v));
                let y1 = line_y[r + 1] + reach(&border, r + 1, v, horizontal_at(r + 1, v));
                let x0 = line_x[v] - half(border.width);
                let rect = Rect {
                    x: x0,
                    y: y0,
                    w: border.width,
                    h: (y1 - y0).max(0.0),
                };
                segments.push((border, segment_fragment(table, border, LEFT, rect)));
            }
        }
        segments.sort_by(|a, b| a.0.precedence(&b.0));
        segments.into_iter().map(|(_, f)| f).collect()
    }
}

/// A collapsed border segment's fragment: the table's, painting one
/// side, `side`, with `border` across `rect`.
fn segment_fragment(table: &LayoutBox, border: Candidate, side: usize, rect: Rect) -> Fragment {
    let mut fragment = box_fragment(rect.w, rect.h, table);
    fragment.x = rect.x;
    fragment.y = rect.y;
    fragment.style = inner_style(&table.style).into();
    Arc::make_mut(&mut fragment.style).background_color = Color::TRANSPARENT;
    Arc::make_mut(&mut fragment.style).overflow = Overflow::Visible;
    Arc::make_mut(&mut fragment.style).border_style = [BorderStyle::None; 4];
    Arc::make_mut(&mut fragment.style).border_style[side] = match border.style {
        BorderStyle::Inset => BorderStyle::Ridge,
        BorderStyle::Outset => BorderStyle::Groove,
        other => other,
    };
    Arc::make_mut(&mut fragment.style).border_width = [0.0; 4];
    Arc::make_mut(&mut fragment.style).border_width[side] = border.width;
    Arc::make_mut(&mut fragment.style).border_color[side] = border.color;
    Arc::make_mut(&mut fragment.style).set_border_color_current(side, border.current);
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
    inner.inset = [const { LengthPercentageAuto::Auto }; 4];
    // The transform, opacity and will-change properties apply to the
    // wrapper, which the captions turn with (css-transforms-1 §3,
    // M19); the table box inside is not transformed again.
    inner.effects = None;
    inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{FragmentKind, layout_document};
    use crate::testing::{Laid, boxes_of};
    use style::computed::{LengthPercentage, VerticalAlign};
    use text::FontCollection;

    const VIEWPORT: f32 = 800.0;

    fn test_fonts() -> FontCollection {
        FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
    }

    /// The document `html` parses to, styled under `css` and laid out
    /// against an 800px viewport. The parser implies html, head and
    /// body: nodes html 1, head 2, body 3, the markup's from 4.
    fn document_of(html: &str, css: &str) -> Laid {
        let fonts = test_fonts();
        let doc = html::parse_document(html).expect("html parsing never fails");
        let sheet = css::parse_stylesheet(css).expect("test css is valid");
        let (document, boxes) = boxes_of(&doc, &[sheet]);
        let layout = layout_document(&boxes, VIEWPORT, &fonts);
        Laid::new(document, layout)
    }

    /// The first fragment in tree order for the node `node`.
    fn fragment_of(layout: &Laid, node: usize) -> &Fragment {
        let id = layout.element(node);
        layout
            .iter()
            .find(|f| f.node == id && matches!(f.kind, FragmentKind::Box))
            .unwrap_or_else(|| panic!("a box fragment for node {node}"))
    }

    /// Every box fragment of `node`, in tree order.
    fn fragments_of(layout: &Laid, node: usize) -> Vec<&Fragment> {
        let id = layout.element(node);
        layout
            .iter()
            .filter(|f| f.node == id && matches!(f.kind, FragmentKind::Box))
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
            .map(|f| layout.number(f.node))
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
            [const { LengthPercentage::Px(1.0) }; 4]
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
    fn columns_of(layout: &Laid, nodes: &[usize]) -> Vec<(f32, f32)> {
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
        // A `calc()` with a percentage is `auto` here, as in the automatic
        // algorithm and the reference browser (calc-width-table-fixed-1):
        // the free column takes the 180 less the other's 60.
        let layout = document_of(
            "<table><tr><td class=h></td><td class=w></td></tr></table>",
            "body { margin: 0; } td { padding: 0; }
             table { table-layout: fixed; width: 210px; border-spacing: 10px; }
             .h { width: calc(50% + 1px); } .w { width: 60px; }",
        );
        assert_eq!(columns_of(&layout, &[6, 7]), [(10.0, 120.0), (140.0, 60.0)]);
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

    fn candidate(width: f32, style: BorderStyle, owner: u8, order: u8) -> Candidate {
        Candidate {
            width,
            style,
            color: Color::BLACK,
            current: false,
            owner,
            order,
        }
    }

    #[test]
    fn border_conflicts_resolve_as_css2_section_17_6_2_1_says() {
        use BorderStyle::*;
        let solid = |w| candidate(w, Solid, 0, 0);
        // `hidden` wins over everything, and leaves no border.
        assert_eq!(
            resolve(&[solid(5.0), candidate(0.0, Hidden, 5, 0)]),
            Option::None
        );
        // `none` loses to everything; all `none` is no border.
        assert_eq!(
            resolve(&[candidate(0.0, None, 0, 0), candidate(1.0, Solid, 5, 0)]),
            Some(candidate(1.0, Solid, 5, 0))
        );
        assert_eq!(
            resolve(&[candidate(0.0, None, 0, 0), candidate(0.0, None, 5, 0)]),
            Option::None
        );
        // The wider wins, whatever its owner or style.
        assert_eq!(
            resolve(&[solid(1.0), candidate(3.0, Inset, 5, 0)])
                .unwrap()
                .width,
            3.0
        );
        // Equal widths: the style higher in the list.
        for (better, worse) in [
            (Double, Solid),
            (Solid, Dashed),
            (Dashed, Dotted),
            (Dotted, Ridge),
            (Ridge, Outset),
            (Outset, Groove),
            (Groove, Inset),
        ] {
            let won =
                resolve(&[candidate(2.0, worse, 0, 0), candidate(2.0, better, 5, 1)]).unwrap();
            assert_eq!(won.style, better, "{better:?} over {worse:?}");
        }
        // Equal styles: the more specific owner, then the one to the left
        // or above.
        assert_eq!(
            resolve(&[candidate(2.0, Solid, 5, 0), candidate(2.0, Solid, 1, 1)])
                .unwrap()
                .owner,
            1
        );
        assert_eq!(
            resolve(&[candidate(2.0, Solid, 0, 1), candidate(2.0, Solid, 0, 0)])
                .unwrap()
                .order,
            0
        );
    }

    /// A 2 by 2 collapsed table: a 4px blue table border, 2px red cell
    /// borders, two 50px columns, cells 20px tall inside.
    const COLLAPSED: &str = "<table><tr><td></td><td></td></tr><tr><td></td><td></td></tr></table>";
    const COLLAPSED_CSS: &str = "body { margin: 0; }
        table { border-collapse: collapse; border: 4px solid #0000ff; table-layout: fixed; width: 100px; }
        td { padding: 0; height: 20px; border: 2px solid #ff0000; }";

    #[test]
    fn collapsed_borders_are_shared_on_the_grid_lines_and_held_half_by_each_cell() {
        // The outer lines resolve to the table's 4px, the inner ones to
        // the cells' 2px. A cell holds half of each line on its side: 2 of
        // an outer line, 1 of an inner one, so each row is 2 + 20 + 1
        // tall. The table's border is half its outer lines, 2 a side, so
        // its border box, 100 wide as given, holds them whole: its
        // columns are 48 wide, and it is 2 + 23 + 23 + 2 tall.
        // Nodes: table 4, tr 5, td 6, td 7, tr 8, td 9, td 10.
        let layout = document_of(COLLAPSED, COLLAPSED_CSS);
        let table = fragments_of(&layout, 4);
        assert_eq!(rect(table[0]), (0.0, 0.0, 100.0, 50.0));
        assert_eq!(rect(table[1]), (0.0, 0.0, 100.0, 50.0));
        assert_eq!(
            table[1].style.used_border_width(),
            [0.0; 4],
            "the lines paint it"
        );
        assert_eq!(rect(fragment_of(&layout, 6)), (2.0, 2.0, 48.0, 23.0));
        assert_eq!(rect(fragment_of(&layout, 7)), (50.0, 2.0, 48.0, 23.0));
        assert_eq!(rect(fragment_of(&layout, 9)), (2.0, 25.0, 48.0, 23.0));
        assert_eq!(fragment_of(&layout, 6).style.used_border_width(), [0.0; 4]);
        // Twelve segments: three lines of two each way, the red ones
        // first so the blue ones win at the joints.
        let segments = &table[2..];
        assert_eq!(segments.len(), 12);
        let blue = Color::rgb8(0, 0, 255);
        let red = Color::rgb8(255, 0, 0);
        let first_blue = segments
            .iter()
            .position(|f| f.style.border_color.contains(&blue))
            .unwrap();
        assert!(
            segments[..first_blue]
                .iter()
                .all(|f| f.style.used_border_width().iter().sum::<f32>() == 2.0)
        );
        // The top line over the first column: 4px across its line, from
        // the table box's top, reaching over the left joint by the outer
        // line's half, to the box's left, and over the middle one by the
        // inner line's 1.
        let top_left = segments
            .iter()
            .find(|f| f.style.used_border_width()[TOP] == 4.0 && f.x == 0.0 && f.y == 0.0)
            .unwrap();
        assert_eq!(rect(top_left), (0.0, 0.0, 51.0, 4.0));
        assert_eq!(top_left.style.border_color[TOP], blue);
        // The inner vertical line in the first row: 2px, from the top
        // line's inner edge, the top line winning that joint, to 1 below
        // the middle line, whose joint it ties with the lines it meets.
        let middle = segments
            .iter()
            .find(|f| f.style.used_border_width()[LEFT] == 2.0 && f.x == 49.0 && f.y == 4.0)
            .unwrap();
        assert_eq!(rect(middle), (49.0, 4.0, 2.0, 22.0));
        assert_eq!(middle.style.border_color[LEFT], red);
    }

    #[test]
    fn a_collapsed_segment_keeps_whether_its_colour_is_currentcolor() {
        // The outer lines are the table's blue, the inner ones the
        // cells' `currentcolor` (M33): each segment says which, for the
        // tones of a 3D style. Nodes: table 4.
        let layout = document_of(
            COLLAPSED,
            "body { margin: 0; }
             table { border-collapse: collapse; border: 4px solid #0000ff; table-layout: fixed; width: 100px; }
             td { padding: 0; height: 20px; border: 2px inset; }",
        );
        let segments = &fragments_of(&layout, 4)[2..];
        assert_eq!(segments.len(), 12);
        for f in segments {
            let side = (0..4)
                .find(|&side| f.style.used_border_width()[side] > 0.0)
                .expect("a segment paints a side");
            let width = f.style.used_border_width()[side];
            assert_eq!(
                f.style.border_color_is_current(side),
                width == 2.0,
                "{side} {width}"
            );
            let ridge = f.style.border_style[side] == BorderStyle::Ridge;
            assert_eq!(ridge, width == 2.0, "an inset line is collapsed as a ridge");
        }
    }

    #[test]
    fn a_hidden_border_leaves_its_segment_bare_and_auto_widths_count_the_halves() {
        // `hidden` on the first cell's top: no segment over its column on
        // the top line. The cell holds none of that line, though the row,
        // and so the cell, stays as tall as its neighbour's 2 + 20 + 1.
        // Nodes: td 6.
        let layout = document_of(
            COLLAPSED,
            &format!("{COLLAPSED_CSS} tr:first-child td:first-child {{ border-top: hidden; }}"),
        );
        assert_eq!(fragments_of(&layout, 4).len(), 2 + 11);
        assert_eq!(rect(fragment_of(&layout, 6)), (2.0, 2.0, 48.0, 23.0));
        // An automatic table measures its cells with the halves of the
        // lines they hold, 2 + 10 + 1 and 1 + 10 + 2, and is as wide as
        // they are with its own border's halves. Nodes: td 6, td 8.
        let layout = document_of(
            "<table><tr><td><div class=b></div></td><td><div class=b></div></td></tr></table>",
            "body { margin: 0; } table { border-collapse: collapse; border: 4px solid #0000ff; }
             td { padding: 0; border: 2px solid #ff0000; } .b { width: 10px; height: 10px; }",
        );
        assert_eq!(fragment_of(&layout, 4).w, 30.0);
        assert_eq!(columns_of(&layout, &[6, 8]), [(2.0, 13.0), (15.0, 13.0)]);
        assert_eq!(
            fragment_of(&layout, 7).x,
            4.0,
            "inside the first cell's 2px"
        );
    }

    #[test]
    fn a_collapsed_table_without_rows_or_columns_has_no_segments() {
        // Columns and no row; a row and no cell; nothing at all. Nodes:
        // table 4, col 5; table 6, tr 7; table 8.
        let layout = document_of(
            "<table><col></table><table><tr></tr></table><table></table>",
            "table { border-collapse: collapse; border: 3px solid #000000; width: 20px; }",
        );
        for node in [4, 6, 8] {
            let table = fragments_of(&layout, node);
            assert_eq!(table.len(), 2, "node {node}");
            // No lines, so no border: the width given, and no height.
            assert_eq!((table[1].w, table[1].h), (20.0, 0.0), "node {node}");
        }
    }

    #[test]
    fn empty_cells_hide_paints_nothing_for_an_empty_cell_in_the_separated_model() {
        // Nodes: table 4, tr 5, td 6, td 7, td 8.
        let html = "<table><tr><td></td><td> </td><td>x</td></tr></table>";
        let css = "table { empty-cells: hide; } td { background-color: #ff0000; border: 1px solid #000000; }";
        let layout = document_of(html, css);
        for node in [6, 7] {
            let cell = fragment_of(&layout, node);
            assert_eq!(
                cell.style.background_color,
                Color::TRANSPARENT,
                "node {node}"
            );
            assert_eq!(cell.style.used_border_width(), [0.0; 4], "node {node}");
        }
        let full = fragment_of(&layout, 8);
        assert_eq!(full.style.background_color, Color::rgb8(255, 0, 0));
        assert_eq!(full.style.used_border_width(), [1.0; 4]);
        // Its border and padding keep their room: 1 + 1 + 1 + 1.
        assert_eq!(fragment_of(&layout, 6).w, 4.0);
        // In the collapsed model `empty-cells` does nothing to the
        // background.
        let layout = document_of(
            html,
            &format!("{css} table {{ border-collapse: collapse; }}"),
        );
        assert_eq!(
            fragment_of(&layout, 6).style.background_color,
            Color::rgb8(255, 0, 0)
        );
    }

    /// The heights of the row fragments of `nodes`.
    fn heights_of(layout: &Laid, nodes: &[usize]) -> Vec<f32> {
        nodes.iter().map(|&n| fragment_of(layout, n).h).collect()
    }

    /// Rows of 10px content in 2px padding (14px) and 3px spacing, as the
    /// browser measurements these tests take their numbers from.
    const ROWS_CSS: &str = "body { margin: 0; } table { border-spacing: 3px; } td { padding: 2px; }
        div { width: 30px; } .e { padding: 0; } .t10 { height: 10px; } .t30 { height: 30px; }
        .t70 { height: 70px; } .t76 { height: 76px; } .t90 { height: 90px; }
        .h20 { height: 20px; } .h100 { height: 100px; } .h200 { height: 200px; }";

    #[test]
    fn a_rowspans_excess_goes_first_to_the_rows_where_another_rowspan_starts() {
        // A cell spanning rows 1 and 2 needs 74px of their 31: the 43
        // go to row 2, where another spanning cell starts, not 21.5 to
        // each. Nodes: tr 5, tr 10, tr 15.
        let layout = document_of(
            "<table><tr><td rowspan=2><div class=t70></div></td><td><div class=t10></div></td></tr>\
             <tr><td><div class=t10></div></td><td rowspan=2><div class=t10></div></td></tr>\
             <tr><td><div class=t10></div></td><td><div class=t10></div></td></tr></table>",
            ROWS_CSS,
        );
        assert_eq!(heights_of(&layout, &[5, 10, 15]), [14.0, 57.0, 14.0]);
        // Two cells ending in row 3: the one spanning fewer rows first,
        // its 77 to row 3 (row 2 has no height), which leaves the longer
        // one nothing to give; the other way round row 2 would take 74
        // and then 3. Nodes: tr 5, tr 10, tr 13.
        let layout = document_of(
            "<table><tr><td rowspan=3><div class=t90></div></td><td><div class=t10></div></td></tr>\
             <tr><td rowspan=2><div class=t76></div></td></tr><tr></tr></table>",
            ROWS_CSS,
        );
        assert_eq!(heights_of(&layout, &[5, 10, 13]), [14.0, 0.0, 77.0]);
    }

    #[test]
    fn a_rowspans_excess_spares_constrained_rows_and_falls_to_the_last_of_no_height() {
        // A row with a length height keeps it while another can grow.
        // Nodes: tr 5, tr 10.
        let layout = document_of(
            "<table><tr class=h20><td rowspan=2><div class=t70></div></td><td><div class=t10></div></td></tr>\
             <tr><td><div class=t10></div></td></tr></table>",
            ROWS_CSS,
        );
        assert_eq!(heights_of(&layout, &[5, 10]), [20.0, 51.0]);
        // Rows of no height: the last takes it all; a constrained row
        // with a height takes it before them. Nodes: tr 5, tr 9, tr 11.
        let rows = "<tr><td class=e></td></tr><tr><td class=e></td></tr></table>";
        let layout = document_of(
            &format!(
                "<table><tr><td rowspan=3><div class=t70></div></td><td class=e></td></tr>{rows}"
            ),
            ROWS_CSS,
        );
        assert_eq!(heights_of(&layout, &[5, 9, 11]), [0.0, 0.0, 68.0]);
        let layout = document_of(
            &format!(
                "<table><tr class=h20><td rowspan=3><div class=t70></div></td><td class=e></td></tr>{rows}"
            ),
            ROWS_CSS,
        );
        assert_eq!(heights_of(&layout, &[5, 9, 11]), [68.0, 0.0, 0.0]);
    }

    #[test]
    fn the_tables_height_goes_to_its_body_groups_then_their_rows() {
        let close = |got: Vec<f32>, want: [f32; 3]| {
            assert!(
                got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-3),
                "{got:?} against {want:?}"
            );
        };
        // 126px past the rows go to the groups by their heights, 14 and
        // 34 + 3 + 14, then within the second by its rows'. Nodes: tr 6,
        // tr 10, tr 13.
        let layout = document_of(
            "<table class=h200><tbody><tr><td><div class=t10></div></td></tr></tbody>\
             <tbody><tr><td><div class=t30></div></td></tr><tr><td><div class=t10></div></td></tr></tbody></table>",
            ROWS_CSS,
        );
        let second = 126.0 * 51.0 / 65.0;
        close(
            heights_of(&layout, &[6, 10, 13]),
            [
                14.0 + 126.0 * 14.0 / 65.0,
                34.0 + second * 34.0 / 48.0,
                14.0 + second * 14.0 / 48.0,
            ],
        );
        // The header and footer groups take none of it while there is a
        // body group. Nodes: tr 6, tr 10, tr 14.
        let layout = document_of(
            "<table class=h200><thead><tr><td><div class=t10></div></td></tr></thead>\
             <tbody><tr><td><div class=t30></div></td></tr></tbody>\
             <tfoot><tr><td><div class=t10></div></td></tr></tfoot></table>",
            ROWS_CSS,
        );
        assert_eq!(heights_of(&layout, &[6, 10, 14]), [14.0, 160.0, 14.0]);
        // A constrained row keeps its height and the unconstrained ones,
        // of no height, share equally. Nodes: tr 5, tr 8, tr 10.
        let layout = document_of(
            "<table class=h200><tr class=h20><td><div class=t10></div></td></tr>\
             <tr><td class=e></td></tr><tr><td class=e></td></tr></table>",
            ROWS_CSS,
        );
        assert_eq!(heights_of(&layout, &[5, 8, 10]), [20.0, 84.0, 84.0]);
        // A row group's own height is a minimum for its rows, shared the
        // same way. Nodes: tr 6, tr 9.
        let layout = document_of(
            "<table><tbody class=h100><tr class=h20><td><div class=t10></div></td></tr>\
             <tr><td><div class=t30></div></td></tr></tbody></table>",
            ROWS_CSS,
        );
        assert_eq!(heights_of(&layout, &[6, 9]), [20.0, 77.0]);
    }

    #[test]
    fn distributing_heights_follows_the_browsers_order() {
        let (f, t) = (false, true);
        let rowspan = |rows: [f32; 3], constrained: [bool; 3], starts: [bool; 3]| {
            let mut rows = rows;
            distribute_rowspan(12.0, &mut rows, &constrained, &starts);
            rows
        };
        // The rows after the first where a spanning cell starts, equally.
        assert_eq!(
            rowspan([14.0, 14.0, 14.0], [f; 3], [t, t, t]),
            [14.0, 20.0, 20.0]
        );
        // Else the unconstrained rows by height, else the constrained
        // ones, else the last row.
        assert_eq!(
            rowspan([10.0, 0.0, 30.0], [f; 3], [t, f, f]),
            [13.0, 0.0, 39.0]
        );
        assert_eq!(
            rowspan([20.0, 0.0, 0.0], [t, f, f], [f; 3]),
            [32.0, 0.0, 0.0]
        );
        assert_eq!(rowspan([0.0; 3], [t, f, f], [f; 3]), [0.0, 0.0, 12.0]);
        let height = |parts: [f32; 3], constrained: [bool; 3]| {
            let mut parts = parts;
            distribute_height(12.0, &mut parts, &constrained);
            parts
        };
        // The unconstrained parts by size, equally when of none; all
        // parts when all are constrained, the same way.
        assert_eq!(height([10.0, 30.0, 20.0], [f, f, t]), [13.0, 39.0, 20.0]);
        assert_eq!(height([20.0, 0.0, 0.0], [t, f, f]), [20.0, 6.0, 6.0]);
        assert_eq!(height([10.0, 20.0, 30.0], [t; 3]), [12.0, 24.0, 36.0]);
        assert_eq!(height([0.0; 3], [t; 3]), [4.0; 3]);
    }

    #[test]
    fn a_collapsed_tables_border_is_half_its_widest_outer_borders_and_halves_are_exact() {
        // A 10px left border in the second row only: the table's left
        // border is its half, 5, for the whole side, and the first column
        // 5 + 40 + 1 wide; the float around the table counts it too.
        // Nodes: div 4, table 5, td 7, td 8, td 10.
        let css = "body { margin: 0; } .f { float: left; } table { border-collapse: collapse; }
             td { padding: 0; width: 40px; height: 20px; border: 2px solid #ff0000; }
             .w { border-left: 10px solid #0000ff; } .c { border: 3px solid #000000; }
             .t { border-top-width: 7px; }";
        let layout = document_of(
            "<div class=f><table><tr><td></td><td></td></tr><tr><td class=w></td><td></td></tr></table></div>",
            css,
        );
        assert_eq!(fragment_of(&layout, 4).w, 94.0);
        assert_eq!(rect(fragments_of(&layout, 5)[1]), (0.0, 0.0, 94.0, 46.0));
        assert_eq!(rect(fragment_of(&layout, 7)), (5.0, 1.0, 46.0, 22.0));
        assert_eq!(rect(fragment_of(&layout, 8)), (51.0, 1.0, 42.0, 22.0));
        assert_eq!(rect(fragment_of(&layout, 10)), (5.0, 23.0, 46.0, 22.0));
        // Odd widths are halved exactly: the table's 3px gives 1.5 a side,
        // the first cell's 7px top the table 3.5 at the top. Nodes: table
        // 4, td 6, td 7.
        let layout = document_of(
            "<table class=c><tr><td class=t></td><td></td></tr></table>",
            css,
        );
        assert_eq!(rect(fragments_of(&layout, 4)[1]), (0.0, 0.0, 88.0, 30.0));
        assert_eq!(rect(fragment_of(&layout, 6)), (1.5, 3.5, 42.5, 25.0));
        assert_eq!(rect(fragment_of(&layout, 7)), (44.0, 3.5, 42.5, 25.0));
    }

    #[test]
    fn a_joint_is_its_winners_and_the_losers_stop_at_its_edge() {
        // A 3px double under the first cell meets the 2px inner line: the
        // double reaches over the joint, the inner line's two segments and
        // the line's other half stop at its edges, so nothing shows in the
        // double's gap. Rows are 1 + 20 + 1.5 tall, lines at y 1, 23.5 and
        // 46 and x 1, 23 and 45. Nodes: table 4.
        let layout = document_of(
            "<table><tr><td class=d></td><td></td></tr><tr><td></td><td></td></tr></table>",
            "body { margin: 0; } table { border-collapse: collapse; }
             td { padding: 0; width: 20px; height: 20px; border: 2px solid #ff0000; }
             .d { border-bottom: 3px double #0000ff; }",
        );
        let segments: Vec<_> = fragments_of(&layout, 4)[2..]
            .iter()
            .map(|f| rect(f))
            .collect();
        for want in [
            (0.0, 22.0, 24.0, 3.0),
            (22.0, 0.0, 2.0, 22.0),
            (22.0, 25.0, 2.0, 22.0),
            (24.0, 22.5, 22.0, 2.0),
        ] {
            assert!(segments.contains(&want), "{want:?} in {segments:?}");
        }
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
