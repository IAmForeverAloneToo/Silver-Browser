# Design note: grid

Roadmap M18. Written before the code, as the record of the decisions;
what ships is described in `docs/architecture.md` "Current state" when
the code lands. Where this note and the code later disagree, the code
wins and this note is corrected, as the notes before it were.

The problem: `display: grid` is not a value this engine knows. The
declaration is dropped, the element stays the block it would otherwise
be, and its children stack down the page one under the other. Grid is
how today's pages lay out in two dimensions at once: a page's header,
sidebar, main column and footer by named areas; a gallery of photographs
that fits as many columns as the window allows; a dashboard of cards,
some two columns wide; a form whose labels and fields line up in
columns. On those pages the engine draws one long column where the page
has a grid. This milestone adds the grid layout model: the grid
container and its items, the explicit grid of rows and columns with
their sizes and names, the implicit grid that grows to hold what is
placed outside it, the placement of items by line, by name, by span and
automatically, the track sizing algorithm, alignment in both axes,
baselines, and absolutely positioned boxes against the grid.

The standards are read at the versions current when this note was
written: CSS Grid Layout Level 2 (Candidate Recommendation Draft of 30
October 2023, which is what the Editor's Draft serves too), whose
sections are Level 1's with subgrid added; CSS Box Alignment Level 3 (4
September 2026) for the alignment properties and baselines; CSS Gaps
Level 1 for the gaps; CSS Box Sizing Level 3 (6 September 2026) for
intrinsic contributions and cyclic percentages; and CSS Display Level 3
(5 June 2026) for blockification. Where the reference browser and the
web-platform-tests agree with each other against the text of a draft,
this engine follows them, and says so where it does. A handful of
questions the drafts leave open were put to the reference browser before
this note was written, and its answers are given where they decide
something.

## 1. Decisions

**The properties.** `display` gains `grid` and `inline-grid`, and these
properties arrive, none of them inherited:

- `grid-template-columns` and `grid-template-rows`: `none | <track-list>
  | <auto-track-list>` (§7.2), with:
  - line names, `[ <custom-ident>* ]`, where `span` and `auto` are not
    names;
  - track sizes: a non-negative `<length-percentage>`, a non-negative
    `<flex>` (`fr`), `min-content`, `max-content`, `auto`,
    `minmax(<inflexible-breadth>, <track-breadth>)`, whose minimum may
    not be a flex, and `fit-content(<length-percentage>)`;
  - `repeat(<integer [1,∞]>, …)` around any track sizes and names, never
    nested;
  - `repeat(auto-fill | auto-fit, …)` around fixed sizes only, at most
    once in a list, whose other sizes must then be fixed too (§7.2.3.1's
    grammar). Adjacent name lists that a repetition produces are merged
    (`repeat(2, [a] 1fr [b])` is `[a] 1fr [b a] 1fr [b]`);
  - `subgrid` is not parsed (section 2), so a declaration with it is
    dropped, as a parse error is.
- `grid-template-areas: none | <string>+` (§7.3): each string tokenized
  into named cells, null cells (one or more `.`) and trash, which makes
  the declaration invalid. Every string must have the same number of
  cells, at least one, and each name's cells must form a filled
  rectangle, or the declaration is invalid.
- `grid-template` (§7.4): `none`, `<'grid-template-rows'> /
  <'grid-template-columns'>`, or the areas form, strings each with an
  optional row size and line names around it, then `/` and an explicit
  track list without `repeat()`.
- `grid-auto-columns` and `grid-auto-rows: <track-size>+` (§7.6),
  initial `auto`. A list is a pattern: the first implicit track after
  the explicit grid takes its first size, and the last implicit track
  before the explicit grid takes its last.
- `grid-auto-flow: [ row | column ] || dense` (§7.7), initial `row`.
- `grid` (§7.8): `<'grid-template'>`, or `<'grid-template-rows'> / [
  auto-flow && dense? ] <'grid-auto-columns'>?`, or `[ auto-flow &&
  dense? ] <'grid-auto-rows'>? / <'grid-template-columns'>`. It resets
  all six of the explicit and implicit grid's properties and none of the
  gaps.
- `grid-row-start`, `grid-row-end`, `grid-column-start` and
  `grid-column-end: auto | <custom-ident> | [ <integer> &&
  <custom-ident>? ] | [ span && [ <integer [1,∞]> || <custom-ident> ] ]`
  (§8.3). An integer of zero is invalid.
- `grid-row` and `grid-column: <grid-line> [ / <grid-line> ]?`, and
  `grid-area: <grid-line> [ / <grid-line> ]{0,3}` (§8.4): an omitted end
  is the start when that is a `<custom-ident>`, else `auto`, and so on
  around as §8.4 lays out.
- `justify-items: normal | stretch | <baseline-position> |
  <overflow-position>? [ <self-position> | left | right ] | legacy |
  legacy && [ left | right | center ]`, initial `legacy`, and
  `justify-self: auto |` the same without `legacy` (css-align-3 §6.1,
  §6.2). `justify-self` applies to grid items and absolutely positioned
  boxes; in a flex container it has no effect (css-align-3 §6.1).
  `legacy` alone computes to the parent's value when that has `legacy`
  in it, and to `normal` otherwise; `justify-self: auto` takes the
  parent's `justify-items` at layout, its `legacy` dropped.
