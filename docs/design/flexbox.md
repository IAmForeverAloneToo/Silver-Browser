# Design note: flexbox

Roadmap M17. Written before the code, as the record of the decisions;
what ships is described in `docs/architecture.md` "Current state" when
the code lands. Where this note and the code later disagree, the code
wins and this note is corrected, as the notes before it were.

The problem: `display: flex` is not a value this engine knows. The
declaration is dropped, the element stays the block or inline box its
other rules make it, and its children stack as blocks or run as a line.
Almost every page made since 2015 lays out its navigation bar, its
cards, its header, sidebar and footer, its centred dialogs and its form
rows with flexbox, so on most of the web the engine draws a column where
the page has a row. This milestone adds the flexible box layout model:
the flex container and its items, the two axes, the flexible lengths,
alignment on both axes, multi-line containers, baselines and absolutely
positioned children. It also adopts the web-platform-tests runner the
roadmap has asked for since M4 (T1.1), since the flexbox tests are where
it starts paying.

The standards are read at the versions current when this note was
written: CSS Flexible Box Layout Level 1 (Editor's Draft of 8 May 2026),
CSS Box Alignment Level 3 (4 September 2026) for the alignment
properties and baselines, CSS Gaps Level 1 for the gaps, CSS Box Sizing
Level 3 (6 September 2026) for the intrinsic size keywords, CSS Display
Level 3 for blockification, CSS Positioned Layout Level 3 (14 May 2026)
for the static-position rectangle, the Compatibility Standard (15 March
2026) for the `-webkit-` names, CSS Color Level 4 for the named colours,
and CSSOM View for what the check-layout tests measure. Where the
reference browsers and the web-platform-tests agree with each other
against the text of a draft, this engine follows them, and says so where
it does.

## 1. Decisions

**The properties.** `display` gains `flex` and `inline-flex`, and these
properties arrive, none of them inherited:

- `flex-direction: row | row-reverse | column | column-reverse`,
  `flex-wrap: nowrap | wrap | wrap-reverse`, and their shorthand
  `flex-flow`, each part at most once in either order;
- `order: <integer>`, initial `0`;
- `flex-grow` and `flex-shrink`: a finite number not below zero, initial
  `0` and `1`;
- `flex-basis: content | <'width'>`, initial `auto`;
- `flex: none | [ <'flex-grow'> <'flex-shrink'>? || <'flex-basis'> ]`:
  `none` is `0 0 auto`, `auto` is `1 1 auto`, an omitted grow factor is
  `1`, an omitted shrink factor `1`, and an omitted basis `0%`. The
  draft says `0` for the last; the web-platform-tests' parsing tests
  expect `0%` (`flex: 1` computes to `1 1 0%`) and so do the three
  engines, and the two differ when the container's main size is
  indefinite, where a percentage basis is `content`. A unitless zero not
  preceded by two factors is a factor (§7.1);
- `justify-content: normal | space-between | space-around | space-evenly
  | stretch | [ safe | unsafe ]? [ center | start | end | flex-start |
  flex-end | left | right ]`;
- `align-items: normal | stretch | [ first | last ]? baseline | [ safe |
  unsafe ]? [ center | start | end | self-start | self-end | flex-start
  | flex-end ]`, and `align-self: auto |` the same;
- `align-content: normal | [ first | last ]? baseline | space-between |
  space-around | space-evenly | stretch | [ safe | unsafe ]? [ center |
  start | end | flex-start | flex-end ]`;
- `row-gap` and `column-gap: normal | <length-percentage [0,∞]>`, and
  `gap: <'row-gap'> <'column-gap'>?` (CSS Gaps 1 §2, which took them
  over from css-align-3 §8). The flexbox draft's algorithm counts them,
  the card layouts that M17.10 asks for use them, and grid (M18) reads
  the same properties;
- the shorthands `place-content` (`align-content` then
  `justify-content`), `place-items` and `place-self`. The last two also
  set `justify-items` and `justify-self`, which apply to grid items and
  absolutely positioned boxes and have no effect on a flex item
  (css-align-3 §6.1). Their half is checked and dropped until grid has a
  use for it, so that `place-items: center` works on a flex container.

What the values mean in a flex container (css-align-3 §5.1, §6.2):
`normal` behaves as `stretch` for `align-content` and `align-items`, and
as `flex-start` for `justify-content`, whose `stretch` is `flex-start`
too: flex items grow with `flex-grow`, not with `justify-content`.
`start` and `end` are the container's own start and end, which
`*-reverse` does not flip. `self-start` and `self-end` are the item's,
the same thing with one writing mode. `left` and `right` are physical in
a row container and behave as `start` in a column container. A
distributed value that cannot distribute takes its fallback: `safe
flex-start` for `space-between`, `safe center` for `space-around` and
`space-evenly`, `flex-start` for `stretch` when the free space is
negative. `safe` aligns an alignment subject that overflows its
container as `start`. Without `safe` or `unsafe` the alignment is
unsafe, as the reference browsers have it, rather than css-align-3
§4.4's blend of the two, which none of them ships.

**Sizes.** `width`, `height`, `min-width` and `min-height` take `auto |
<length-percentage [0,∞]> | min-content | max-content | fit-content`,
and `max-width` and `max-height` the same with `none` in place of `auto`
(css-sizing-3 §3). `min-width` and `min-height` become `auto` initially,
which is `0` everywhere but on a flex item's main axis (below). This is
M17.3's "intrinsic sizing v1". In the inline axis, a width:

- `min-content` is the box's min-content width (`layout::intrinsic`);
- `max-content` is its max-content width;
- `fit-content` is `min(max-content, max(min-content, stretch))`, where
  `stretch` is the width an `auto` block would take.

The same sizes apply as floors in `min-width` and as ceilings in
`max-width`. The keywords apply to blocks, floats, inline-blocks,
absolutely positioned boxes and tables, and contribute to an ancestor's
intrinsic widths as `(min, min)`, `(max, max)` and `(min, max)`. In the
block axis, `height` and `min-height` take a keyword as `auto` and
`max-height` as `none`: for a box's block size the keywords are its
automatic size, unless otherwise specified (§3.2). The bare
`fit-content` is Level 3's, and the `fit-content()` function is Level
4's (see section 2). The computed style keeps the keywords as written,
in two new types for the sizes and the limits, and layout resolves them
as it resolves percentages.

**The `-webkit-` names.** The Compatibility Standard requires
`-webkit-flex` and `-webkit-inline-flex` as `display` keywords for
`flex` and `inline-flex` (§3.4.5). It also requires prefixed names as
aliases of the unprefixed properties (§3.4.1). Of those, this engine has
`-webkit-align-content`, `-webkit-align-items`, `-webkit-align-self`,
`-webkit-flex`, `-webkit-flex-basis`, `-webkit-flex-direction`,
`-webkit-flex-flow`, `-webkit-flex-grow`, `-webkit-flex-shrink`,
`-webkit-flex-wrap`, `-webkit-justify-content`, `-webkit-order` and
`-webkit-box-sizing`. Each is the same property under a second name,
mapped before the cascade looks the name up, so it competes in the
cascade as the property itself. `-webkit-box` and `-webkit-inline-box`
stay invalid: the Standard maps them to flexbox and notes in the same
breath that the mapping is not web compatible (issue 87).

**The named colours.** The cascade knows eighteen colour names, M5's v1
scope. This milestone replaces them with CSS Color 4's whole table of
148 (§6.1), `rebeccapurple` and both spellings of every `gray` included.
This is not flexbox, and it is taken here for the test suite: nearly a
third of the web-platform-tests' flexbox reftests and check-layout tests
paint with `lightblue`, `pink`, `lightgreen` or another name outside the
eighteen. The table is fixed by the standard and costs one `match`.
`rgb()` and `hsl()` stay out: the flexbox tests hardly use them.

**Blockification.** The children of a flex container are blockified
(css-display-3 §2.7, flexbox §4). The cascade does it as it does for
floats: an element, or a `::before` or `::after`, whose parent's
computed `display` is `flex` or `inline-flex` takes
`display.blockified()`, where `inline-flex` becomes `flex`. Their
`float` and `clear` keep their computed values, and the box tree ignores
them (§3: they do not create floating or clearance of a flex item). A
`::first-line` and a `::first-letter` are not blockified: neither
applies to a flex container. Nor is an element on a first line, which
inherits from the line's style, so it never sees a flex container as its
parent. The incremental restyle already recomputes an element's children
when its style changes in any property, so a container that becomes a
flex container blockifies its children on the next restyle.

**The box tree.** A flex container is a box kind of its own, `Flex`. It
is block-level when its `display` is `flex`. When it is `inline-flex` it
is an atomic inline-level box, an item on its line like an inline-block,
as `Table` is block-level or inline by its display. A floated or
absolutely positioned flex container is the `Float` or `Absolute` box
M11 gives such a box, with a flex container's children, as a floated
table is (M12). `LayoutBox::is_flex_container` answers for all of them,
and layout chooses the flex path by it. The children follow §4:

- an in-flow child element, `::before` or `::after` is a flex item: the
  box it would generate in a block container, now that its display is
  blockified (a block container, a table or a nested flex container);
- a floated child is built as it would be in flow;
- an absolutely positioned child is an `Absolute` box, which is not an
  item (see below);
- each run of consecutive text children is an anonymous flex item. That
  is an anonymous `Block` box wrapping the run's anonymous block, so the
  run has a box of its own to place, inheriting from the container as an
  anonymous block does. A run of nothing but white space makes no item,
  whatever `white-space` says (§4).

The children stay in document order in the box tree. `::first-line` and
`::first-letter` do not apply to a flex container (§3). Their finders
take a flex container for content that ends the search, as a table is
now, and apply inside a flex item as in any block container.

**`order`.** The flex layout takes the items in order-modified document
order: the box tree's order, stably sorted by `order` (§5.4). Their
fragments go into the container's fragment in that order, so painting
and hit testing, which follow fragment order, follow it too. `order`
does not apply to an absolutely positioned child, which is not an item.

**The layout pass.** A flex item is laid out more than once: to measure
it, then at its flexed or stretched size. A flex container inside an
item is laid out each time its item is. Without help, a chain of nested
flex containers costs two to the power of its depth, and the DOM may
nest 512 elements. Chains of nested flex containers ten or twenty deep
are ordinary in today's applications. So layout keeps, for one pass, the
result of laying out each flex item, keyed by the item's box and every
input the result depends on:

- its content width;
- the height imposed on it, and whether that height is definite;
- its containing block's width and height.

Each pair of box and inputs is laid out once per pass, and asking again
clones the kept result. The kept results live in a pass context that the
layout functions of `block`, `inline` and `table` take in place of the
font collection they take today: the fonts, and the item layouts. It is
made by `layout_document` and its siblings and dropped at the end, so
nothing outlives the box tree it describes. The layout cache (M13) is
unchanged: it keeps results across passes by node, and this keeps them
within one pass by box.

**The imposed height.** `layout_contents`, through which every block,
table and flex container's content is laid out, takes the height the
formatting context around the box imposes on it, if any, and whether
that height is definite:

- a block container takes it as its height, unclamped, since the flex
  algorithm has already clamped it. Its children's percentage heights
  resolve against it when it is definite and are `auto` when it is not;
- a table takes it as it takes its own `height`, a minimum its rows grow
  to fill;
- a flex container takes it as its height.

The flex layout imposes the stretched and the flexed sizes this way. An
absolutely positioned box whose height is known before its content is
laid out has it imposed too (see below). Every other caller imposes
nothing.

**Items are formatting context roots.** A flex item establishes an
independent formatting context for its content (§4), whatever its own
style says. Its margins do not collapse with its children's, and the
floats inside it stay inside. `layout_block_contents` takes its being a
root from the content it is given, `Content::Root`, rather than from its
style alone. For every caller today the two agree. The flex layout lays
every item out as a root.

**The algorithm.** `layout::flex` follows §9 step by step, in the
abstract main and cross axes, and maps to physical coordinates at the
end. In a row container the main axis is horizontal; in a column
container it is vertical. `*-reverse` swaps main-start and main-end, and
`wrap-reverse` swaps cross-start and cross-end. Everything runs left to
right and top to bottom, as the rest of layout does (section 2). The
steps:

1. *Items* (§9.1): from the box tree, the anonymous ones built there.
2. *Available space* (§9.2): in the main axis, the container's content
   width for a row container. For a column container, its definite
   height (a length, a percentage of a definite containing-block height,
   or an imposed height), else infinite. In the cross axis, the other.
3. *Flex base size and hypothetical main size*: see the next paragraph.
4. *The container's main size*: a row container's is its content width,
   which block layout gives it. A column container's is its definite
   height, else the sum of its items' outer hypothetical main sizes and
   gaps on one line (with infinite room nothing wraps), clamped by its
   `min-height` and `max-height`. With a `max-height` that clamps it, a
   wrapping column container wraps at that height.
5. *Lines*: one line for `nowrap`. Otherwise items are collected until
   the next one's outer hypothetical main size, plus a gap, would not
   fit; the first item of a line goes on it whatever its size. Sums are
   compared with a tolerance of a sixty-fourth of a pixel, the reference
   browser's layout unit, so a fraction lost to rounding does not push
   an item to the next line.
6. *Flexible lengths* (§9.7): as written, per line. The free space, the
   frozen and unfrozen items, the scaled shrink factor, and the clamping
   and the freeze in each loop are computed in `f64`, so that sums of
   large factors and sizes stay finite. The sum of factors below one
   takes its share of the initial free space.
7. *Hypothetical cross size*: in a row container, the item laid out at
   its used main size with its height `auto`, giving its height. In a
   column container, its width: its definite width, else `fit-content`
   in the line's room. Either way it is clamped by its own cross-axis
   minimum and maximum.
8. *Line cross size*: a single-line container with a definite cross size
   gives its line that size. Otherwise each line is as large as its
   largest outer hypothetical cross size, and as large as the baseline
   group's largest ascent plus its largest descent (below).
9. *Single-line clamp*: the line clamped by the container's minimum and
   maximum cross size.
10. *`visibility: collapse`*: not done (section 2).
11. *Stretch*: an item with `align-self: stretch` (or `normal`), whose
    cross size is `auto` and neither of whose cross-axis margins is
    `auto`, takes the line's cross size less its margins, clamped by its
    own minimum and maximum. In a row container it is laid out again at
    that height, imposed and definite. In a column container that width
    is the width of its final layout.
12. *Main-axis alignment*: auto margins in the main axis share the
    positive free space equally. Without any, `justify-content` places
    the items.
13. *Cross-axis auto margins*: these share the line's free cross space
    equally. When there is none, the start margin is zero and the end
    margin takes the rest.
14. *`align-self`*: places the items with no auto cross margin.
15. *The container's cross size*: a row container's height is its
    definite height, else the sum of its lines' cross sizes and the gaps
    between them, clamped as a block's height is. A column container's
    width is the width block layout gave it.
16. *`align-content`*: in a multi-line container, distributes the free
    cross space among the lines. `stretch` grows each line by an equal
    share, and only when there is space to share.

Every item is then laid out at its used size, the kept result for that
size when there is one, and its fragment placed.

**Flex base size and hypothetical main size** (§9.2 step 3). A definite
basis is used as it is: a length, or a percentage of a definite main
size, `box-sizing` applied (case A). `flex-basis: auto` is the item's
main size property, and an `auto` one there is `content`. A percentage
of an indefinite main size is `content` too. Otherwise (case E) the item
is sized into the available space with its basis in place of its main
size, `content` meaning `max-content`:

- in a row container that is its max-content width, or its keyword width
  for `min-content` or `fit-content`;
- in a column container it is the height the item gets when laid out at
  its cross size. That is its definite width, or, stretched in a
  single-line container whose width is known, the container's content
  width less its margins, which §9.8 calls definite. Otherwise it is
  `fit-content` in the container's width.

Case B, an aspect ratio, waits for replaced elements (M23). Case C,
sizing under a min-content or max-content constraint, is the intrinsic
sizes' (below). Case D, an orthogonal flow, needs writing modes. The
hypothetical main size is the base size clamped by the item's minimum
and maximum main size.

**The automatic minimum size** (§4.5). An item's `min-width` or
`min-height: auto`, in the main axis, on an item that is not a scroll
container (its `overflow` is `visible`), is its content-based minimum
size. That is the smaller of its specified size suggestion (its main
size, when definite) and its content size suggestion (its min-content
size in the main axis). Without a specified size it is the content size
suggestion alone. Either way it is clamped by a definite maximum main
size. The min-content size in a row container is the item's min-content
width. In a column container it is the height from the measuring layout
above, since a block container's min-content height is its height at its
width. Elsewhere, `auto` is zero: in the cross axis, on a scroll
container, and on anything that is not a flex item.

**Definite sizes** (§9.8). Percentages inside an item resolve against
its size when that size is definite:

- in a row container, an item's flexed width always is, since a width
  always is here;
- in a row container, a stretched item's height is definite once its
  line is sized (step 11);
- in a column container, a flexed height is definite when the container
  has a definite height or the item a definite basis. Otherwise the item
  is laid out at that height, imposed but not definite, and the
  percentages inside it are `auto`;
- a single-line container with a definite cross size gives its stretched
  items a definite cross size from the start.

**Intrinsic sizes** (§9.9). The intrinsic module gives a flex
container's min-content and max-content widths by the Web-compatible
algorithm (§9.9.1.2). Shrink-to-fit needs them (`inline-flex`, floats,
absolutely positioned boxes, table cells), and so does the base size of
an item that is itself a flex container. For a row container, the
max-content width is the sum of its items' max-content contributions and
the gaps between them. The min-content width is the sum of their
min-content contributions for a single line, and the largest of them for
a multi-line container (§9.9.1.3). For a column container, both are the
largest of the items' contributions (§9.9.2, single-line; for a
multi-line column see section 2). An item's main-axis contribution
(§9.9.3) is the larger of its outer min-content or max-content size and
its outer preferred size when that is not `auto`. It is capped by its
flex base size when it cannot grow, floored by it when it cannot shrink,
then clamped by its minimum and maximum main size. The flex base size
here is case C's: under the constraint being computed. Auto margins
count as zero (§9.9).

