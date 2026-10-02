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

**The box tree.** Each display value is a box kind of its own: `Table`
(the wrapper and the table box in one box, which layout gives a fragment
each, see captions below), `RowGroup` (header, body and footer, with
which it is recorded), `Row`, `ColumnGroup`, `Column`, `Cell` and
`Caption`; `inline-table` is a `Table` that is inline-level, an item on
its line like an inline-block, and a floated or absolutely positioned
table is the `Float` or `Absolute` box M11 gives such a box, with a
table's children, which layout lays out as a table by its display. A
cell and a caption are block containers, built by the same builder as
any block (their children all block-level, an inline run in an anonymous
block), and block formatting context roots (CSS2 §9.4.1); a column or
column group has no children, whatever the element holds. The missing
structure is supplied by CSS2 §17.2.1's anonymous table objects, in the
builder, in the order the specification gives: a table child that is not
a caption, a column, a column group, a row group or a row is wrapped in
an anonymous row, consecutive ones in the same row; a row-group child
that is not a row, in an anonymous row; a row child that is not a cell,
in an anonymous cell, consecutive ones in the same cell; a cell, row or
row group whose parent is not what it needs is wrapped in an anonymous
row, row group or table, consecutive misplaced ones in the same one, so
that `display: table-cell` on sibling `div`s inside a plain `div` makes
a one-row table of them, as it does in a browser. An anonymous box
inherits from its parent like an anonymous block does and has no
background, border, padding or margin of its own. A table's children are
kept in the order they are painted and laid out: captions with
`caption-side: top`, then column groups and columns, then the header
group, the other row groups and rows in document order, then the footer
group, then the bottom captions (§17.2: a header group is displayed
before every other row and row group, a footer group after them,
whatever their place in the source; only the first of each is one, any
other a body group). Out-of-flow boxes inside a table — a float or an
absolutely positioned child of a row, say — are treated as what they
are, not as cells.

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
their heights and widths apply. A run of rows directly in a table acts
as one row group, a body group, for a `rowspan` reaching past it and
for the table's height, as the `tbody` a parser's table insertion mode
would have put them in does.

**Column widths, the automatic algorithm (M12.3).** CSS2 §17.5.2.2
leaves the distribution to the UA; this engine takes what the browsers
have in common, which css-tables-3 §3.9 writes down. First each column
is measured from the cells that span it alone, over their border boxes
(`layout::intrinsic` on the cell as a block container, plus its padding
and border): a cell's minimum is its content's min-content width or its
`min-width`, whichever is greater, and its maximum its length `width`,
`box-sizing` applied, else its content's max-content width, never less
than its minimum. A cell with a length `width` constrains its column,
and that width is the column's maximum, not a minimum (§3.9.1): a table
too narrow for its columns' maximums can make such a column narrower,
down to its content. A cell with a percentage `width` gives its column
that percentage, the largest among its cells. A column box's length
`width` raises its columns' maximums and constrains them, a column
group's counting for its columns that give none, and a percentage one
is their percentage. Cells spanning several columns are taken next, in
order of increasing span: the part of the cell's minimum beyond its
columns' minimums and the spacing between them goes to those columns
in proportion to their maximums, equally when all are zero, and its
maximum the same way; a spanning cell's percentage, less what its
columns already have, is split equally among those that have none.
Last, percentages are cut in column order so that together they ask
for no more than the whole: 70% and 60% become 70% and 30%. The
table's minimum (MIN) is the sum of the column minimums and the spacing
between and around them; its maximum (MAX) the same with the maximums,
raised so that each percentage column's maximum is its share of the
whole and the other columns fill what the percentages leave, and as
wide as a table gets when they leave nothing and other columns want
room; CAPMIN, the widest caption's minimum, is a floor under both; the
table's border and padding come on top. The table's used width: a
length `width` as given (`box-sizing` applied), a percentage of the
containing block's width, each no less than MIN; `auto` is MAX when the
room its margins leave holds it, that room otherwise, and never less
than MIN, which is shrink-to-fit with the table's minimum as the floor,
as §17.5.2.2's rule comes to. Then the width is given out to the
columns by §3.9.3's sizing guesses: every column at its minimum; then
the percentage columns at their percentage of the width (never under
their minimum); then the constrained columns at their maximum too; then
every column at its maximum. A width between two guesses is their
linear combination, every column moving the same fraction of the way
from the one to the next; a width past the last goes to the
unconstrained columns in proportion to their maximums, else to the
constrained ones the same way, else to the percentage columns in
proportion to their percentages, else equally. `margin: auto` on a
table centres it: once the used width is known, §10.3.3's rules
resolve the margins with it. Each cell is measured twice, once for the
table's own width by whatever places the table and once for its
columns: a linear cost, nested tables included, which a cache of
measures would halve.