- `place-items` and `place-self` now set their justify half, which M17
  checked and dropped: the second value, or the first when there is
  none.
- the legacy names `grid-row-gap`, `grid-column-gap` and `grid-gap`,
  which css-align-3 §8.3 requires as aliases of `row-gap`, `column-gap`
  and `gap`, mapped before the cascade looks the name up, as the
  `-webkit-` names are.

`row-gap`, `column-gap`, `gap`, `align-items`, `align-self`,
`align-content`, `justify-content`, `place-content` and `order` came
with M17; grid reads them as they are. Lengths in track sizes are
absolute once computed, as everywhere in the cascade: `em`, `rem`, the
viewport units and the rest resolve there, and percentages are kept for
layout. `calc()` waits for M21, as it does everywhere.

**Where the values live.** The lesson of M17's T4.3 was that every byte
in `ComputedStyle` is paid for on every box and every fragment that
copies its style: 48 bytes for flexbox cost 5% on the appended
paragraph's frame. The grid's properties are large — a track list is a
list, a template a table of names — and most elements never set any of
them. So they live in one value, `style::computed::GridStyle`, behind
`ComputedStyle::grid: Option<Arc<GridStyle>>`. `None` is every initial
value, and `ComputedStyle::grid()` reads either through one accessor. A
declaration sets one through `Arc::make_mut`, which copies the shared
value once for the element that changes it. None of the properties is
inherited, so a child starts from `None`. The two justify properties are
a few bytes each, and sit in `ComputedStyle` beside `align-items` and
`align-self`, since `justify-items` has to read its parent's for
`legacy`. The struct grows by the pointer's eight bytes and whatever of
the pair its padding does not absorb; a test states its size, so that
growth is a decision rather than a discovery.

Inside `GridStyle`:

- a track list is its tracks and its lines' names, with the
  repetitions kept as written, `auto-fill` and `auto-fit` among them,
  since a repetition's count is known only at layout;
- a name is an `Arc<str>`, compared as written, since a
  `<custom-ident>` is case-sensitive;
- the areas are their names with each one's rectangle of rows and
  columns, worked out once by the cascade;
- the four placement properties are each `auto`, a line, or a span.

**Blockification.** The children of a grid container are blockified
(§6.1, css-display-3 §2.7), as a flex container's are: an element, or a
`::before` or `::after`, whose parent's computed `display` is `grid` or
`inline-grid` takes `display.blockified()`, where `inline-grid` becomes
`grid`. A floated or absolutely positioned `inline-grid` computes to
`grid` (§5.1). `float` and `clear` keep their computed values and have
no effect on an item; nor has `vertical-align`. `::first-line` and
`::first-letter` do not apply to a grid container (§5.1), and their
finders take one for content that ends the search, as they take a table
or a flex container now.

**The box tree.** A grid container is a box kind of its own, `Grid`:
block-level when its `display` is `grid`, an atomic inline-level box on
its line when it is `inline-grid`, and a `Float` or `Absolute` box with
a grid container's children when it floats or is absolutely positioned,
as a table and a flex container are. `LayoutBox::is_grid_container`
answers for all of them. The children follow §6, which is word for word
what a flex container's are: each in-flow child element, `::before` and
`::after` is an item, the box its blockified display makes; an
absolutely positioned child is an `Absolute` box and no item; each run
of text children is an anonymous item, and a run of nothing but white
space makes none. So the builder's flex-container function becomes the
function for both, by a name that says so. Children stay in document
order in the box tree.

