# Design note: floats, positioning, stacking

Roadmap M11. Written before the code, as the record of the decisions;
what ships is described in `docs/architecture.md` "Current state" when
the code lands. Where this note and the code later disagree, the code
wins and this note is corrected, as `box-model.md` was.

The problem: normal flow is complete (M10), and a real page is not
normal flow. It floats an image into a paragraph and a sidebar beside
a column; it positions a menu under a button, a badge over a corner, a
header that stays while the rest scrolls; it says `z-index: 10` and
expects the badge above the image; it highlights a word with a
background on a `span`. Each of those is either dropped or wrong today:
`float`, `clear`, `position`, the four offsets and `z-index` are not
parsed; a span's background is not painted; the display list is paint
in tree order, which is CSS2 Appendix E's order only for the flow M10
has. And M10's reference cases found two places where paint, not
layout, differs from a browser: a fractional edge, which a browser
snaps to a pixel, and a mitre between two border colours, which a
browser anti-aliases. This milestone adds the rest of CSS2's visual
formatting model — chapter 9's floats and positioning, Appendix E's
painting order, the inline box's own decorations — and those two pixel
rules, so that the pages M12 (tables), M17 (flexbox) and M19
(compositing) build on paint the same pixels a reference browser does.

## 1. Decisions

**The properties.** `float: none | left | right`; `clear: none | left |
right | both`; `position: static | relative | absolute | fixed |
sticky`; `top`, `right`, `bottom` and `left` as `auto`, a length or a
percentage, kept as written for layout (a percentage of the containing
block's width for `left`/`right`, of its height for `top`/`bottom`, and
`auto` when that height is not definite, CSS2 §9.3.2); `z-index: auto`
or an integer. None is inherited. CSS2 §9.7's computed-value rules are
applied in the cascade, after the longhands: `position: absolute` or
`fixed` makes `float` compute to `none`; a float or an absolutely
positioned box whose `display` is inline-level computes to `block`
(`inline-block` included, there being no `inline-table` yet), so the
box tree sees a block; the root element's `float` is `none`. The
`inset` shorthand is css-position-3's, not CSS2's, and is not parsed.

**Out-of-flow boxes keep their place in the box tree.** A float or an
absolutely positioned box stays a child of its parent in the box tree,
where its element is, with its style saying it is out of flow; the box
tree stays the tree of the document's boxes. The builder treats it as
inline-level content when it occurs inside an inline element or an
open run — a float in the middle of a paragraph belongs to the
paragraph's anonymous block, placed when its line is laid out, and it
does not split the inline it is in, since only in-flow blocks do (CSS2
§9.2.1.1) — and as a block-level child when it occurs between blocks,
placed at the flow position it is met at. Either way the box's own
content is a block container's, laid out as a block formatting context
root of its own.

**Floats: a context per block formatting context.** A block formatting
context owns the floats placed in it: each as its margin box in the
context root's coordinates, left or right, in placement order. The
context is threaded through block and inline layout as one mutable
value with the current box's offset from the root, so every box in the
context sees the floats of its ancestors and siblings and converts the
coordinates; a new root (`overflow` other than `visible`, an
inline-block, a float, an absolutely positioned box, the initial
containing block) starts a fresh, empty context, which is why floats
never leak out of one. A float's size is an inline-block's: shrink-to-fit
width when `auto` (CSS2 §10.3.5, `layout::intrinsic`), its height its
content's, floats inside it included (§10.6.7). Placement is CSS2
§9.5.1's rules in their order: the float's top is the current flow
position — after the margins resolved so far, which is what a browser
does with a float between two blocks whose margins collapse across it —
and never above an earlier float's top or an earlier line's top; its
left (right) edge is as far left (right) as the containing block's
content edge and the earlier floats of its side overlapping it
vertically allow; when the space between the two sides' floats at that
height is narrower than the float's margin box, the float moves down to
the next height at which an overlapping float ends, and tries again; a
float too wide for the containing block is placed at the edge and
overflows, as rule 7 allows for the outermost. Margins of floats never
collapse (§8.3.1). A float met in inline content is placed at the
current line's top when its margin box fits in what is left of the
line, and the line is shortened; otherwise at the bottom of the current
line, and the next line avoids it (§9.5.1 rule 3, the way browsers read
it).

**Line shortening.** A line box beside floats runs between the floats
overlapping it (CSS2 §9.5): the inline layout asks the context, for
the line's top and its height so far, where the line's left edge is and
how wide it may be, and packs words into that; `text-align` works
within the shortened line. A line that nothing fits in beside the
floats moves down to the next float's bottom and tries again; with no
floats it overflows as it always has. The line's height is taken as the
strut's until an item makes it taller, and a float the line grows into
after that is not re-checked: a browser relays the line; this engine
does not yet, and says so.

**Heights and floats.** A block formatting context root's `auto`
content height reaches the bottom margin edge of the lowest float in
it (CSS2 §10.6.7), and so does the document's: the initial containing
block is the outermost context, and a page ending in a float is as
tall as the float. Any other block's `auto` height ignores floats: they
overflow it, which is the classic behaviour the BFC root exists to end.
A scroll container's scrollable height (M10.5) is a root's, so it
counts its floats.

