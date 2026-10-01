# Design note: tables

Roadmap M12. Written before the code, as the record of the decisions;
what ships is described in `docs/architecture.md` "Current state" when
the code lands. Where this note and the code later disagree, the code
wins and this note is corrected, as `box-model.md` and
`floats-positioning.md` were.

The problem: a `<table>` is a `<div>` today. The UA sheet makes the
table elements blocks (M10's stopgap), so rows stack and cells stack
inside them, and every page with a timetable, a price list, a form laid
out in cells or a layout done with `display: table-cell` is wrong.
CSS2 chapter 17 is the whole of the table model — the box kinds and the
anonymous boxes that complete a broken structure, the grid with cells
that span, the two width algorithms, row heights with cells aligned in
them, the two border models, captions — and this milestone adds it to
the engine, so that the pages M13 (the incremental engine) and M17
(flexbox) build on lay a table out the way a reference browser does.

## 1. Decisions

**The properties.** `display` gains CSS2 §17.2's values: `table`,
`inline-table`, `table-row-group`, `table-header-group`,
`table-footer-group`, `table-row`, `table-column-group`,
`table-column`, `table-cell` and `table-caption`. `table-layout: auto |
fixed` (not inherited); `border-collapse: separate | collapse`,
`border-spacing: <length> <length>?` (horizontal then vertical, one
value for both), `caption-side: top | bottom` and `empty-cells: show |
hide`, the four inherited as the specification says. `vertical-align`
is parsed in full — `baseline`, `sub`, `super`, `top`, `text-top`,
`middle`, `bottom`, `text-bottom`, a length or a percentage — and used
by this milestone on cells alone, where `top`, `middle` and `bottom`
mean what §17.5.3 says and every other value is `baseline`, as §17.5.3
says too; its meaning on inline boxes is M42's. The HTML attributes
`colspan`, `rowspan` and `span` are read by the box tree builder, as
the structure they are, clamped as the HTML standard has them
(`colspan` 1 to 1000, `rowspan` 0 to 65534 with 0 meaning to the end
of the row group, `span` 1 to 1000). The presentational attributes
`border`, `cellpadding`, `cellspacing`, `width`, `height`, `bgcolor`
and `align` are not read: they are presentational hints, a cascade
origin this engine does not have yet, and are said so in §2.

**The UA sheet.** The HTML standard's rendering section for tables,
in place of M10's blocks: the display values above for `table`,
`caption`, `colgroup`, `col`, `thead`, `tbody`, `tfoot`, `tr`, `td`
and `th`; `table { box-sizing: border-box; border-spacing: 2px;
border-collapse: separate }`; `td, th { padding: 1px }`; `thead,
tbody, tfoot, table > tr { vertical-align: middle }` and `tr, td, th
{ vertical-align: inherit }`, which is why an HTML cell is centred
vertically by default while a CSS table cell sits on the baseline;
`th { font-weight: bold; text-align: center }` and `caption {
text-align: center }`, the `center` standing in for the standard's
internal value that yields to an inherited alignment, a stated
approximation. The parser's table insertion modes stay out: it does
not wrap rows in a `tbody` or foster-parent a stray block out of a
table, both said in its own documentation, and the anonymous table
objects below give such a tree the boxes a browser's tree has.

**The box tree.** Each display value is a box kind of its own:
`Table` (the wrapper and the table box in one — the wrapper is implicit,
see captions below), `RowGroup` (header, body and footer, with which it
is recorded), `Row`, `ColumnGroup`, `Column`, `Cell` and `Caption`;
`inline-table` is a `Table` that is inline-level, an item on its line
like an inline-block. A cell and a caption are block containers, built
by the same builder as any block (their children all block-level, an
inline run in an anonymous block), and block formatting context roots
(CSS2 §9.4.1); a column or column group has no children, whatever the
element holds. The missing structure is supplied by CSS2 §17.2.1's
anonymous table objects, in the builder, in the order the
specification gives: a table child that is not a caption, a column, a
column group, a row group or a row is wrapped in an anonymous row,
consecutive ones in the same row; a row-group child that is not a row,
in an anonymous row; a row child that is not a cell, in an anonymous
cell, consecutive ones in the same cell; a cell, row or row group
whose parent is not what it needs is wrapped in an anonymous row, row
group or table, consecutive misplaced ones in the same one, so that
`display: table-cell` on sibling `div`s inside a plain `div` makes a
one-row table of them, as it does in a browser. An anonymous box
inherits from its parent like an anonymous block does and has no
background, border, padding or margin of its own. A table's children
are kept in the order they are painted and laid out: captions with
`caption-side: top`, then column groups and columns, then the header
group, the other row groups and rows in document order, then the
footer group, then the bottom captions (§17.2: a header group is
displayed before every other row and row group, a footer group after
them, whatever their place in the source). Out-of-flow boxes inside a
table — a float or an absolutely positioned child of a row, say — are
treated as what they are, not as cells.

**The grid (M12.4).** Before any width is known, the table's rows are
walked into a grid of slots, css-tables-3 §3.4's way: each cell takes
the next free slot in its row, after any slot a cell from a row above
still spans, and occupies `colspan` columns and `rowspan` rows; a
`rowspan` past the end of its row group is cut at the group's end, and
0 means that end; a `colspan` past the columns seen so far adds
columns. The number of columns is the widest row's, or the columns
the column boxes declare, whichever is greater; a column box with no
cells under it still takes a column, and a `span` on a column or
column group counts as that many. Every cell knows its first column
and row and its spans; every slot knows the cell on it. Row groups,
rows and columns keep their boxes, so their backgrounds paint and
their heights and widths apply.

**Column widths, the automatic algorithm (M12.3).** CSS2 §17.5.2.2
leaves the distribution to the UA; this engine takes what the browsers
have in common, which css-tables-3 §3.9 writes down. First each cell's
outer min-content and max-content widths (`layout::intrinsic` on the
cell as a block container, plus its padding and border); a cell with a
length `width` has min and max no smaller than it, `box-sizing`
applied, so a column is never narrower than a cell's given width; a
cell with a percentage `width` gives its column that percentage, the
largest among its cells, and a column box's own `width` sets the
column's minimum and maximum, or its percentage, the same way. A cell
spanning one column sets its column's minimum to the largest cell
minimum and maximum to the largest cell maximum. Cells spanning
several columns are taken next in order of increasing span: the part
of the cell's minimum beyond the sum of its columns' minimums and the
spacing between them is distributed to those columns in proportion to
their maximums, equally when all are zero, and its maximum the same
way; a spanning cell's percentage is split among its columns that have
none, equally, after the ones that have one are counted. The table's
own minimum (MIN) is the sum of the column minimums, the spacing
between and around them and the table's border and padding; its
maximum (MAX) the same with the maximums, and a percentage column's
maximum raised to what makes its percentage of the whole hold where it
can. The table's used width: a length `width` as given (`box-sizing`
applied), a percentage of the containing block's width, each no less
than MIN; `auto` is the greater of MIN and the lesser of MAX and the
containing block's width, which is shrink-to-fit with the table's own
minimum as the floor, as §17.5.2.2's rule comes to. Then the width is
given out to the columns: every column gets its minimum; percentage
columns are raised toward their percentage of the table's width, with
the percentages scaled down together when they sum past what is left;
the remainder goes to the columns that have neither a percentage nor a
given width, each raised toward its maximum in proportion to its
maximum, then to the columns with a given width the same way, then to
the percentage columns; and width still left after every column has
its maximum is given to the columns without a percentage in proportion
to their maximums, equally when all are zero. `margin: auto` on a
table centres it: once the used width is known, §10.3.3's rules
resolve the margins with it.