**`order`.** Auto-placement takes the items in order-modified document
order (§8.5), the box tree's order stably sorted by `order`. Painting
does too (§6.5), so the items' fragments go into the container's in that
order, as a flex container's do, and the box tree numbers a grid
container's children by `order` for the places in tree order that the
paint walk sorts positioned boxes by (M17's rule, unchanged).

**The grid's lines.** Lines are numbered from 1 at the start of the
explicit grid. The explicit grid has as many columns as the larger of
`grid-template-columns`' tracks, after its repetitions are counted, and
`grid-template-areas`' columns; the columns the areas add beyond the
track list take `grid-auto-columns`' sizes; the same goes for rows
(§7.1). Each explicit line has the names the track list gives it, and an
area `foo` names the lines at its edges `foo-start` and `foo-end`, in
both axes (§7.3.2), as more names beside the written ones. Layout keeps,
for each name, the sorted list of explicit lines that carry it, so that
"the third line called `col`, counting from the end" is a search in one
short list.

**The limit (§5.4).** Lines are clamped to the range from -10,000 to
10,000, the floor the draft suggests and the limit another engine ships.
A placement that reaches outside is clamped as §5.4 says: a span is cut
at the last line, and an area wholly outside is moved to the last track
on its side with a span of one. A repetition's count is limited so that
the explicit grid has no more than 10,000 tracks. The reference browser
allows far more: an item at line 20,000 made it 39,996 columns. That
difference is chosen, since a grid of a million tracks a page can ask
for in one declaration is a million tracks to size, and no page lays
anything out past the ten-thousandth line on purpose.

**Placement (§8.5).** Each item's four placement properties are resolved
to a placement in each axis: a definite position (a start line and an
end line), or automatic with a span.

- A line number counts lines from the start, a negative one from the
  end of the explicit grid. With a name, only lines with that name are
  counted, and past the explicit grid every implicit line counts as
  having it.
- A bare `<custom-ident>` is first the edge of the named area (the first
  line called `foo-start` for a start, `foo-end` for an end), and
  otherwise the first line with that name.
- A span counts lines from the other edge, in its direction. A named
  span counts only lines with the name, and every implicit line on the
  side it searches.
- §8.3.1's conflicts: lines in the wrong order swap, an end line equal
  to the start is removed, of two spans the end's is removed, and a
  named span on its own is a span of one.

The algorithm then runs as §8.5 writes it, in `grid-auto-flow`'s
direction (with `column` the rows and columns trade places):

1. Items with a definite position in both axes are placed.
2. Items locked to a row, in order-modified document order, take the
   first column where they overlap nothing: after the last item this
   step placed in the same row when the packing is sparse, from the
   start when it is `dense`.
3. The implicit grid gets the columns that any item with a definite
   column reaches before or after the explicit grid, and the columns the
   widest automatic span needs.
4. The remaining items are placed by the auto-placement cursor, which
   moves only forward when the packing is sparse and starts again from
   the first row and column for each item when it is `dense`. An item
   with a definite column moves the cursor to that column, going a row
   down if that is behind it, and then down until the item fits. An
   item with neither looks along each row for a place that overlaps
   nothing, and adds rows as it needs them.

Lines may be negative until placement ends, and are then shifted so that
the implicit grid starts at zero. Which cells are taken is kept per row
as a short sorted list of the column ranges in use, rather than as a
table of rows by columns: the limit allows 20,000 lines in each axis,
and a table of every cell could be 400 million entries for a page that
places two items far apart. The search for a free place checks only the
rows an item would cover.

Placement depends on the explicit grid, whose `auto-fill` and `auto-fit`
counts depend on the container's size. So it runs where layout does, at
the container's content size, each time the container is laid out; it is
one pass over the items, small beside the sizing that follows it.

**Automatic repetitions (§7.2.3.2).** `auto-fill` repeats its tracks as
many times as fit in the content box, with the gaps, when the container
has a definite size or maximum size in the axis; at least once. With a
definite minimum size instead, it takes the fewest repetitions that fill
it; otherwise one. Each track counts as its maximum sizing function when
that is fixed, else its minimum; a fixed minimum floors a fixed maximum;
a track is never taken as less than one pixel, and a percentage of an
indefinite size makes the count one. In this engine a block-level grid
container's content width is always definite, and its height is definite
when a length, a percentage of a definite height or an imposed height
gives it one. A float, an `inline-grid` or an absolutely positioned
container with an `auto` width sizes itself to fit: its own widths are
measured with one repetition, and its final layout, at the width found,
counts again, as in the reference browser, where such a float with
`repeat(auto-fill, 30px)` and three items is 30 pixels wide.
`auto-fit` is the same, except that after placement a repeated track
that no item occupies or spans collapses: its sizing functions become a
fixed `0px`, and the gaps on either side of it become one (§7.2.3.2,
§11.1).

**Track sizing (§12).** The grid layout algorithm is §12's:

1. place the items (above);
2. find the container's size in the axis whose size the context gives
   (below);
3. size the columns, then the rows, with the track sizing algorithm;
4. align the tracks (`justify-content` and `align-content`);
5. lay out each item in its grid area.

§12.1 also has the columns sized again after the rows, and the rows
again after that, when an item's min-content contribution has changed
in between. That happens only for an item whose inline size depends on
its block size: an orthogonal flow, a multi-column container, a wrapping
column flex container measured by its height, or an aspect ratio. This
engine has none of them (section 2), its multi-line column flex
containers taking the single-line answer for their widths (M17). So the
second rounds would change nothing, and are not run until one of those
arrives with something for them to do.

The track sizing algorithm is written once for an axis, over a list of
tracks and the items' spans and contributions in that axis, so that the
same code sizes the columns and the rows and can be tested without a box
tree. Each track has a minimum and a maximum sizing function (§12.2:
`minmax()`'s two arguments; `auto` as the minimum of a flex or
`fit-content()` track; `auto` and `fit-content()` as a maximum treated
as `max-content`, `fit-content()` limited by its argument), a base size
and a growth limit. The gaps are fixed tracks between them, spanned by
any item that crosses their line (§11.1). The steps are §12.4 to §12.8:

1. *Initialize* (§12.4): a fixed minimum is the base size, a fixed
   maximum the growth limit; otherwise zero and infinity, the growth
   limit never below the base size.
2. *Intrinsic sizes* (§12.5):
   - shims for the items aligned by a baseline (see Baselines);
   - the items that span one track, by the five rules for min-content,
     max-content and `auto` minimums and for min-content and max-content
     maximums, `fit-content()` clamping;
   - the items that span more than one track and no flexible one, a
     span at a time, from two upwards. Each span's items accommodate,
     in this order, their minimum contributions in the base sizes of
     tracks with an intrinsic minimum; their min-content contributions
     in those with a `min-content` or `max-content` minimum; their
     limited max-content contributions under a max-content constraint;
     their max-content contributions in those with a `max-content`
     minimum; then their min-content and max-content contributions in
     the growth limits, with §12.5's "infinitely growable" flag. Space
     is distributed by §12.5.1: a planned increase per track, so the
     result does not depend on the order of the items; the space shared
     equally up to each track's limit; then into the tracks spanned but
     not affected; then beyond the limits, into the tracks §12.5.1 names
     for each kind of contribution;
   - the items that cross a flexible track, all together, distributing
     only into the flexible tracks and by their flex factors;
   - an infinite growth limit left over becomes the base size.
3. *Maximize* (§12.6): positive free space goes to the base sizes
   equally, each track frozen at its growth limit. Under a max-content
   constraint the free space is infinite, under a min-content constraint
   zero. When that makes the grid larger than the container's
   `max-width` or `max-height` allows, the step runs again in that room.
4. *Flexible tracks* (§12.7): the used flex fraction is found by §12.7.1
   from the available space when it is definite, which takes the
   flexible tracks whose share would fall below their base size out and
   tries again; when it is not, the largest of each flexible track's
   base size over its factor (a factor below one counting as one) and,
   for each item that crosses a flexible track, the size of an fr that
   fits its max-content contribution into the tracks it crosses
   (§12.7.1 again). Under a min-content constraint, or with no free
   space, it is zero. Flex factors whose sum is below one take that
   share of the space and leave the rest. A fraction that makes the grid
   smaller than the container's minimum size, or larger than its
   maximum, is found again in that room.
5. *Stretch `auto` tracks* (§12.8): when the axis's `justify-content` or
   `align-content` is `normal` or `stretch`, the tracks with an `auto`
   maximum share the remaining positive free space equally; with an
   indefinite size and a definite minimum size, the free space is
   measured against the minimum.

Sums of factors and sizes are taken in `f64`, as flex's loop takes them,
so that a page of very large values stays finite; the sizes come back
as `f32`.

**The available space and percentages.** The available grid space in an
axis is the content box when its size is definite, and otherwise the
constraint the container is being sized under (§12.2). A block-level
grid's columns are sized in its content width; its rows, when its height
is not definite, under a max-content constraint, since an `auto` block
size is the max-content size (§5.2). Its height is then the rows and
gaps, clamped by `min-height` and `max-height`. A percentage track
resolves against the content box. When the container's size depends on
its tracks, the percentage counts as `auto` while that size is found,
and resolves against the size found for the layout itself (§7.2.1).
That is what the reference browser does: rows of `50% 20px` in an
`auto`-height grid make it 60 pixels tall, its first row's item
counting as `auto`, and then put the second row 30 pixels down. A float
with columns `50% 20px` does the same across. Percentage gaps resolve
the same way (css-align-3 §8.1): against zero while the container's
size depends on them, and against the size found when the items are
placed. In the reference browser, a row gap of 10% in an `auto`-height
grid adds nothing to its height and puts the second row 4 pixels lower
in a grid 40 pixels tall. That is not flexbox's rule, where such a gap
stays zero (M17), and the two are kept apart.

**Contributions and the automatic minimum (§6.6, css-sizing-3 §5.2).**
An item's contributions are its outer sizes: margins, border and padding
included, `auto` margins counting as zero (§11.2). Percentage margins
and paddings, which resolve against the grid area's width when the item
is laid out (§6.4), count as zero in a contribution, and so does a
percentage minimum. A percentage width, height or maximum counts as
`auto` or `none` (css-sizing-3 §5.2.1). In the columns:

- the min-content and max-content contributions are the item's
  `contribution` from `layout::intrinsic`, as a block container's
  children give theirs, its `width`, `min-width` and `max-width` and
  their intrinsic keywords applied;
- the minimum contribution, which `auto` minimums use, is the outer size
  the item would have with its minimum size as its preferred size, when
  its `width` is `auto` or a percentage; else its min-content
  contribution.

In the rows, each item is laid out at its column area's width less its
margins (or `fit-content` in it when it does not stretch, below), and
its outer height is both its min-content and its max-content
contribution, as a block container's min-content height is its height
at its width (css-sizing-3 §5.1). Its minimum contribution is found the
same way from `min-height`. While the rows are being sized, the item's
area has no height, so a percentage height inside it is `auto`, unless
every row it spans has a fixed maximum. In that case the area's height
is taken as those rows' maximums and the gaps between them (§12.1's
estimate for the block axis).

