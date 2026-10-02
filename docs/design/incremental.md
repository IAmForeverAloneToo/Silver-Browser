# Design note: the incremental engine

Roadmap M13. Written before the code, as the record of the decisions;
what ships is described in `docs/architecture.md` "Current state" when
the code lands. Where this note and the code later disagree, the code
wins and this note is corrected, as the notes before it were.

The problem: a document is a value today. The parser hands the engine
an owned tree of `dom::Node`s; the cascade flattens it into a
`FlatDom` whose preorder indices are every later stage's name for an
element — the box tree's, the fragments', hit testing's, the scroll
offsets' — and nothing can change the document but replacing it, which
throws every cache away. Inserting one element would shift the index
of every element after it, so there is no way to say "this element
changed" that survives the change. The caches the engine has (M9.5)
are whole-document: the cascade per document, the layout per document
and viewport width. Measured on a page of 1,000 paragraphs at 1,280
pixels wide, release build, the bundled test font: parsing 1.4ms, the
cascade 2.3ms, the box tree 0.8ms, layout about 700ms, and a frame
from the cached layout 2.3ms (paint and raster); the debug build's
layout takes about as long, since the dependencies that shape text are
optimised even there. Layout is mostly text shaping: about 25
microseconds a word, of which rebuilding the font's `rustybuzz::Face`
on every call is 8, and the page has some 22,000 words. So any change
to a page of that size costs most of a second, the price of laying out
every paragraph again.

M14.6 (a stylesheet that arrives restyles the page), M25 (scripts that
call `appendChild` and `setAttribute`) and M26 (hover) all need a
document that changes in place, each change costing what it touches.
This milestone gives nodes a stable identity, adds a mutation API,
makes each stage — style, layout, paint, raster — redo only what a
change invalidates, puts a frame scheduler with a budget in front of
it, and measures the result.

## 1. Decisions

**Node identity: an arena with generational ids (D16).** A
`dom::Document` owns every node in one `Vec` of slots, and a node is
named by a `NodeId`, its slot's index and the slot's generation. Each
node keeps links to its parent, its first and last child and its
previous and next sibling, so every navigation the matcher and the
box tree builder need is one step; an element keeps its tag and
attributes, a text node its data. Freeing a node bumps its slot's
generation before the slot is reused, so a stale id resolves to
nothing: every accessor returns an `Option`, and no id, stale or
forged, can make the crate panic (the layering contract's rule 5;
scripts will hold ids). Three alternatives were weighed. Ids stored on
the owned tree's nodes: finding a node by id needs a map from id to
path that every insertion invalidates. Reference-counted nodes with
interior mutability, the shape of a scripted DOM: a borrow conflict
becomes a runtime failure, and the tree cannot be shared with the
parallel layout M35.3 plans. Preorder indices remapped on every
mutation: linear per change, which is the cost this milestone exists
to remove. The arena is what the decision register records as D16.
The owned `dom::Node` stays, as the literal a test, a fuzz target or
the parser writes a tree in: `Document::from_node` builds the arena
from one in linear time and `Document::to_node` takes a snapshot back,
so the hundreds of tests that build trees with `Element::new(..)`
keep doing so. The parser keeps returning a `Node` and the engine
converts it once per load; parsing straight into the arena belongs
with M14.6's streamed loading.