**Column widths, the fixed algorithm (M12.2).** `table-layout: fixed`
with a `width` other than `auto` (with `auto`, the automatic algorithm
is used, as §17.5.2.1 says): the table's width is the given one, and the
columns take their widths from the column boxes with a `width` (a
column's own, else its group's), then from the cells of the first row
with a `width`, their border box (a spanning cell's split equally across
its columns, less the spacing between them), a percentage being of the
width left after the spacing, as the browsers have it; the columns left
over share what remains equally, and when nothing remains they are zero;
width left with no column to take it goes to every column in proportion
to its width, equally when all are zero. The table is never narrower
than the widths its columns are given. No cell is measured: a wide
cell's content overflows its column, which is the point of the
algorithm, and the rows after the first do not change the widths.

**Row heights and vertical alignment (M12.6).** Each cell's content is
laid out as a block container at its columns' width, less the cell's
padding and border, and the cell's own `height` and `min-height` are a
minimum for it. The cells whose `vertical-align` is `baseline` — every
value but `top`, `middle` and `bottom` counts as it (§17.5.3) — share
their row's baseline, the lowest of their own: a cell's first line
box's, or the first row's of a table first in it, else its content's
bottom edge; a cell spanning rows aligns to its first row's. A row is
as tall as its own length `height`, as each cell spanning it alone
needs, and as each of its baseline cells needs moved down to the
row's baseline. A row is constrained when its own `height`, or that of
a cell spanning it alone, is a length; a percentage counts as `auto`.
A cell spanning rows gives what it needs beyond the rows it spans and
the spacing between them to those rows as the browsers do, which the
specifications leave open: the cells ending in an earlier row first,
and of those ending together the one spanning fewer rows; each to the
rows after its first where another cell spanning rows starts, equally;
else to its unconstrained rows in proportion to their heights; else to
its constrained rows the same way; else, every row being of no height,
to the last. A row group's length `height` is a minimum for its rows
and the spacing between them, given out as a table's is. A table's
`height` and `min-height` are a minimum for the table: what they add
past the rows' sum goes to the body groups — to the header and footer
groups only when there is none — in proportion to their heights, a
group with a length `height` being constrained among them, and in each
group to its rows: the unconstrained ones, or all of them when every
one is constrained, in proportion to their heights, equally when those
are all zero. css-tables-3 §3.10.5 has equal increments instead, which
is not what the browsers do. Then every cell is as tall as the rows it
spans — its border box is stretched to them — and its content is
placed inside it by its alignment: at its top, centred, at its bottom,
or on its row's baseline. The table's baseline is its first row's (a
row with no cell on its baseline having the lowest bottom of its
cells' content for one), which is what an `inline-table` aligns by on
its line, and a table with no row aligns by its bottom margin edge; the
last row's is what a box ending in the table takes as its last
baseline, and to carry both, layout records a box's first baseline
beside its last.