**Column widths, the fixed algorithm (M12.2).** `table-layout: fixed`
with a `width` other than `auto` (with `auto`, the automatic algorithm
is used, as §17.5.2.1 says): the table's width is the given one, and
the columns take their widths from the column boxes with a `width`,
then from the cells of the first row with a `width` (a spanning cell's
split equally across its columns), percentages of the table's width;
the columns left over share what remains equally, and when nothing
remains they are zero. No cell is measured: a wide cell's content
overflows its column, which is the point of the algorithm, and the
rows after the first do not change the widths.

**Row heights and vertical alignment (M12.6).** Each cell's content is
laid out as a block container at its column's width, less the cell's
padding and border, and the cell's own `height` is a minimum for it.
A row's height is what its cells need (CSS2 §17.5.3): the cells with
`vertical-align: baseline` share the row's baseline, which is the
largest distance from a cell's top to its first baseline — a cell's
first line box's baseline, else its content's bottom edge — and the
row is as tall as the tallest of their bottoms below that baseline and
of the other cells, and no less than the row's own `height`. A cell
spanning rows is held until its last row: what it needs beyond the sum
of the rows it spans and the spacing between them is given to those
rows in proportion to their heights, equally when all are zero. Then
every cell is as tall as the rows it spans — its border box is
stretched to them — and its content is placed inside it by its
alignment: at its top, on the baseline, centred, or at its bottom. A
table's `height` is a minimum for the table: height past the rows'
sum is given to the rows in proportion to their heights, equally when
all are zero. The first row's baseline is the table's, which is what an
`inline-table` aligns by on its line, and a table with no row aligns by
its bottom margin edge.