**Clearance (M11.1).** A block with `clear` whose hypothetical top
border edge — where it would sit after margin collapsing — is above
the bottom margin edge of the relevant floats (left, right or both)
gets clearance: its top border edge is put level with the lowest such
float's bottom margin edge (CSS2 §9.5.2). Clearance is space above the
box's top margin, and it ends the collapsing above it: the margins
pending before the box are resolved and materialised, and the box's
own top margin, with whatever escaped from its first child, sits inside
the clearance rather than joining them. A cleared box that is
self-collapsing is treated as not self-collapsing for the margins above
it (§9.5.2's finer rules for that case wait for a page that needs
them). A float with `clear` is placed below the floats it clears by the
same rule.

**Block formatting context roots beside floats (M11.2).** An in-flow
block that is a context root (an `overflow` other than `visible`,
later a table) must not overlap a float's margin box (CSS2 §9.5): after
its margins are resolved, its border box is placed in the space between
the floats at its top, as far left as that space allows, and its
`auto` width is computed with that space as the containing width (the
box may be narrower than §10.3.3 says, as the specification permits);
when the space is narrower than its minimum, the box moves down to the
next float's bottom and tries again, like a float does. Its margins
still collapse with its siblings' as usual: the dodge is after that.

**Relative positioning (M11.3).** A box with `position: relative` is
laid out in flow where it is and then moved: by `left` (else minus
`right`; both `auto` is no move) and `top` (else minus `bottom`),
percentages of the containing block (CSS2 §9.4.3). The move is applied
in layout, to the box's fragment and its subtree, as the box is placed,
so that an absolutely positioned descendant, whose containing block it
is, and hit testing see the moved box; the flow around it is not
affected, and layout is not rerun for it. An inline element with
`position: relative` moves each of its line fragments and their content
the same way, which the inline fragments of this milestone make
possible.

**Absolute positioning (M11.4).** The containing block of an absolutely
positioned box is the padding box of its nearest ancestor whose
`position` is not `static`, else the initial containing block — the
viewport's rectangle at the document's top. Layout records, when it
meets such a box in flow, its static position (CSS2 §10.3.7's
hypothetical box: the flow position it would have taken as a block,
after the margins resolved so far, at its parent's content left; in
inline content, the current line's top and the current x on the line,
the approximation the specification allows) and defers it; when the
containing block's own content is laid out and its padding box is
known, each deferred box is laid out against it. Width is §10.3.7:
`left`, `width` and `right` all `auto` is shrink-to-fit at the static
position; one `auto` among them is solved from the other two (`auto`
margins zero, except when all three are given and both margins are
`auto`, which centres the box); none `auto` and over-constrained
ignores `right`, this engine being left-to-right throughout. Height is
§10.6.4 the same way with `top`, `height` and `bottom`, `auto` height
the content's, over-constrained ignoring `bottom`. The box is a block
formatting context root with the containing block's padding box as
its containing size. Its fragment goes under the containing block's
fragment, after the in-flow content, which is what makes an
`overflow: hidden` box between the two not clip it, and a scroll
container between them not scroll it — the tree says what contains
what. `position: fixed` is the same with the viewport as the
containing block, the fragment at the document's root marked as not
scrolling: paint and hit testing leave the viewport's offset out of
it. A layout with a fixed box whose offsets or sizes depend on the
viewport's height says so, as a percentage root height already does,
so the engine's layout cache is keyed on the height for it.

**Sticky (M11.5).** A box with `position: sticky` is laid out in flow,
like `relative` with no offset. When painted or hit it is shifted, as
much as its containing block's content box allows, so that it keeps
its `top`, `right`, `bottom` and `left` insets — those not `auto` —
from the edges of the visible part of its scrollport: the nearest
ancestor scroll container, else the viewport. The shift is one function
in `paint`, of the box's rectangle, its constraints, its containing
block's rectangle and the scrollport's visible rectangle, which
`paint_fragments_in` and the engine's hit test both call with the same
offsets, so the two agree. v1 is the roadmap's: the nearest scrollport
only; a sticky box inside a scroll container does not also stick to
the viewport.