**The separated border model.** `border-collapse: separate`, the
initial value and the UA sheet's: `border-spacing` lies between every
two cells and between the cells and the table's padding edge; each
cell paints its own border and background; a column, a column group, a
row group and a row paint their backgrounds under the cells of their
area (§17.5.1's layers, in that order, which is the order the fragment
tree keeps them in), and a cell with no in-flow or floating content —
white space collapsed away being none, an absolutely positioned box
alone leaving it empty — under `empty-cells: hide` paints neither
border nor background, its border and padding keeping their room.
Spacing is counted in MIN, MAX and the table's used width, and in the
rows' sum for its height.

**The collapsed border model (M12.5).** `border-collapse: collapse`:
`border-spacing` and `empty-cells` are ignored, and every segment of
every grid line — per column of a horizontal line, per row of a
vertical one — is resolved to one border among those that meet there,
the cells' on either side and, where the line is their edge, the
rows', row groups', columns', column groups' and the table's, by CSS2
§17.6.2.1's rules in order: `hidden` wins over everything and leaves
no border; `none` loses to everything, and all `none` is no border;
the wider wins; among equal widths the style wins in the order
`double`, `solid`, `dashed`, `dotted`, `ridge`, `outset`, `groove`,
`inset`; among equal styles the element further up the list cell, row,
row group, column, column group, table wins, and for two of the same
kind the one further left or up. A line inside a cell spanning across
it has no segment there. The geometry is css-tables-3 §3.7.1.1's,
which is what the browsers do, not CSS2 §17.6.2's: every resolved
border is halved exactly, a 3px line putting 1.5 on each side, its
edges snapped where it is painted; each cell holds half of each line
on its side as its border, in layout and in its measures — the widest
half along a side where the line changes under a spanning cell — so
the automatic algorithm counts the halves where the separated model
counts spacing; and the table has no padding, its border being half
the widest border along each of its outer lines, so that its border
box holds them whole with its background beneath them, where CSS2 ends
the table's box at the outer lines' middle and puts their outer halves
in its margin. A grid with no rows or no columns has no lines and no
border. Neither a cell's own border nor the table's is painted in this
model: layout emits each resolved segment as a fragment of its own,
the table's, with just that one side set in its style, which `paint`
paints with the side's own style (`double`, `dashed` and the rest
included, `inset` and `outset` as `ridge` and `groove`, §17.6.3) as it
paints any border. A segment lies across its line as the line is split
and, at each end, where lines cross, reaches over the joint by half
the widest border crossing it there when no border meeting at the
joint wins over it, and otherwise stops at the joint's edge: a joint
is its winner's, so neighbouring segments meet square and nothing
shows through a winning `double`'s gap, as in the browsers. The
segments are the table box's last children, after the cells'
backgrounds, lowest precedence first, and a point on a line is the
table's.

**Captions.** A `table-caption` box is laid out as a block at the
width of the table's border box, above it for `caption-side: top` and
below it for `bottom`, several of a side stacked in source order, each
with its own margins. The `Table` box has two fragments, CSS2 §17.4's
two boxes: the wrapper's, which holds the captions and the table box,
paints nothing and carries the table's positioning and its place in the
flow, the table's margins standing outside captions and grid together;
and the table box's inside it, which paints the table's background and
border over the grid's part alone. A caption's own minimum content
width (CAPMIN) is one of the floors of the table's used width.

**The table in its flow.** A table is a block formatting context root
for the flow around it: its margins collapse with its siblings' and
never with anything inside it, it dodges floats like any root (M11.2,
its `auto` width computed in the room it is given), and floats inside
a cell or a caption stay in that cell's or caption's own context. A
floated or absolutely positioned table is a grid at its shrink-to-fit
width. Its intrinsic contribution to a shrink-to-fit ancestor — a
float, an inline-block, a cell — is its MIN and MAX, a length `width`
raising both and never lowering them. An `inline-table` is placed on
its line as an inline-block is, as wide as its margin box, its baseline
the first row's whatever its `overflow`. `overflow` on a table is not
applied: CSS2 gives it to block containers, and a table is not one.