An item's `min-width: auto` or `min-height: auto` is its automatic
minimum size (§6.6): the content-based minimum size when the item is not
a scroll container, it spans at least one track whose minimum is `auto`,
and, if it spans more than one track, none of them is flexible; zero
otherwise. The content-based minimum is the item's size when that is
definite (the specified size suggestion), else its min-content size (the
content size suggestion), clamped by a definite maximum size. When every
track it spans has a fixed maximum, the two suggestions are also clamped
to fit, with the item's margins, border and padding, within those
maximums and the gaps between them. That is what `minmax(auto, 40px)` is
for: in the reference browser a long word in such a column leaves it 40
pixels wide, where `auto` would grow it to the word. The same minimum
holds the item when it is laid out, not only in the track sizing, as
for a flex item.

**Laying out the items (§6.2, §12 step 5).** Each item is laid out as a
formatting context root (`Content::Root`), its containing block its grid
area: the tracks it spans and the gaps between them, the space that
`justify-content` and `align-content` add between those tracks included.
The area's width and height are definite (§12):

- an item stretches in an axis when its `justify-self` or `align-self`
  is `stretch`, or `normal` (an item here is never replaced, section 2),
  its size in that axis is `auto`, and neither margin there is `auto`:
  it takes the area's size less its margins, clamped by its minimum and
  maximum; a stretched height is imposed and definite, so percentages
  inside it resolve;
