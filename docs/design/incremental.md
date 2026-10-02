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
journal — children changed under a parent (which child, inserted or
removed), an attribute changed on an element (its name), a text node's
data changed — and `take_changes` drains it. The journal states facts
in the DOM's own terms and knows nothing of style or layout (rule 3:
the DOM is `dom`'s); each stage reads it for what it cares about.

**Styles keyed by node.** The cascade reads the arena: the matcher
walks parent and previous-sibling links, and the positional counts
the structural pseudo-classes need (an element's index among its
siblings, the sibling count, the same for its type) are computed once
per parent per pass and cached in the `Matcher`, as the ranks for
`of S` are today. `FlatDom` is retired. The result is a
`style::Styles`: a computed style per node slot, with the facts the
whole document shares (the root's font size for `rem`, whether any
declaration used a viewport unit). A full cascade gives exactly
today's styles, which the existing style tests check through
`Document::from_node`. The user-agent stylesheet, parsed on every
cascade today, is parsed once per process.

**Style invalidation (M13.1).** The journal is turned into a set of
elements to restyle, conservatively where precision would need a
selector's structure. When the stylesheets are parsed, their
selectors' features are gathered: the class names, ids and attribute
names any selector mentions, inside `:not()`, `:is()`, `:where()` and
`of S` included; and whether any uses a sibling combinator (`+`,
`~`), a structural pseudo-class that looks back (`:first-child`,
`:first-of-type`, `:nth-child`, `:nth-of-type`), one that looks ahead
(`:last-child`, `:last-of-type`, `:nth-last-child`,
`:nth-last-of-type`), one that looks both ways (`:only-child`,
`:only-of-type`, `of S`), or `:empty`. Then:

- An attribute changed on element E. `style` and `dir` restyle E,
  since the cascade reads them directly. A `class` change restyles E
  when a class added or removed is one some selector names; an `id`
  change, when the old or new id is named; any other attribute, when
  its name is. Such an E is restyled with its subtree, since
  descendant and child combinators make descendants depend on it, and
  with its following siblings' subtrees when sibling combinators are
  in use, or all its siblings' when `of S` is. `colspan`, `rowspan`
  and `span` restyle nothing and make E's table layout-dirty. Any
  other attribute changes nothing.
- A child inserted under or removed from parent P. An inserted
  subtree is restyled whole. P is restyled when `:empty` is in use.
  P's children after the change point are restyled with their
  subtrees when sibling combinators or backward-looking structural
  pseudo-classes are in use, those before it when forward-looking
  ones are, and all of them when ones looking both ways are.
- Text inserted, removed or changed under P restyles P when `:empty`
  is in use, since a text child is content to it.
- A change inside a `<style>` element, or one adding or removing
  one, reparses the stylesheets, gathers their features again and
  restyles the whole document.

Restyling walks the dirty elements in tree order, topmost first, each
from its parent's style. An element whose new style differs from its
old one has its children restyled too, since they inherit from it —
the inherited properties by default and any property by `inherit`, as
the UA sheet's `vertical-align: inherit` on cells does — and one whose
style is unchanged stops there. A change to the root element's font
size restyles everything, since `rem` reads it, as a viewport change
does for a document using viewport units. The result names the
elements whose computed style changed, which is what layout reads. The
flag for viewport units can only be set by an incremental pass, never
cleared, until the next full cascade: a stale `true` costs a needless
restyle on resize, a stale `false` would be wrong.

**Layout invalidation (M13.2).** An element is layout-dirty when its
computed style changed, its children changed, a text child changed,
or an attribute layout reads (`colspan`, `rowspan`, `span`) changed;
each ancestor of a layout-dirty node has a dirty descendant. The box
tree is rebuilt from the styles on every layout pass: it costs 0.8ms
for the 1,000 paragraphs, its boxes now carry `NodeId`s, and building
it incrementally can wait until it is measured to matter. What is
kept between passes is a layout cache keyed by node: for an
element's principal block-level box laid out in flow (`layout_block`,
a table's wrapper included), the result — its fragment subtree
relative to the box, which is how layout holds fragments until its
final pass, its height, its margins and its baselines — with the
inputs it was computed from. A later pass reuses the result instead of
laying the box out when:

- the element is not layout-dirty and has no dirty descendant;
- the inputs are the same: the containing block's width, the width
  available beside floats, and the containing block's height (for
  percentage heights);
- the box's layout touched no float: it met none in its formatting
  context and placed none, which the first pass records, and at reuse
  the context holds no float, placed or pending, that could reach it;
- no absolutely positioned box was carried out of it, since such a
  box is laid out against an ancestor.

Everything else is laid out as today: the dirty elements, their
ancestors — whose child loops run again, their clean children hits —
anonymous boxes, floats and inline-blocks inside a container that
changed. That is the roadmap's propagation in this engine's terms: a
change of content size reflows the ancestors, and a box that only
moved, because something before it grew, is placed by one translation
and not laid out again. The cache's entries for freed nodes are
dropped, and a new viewport width misses every entry, a full relayout
as today. Two counters, boxes laid out and boxes reused, say what each
pass did, and the engine exposes them as it does its run counters.