**Mutation.** `Document` offers the operations M25 will bind:
`create_element`, `create_text`, `append_child`, `insert_before`,
`remove_child`, `set_attribute`, `remove_attribute` and `set_text`.
Each checks what the DOM standard checks before it changes anything —
no node inserted into itself or its descendants, no child under a
text node, the reference child of `insert_before` a child of the
parent — and two limits of this engine: the parser's nesting depth,
512, which deep recursion elsewhere relies on, and a cap on the number
of live nodes. A failed check returns an error and leaves the document
as it was. A removed node stays valid and detached, and can be
inserted again, as the DOM's `removeChild` leaves it; freeing a
detached subtree is explicit (`Document::free`), the engine's job
until M24's collector decides when nothing holds a node. Every
successful mutation appends a record to the document's change
journal — a child inserted under a parent, a child removed from one
with the siblings it stood between, an attribute set or removed on an
element with its value before, a text node's data changed — and
`take_changes` drains it. Past 65,536 records unread, the journal
holds one that says everything changed, which is always correct. The
journal states facts in the DOM's own terms and knows nothing of style
or layout (rule 3: the DOM is `dom`'s); each stage reads it for what
it cares about.

**Styles keyed by node.** The cascade reads the arena: the matcher
walks parent and previous-sibling links, and the positional counts
the structural pseudo-classes need (an element's index among its
siblings, the sibling count, the same for its type) are computed once
per parent per pass and cached in the `Matcher`, as the ranks for
`of S` are. `:empty` is a node with no children at all, as
selectors-4 has it. `FlatDom` and `StyledDom` are retired. The result
is a `style::Styles`: a computed style per node slot, with the facts
the whole document shares (the root's font size for `rem`, whether
any declaration used a viewport unit). A full cascade gives exactly
the styles the preorder cascade gave, which the existing style tests
check through `Document::from_node`. The user-agent stylesheet, parsed
on every cascade before, is parsed once per process. A document's
author sheets are prepared once, as a `style::Stylesheets`: each
rule's selectors parsed, and the features below gathered, for every
cascade and restyle until the `<style>` text changes.

**Style invalidation (M13.1).** The journal is turned into a set of
elements to restyle, conservatively where precision would need a
selector's structure. When the stylesheets are parsed, their
selectors' features are gathered, the UA sheet's included: the class
names, ids and attribute names any selector mentions, inside `:not()`,
`:is()`, `:where()` and `of S` included; and whether any uses a
sibling combinator (`+`, `~`), a structural pseudo-class that counts
from the start (`:first-child`, `:first-of-type`, `:nth-child`,
`:nth-of-type`), one that counts from the end (`:last-child`,
`:last-of-type`, `:nth-last-child`, `:nth-last-of-type`), one that
needs every sibling (`:only-child`, `:only-of-type`, `of S`), or
`:empty`. Then:

- An attribute changed on element E. `style` and `dir` restyle E
  alone, since the cascade reads them directly. A `class` change
  restyles E when a class added or removed is one some selector
  names; an `id` change, when the old or new id is named; any other
  attribute, when its name is. Such an E is restyled with its subtree,
  since descendant and child combinators make descendants depend on
  it, and with its following siblings' subtrees when sibling
  combinators are in use, or all its siblings' when `of S` is. Any
  other attribute restyles nothing; `colspan`, `rowspan` and `span`
  are layout's (below).
- A child, element or text, inserted under or removed from parent P.
  An inserted element is restyled with its subtree. P is restyled
  when `:empty` is in use. P's children after the change point are
  restyled with their subtrees when sibling combinators or positions
  counted from the start are in use, those before it when positions
  counted from the end are, and all of them when every sibling counts.
- A text node's data changed: nothing is restyled. No selector reads
  text, and `:empty` asks only whether there are children.
- A change to the text of a `<style>` element, or one adding or
  removing one, reparses the stylesheets, gathers their features again
  and cascades the whole document.

The records are read after all of them were made. An element no
longer in the tree is skipped. An attribute is compared from its value
before its first change to its value now, so one changed and changed
back costs nothing. A change point whose neighbours have moved since
counts as anywhere among the children.

Restyling goes by depth, shallowest first, each element from its
parent's final style. An element whose new style differs from its old
one has its children restyled too, since they inherit from it — the
inherited properties by default and any property by `inherit`, as the
UA sheet's `vertical-align: inherit` on cells does — and one whose
style is unchanged stops there, unless it was marked with its
subtree. Removed subtrees' styles are forgotten. A change to the root
element's font size, which `rem` reads, or a journal that overflowed
cascades the document whole, as a viewport change does for a document
using viewport units. The result names the elements whose computed
style changed, which is what layout reads. The flag for viewport units
can only be set by an incremental pass, never cleared, until the next
full cascade: a stale `true` costs a needless cascade on resize, a
stale `false` would be wrong.

**Layout invalidation (M13.2).** The box tree is rebuilt from the
styles on every layout pass: it costs about a millisecond for the
1,000 paragraphs, its boxes carry `NodeId`s, and building it
incrementally can wait until it is measured to matter. A box generated
for an element's content rather than for the element — a text box, or
an anonymous block, table, row or cell — is marked anonymous. What is
kept between passes is a layout cache keyed by node: for an element's
own block or table box placed in flow by block layout, the result —
its fragment subtree relative to the box, which is how layout holds
fragments until its final pass, its height, the margins it exposes and
its baselines — with the inputs it was computed from. A later pass
takes the result instead of laying the box out when:

- nothing in the element's subtree changed since: the cache forgets,
  with all their ancestors' entries, the entry of a parent whose
  children changed, of the parent of a text node whose data changed,
  of an element whose `colspan`, `rowspan` or `span` changed (the only
  attributes layout reads), and of every element whose computed style
  changed;
- the inputs are the same: the containing block's width and height
  (the latter for percentage heights), and the width available, which
  is the containing block's for every box the cache holds;
- no float was in the box's formatting context, placed or waiting,
  when it was laid out or when it is taken, and it placed none: the
  pass checks the context before and after the box. A float so keeps
  out of the cache every box after it in its context, and every box
  around it in that context, which placed it; a box that is a
  formatting context root keeps its own floats to itself, and is held;
- no absolutely positioned box was carried out of it, since such a
  box is laid out against an ancestor.

An entry does not copy the fragments of the children the cache holds
too: it names each by its node and the serial its entry was kept
with, and the place and position its fragment had, and taking the
entry puts the children's fragments back from theirs. So the cache is
as large as the document however deeply its boxes nest; copying would
have grown with depth times size. A child whose entry has since gone
or been replaced makes its parent's entry useless, and the parent is
laid out again.

Everything else is laid out as before: the changed elements, their
ancestors — whose child loops run again, their unchanged children
taken from the cache — anonymous boxes, and the boxes inside
inline-blocks, floats, absolutely positioned boxes and table cells and
captions, which are laid out with the box that holds them. That is
the roadmap's propagation in this engine's terms: a change of content
size reflows the ancestors, and a box that only moved, because
something before it grew, is placed by one translation and not laid
out again. A new viewport width misses the entries whose containing
block it changes; boxes inside a container of fixed width still hit.
The entries of freed nodes are dropped, and a whole cascade, a new
document or an overflowed journal empties the cache. Two counters,
`Engine::boxes_laid_out` and `Engine::boxes_reused`, say how many boxes
placed in flow each pass laid out and took whole from the cache.

**Paint and raster invalidation (M13.3).** The display list is built
from the fragments on every frame, as before, which costs little. The
software rasterizer keeps the last frame and its list, and compares
the new list with the last one: the longest common prefix and the
longest common suffix of equal items, and the differing middle of each
list is what changed. The damage is the pixels the middle items, in
both lists, can change: a rectangle or an anti-aliased polygon as far
as its raster reaches, a quad a pixel further, a run of text by the
extent of its glyph masks, kept per run, and a pixel more. Where the
shared suffix draws under clips that differ between the two lists — a
clip begun or ended in between, or a clip stack deeper in one list
than in the other — both clips are damaged, since the suffix may draw
differently anywhere in them. The damage is a set of whole-pixel
rectangles rounded outwards and merged where they touch, at most 32,
else the one around them all.

Each damaged rectangle is cleared and every item that meets it drawn
again there, every clip and its end met whatever its place, so the
clips nest as in a full raster. An item all inside the rectangle is
drawn as a full raster draws it. One across its edge is kept inside by
means that cannot change its pixels there: a rectangle is clipped to
the rectangle's whole pixels, since tiny-skia supersamples each pixel
from the edges within it (a test holds this over 20,000 random cases;
a million found no difference); a glyph and an anti-aliased polygon
write only the pixels inside; and a quad, whose slanted edges tiny-skia
would snap differently from new end points, is drawn on a copy of the
frame of which the inside is copied back. The bar is exact: a damaged
frame's bytes equal a full raster's, which tests check after random
edits of random lists, after random mutations of pages, and on every
reference page, and which the pipeline fuzz target checks on every
frame. The frame is rasterized whole when there is no last frame of
its size, when the damage covers half the frame or more, when either
side is over 8,191 pixels, past which tiny-skia stops anti-aliasing,
and when the viewport's scroll offset changed, which moves everything.
`Engine::redrawn` says what the last frame drew anew.

**The frame scheduler (M13.4).** The shell asks the engine whether a
frame is wanted (`needs_frame`: the document changed, a scroll offset
was set or is still on its way to its target; a new window size is the
shell's to know) and draws it with `Engine::frame_at`, passing the time
it is drawn for and a deadline (`FrameTime`), by default that time
plus the budget. The engine reads the clock only to time its stages,
so what a frame shows is a function of the document, the viewport, the
scroll inputs and the times passed in, which tests drive. The frame's
pixels are lent out until the next frame rather than copied. Input
between frames is applied as it comes and its work deferred to the
frame, so a burst of mutations costs one pass. The scroll animation is
time-based, keeping its feel — a third of the remaining distance every
sixtieth of a second — so a slow frame no longer slows it; the first
frame of an animation moves one sixtieth's step, however long the
engine was idle before it. `Engine::tick` still advances one sixtieth,
for a caller without a clock, and `Engine::frame` draws without
advancing anything. The budget is 16.7ms a frame, `FRAME_BUDGET`. The
roadmap asks what a frame over it skips. Nothing that affects what is
shown: what is visible can depend on content anywhere before it in the
tree and on positioned boxes anywhere, so skipping layout outside the
viewport is not safe in this engine, and there is no other deferrable
layout work yet. What a late frame gives up is the animation frames it
could not draw, which time-based animation makes up in one step. Every
frame records its stages' durations and whether it overran
(`Engine::frame_stats`), and the deadline is there for the deferrable
work to come (M19's layer re-raster, M22's animations) to honour. The
policy is written down in `docs/perf-budgets.md`, which this milestone
creates (T4.1).

**The public API (M13.5).** `Engine::set_document` installs a
`Document`; `Engine::load_document` parses HTML and calls it.
`Engine::document` lends the document, and `Engine::mutate` lends it
mutably to a closure, after which the engine reads the journal and
marks what changed, and `needs_frame` says so. `hit_test` answers
with a `NodeId`; element scroll offsets are kept by `NodeId`, so they
survive mutations, and are dropped when their node is freed; `link_at`
walks the document's parent links. `Engine::styles` returns the
cascade the frames use, the engine's own, so that a test can hold what
the engine keeps to what it should; `Engine::fragments` computes a
fresh layout for inspection. The counters prove work not done:
`style_runs` (whole cascades), `restyled_elements`, `layout_runs`,
`boxes_laid_out`, `boxes_reused` and `redrawn`.

**Acceptance (M13.6).** A benchmark, `crates/engine/benches/mutation.rs`,
draws the 1,000-paragraph page, then appends 1,000 paragraphs one a
frame, and 1,000 more in one frame, reporting the milliseconds per
frame and per stage and the boxes laid out and reused; its numbers are
in `docs/perf-budgets.md` beside the scroll benchmark's. No frame
after the first lays the document out whole: an appended paragraph's
frame lays out the paragraph, the body and the root and takes every
other box from the cache. An engine test holds the claim itself.

**Incremental equals from scratch.** Every stage's incremental result
must be what a full pass over the same document gives: the styles, the
fragments, the display list and the pixels. `engine::differential`
turns bytes into mutations of every kind (`Mutator`) and compares a
frame, every element's style and every fragment with a fresh engine's
for a snapshot of the document (`compare_with_fresh`). Engine tests
apply seeded random sequences of mutations to two pages — one with
floats, positioned boxes, tables and scroll containers under
selectors that depend on siblings, positions and children, one whose
boxes the layout cache can hold — and compare after every step. A
fuzz target, `mutation` (cargo reserves the name `incremental`), reads
a document and a mutation script from its input and asserts the same
equality, so the fuzzer hunts for the case the invalidation rules
missed. The reference pages, each loaded, mutated and mutated back,
still match their captures exactly.

## 2. Not in this milestone, said rather than hidden

Building the box tree incrementally; it is rebuilt per pass, which is
measured, and grows faster than the page. A paint-only style change,
such as a new colour, relays out the element's subtree, its ancestors
and their other children being reused: fragments hold copies of their
styles, and fragments that refer to styles instead, so that such a
change only repaints, are M35.9's. So is the other cost every layout
pass has in proportion to the page: fragments hold absolute
coordinates, so a pass that reuses every box still copies every
fragment into the new layout. Positions counted from the end make an
insertion restyle every sibling before it, the conservative rule. Text
shaping's cost, measured above: keeping each font's face instead of
rebuilding it per call, and caching word measures in layout, are
recorded as a new item, M35.8. Damage from a scroll as a shifted frame
(M19); compositing layers (M19); parallel layout (M35.3); rule
hashing, Bloom filters and style sharing (M35.2); parsing into the
arena as bytes arrive (M14.6); scripting, the collector that frees
detached nodes, and mutation observers (M24, M25); input events (M26);
benchmarks tracked by criterion in CI (T4.2).

## 3. Order of work

Each step a vertical slice: build, tests, clippy green, smoke run, then
the next. (1) `dom::Document`: the arena, `NodeId`, navigation, the
mutation API with its checks, the journal, `from_node` and `to_node`,
with no consumer changed. (2) Style over the arena: the matcher and
the cascade read the document, the cascade's result is `Styles`, the
UA sheet is parsed once; every style test unchanged in what it
asserts. `FlatDom` and `StyledDom` stayed, built from the document,
for the stages that still read preorder indices. (3) Layout, paint and
the engine on `NodeId`s: boxes and fragments carry them, the engine
holds a `Document`, hit testing, `link_at` and scroll offsets use
them, and `FlatDom` and `StyledDom` are retired; every test and
reference page unchanged. (4) Mutation in the engine (M13.5):
`set_document`, `mutate`, `needs_frame`, every mutation running the
full pipeline — the correct baseline — with the differential tests and
the `mutation` fuzz target landing here, so that every later step is
checked against it. (5) Style invalidation
(M13.1): selector features, dirty marking from the journal, the
incremental restyle and its counter. (6) Layout invalidation (M13.2):
dirty propagation, the layout cache and its conditions, its counters.
(7) Paint and raster invalidation (M13.3): item equality, the diff,
the retained frame and the clipped raster, the exactness test. (8) The
frame scheduler (M13.4): time passed in, time-based scrolling, stage
timings and the deadline, the shell's loop, `docs/perf-budgets.md`
with the policy. (9) The benchmark (M13.6) and its numbers. (10) The
documents.