- an item that does not stretch takes its `width` if it has one, and
  otherwise `fit-content` in the area's width less its margins, border
  and padding; its height is its content's, unless its `height` says
  otherwise, against the area's height, which is definite;
- percentage margins and paddings resolve against the area's width
  (§6.4), and the margins of adjacent items do not collapse.

Items are laid out through the pass's kept item layouts, keyed as a flex
item's are, with the area's width as the containing block's width and
its height as the containing block's height when the item's own height,
minimum or maximum is a percentage. An item measured for the rows at its
area's width and then laid out with its stretched height costs two
layouts, the measuring one read where the pass keeps it.

**The container's own sizes (§5.2).** A block-level grid container is
sized as a block that establishes a formatting context: its width as a
block's, floats kept out of it as from any formatting context root, its
margins its own. Its height is its definite height, else its rows and
gaps, clamped by `min-height` and `max-height`, a keyword limit being
its content's height, as a flex container's is. Its min-content and
max-content widths are the sums of its columns and gaps when they are
sized under a min-content and a max-content constraint (§5.2):
`layout::intrinsic` asks `crate::grid::content_widths`, as it asks flex
and tables, and the pass keeps the answer. That gives the shrink-to-fit
width of an `inline-grid`, a float, an absolutely positioned grid and a
table cell, and a grid item's contribution to an outer grid or flex
container. Under a max-content constraint, the flexible tracks are sized
by §12.7's indefinite rule: in the reference browser, a float with
columns `1fr 2fr` and two items 30 pixels wide is 90 pixels wide.

**Alignment (§11, css-align-3).** What the values mean in a grid
container:

- `justify-items`, `justify-self`, `align-items` and `align-self` place
  each item in its area. `normal` is `stretch` (§6.2). `stretch` on an
  item that cannot stretch, because its size or a margin is `auto`,
  takes its fallback, `start`. `start` and `end` are the area's,
  `self-start` and `self-end` the same with one writing mode, and
  `flex-start` and `flex-end` are `start` and `end` outside a flex
  container. `left` and `right` are physical in `justify-self`. A
  baseline value aligns the item with its baseline-sharing group (see
  Baselines).
- `auto` margins take the positive free space in their axis before
  alignment does, equally between the two when both are `auto`, and turn
  the item's self-alignment off in that axis. An item that overflows
  its area has `auto` margins of zero (§11.2).
- `justify-content` and `align-content` place the tracks in the content
  box when they do not fill it. `normal` behaves as `stretch`, which has
  grown the `auto` tracks already (§12.8) and otherwise places the
  tracks at the start. `space-between`, `space-around` and
  `space-evenly` spread the free space between the tracks, so that an
  item spanning a gap gains that space in its area (§11.5). In the
  reference browser, with rows of 20, 20 and 5 pixels and
  `space-between` in a grid 100 pixels tall, an item spanning the first
  two rows is 67.5 pixels tall. The fallbacks are flexbox's: `start` for
  `space-between`, `safe center` for `space-around` and `space-evenly`,
  and `start` for `stretch` with negative free space. `baseline` in
  `align-content` is `start`, and `last baseline` `safe end`, as for a
  flex container, which step 5 checks against the reference browser for
  a grid. Collapsed tracks take none of the space.
- Without `safe` or `unsafe`, alignment is unsafe, as in the reference
  browser: an item wider than its area, centred in it, overflows on both
  sides. With `safe`, an item that overflows is aligned at the start.

The alignment arithmetic that flex has — where an alignment puts its
subjects, how a distribution spaces them, the fallbacks, `safe`, and the
static position's alignment — moves out of `layout::flex` into a module
of its own, `layout::align`, which both use. Flex keeps its own
flex-relative mapping of the keywords; grid maps them with every axis
running left to right and top to bottom.