**Paint and raster invalidation (M13.3).** The display list is built
from the fragments on every frame, as today, which costs little. It is
compared with the last frame's: the longest common prefix and the
longest common suffix of equal items, and the differing middle of each
list is what changed. The damage is the union of the middle items'
bounds in both lists. A text item carries only its origin, size and
string today, so it gains its run's width and line extent from its
fragment, and its bounds are those padded by a full em on every side,
since a glyph may reach outside its advance and its line. A scroll or
a new viewport size damages the whole frame. `render` keeps the last
frame's pixels and rasterizes only the damage, as whole-pixel
rectangles rounded outwards and merged: under a clip to each rectangle
it replays the items that meet it, and every clip and its end whatever
its place, so the clips nest as in a full raster. The bar is exact: a
damaged frame's bytes equal a full raster's, which a test checks after
random mutations and on every reference page. Where tiny-skia cannot
give that, or the damage covers most of the frame, the frame is
rasterized whole.

**The frame scheduler (M13.4).** The shell asks the engine whether a
frame is wanted (`needs_frame`: a mutation, a resize, an animation in
progress) and passes each frame the time it is drawn for; the engine
reads the clock only to measure its stages for reporting, so what a
frame shows is a function of the document, the viewport, the scroll
and that time, which tests drive. Input between frames is applied as
it comes and its work deferred to the frame, so a burst of mutations
costs one pass. The scroll animation becomes time-based, keeping
today's feel — a third of the remaining distance per frame at sixty
frames a second — so a slow frame no longer slows it. The budget is
16.7ms a frame. The roadmap asks what a frame over it skips. Nothing
that affects what is shown: what is visible can depend on content
anywhere before it in the tree and on positioned boxes anywhere, so
skipping layout outside the viewport is not safe in this engine, and
there is no other deferrable layout work yet. What a late frame gives
up is the animation frames it could not draw, which time-based
animation makes up in one step. Every frame records its stages'
durations and whether it overran, and the frame takes a deadline that
later deferrable work (M19's layer re-raster, M22's animations) is to
honour. The policy is written down in `docs/perf-budgets.md`, which
this milestone creates (T4.1).

**The public API (M13.5).** `Engine::set_document` installs a
`Document`; `Engine::load_document` parses HTML and calls it.
`Engine::document` lends the document, and `Engine::mutate` lends it
mutably to a closure, after which the engine reads the journal and
marks what changed, and `needs_frame` says so. `hit_test` answers
with a `NodeId`; element scroll offsets are kept by `NodeId`, so they
survive mutations, and are dropped when their node is freed; `link_at`
walks the document's parent links. The inspection calls that return
the cascade and the fragments stay, answering in `NodeId`s.

**Acceptance (M13.6).** A benchmark, `crates/engine/benches/mutation.rs`,
lays out the 1,000-paragraph page, then appends 1,000 paragraphs one a
frame, and 1,000 more in one frame, reporting the milliseconds per
frame and per stage and the boxes laid out and reused; its numbers go
into `docs/perf-budgets.md` beside the scroll benchmark's. An engine
test holds the claim itself: after the first frame, appending a
paragraph lays out the new paragraph and the boxes around it that
changed, its ancestors', and reuses every other.

**Incremental equals from scratch.** Every stage's incremental result
must be what a full pass over the same document gives: the styles, the
fragments, the display list and the pixels. Engine tests apply seeded
random sequences of mutations and compare each frame with one drawn by
a fresh engine from a snapshot of the document; a new fuzz target,
`incremental`, reads a document and a mutation script from its input
and asserts the same equality, so the fuzzer hunts for the case the
invalidation rules missed. The reference pages, each loaded, mutated
and mutated back, must still match their captures exactly.

## 2. Not in this milestone, said rather than hidden

Building the box tree incrementally; it is rebuilt per pass, which is
measured. A paint-only style change, such as a new colour, relays out
the element's subtree, its ancestors and their other children being
reused: fragments hold copies of their styles, and fragments that
refer to styles instead, so that such a change only repaints, are
M35's. Text shaping's cost, measured above: keeping each font's face
instead of rebuilding it per call, and caching word measures in
layout, are recorded as a new item, M35.8. Damage from a scroll as a
shifted frame (M19); compositing layers (M19); parallel layout
(M35.3); rule hashing, Bloom filters and style sharing (M35.2);
parsing into the arena as bytes arrive (M14.6); scripting, the
collector that frees detached nodes, and mutation observers (M24,
M25); input events (M26); benchmarks tracked by criterion in CI
(T4.2).

## 3. Order of work

Each step a vertical slice: build, tests, clippy green, smoke run, then
the next. (1) `dom::Document`: the arena, `NodeId`, navigation, the
mutation API with its checks, the journal, `from_node` and `to_node`,
with no consumer changed. (2) Style over the arena: the matcher and
the cascade read the document, `Styles` replaces `StyledDom`,
`FlatDom` is retired, the UA sheet is parsed once; every style test
unchanged in what it asserts. (3) Layout, paint and the engine on
`NodeId`s: boxes and fragments carry them, the engine holds a
`Document`, hit testing, `link_at` and scroll offsets use them; every
test and reference page unchanged. (4) Mutation in the engine
(M13.5): `set_document`, `mutate`, `needs_frame`, every mutation
running the full pipeline — the correct baseline — with the
differential tests and the `incremental` fuzz target landing here, so
that every later step is checked against it. (5) Style invalidation
(M13.1): selector features, dirty marking from the journal, the
incremental restyle and its counter. (6) Layout invalidation (M13.2):
dirty propagation, the layout cache and its conditions, its counters.
(7) Paint and raster invalidation (M13.3): item equality, the diff,
the retained frame and the clipped raster, the exactness test. (8) The
frame scheduler (M13.4): time passed in, time-based scrolling, stage
timings and the deadline, the shell's loop, `docs/perf-budgets.md`
with the policy. (9) The benchmark (M13.6) and its numbers. (10) The
documents.