**The separated border model.** `border-collapse: separate`, the
initial value and the UA sheet's: `border-spacing` lies between every
two cells and between the cells and the table's padding edge; each
cell paints its own border and background; a column, a column group, a
row group and a row paint their backgrounds under the cells of their
area (§17.5.1's layers, in that order, which is the order the fragment
tree keeps them in), and a cell with no content at all — no box, no
text — under `empty-cells: hide` paints neither border nor background.
Spacing is counted in MIN, MAX and the table's used width, and in the
rows' sum for its height.

**The collapsed border model (M12.5).** `border-collapse: collapse`:
`border-spacing` and `empty-cells` are ignored, and every grid edge —
each cell's four sides, with the row, row group, column, column group
and table edges that fall on the same line — is resolved to one border
by CSS2 §17.6.2.1's rules in order: `hidden` wins over everything;
`none` loses to everything; the wider wins; among equal widths the
style wins in the order `double`, `solid`, `dashed`, `dotted`,
`ridge`, `outset`, `groove`, `inset`; among equal styles the element
further up the list cell, row, row group, column, column group, table
wins, and for two of the same kind the one further left or up. The
resolved borders are centred on the grid lines: a column is as wide as
its cells' content plus half the borders on its two sides, the half
on the left and top being the width rounded down and the other half
the rest, a stated choice the acceptance page checks; the table's
border box is widened by half its outer borders and the other half
lies in its margin (§17.6.2), so its width includes half the border.
A cell's own border is not painted in this model: layout emits each
resolved border segment — one per grid line per column or row it
crosses, so that neighbouring segments of different widths or styles
meet with square ends, as a browser's do — as a fragment of its own
with just that one side set in its style and the table's node, which
`paint` paints with the side's own style (`double`, `dashed` and the
rest included) as it paints any border, after the cells' backgrounds,
since the segments are the table fragment's last children. Cell
minimums and maximums count the half borders where the separated
model counts spacing.