**Baselines (§11.6, css-align-3 §9).** An item takes part in baseline
alignment when its `align-self` (or `justify-self`) is a baseline value,
neither margin in that axis is `auto`, and its size in that axis does
not depend on an intrinsically sized track it spans (§11.3, §11.4): a
percentage size in a track sized by content. When the container's size
in that axis is indefinite, a flexible track counts as intrinsically
sized. Such an item takes the fallback, `start` or `end`.

- *Groups.* The items with `align-self: baseline` (or `first baseline`)
  whose area starts in the same row share that row's first baseline. The
  items with `last baseline` whose area ends in the same row share its
  last. Within a group the baselines meet. In the first group, the item
  whose baseline is furthest below its outer top sits at the top of its
  area; in the last group, the item whose baseline is furthest above its
  outer bottom sits at the bottom.
- *Shims.* The track sizing gives each such item a shim (§12.5 step 1),
  so that the row is as tall as the group needs. The shim is the
  group's largest distance from its aligning edge to a baseline, less
  the item's own: at the item's start for the first baseline, at its
  end for the last. It is counted in the item's contributions.
- *The item's baselines.* An item's first and last baselines are its
  layout's. One with none synthesizes them from its border box's bottom
  edge, as a flex item's are (css-align-3 §9.1).
- *`justify-self: baseline`.* In the inline axis, items have no
  baselines in this engine's one writing mode, and the reference browser
  synthesizes them from the border box's left edge, as it does for a
  column flex container. So the column group lines up its items' left
  edges. Step 6 confirms that against the reference browser.

A grid container's first baseline (§11.6) is taken from its first row
that has an item. If items there take part in first-baseline alignment,
it is their group's baseline; else, if items there take part in
last-baseline alignment, theirs; else the first baseline of the first
item in grid order in that row that has one; else one synthesized from
the first item in grid order. For this purpose an item that spans rows
counts only in the row where its area starts, except for last-baseline
alignment, where it counts in the row where its area ends. Grid order is
the order of the cells row by row, items in the same cell taken in
order-modified document order. The last baseline is the same from the
last row, with first and last turned round. A grid with no items has no
baseline, and its context synthesizes one as it does for any box.

Around the container, the baselines are used as a flex container's are:

- an `inline-grid` sits on its line by its first baseline;
- a grid container gives a table cell its first baseline, and a block
  around it both of its baselines;
- an inline-block around it takes its first baseline, by the legacy rule
  M17 found for flex containers; step 6 confirms the rule holds for grid
  in the reference browser.

**Absolutely positioned boxes (§10).** When a grid container is an
absolutely positioned box's containing block, the containing block is
the grid area that the box's placement properties give, resolved against
the grid as laid out (§10.1). There is no auto-placement here:

- an `auto` line is the padding edge on its side;
- a line the grid does not have, named or numbered or reached by a span,
  is `auto` too, and no implicit tracks are made for it;
- a placement that is only a span is two `auto` lines;
- an area's edges are the tracks' edges, the gaps outside it.

In the reference browser, `grid-column: 2` with the four insets zero
covers the padding box from the start of the second column to the right
edge, since the end is `auto`. This holds for every box whose
containing block the grid container is, not only for its own children:
`settle_absolutes` takes, from the grid, a way to find each box's
rectangle, and lays the box out against that rectangle as it lays one
out against the padding box now.

An absolutely positioned child of a grid container, whether or not the
container is its containing block, is no item and takes no part in the
placement or the sizing. Its static position is found as if it were the
sole item in an area whose edges are the content box's (§10.2). That is
M17's static-position rectangle, the content box, with its alignment
taken from the child's `justify-self` and `align-self`, `auto` being the
container's `justify-items` and `align-items`, and `normal` and
`stretch` being `start`; never safe. In the reference browser, a
10-pixel box in a 200 by 100 grid with `justify-items: center` and
`align-items: end` sits at 95 across and 90 down, and `justify-self:
end` on it moves it to 190.