**Paint order: one walk, two consumers (M11.6).** The display list is
built by walking the fragment tree in CSS2 Appendix E's order, and the
engine's hit test walks the same sequence backwards, so what is painted
last is what is hit first and there is one definition of the order. A
stacking context is made by the root element and by a positioned box
whose `z-index` is not `auto` (M19 adds opacity and transforms). Within
a stacking context, in order: the root's own background and border;
child stacking contexts with negative `z-index`, in `z-index` then tree
order; the backgrounds and borders of in-flow, non-positioned,
block-level descendants, in tree order; the non-positioned floats, each
painted as a unit (its background, then its content as if it were a
stacking context, except that its positioned descendants belong to the
real one); the in-flow inline content — inline boxes' backgrounds,
borders and text, inline-blocks as units — in tree order; the
positioned descendants with `z-index: auto` or `0`, in tree order, an
`auto` one painted as a unit the same way, a `0` one being a real
stacking context; and the child stacking contexts with positive
`z-index`, in `z-index` then tree order. The walk carries the chain of
clips and scroll offsets of each fragment's ancestors in the *tree*,
so a positioned box painted out of tree order is still clipped by the
`overflow` boxes between it and its containing block and scrolled by
the scroll containers there, and by nothing outside that path; a fixed
box has neither. The walk yields, in paint order, each box's background
step and each text or decoration item with its final rectangle (scroll
and sticky applied) and the clip in force; `paint` turns that into
display items, the engine turns the reverse into a hit test, and
`Fragment` carries what the walk needs to classify a box: its style's
`position`, `float` and `z-index`, and a flag for a fixed box.

**Inline box decorations.** An inline element gets a fragment per line
box it appears on, a new `FragmentKind::InlineBox`, with the text,
inline-block and nested inline fragments of that line as its children;
its content area runs from the line's baseline up by the inline's
font ascent and down by its descent (CSS2 §10.6.1) and horizontally
from its first item to its last. Its background and border paint around
the content area plus its padding, in the inline-content step before
the text; its horizontal margin, border and padding take space on the
line at the element's start and end only (`box-decoration-break:
slice`, the only value), so the item collection emits a start and an
end item with those widths for the line breaker to count; its vertical
padding and border are drawn but do not change the line's height, as
the specification says and the strut makes consistent. Hit testing
answers with the inline element over its padding box, which is what a
link's clickable area is. Text fragments are children of inline
fragments from here on; a block's own text stays directly under the
block.

**Two pixel rules, so the reference pages can say more.** First, a box's
edges are snapped to whole device pixels when its background, border
and clip are painted: each edge rounded to the nearest pixel, both
edges independently, so that neighbouring boxes still meet without a
seam — what browsers do, and what let M10's pages only use whole-pixel
lengths; layout keeps its fractions, and text is not snapped yet (a
browser snaps the baseline; that waits for a text reference case to
need it). Second, where two sides of a border differ in colour, their
mitre is drawn anti-aliased, as a browser draws it; sides of one colour
keep the exact tiling of M10. With both, the reference pages of this
milestone use fractional lengths and multi-colour borders where M10's
could not.

**Acceptance against a reference browser (M11.7).** The harness of
M10.7, unchanged: box-only pages under `crates/engine/tests/reference/`,
captured by `capture.sh` from the headless shell, compared byte for
byte. The cases: the classic two-column layout, a left float beside
rows of inline-blocks that wrap around it (an inline-block row is a
line, so it is shortened; text is not allowed); float placement, left
and right, with one that does not fit beside the others and drops;
`clear` on each side with margins before and after; the dodge of an
`overflow: hidden` box beside a float; relative offsets, the siblings
unmoved; absolute positioning in every combination of the offsets,
width and height, the static position, a nested positioned containing
block with padding; a fixed box in an unscrolled capture; stacking —
negative, `auto`, zero and positive `z-index`, a nested stacking
context, floats between in-flow backgrounds and positioned boxes;
inline decorations, a span with padding, border and background around
inline-blocks across two lines; and fractional lengths with
multi-colour borders. Fixed under scroll, sticky and hit testing
through the stacking order are engine tests, since a capture does not
scroll.

## 2. Not in this milestone, said rather than hidden

`line-height` and `vertical-align` beyond the baseline (M21, M42). A
line that grows into a float after being started is not relaid.
`clear` on a self-collapsing box takes the simple rule above. `inset`,
`float: inline-start`/`inline-end` and `box-decoration-break: clone`
are not parsed. `direction: rtl` does not change the over-constrained
rules (left-to-right throughout, as since M6). Sticky in nested
scrollers, and `position: fixed` inside a transformed ancestor (M19).
Opacity and transforms as stacking contexts (M19). Text positions are
not snapped to pixels. Absolutely positioned boxes do not shrink-to-fit
against their containing block's height (`height: auto` is the
content's, as the specification says). Floats do not affect margin
collapsing beyond clearance's rule. Inline boxes' decorations do not
split or stretch across lines other than by slicing.

## 3. Order of work

Each step a vertical slice: build, tests, clippy green, smoke run, then
the next. (1) The properties and §9.7's computed-value rules in the
cascade. (2) The paint-order walk shared by paint and hit testing, on
today's tree, with today's order — Appendix E's for in-flow content —
so that the change in structure lands alone and provably changes
nothing. (3) Floats: the context, placement, line shortening, context
roots' heights. (4) Clearance, and the dodge. (5) Relative positioning.
(6) Absolute and fixed positioning: containing blocks, the static
position, §10.3.7 and §10.6.4, the fixed layer. (7) Sticky. (8) Stacking
contexts and `z-index` in the walk. (9) Inline box fragments and their
decorations. (10) The two pixel rules. (11) The reference pages, and
whatever they find. (12) The documents.