**Fragments, painting and hit testing.** Layout produces fragments for
the table (the wrapper and the table box), each caption, each column
group and column, each row group, each row, each cell (its border box,
stretched to its rows, with its content as children, offset by its
alignment) and, in the collapsed model, each border segment. A column
group, a column, a row group and a row paint nothing of their own: in
the separated model their backgrounds show only under the cells of
their area, never in the spacing (§17.6.1), so each paints its
background as one fragment per cell, under the cell, in §17.5.1's order
— column group, column, row group, row, cell — which is the tree's. A
row's and a row group's fragments cover their rows' area; a column's
has no size, since a column is never hit. The paint-order walk needs
nothing new: these are block-level boxes in flow, painted in tree order
in the blocks pass and their inline content in the inline pass; a
positioned table part is painted at its layer as any positioned box
is. Hit testing answers with the cell under the pointer, the row in
the spacing between its cells, the row group between its rows, the
table outside the groups, and the table on a collapsed line.

**Acceptance against a reference browser (M12.7).** The harness of
M10.7 and M11.7: box-only pages under `crates/engine/tests/reference/`,
captured by `capture.sh` from the headless shell, compared byte for
byte, text kept out since text rasterizers differ by design — so the
"real-world" tables the roadmap names are real-world structures with
sized boxes for content. Five pages, all exact: a plain grid with the
UA sheet's spacing and padding, a caption at the top and one at the
bottom, a header and a footer group out of source order, cells at the
top, middle, bottom and on the baseline, and a second grid with
spacing of its own (22); the automatic algorithm sharing out a given
width past the maximum and between the guesses, percentage columns,
cells with widths, a column box with one, and `margin: auto` (23);
`colspan` and `rowspan` in the shapes real pages use, a cell pushed
right by a span from above, a span past its row group and `rowspan`
0 (24); the collapsed model with every conflict rule exercised,
`hidden` and `none` included, the halves of odd widths, a joint a
`double` wins, a column's border, and the table's background in its
own border beside a hidden segment (25); and nested tables, an
automatic one in a cell of another beside a `table-layout: fixed` one
whose content overflows, and `display: table-cell` on plain elements
(26). Hit testing through a table and the pixels on its collapsed
lines are engine tests.

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
percentage heights on rows, row groups and cells, which count as
`auto` (CSS2 leaves a row's undefined; the browsers resolve them
against the table); `min-height` and `max-height` on rows and row
groups; and a row group with no rows, which takes no share of a table's
height, where a browser gives it one.

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
whatever they find. (8) The documents. All eight landed on 2026-10-01
and 2026-10-02, one commit each.

## 4. What the code taught

Ten things this note did not foresee, each corrected above where it
stands. A cell's length `width` is its column's maximum and constrains
the column, not a minimum (css-tables-3 §3.9.1), and among the sizing
guesses the constrained columns grow before the unconstrained ones, the
order §3.9.3 gives: the note had both the other way round. The fixed
algorithm takes percentages of the width left after the spacing, as
the browsers do, not of the table's width, and a column without a
width takes its group's. A floated or absolutely positioned table is
built as the float or absolute box with a table's children, and step 3
chose the table path by box kind, which stacked such a table's rows as
blocks until step 4 chose it by display. An inline table aligns by its
first row and a block ending in a table by its last, so layout now
records a box's first baseline beside its last. The reference pages
found the rest. Page 24: a rowspan's excess, which the note shared
among its rows in proportion to their heights, goes first to the rows
where another rowspan starts, constrained rows taking it only when no
other row can and the last row when no row has a height, and of two
cells ending together the shorter goes first. Measuring the reference
browser on small tables to pin that down showed two more: a table's
own height goes to its body groups before their rows, the header and
footer groups taking none, and a row group's `height` is a minimum,
neither of which the note had. Page 25: the collapsed model in the
browsers is css-tables-3's, not CSS2's — exact halves where the note
chose to round, and the outer lines whole inside the table's border
box where the note put half of them in its margin — and a joint is its
winner's, a losing segment stopping at its edge, which a `double`'s
gap showed at one pair of pixels.