**Paint and hit testing (§6.5).** A grid item paints as an inline-block
does, as a flex item does: as a unit in the inline-level pass of its
stacking context, in order-modified document order, which the fragments
already have. An item whose `z-index` is not `auto` makes a stacking
context even when it is not positioned. Items that overlap in the same
cells are painted in that order, so that `z-index` and `order` decide
which is on top (M18.3's z-order). The paint walk knows an item by its
parent fragment, whose in-flow children are items when the parent is a
flex or a grid container; hit testing reads the same walk.

**The layout cache.** An in-flow grid container placed by `block` is
kept under its node like a block, a table or a flex container, with the
same conditions. Its items are laid out with it and not kept on their
own.

**Scroll containers (§5.3).** A grid container whose `overflow` clips
records its scrollable overflow area as a flex container does: the
items' margin boxes where the grid put them, and the grid itself, its
tracks and gaps, with the end padding after them; anything reaching
further by its own border box; never less than the padding box. An item
that is a scroll container has no automatic minimum size (§6.6).

**The layout pass.** A grid inside a grid item is laid out each time its
item is. The item is measured at its area's width for the rows, then
laid out at its stretched height; the pass keeps both, as it keeps a
flex item's. A chain of nested grids must cost a few layouts a level,
not two to the power of the depth, and the benchmark checks it (below).
The pass's kept content widths serve a grid's columns, whose sizing
reads each item's min-content and max-content widths, and a grid
container's own, which the intrinsic sizes of the grids around it read.

**The web-platform-tests.** M18's subset comes from the upstream
`css/css-grid` directory, pulled by M17's rule with nothing added to it
for grid. The rule's `style::supports` answers from the cascade's own
parsers, so tests that need subgrid, masonry (the directory's
`grid-lanes`), writing modes, the Ahem font or scripts fall out by the
reasons the README counts. The local checkout, at the upstream commit
the flexbox subset was vendored from, was widened to the directory to
see what it holds: 3,591 files, among them 1,000 masonry tests and 140
subgrid ones. Across the directory 659 files use a writing mode or
right-to-left text, 898 the Ahem font, and 405 are check-layout tests.
The subset is pulled at step 4 from that same commit, so the README
still names one, and `expectations.txt` gains a section for each step
that will make failing tests pass, emptied as the steps land.

**The golden cases.** Twenty box-only pages join `tests/reference/`,
numbered 50 to 69, captured from the reference browser by `capture.sh`
and compared byte for byte, as M10 to M17's are. Fixed-size boxes and no
text cover the corners of the algorithm:

- explicit tracks of every kind: lengths, percentages, `fr`, `minmax()`,
  `fit-content()`;
- `repeat()`, with `auto-fill` and `auto-fit` and the gaps;
- line names, named areas, and placement by name and number from both
  ends;
- implicit tracks before and after the explicit grid, sized by a
  pattern of `grid-auto-rows` and `grid-auto-columns`;
- sparse and dense auto-placement, by row and by column;
- spans by number and by name;
- intrinsic tracks with content of fixed size, and items spanning them;
- `fr` in an indefinite space: an `inline-grid` and a float;
- the automatic minimum, and `minmax(0, 1fr)` without it;
- `justify-items`, `justify-self`, `align-items` and `align-self`;
- `auto` margins in grid areas;
- `justify-content` and `align-content`, an item spanning a distributed
  gap;
- overlapping items, painted by `order` and `z-index`;
- absolutely positioned boxes: the grid area as containing block, and
  the static position of a child;
- nested grids, an `inline-grid` on a line, and grids sized to fit;
- gaps with percentages, and a grid that is a scroll container;
- baseline alignment in rows, and a grid's baselines on its line;
- `grid-auto-flow: column` and each form of the `grid` shorthand.

**Acceptance (M18.7).** Two parts, both check-layout pages under
`crates/engine/tests/layouts/`, captured from the reference browser by
M17's `layouts -- capture` and checked by the WPT runner's check-layout
code, every element of the body within a pixel. Each gives the reference
browser the engine's own font, DejaVu Sans, by an `@font-face` this
engine ignores.

- *A grid garden.* "CSS grid garden-style": a page of two dozen small
  gardens, a five by five grid each, in the manner of the puzzle game
  that teaches grid by watering plots. Each level adds one of the
  placement properties to those before it: start and end lines,
  negative lines, spans, `grid-column` and `grid-row`,
  `grid-area` with four lines and with a name, `order`, track sizes in
  `fr` and percentages and mixtures of them, `grid-template` and
  `grid-auto-flow`. The levels are written for this suite, not copied
  from the game.
- *Five real layouts,* with text: the roadmap's dashboard (cards of
  different spans in `auto` and `fr` tracks, a header across them), its
  photo gallery (`repeat(auto-fill, minmax(…, 1fr))` with captions) and
  its form grid (labels and fields in two columns, a field that spans
  both, the buttons in a row of their own), and two more that pages are
  full of: a page laid out by named areas (header, navigation, main,
  aside and footer by `grid-template-areas`) and a calendar month (seven
  columns, a week a row, events that span days).

`about:silver` gains a section laid out by grid, so the built-in page
shows it.

**Performance and fuzzing.** A benchmark, `benches/grid.rs`, lays out:

- a gallery of a thousand auto-placed items in `repeat(auto-fill,
  minmax(…, 1fr))` columns;
- a dashboard of spanning items in `auto` and `fr` tracks, which runs
  the spanning steps of §12.5;
- a chain of 64 nested grids, whose time must grow linearly with the
  depth, or the pass's kept layouts are not doing their job;
- a relayout after one item changes.

Its numbers go in `docs/perf-budgets.md`, and the existing benchmarks
must stay within 10% of the commit before this milestone (T4.3), by the
method M17 settled: base and head built side by side, runs alternated,
and instruction counts where the timings are noisy. The three fuzz
targets that lay pages out, `pipeline`, `mutation` and `style_document`,
run for fifteen minutes each with grid pages among their seeds and the
grid properties and keywords in a dictionary. The repository keeps
neither corpora nor dictionaries, so both are given to the run by hand.
Grid gives a page new ways to ask for a lot of work: a line at 10,000,
an `auto-fill` of one-pixel tracks in a wide container, thousands of
items spanning thousands of tracks. Unit tests hold each of those to
the limit above and to a time.