**Captions.** A `table-caption` box is laid out as a block at the
table's used width, above the grid for `caption-side: top` and below
it for `bottom`, several of a side stacked in source order; the
table's margins stand outside the captions and the grid together
(CSS2 §17.4's wrapper box, which this engine does not make a separate
box of: the `Table` box's fragment is the wrapper's, its border and
background painted over the grid's part alone), and a caption's own
minimum content width (CAPMIN) is one of the floors of the table's
used width.

**The table in its flow.** A table is a block formatting context root
for the flow around it: its margins collapse with its siblings' and
never with anything inside it, it dodges floats like any root (M11.2,
its `auto` width computed in the room it is given), and floats inside
a cell or a caption stay in that cell's or caption's own context. Its
intrinsic contribution to a shrink-to-fit ancestor — a float, an
inline-block, a cell — is its MIN and MAX. An `inline-table` is placed
on its line as an inline-block is, as wide as its margin box, its
baseline the first row's. `overflow` on a table is not applied: CSS2
gives it to block containers, and a table is not one.

**Fragments, painting and hit testing.** Layout produces fragments for
the table, each caption, each column group and column (an empty box
over its columns' area, for its background), each row group, each row,
each cell (its border box, stretched to its rows, with its content as
children, offset by its alignment) and, in the collapsed model, each
border segment. The paint-order walk needs nothing new: these are
block-level boxes in flow, painted in tree order in the blocks pass,
which is §17.5.1's order, and their inline content in the inline pass;
a positioned table part is painted at its layer as any positioned box
is. Hit testing answers with the cell under the pointer, the row
between cells (spacing is the row's, and the row group's outside the
rows, and the table's outside the groups), and a border segment with
the table.

**Acceptance against a reference browser (M12.7).** The harness of
M10.7 and M11.7: box-only pages under `crates/engine/tests/reference/`,
captured by `capture.sh` from the headless shell, compared byte for
byte, text kept out since text rasterizers differ by design — so the
"real-world" tables the roadmap names are real-world structures with
sized boxes for content. Five pages: a plain grid with the UA sheet's
spacing and padding, a caption top and bottom, a header and a footer
group out of source order, and cells aligned at the top, middle,
bottom and baseline; the automatic algorithm with a given table width
to distribute, percentage columns, cells with widths and a column box
with one; `colspan` and `rowspan` in the shapes real pages use,
including a span past its row group and a cell pushed right by a span
from above; the collapsed model with every conflict rule exercised,
`hidden` and `none` included, and the half-border rounding; and nested
tables, one in a cell of another with a `table-layout: fixed` one
beside it, and `display: table-cell` on plain elements. Hit testing
through a table is an engine test.

## 2. Not in this milestone, said rather than hidden

The presentational attributes `border`, `cellpadding`, `cellspacing`,
`width`, `height`, `bgcolor` and `align` on table elements, which a
browser maps to CSS at a cascade origin of their own; `visibility:
collapse` on rows and columns; `caption-side` values other than `top`
and `bottom`; `vertical-align` on anything but a cell (M42); `th`'s
bold, which waits for `font-weight` (M21); the parser's table
insertion modes, `tbody` insertion and foster parenting; `overflow`
and scrolling on a table; `border-radius` on cells (M18); `text-indent`;
`direction: rtl` in tables (left-to-right throughout, as since M6);
`page-break-*` inside tables; the HTML `rules` and `frame` attributes;
percentage heights on rows and cells; a `rowspan` cell's height split
other than in proportion.

## 3. Order of work

Each step a vertical slice: build, tests, clippy green, smoke run, then
the next. (1) The properties and the UA sheet; `colspan`, `rowspan` and
`span` read into the box tree. (2) The box kinds and the anonymous
table objects in the builder, with the header and footer groups moved
to their places. (3) The grid, and the fixed algorithm: `layout::table`
laying a `table-layout: fixed` table out with row heights from the
cells, the separated model's spacing, cells stretched to their rows,
top-aligned, painted and hit. (4) The automatic algorithm: cell
measures, spanning cells, percentages, the used width, the
distribution, `margin: auto`; the table's intrinsic contribution. (5)
Row heights in full: baselines and `vertical-align`, `rowspan`
distribution, the table's `height`, captions, `inline-table`. (6)
`empty-cells` and the collapsed border model: conflict resolution, the
half borders, the segment fragments. (7) The reference pages, and
whatever they find. (8) The documents.