**Baselines** (§8.3, §8.5, css-align-3 §9). An item takes part in
baseline alignment when its `align-self` is `baseline`, `first baseline`
or `last baseline`, neither cross-axis margin is `auto`, and the
container is a row container. In a column container the cross axis is
the items' inline axis, and they align by their fallback, `safe
self-start` or `safe self-end`. An item's first and last baselines are
its layout's, measured from its border box. One with none synthesizes
both from its border box's bottom edge, as a flex item does (css-align-3
§9.1).

On each line, the first-baseline items are shifted so their baselines
meet at the largest distance from cross-start to baseline among them.
The last-baseline items are aligned the same way from cross-end. The
line is at least as large as that group's largest ascent plus its
largest descent.

A flex container's own first baseline (§8.5) is:

1. the shared baseline of its first line's first-baseline items, if any;
2. else its last-baseline group's;
3. else the baseline of the first item on that line, in order-modified
   document order, that has one;
4. else one synthesized from the startmost item's border box;
5. with no items, none, and its context synthesizes one as it does for
   any box.

Its last baseline is the same from its last line. An `inline-flex`
aligns on its line by its first baseline, as an inline table does, and a
flex container gives a table cell its baseline as a line box does.

**Absolutely positioned children** (§4.1). An absolutely positioned
child of a flex container is not an item. Its static position is where
it would be as the sole item, both it and the container at their used
sizes. This uses css-position-3's static-position rectangle (§3.5.3,
§4.3): the deferred box carries a rectangle and an alignment per axis,
where it carried a point. The rectangle is the container's content box.
The alignment is:

- in the main axis, the container's `justify-content`, its distributed
  values taking their fallbacks and `normal` and `stretch` taking
  `flex-start`;
- in the cross axis, the child's `align-self`, `auto` taking the
  container's `align-items`, `normal` and `stretch` taking `flex-start`
  and a baseline its fallback;
- `*-reverse` and `wrap-reverse` turned around as for an item.

`layout_absolute` aligns the box's margin box in the rectangle along an
axis whose two insets are both `auto`, its `auto` margins zero. Its
shrink-to-fit width then takes the rectangle's width as the room. A box
with a point keeps today's behaviour.

A flex container that is positioned is the containing block of its
absolutely positioned descendants, settled as a block's are.

The same step gives an absolutely positioned box whose height does not
depend on its content its height before its content is laid out:

- `top`, `height` and `bottom` with `height` a length;
- `top` and `bottom` with `height: auto` against a known containing
  block.

That height is imposed and definite. Today the content is laid out at
its own height and the box resized after, which is wrong for any box
whose content depends on its height. It is wrong first of all for the
overlay that centres its content with `position: absolute; inset: 0;
display: flex`.

**Paint and hit testing** (§4.3). A flex item paints as an inline-block
does: as a unit in the inline-level pass of its stacking context, its
`float` ignored. One whose `z-index` is not `auto` makes a stacking
context even when it is not positioned, and is painted at its `z-index`
layer. The paint-order walk knows an item by its parent fragment, a flex
container's, whose in-flow children are its items. Order-modified
document order comes from the fragment order above. Hit testing reads
the same steps, so it agrees.

**The layout cache.** An in-flow flex container placed by `block` is
kept under its node like a block or a table, with the same conditions.
Its items are laid out with it and not kept on their own, as the boxes
inside an inline-block or a table cell are.

**Scroll containers.** A flex container whose `overflow` clips records
its scrollable overflow area as a block container does (M10.5). That
area is the items' margin boxes plus the end padding, from the padding
box's top. An item that is a scroll container has no automatic minimum
size, so a row of them can shrink below their content. That is the
reason `overflow: hidden` on an item is the usual remedy for text
overflowing a flex row.

**The web-platform-tests runner (T1.1).** M17.9's subset comes from the
upstream `css/css-flexbox` directory, and is chosen by a rule rather
than by hand:

- every reftest and check-layout test whose test, references and
  stylesheets use only what this engine has;
- no script but `testharness.js`, `check-layout-th.js` and calls to
  `checkLayout`;
- no element that makes no box here: images, SVG, canvases, frames,
  media, form controls, lists or `br`;
- every declaration a property and value the cascade accepts, decided by
  a new `style::supports(name, value)`, the cascade's own parsers
  answering, so the rule can never disagree with the cascade;
- no `@font-face` and no Ahem font, no writing modes, no `direction:
  rtl`.

A scan of the directory at the commit used here finds nearly 500 such
tests among its 1,234 reftests and check-layout tests, once this
milestone's properties and colours are counted. The rest wait mostly on
writing modes (201 tests), `line-height` (136), `br` (131), images (112)
and scripts (103).

- *Pulling.* `cargo run -p engine --example wpt -- pull <checkout>`
  applies the rule to a checkout of web-platform-tests. It copies the
  tests, their references, their stylesheets and the support files they
  link, at their own paths, under `crates/engine/tests/wpt/`, and
  rewrites the manifest there. The upstream commit and date go in the
  directory's README, and the BSD licence beside them, as for the URL
  data M14 took from the same repository.
- *Running.* `crates/engine/tests/wpt.rs` serves that directory from the
  `net` crate's test server on 127.0.0.1, so tests and their stylesheets
  load through the engine's own loader by their URLs. Each test is
  loaded at 800 by 600 in an engine of its own, on as many threads as
  there are cores.
  - A reftest renders its reference the same way, and the two frames
    must match exactly, or within the test's `fuzzy` metadata (a maximum
    difference per channel and a number of pixels).
  - A check-layout test is run natively. The selectors come from its
    `onload` attribute or its script, and `style`'s matcher finds the
    elements. Each element, its parent and its descendants are checked
    as `check-layout-th.js` checks them, within its tolerance of a
    pixel: `data-expected-width` and `-height` (the border box, rounded
    as `offsetWidth` is), `data-offset-x` and `-y` (`offsetLeft` and
    `offsetTop`, against the `offsetParent` CSSOM View defines),
    `data-expected-client-width` and `-height` (the padding box, the
    scrollbars being overlays), `data-total-x` and `-y`,
    `data-expected-bounding-client-rect-width` and `-height`, and
    `data-expected-display`. A test that uses any other attribute is
    left out by the rule.
  - A crashtest must load and render without panicking.
- *Expectations.* `expectations.txt` names each test expected to fail,
  with the reason. A test that fails without being named fails the run,
  and so does a named one that passes, so the file stays the true list
  of what does not work and why.
- *Triage.* Every run writes `target/wpt/report.html`. It lists each
  test with its result and reason, and for a failing reftest shows the
  test's render, the reference's and the difference side by side. That
  page is T1.1's triage view.

**The golden cases.** Twenty box-only pages join `tests/reference/`.
They are captured from the reference browser by `capture.sh` and
compared byte for byte, as M10 to M16's are. They cover the corners of
the algorithm with fixed-size boxes, no text:

- growing and shrinking against minimum and maximum sizes;
- each kind of basis;
- the automatic minimum, with fixed-size content;
- every `justify-content`, and auto margins on both axes;
- `align-items` and `align-self` with stretch;
- wrapping with each `align-content` and `wrap-reverse`;
- column and reverse directions;
- `order` with overlapping items and `z-index`;
- nested containers;
- `inline-flex` on a line;
- absolutely positioned children;
- gaps, percentages, and an item that is a scroll container.

**Acceptance (M17.10).** Five real layouts, with text, live under
`crates/engine/tests/layouts/`: a navigation bar, a card grid, the holy
grail, a list of media objects, and an application shell with a sticky
footer. They are check-layout pages. A capture script loads each in the
reference browser and writes every element's border box into its
`data-expected-*` and `data-offset-*` attributes. The engine test runs
them through the WPT runner's check-layout code, at its tolerance of a
pixel. The capture gives the reference browser the engine's own font,
DejaVu Sans, by an `@font-face` this engine ignores, so both sides
measure the same glyphs. `about:silver` lays its cards out with flexbox,
so the built-in page shows it.

**Performance and fuzzing.** A benchmark, `benches/flex.rs`, lays out:

- a grid of a thousand cards in a wrapping row;
- a chain of 64 nested flex containers, alternating row and column,
  whose time must grow linearly with depth, or the pass's kept layouts
  are not doing their job;
- a relayout after one card changes.

Its numbers go in `docs/perf-budgets.md`, and the existing benchmarks
must stay within 10% (T4.3): the pass context and the new size types
touch every layout. The three fuzz targets run for fifteen minutes each
with flex pages among their seeds and the new keywords in their
dictionaries.

## 2. Not in this milestone, said rather than hidden

- *Writing modes and `direction: rtl`*: flex rows run left to right and
  columns top to bottom, as everything else in layout does (M42).
  Flexbox's case D, an orthogonal flow, goes with them.
- *`visibility: collapse`* (§4.4, step 10): the engine has no
  `visibility` property.
- *Aspect ratios and replaced items*: case B of the base size and the
  transferred size suggestion of the automatic minimum, with images
  (M23).
- *The sizing functions and keywords of css-sizing-4*: `fit-content()`,
  `stretch`, `contain` and `calc-size()`.
- *The intrinsic keywords on table cells and columns*: the table
  algorithms take them as `auto`.
- *The multi-line column container's max-content width*, which §9.9.2
  computes by laying the container out under a max-content constraint.
  This engine gives the largest contribution, the single-line answer,
  until that layout is worth what it costs.
- *Fragmentation* (§10) and the `break-*` properties: there is no paged
  medium yet (M20).
- *`-webkit-box`*, *`margin-trim`* and the other proposals the test
  directory exercises.
- *The Ideal intrinsic algorithm* (§9.9.1.1), which the draft itself
  calls not Web-compatible.
- *Percentage gaps against an indefinite size* resolve to zero, as in
  the reference browsers. css-align-3 resolves them against the size
  once it is known.
- *What the WPT subset leaves out*, among it `line-height`, `br`, the
  `font` shorthand, `rgb()`, `outline`, logical properties, images and
  scripts: the tests that need them wait for them. `expectations.txt`
  and the report say which, and the rule picks them up again when they
  land.

## 3. Order of work

Each step is a vertical slice: build, tests, clippy green and a smoke
run before the next, with its sabotage pass. A step is done when its
tests pass.

1. *Style.* `display: flex | inline-flex` and the `-webkit-` keywords;
   blockification; the flex, alignment and gap properties with their
   shorthands and aliases; the size types with the intrinsic keywords
   and `min-*: auto`, layout reading the keywords as `auto` until step
   2; the named colours.
2. *The intrinsic size keywords in layout*: widths, minimums and
   maximums of blocks, floats, inline-blocks, absolutely positioned
   boxes and tables, and their contributions.
3. *The layout pass*: the pass context in place of the font collection;
   context roots taken from the content. No result changes.
4. *Flex layout*: the imposed height through `layout_contents`, which
   nothing could impose before; the box kind and the items; `order`;
   §9's steps with `flex-start` packing and stretch in both directions
   and both reverses, wrapping, gaps, the automatic minimum, the
   definite sizes, the kept item layouts; `inline-flex` on its line; the
   intrinsic sizes; the first-line and first-letter finders. M17.1 to
   M17.4.
5. *The WPT runner*: `supports`, the pull, the vendored subset, the
   runner, the expectations and the report. From here on, each step runs
   it and shortens `expectations.txt`.
6. *Alignment*: every `justify-content`, auto margins on both axes,
   `align-items` and `align-self`, `align-content`, `wrap-reverse`,
   `safe` and `unsafe`. M17.5 and M17.6.
7. *Baselines*: first and last baseline alignment, the container's
   baselines, `inline-flex` on a line, a flex container in a table cell.
   M17.7.
8. *Absolutely positioned children*: the static-position rectangle and
   its alignment, and the height imposed on an absolutely positioned box
   that knows it. M17.8.
9. *Paint and the cache*: items painted as inline-blocks, `z-index` on
   items, hit testing; flex containers in the layout cache.
10. *The golden cases*: the twenty reference pages, and whatever they
    find. M17.9 with step 5.
11. *Acceptance*: the five layouts and `about:silver`. M17.10.
12. *Performance and fuzzing*: the benchmark, the T4.3 comparison, the
    fuzz targets run.
13. *The documents*: this note corrected to the code, the architecture's
    M17 entry, the roadmap's ticks (M17.1 to M17.10, and T1.1), the
    numbers in `docs/perf-budgets.md`.