**The roadmap.** M18.6 asks for subgrid and masonry to be registered as
items of the long tail, Phase G, rather than built here: G1 for subgrid
(css-grid-2 §9) and G2 for masonry (css-grid-3, which now names it
`grid-lanes`). Neither blocks this milestone. The documents step adds
them.

## 2. Not in this milestone, said rather than hidden

- *Subgrid* (§9) and *masonry*: registered as G1 and G2 (M18.6), above.
  `grid-template-columns: subgrid` is not parsed, so it is dropped as
  any declaration this engine does not know is dropped; the element is
  a grid with its other template, as in a browser without subgrid.
- *Writing modes and `direction: rtl`* (M42): columns run left to right
  and rows top to bottom, as everything in layout does. With them go
  orthogonal flows, the second rounds of §12.1, and the inline-axis
  baselines that `justify-self: baseline` would align.
- *Aspect ratios and replaced items* (M23): `normal` behaves as `start`
  for a replaced item with a natural size (§6.2), and the transferred
  size suggestion of §6.6. Without images every item is a non-replaced
  box, for which `normal` is `stretch`.
- *Fragmentation* (§13) and the `break-*` properties: there is no paged
  medium yet (M20).
- *`calc()`* in track sizes and lengths (M21), and the *interpolation of
  track lists* (§7.2.3.3) with animations (M22).
- *The resolved values of `grid-template-rows` and `-columns`* (§7.2.6),
  which `getComputedStyle` gives: there is no script yet (M25).
- *The line limit* is ±10,000, the draft's suggestion, where the
  reference browser allows far more (above).
- *Self-alignment of an absolutely positioned box in its inset-modified
  containing block* (css-align-3 §6.1, css-position-3 §4). The reference
  browser applies `justify-self` and `align-self` to every absolutely
  positioned box whose insets leave it room: one 50 pixels wide between
  `left: 0` and `right: 0` in a 200-pixel container goes to 150 with
  `justify-self: end`, where CSS2 puts it at 0. That holds whatever the
  containing block, a grid area or a padding box, so it is
  positioning's, M11's area, and is registered in the long tail at the
  documents step rather than built for grid alone. Grid uses the two
  properties where §10.2 does, for a child's static position.
- *What the WPT subset leaves out*, among it the Ahem font, writing
  modes, scripts, images and `line-height`: the tests that need them
  wait for them, and the rule picks them up when they land.

## 3. Order of work

Each step is a vertical slice: build, tests, clippy green and a smoke
run before the next, with its sabotage pass. A step is done when its
tests pass.

1. *Style.* `display: grid | inline-grid`; blockification; every grid
   property and shorthand above into `GridStyle`, behind the pointer;
   `justify-items` and `justify-self`, `place-items` and `place-self`
   setting them; the legacy gap names; the size of `ComputedStyle`
   stated by a test. Layout reads none of it yet.
2. *The box tree and placement.* The `Grid` box kind, its items built by
   the builder flexbox uses, numbered by `order`; the explicit grid,
   line names and areas; resolving placements; §8.5's algorithm, sparse
   and dense, by row and by column; the implicit grid; the limit;
   automatic repetitions. Placement is tested on its own, without
   layout. M18.2 and M18.3's placement.
3. *Track sizing and layout.* §12's steps for an axis, tested on their
   own; contributions and the automatic minimum; the items laid out in
   their areas, stretched or at the start; the container's height;
   percentages and gaps; `auto-fit`'s collapse; `content_widths` and
   `inline-grid`; the first-line and first-letter finders. Here grid
   starts to draw. M18.1 and M18.4.
4. *The WPT subset*: the pull, the vendored directory, the expectations
   in sections by the steps to come. From here on each step runs it and
   shortens `expectations.txt`.
5. *Alignment*: `layout::align` shared with flex; every self-alignment
   value with `auto` margins; `justify-content` and `align-content` with
   the distributed gaps; `safe` and `unsafe`. M18.5.
6. *Baselines*: groups and shims, the container's baselines,
   `inline-grid` on its line, a table cell, the inline-block rule.
7. *Absolutely positioned boxes*: the grid area as containing block for
   any box the grid contains, and a child's static position.
8. *Paint and the cache*: items painted as inline-blocks, `z-index` on
   items, hit testing; grid containers in the layout cache. M18.3's
   z-order.
9. *The golden cases*: the twenty reference pages, and whatever they
   find.
10. *Acceptance*: the grid garden, the five layouts and `about:silver`.
    M18.7.
11. *Performance and fuzzing*: the benchmark, the T4.3 comparison, the
    fuzz targets run.
12. *The documents*: this note corrected to the code, the architecture's
    M18 entry, the roadmap's ticks (M18.1 to M18.7) with G1 and G2 and
    the positioning item registered, and the numbers in
    `docs/perf-budgets.md`.
