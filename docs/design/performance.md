# Design note: the performance pass

Roadmap M35, with T4.2. Written before the code, as the record of the
decisions, and corrected to the code when the milestone ends: where
they disagree, the code wins, and section 4 lists the places, marked
*(corrected)*. What ships is described in `docs/architecture.md`
"Current state", and its numbers live in `docs/perf-budgets.md`.

Every milestone since M13 has measured what it built and recorded what
it left over the frame's budget, each item with the milestone that is
to take it on; most of them name this one. They come to a short list
of causes, and the numbers below are the code before this milestone
(8876b28) on the machine this note was written on, release builds,
medians (logs `m35-base`):

- *Every rule is tried on every element.* A stylesheet is one list in
  sheet order, the user agent's ahead of the page's, and the
  cascade runs each element through every selector of every rule; the
  list of the rules in force is made again at each cascade, and a
  class is found by splitting the `class` attribute at every test.
  The style benchmark matches 1,000 selectors over 10,003 elements in
  309ms. An opacity animated on each of 1,000 paragraphs matches every
  paragraph's selectors again at every frame: 17.7ms of style in a
  23.9ms frame. A class toggled on a table of 1,000 rows restyles its
  5,002 elements in 33ms. A selector that fails at its leftmost
  compound after descendant combinators tries every pair of an
  element's ancestors, the square of the depth.
- *A style is copied wherever it goes.* A computed style is 704
  bytes, and the cascade owns one by value for each element; each
  layout box owns a copy, each fragment owns a copy, every word is a
  fragment with a copy and a string of its own, and the layout cache
  copies every fragment it hands back.
- *Layout's work grows with the page, not with the change.* The box
  tree is built anew for every layout pass and dropped after it;
  fragments are made absolute at the end of each pass, so the cache
  can only reuse a subtree by copying it and moving every copy; flex
  items, grid items and table cells are never kept by the cache, so a
  change inside one lays out its whole container. A paragraph
  appended to 1,000 lays out in 11.8ms (13.1ms the frame, 89 frames
  in 1,000 over budget); a card's title changed in a flex grid of
  1,000 costs 198ms of layout, a caption in a grid gallery of 1,000
  89ms, a cell's text in a table of 1,000 rows 36ms; a colour
  animated on one of 1,000 paragraphs lays out 360 boxes a second for
  a change that only repaints.
- *Text is shaped from scratch.* Each shaping call parses the font's
  face and builds a shaping plan again; a word missing from the
  measure cache is itemized twice, once to measure it and once for
  its line's metrics; justification by character measures letter by
  letter (165ms against 61ms for 1,000 paragraphs). The first frame
  of 1,000 paragraphs is 63ms, 50ms of it layout.
- *A realm compiles its bindings.* Every realm, and every worker,
  compiles and runs the DOM's bootstrap, now 15,998 lines and 512KB
  of JavaScript: 30.5ms before a page's first script runs.
- *One thread does it all.* Nothing in style, layout or raster runs on
  more than one core; only a large gradient is banded across threads
  (M23). The machine has sixteen.
- *Nothing measures memory.* The back-forward cache keeps six
  documents whatever they hold, their decoded images and realms with
  them; the layer cache has a budget, nothing else does, and no
  number says what a page of a given kind costs to keep.
- *Startup was never measured.* The browser builds a whole engine to
  borrow its fonts, scans the system's fonts in itself and again in
  each renderer, opens the profile before the window, and starts two
  renderers before the first frame.

This milestone takes each cause on where the work is, keeps what
was measured as the record, and makes the numbers a rule: budgets in
`docs/perf-budgets.md` that a milestone's exit must meet (M35.7).

The documents and references read, at the versions current when this
note was written (15 October 2026):

- CSS Cascading and Inheritance Level 5 and Selectors Level 4 (W3C
  Working Drafts): what an index of rules must preserve, the cascade's
  order of origins, layers, specificity and order of appearance, and
  which selectors depend on what (`:has()`, `:is()`, `:where()`,
  `:nth-last-child()`, the structural pseudo-classes);
- the HTML Standard (Living Standard): the event loop's "update the
  rendering", what a frame owes the page;
- Speedometer 3.0 (released in 2024 by the browser vendors together):
  its TodoMVC workloads add a hundred items through the input, mark
  them complete and delete them, timed through the frames that follow;
- the reference engines' documents of their style and layout
  systems, as they describe themselves: rules kept in buckets by the
  rightmost compound's id, class and tag, with a universal bucket for
  the rest, and an ancestor Bloom filter that rejects a descendant
  selector without walking the tree (Blink's rule set and Stylo's
  selector map); a style sharing cache that gives an element its
  sibling's or cousin's style when nothing could tell them apart; and
  an immutable fragment tree whose fragments hold their offsets from
  their parent and are reused whole by the next layout when their
  constraints did not change (Blink's LayoutNG);
- QuickJS-ng's bytecode reader and writer (`JS_WriteObject`,
  `JS_ReadObject`) and rquickjs 0.14's `Module::declare`,
  `Module::write` and the `unsafe` `Module::load`.

No reference browser is timed against: its machine and its work are
not this engine's, and the budgets below are this engine's own, set by
what a frame and a user's patience allow.

## 1. Decisions

### What is measured, and the rule

**Benchmarks of the browser's use, not only of its parts (M35.1).**
The twenty-one benchmarks so far each time one feature. This milestone
adds four that time what a user meets:

- `benches/pages.rs`, the page-load benchmark: twenty pages, each of
  a class the web is made of (an article, a news front page, a
  documentation page with code, a forum thread, a wiki page with
  tables and an infobox, a search results page, a product grid, a
  dashboard of cards, a long form, a data table, a photo gallery, an
  SVG chart page, a page of nested lists, a page built by script, a
  page of many small images, a page of frames, a page of many
  stylesheets, a page in several scripts and directions, a print-like
  page of columns, and the browser's own `about:silver`), written for
  the benchmark into `crates/engine/benches/pages/`, each loaded from a
  local server to its first frame and timed by stage. The pages are
  written, not saved: a saved page is its owner's work, and could not
  be committed. A maintainer's own saved pages are measured the same
  way from a folder named by `SILVER_PAGES`.
- `benches/interaction.rs`, Speedometer's TodoMVC in plain DOM and
  scripts: a hundred items added through the input's events, marked
  complete and deleted, each step timed with the frames that follow
  it.
- `benches/jank.rs`: ten seconds of a page scrolled by the wheel while
  three animations run, one composited, one that repaints and one that
  lays out, the frames over budget counted and the worst kept.
- *Memory per page class*: the shell's `--check-processes memory`
  serves the twenty pages, shows each in a renderer of its own, and
  says what each renderer holds once the page is shown (its private
  bytes and its working set), with the browser's.

**The rule (M35.7, T4.3).** `docs/perf-budgets.md` gains a table of
budgets, each a number a benchmark prints: a frame's 16.7ms for what a
user does once a page is shown (a scroll, a change of one element, a
paragraph appended, an animation's frame), and stated budgets for a
first frame, a realm, a cold start and a page class's memory. A
milestone's exit runs the benchmarks; a number past its budget, or
more than 10% worse than the code before, blocks the exit unless an
exception is written there with the item that will undo it (T4.3 as it
was). What is over its budget at this milestone's end stays in "Over
budget, and why" with its item.

**T4.2: the benchmarks run in CI, not timed there.** The benchmarks
stay plain binaries (`harness = false`) that print medians and worsts:
Criterion would add a tree of crates for statistics the turns of T4.3
already give, and a shared runner's timings say nothing about the
engine. What T4.2 is for, keeping the benchmarks from rotting, CI
does: each benchmark runs once in a quick mode (`SILVER_BENCH_QUICK`,
one round, the smallest sizes) on every push, so a benchmark that no
longer builds or panics fails the run. The numbers are taken on the
maintainer's machine, by turns, as T4.3 says.

### Style

**Rules in buckets by their rightmost compound (M35.2).** A rule set
keeps, besides its list, an index: each selector of each rule goes in
one bucket, chosen from its subject compound in this order: its id;
else one of its classes (the first); else its tag; else, for a
compound of one `:is()` or `:where()` argument list whose members all
fall in buckets, each member's; else the universal bucket. An element
gathers the rules of its id's bucket, each of its classes' buckets,
its tag's bucket and the universal bucket, in their indices' order,
which is the sheet's, so the cascade's order of appearance holds;
a rule with two selectors in two buckets is matched once. The lists
of rules in force under the media's answers are made once for each
answer and kept until a sheet or an answer changes, not at each
cascade. An element's classes are split once per cascade, not at
each test.

**An ancestor Bloom filter (M35.2).** A cascade's walk keeps a counting
Bloom filter of the ids, classes and tags of the element's ancestors:
`ANCESTOR_FILTER_SLOTS`, 4,096 eight-bit counters, two hashes each. A
selector records, when its rule set is made, the hashes of the ids,
classes and tags its descendant and child combinators require of some
ancestor (up to four), and is not matched when any is absent from the
filter: no false negatives, so no answer changes. A restyle, which
does not walk the tree in order, fills the filter for an element from
its ancestors, kept for siblings. The matcher's memo of compounds it
failed on lives for a whole cascade rather than one element's match,
keyed by the compound's place and the ancestor, so that the quadratic
case M17's fuzzing found is linear.

**Style sharing between siblings (M35.2).** An element whose previous
sibling has the same tag, the same id (none), the same class list and
the same attributes that some selector of the page names, the same
state (hover, focus, active, checked, disabled and the rest), no
`style` attribute, no animation, no pseudo-elements of its own and the
same parent style is given the sibling's computed style rather than
cascading again, when no selector of the page depends on position
among siblings (`:nth-*`, `:first-*`, `:last-*`, `:only-*`, sibling
combinators), on `:has()`, or on the element's own content
(`:empty`); `STYLE_SHARING_CANDIDATES`, 8 earlier siblings, are
looked at. The style is shared, not copied (below).

**Matched rules kept for what animates (M35.2, M35.11).** An element
restyled because an animation or a transition sampled a new value
keeps the declarations its rules matched at its last full restyle,
and its style is computed again from those and the sampled values,
without matching a selector; anything that could change what matches
(a mutation, an attribute, a state, a sheet, the media) drops what was
kept, as now. An opacity on 1,000 paragraphs should cost no more than
the cascade of its values.

### Styles shared, fragments shared

**A computed style is shared, never copied (M35.9, M35.5).** The
cascade keeps each element's style as an `Arc<ComputedStyle>`; a box
and its fragments hold the same `Arc`; an anonymous box's style and a
pseudo-element's are made once and shared the same way; a style
sharing hit and an inherited style that changes nothing are the same
`Arc`. A fragment's text is an `Arc<str>` shared with the box it came
from where the word is the box's whole text, else its own. A
`ComputedStyle` stays 704 bytes but exists once per distinct style;
a size test keeps `LayoutBox` and `Fragment` from growing unseen.

**Fragments relative to their parent, children shared (M35.9).** A
fragment keeps the offset of its border box from its parent's, and
its children as `Arc<[Fragment]>`: layout no longer makes coordinates
absolute at the end of a pass, and the layout cache hands back a kept
subtree as the same `Arc`, moved by changing the offset its parent
holds, so a pass copies only what it laid out. What needs a
fragment's place on the page (paint's walk, hit testing, scrolling,
caret and selection, `getBoundingClientRect` and the rest) gets it
from a walk that adds offsets as it goes down (`DocumentLayout::iter`
yields each fragment with its absolute origin), as paint's walk
already recurses. A change that only repaints (a colour, a
background, `visibility`, an outline) lays nothing out: the cache
keeps a box whose inputs and layout-affecting style did not change,
and its fragments take the new style's `Arc` without layout (the
"paint-only change" M22 and the compositing note asked for).

**Flex items, grid items and table cells kept by the cache (M35.9).**
An item or a cell is laid out through the same `take` and `keep` as a
block, keyed by its node and the inputs its container gave it (the
available inline size, the definite block size if any, and whether
its size is a measure or a layout); the per-pass memo of items stays
for the measures a container asks more than once in a pass. A card's
title changed in a grid of 1,000 lays out the card and the container's
placement, not 1,000 cards.

**The box tree kept and changed in place (M35.9).** The engine keeps
the box tree between passes; a pass builds anew only the subtrees of
the boxes whose element, style or children changed (what the
invalidation already marks), and keeps the rest, their identity
being what the cache and the per-pass memos key by. A `::first-line`
or `::first-letter` style is kept with its box rather than cascaded
again at every build.

### Text

**A font's shaping face and plans kept (M35.8).** Each font keeps its
parsed face for its life (the face borrows the font's bytes, which the
font owns, held together by `self_cell` rather than `unsafe`), and its
shaping plans by direction, script and language, at most
`MAX_SHAPE_PLANS`, 64, a font. A word's measure is looked up without
making a `String` of it; the line's metrics of a run are taken from
the font found for the measure, not itemized again; the measure
cache's lock is split by font size into `MEASURE_SHARDS`, 16, so
that layout on several threads does not queue on one lock (below).
Justification by character measures a word's letters in one shaping
call and reads their advances, not a call a letter.

### Scripts

**A realm's bindings loaded compiled (M35.12).** The bootstrap becomes
a module whose default export is the function a realm calls with its
natives. Each process compiles it once, at the first realm or ahead
of it, writes its bytecode with QuickJS's own writer
(`Module::write`, safe) into a process-wide `OnceLock`, and every
realm after reads it (`Module::load`) and runs it: the compile is paid
once a process, not once a realm. A renderer compiles it on a thread
of its own as it starts, so the spare renderer has it before its first
page, and so has a renderer whose first page has no script yet.

The reading is `unsafe` in rquickjs, because QuickJS's reader trusts
its input, and the `js` crate forbids `unsafe`. This milestone makes
the one exception the roadmap asked for, reviewed: a single function
in `js` (`bytecode::load_bootstrap`) under `#[allow(unsafe_code)]`,
which reads only bytes this process's own writer wrote from the
bootstrap compiled into the executable, in memory, never from a
file, a pipe, another process or a page. Its argument is written
where it is, `docs/security.md` lists it, and the shell's test that
finds every `unsafe` admits this one site by name and no other.

### Threads

**Independent work on a pool of threads (M35.3, M35.4).** `rayon`'s
pool, at most `MAX_WORKER_THREADS`, 8, threads a process (fewer on a
machine with fewer cores), runs what has no order between its parts:

- *Layout*: a flex container's items, a grid's items and a table's
  cells, each laid out with the inputs its container gave it, when
  there are at least `PARALLEL_MIN_ITEMS`, 32, of them; a block's
  children stay in order, since floats, margins that collapse and the
  space they share make each depend on the last. What layout shares
  becomes safe to share: the per-pass memo of items and widths a
  concurrent map, the layout cache's map behind a lock taken once a
  box, the font measures sharded. The fragments made are the same
  whichever thread made them, and a test lays the benchmarks' pages
  out on one thread and on eight and compares them.
- *Raster*: the software backend's tiles, and the bands of a partial
  redraw, drawn each into its own pixmap on the pool, at least
  `PARALLEL_MIN_PIXELS`, 256KB of them, and composited in order; the
  pixels are the same as one thread's, which a test compares.

A renderer's pool is its own; the sandbox's job limits a renderer to
one process, not one thread.

**The display list kept while nothing that paints changed (M35.4).** A
frame whose layout, styles and scroll offsets are those of the last
frame lists nothing again; the backend's retained list is reused as
it is.

### Compositing v2

**Tiles drawn opaque, composited as copies (M35.10).** The document's
tiles are drawn over the canvas colour, opaque, rather than into
transparency; compositing a tile is then a copy of its rows rather
than a blend, and a frame drawn before its tiles are ready draws the
same pixels as one composited from them, which removes the reason a
page's first frame drew tiles it did not need yet.

**Layers above the tiles (M35.10).** A `will-change` layer inside the
document's content is composited over the document's tiles rather
than into them: a frame that moves or turns it composites the tiles
as they are and the layer over them, and draws neither again. The
document's tiles leave the layer's box out, as they leave a scrolled
layer's.

### Memory

**What a document holds, counted (M35.5).** An engine says what each
of its documents holds (`Engine::memory`): its nodes' data, its
styles, its layout's fragments, its decoded images, its layers and
its realm's heap, from the sizes each part knows, not from the
allocator. The back-forward cache is bounded by bytes as well as
count: `MAX_KEPT_BYTES`, 256MB, the oldest documents dropped first; a
kept document's decoded images and layers are let go when it is put
away, to be decoded again if it is shown. The layer cache's eviction
no longer formats a key to find the oldest.

**An idle renderer trims (M35.5).** A renderer whose views are all
hidden for `IDLE_TRIM_AFTER`, 30 seconds, drops its layers, its
raster caches, its decoded images not on a shown page, and its text
measures, and runs its realms' garbage collectors.

**Arenas: the shared style is the arena.** The roadmap names shared
string and data arenas a pipeline stage. A style shared rather than
copied, text shared with its box, and fragments shared between layouts
are what an arena would have bought here; interning the DOM's tag and
attribute names is section 2's.

### Startup

**A cold start measured, and nothing done before the window that can
wait (M35.6).** `--startup-trace` prints the time from the process's
start to each step and to the first frame of the blank tab; the
budget is `STARTUP_BUDGET`, 300ms, to that frame. The browser no
longer builds an engine to borrow its fonts; the system's fonts are
scanned once a process, in the background, while the window opens;
the profile is opened after the window shows; the first renderer
starts at once and the spare after the first frame.

### Bounds

Every constant above is a bound a hostile page cannot pass:
`ANCESTOR_FILTER_SLOTS` and `STYLE_SHARING_CANDIDATES` fix their
structures' sizes; `MAX_SHAPE_PLANS` and `MEASURE_SHARDS` bound the
text caches beside `MAX_MEASURES`; `MAX_WORKER_THREADS` bounds the
threads, and the work on them is bounded by what the page could
already make one thread do; `MAX_KEPT_BYTES` and `IDLE_TRIM_AFTER`
bound what a renderer keeps. The bytecode is never a page's.

## 2. Not in this milestone, said rather than hidden

- *Layout without recursion* (M35.9's last sentence): the box tree's
  build, layout and paint's walk recurse, and the shell's threads have
  32MB of stack for a page nested to `dom::MAX_DEPTH`. Turning each
  into a loop over an explicit stack is every algorithm of the layout
  crate rewritten; the stack is enough, and measured, at the depth the
  parser allows. A G item.
- *A block's in-flow children laid out in parallel* (M35.3): floats,
  collapsing margins and the space a block's children share make each
  child depend on the one before; only work with no order between its
  parts runs in parallel here. A G item, should a design that keeps
  the order be found.
- *Criterion* (T4.2): declined above, for plain binaries run in CI in a
  quick mode.
- *Speedometer itself*: its workloads are written in frameworks this
  engine has not been tried on, and are not vendored; the interaction
  benchmark is its TodoMVC in plain DOM.
- *Pages saved from the web* for the page-load benchmark: written
  pages of the same classes, and a folder for a maintainer's own.
- *Interning names* (M35.5's arenas): the DOM keeps an element's tag
  and attributes as strings; interning them, and matching by atom, is
  a G item.
- *Raster at the device transform* for a layer that does not animate,
  and a layer's raster scale kept through an animation
  (`compositing.md`, section 2): a layer is rasterized at its own
  scale and resampled, as now. A G item.
- *Presenting the GPU backend's frame without reading it back*: waits
  for the toolbar to draw through the same backend (M37).
- *Shared surfaces and drawing at once after input* (G125), and the
  GPU backend's tiles drawn in parallel: the GPU backend draws as now.
- *A worker's transfer and WebAssembly's memory* (G85, G79): as now.
- *2,000 particles on a canvas* (G81): as now.

## 3. Order of work

Each step a vertical slice, with its tests, a sabotage pass and the
gate, measured against the base before it is committed.

1. *Benchmarks*: `pages`, `interaction` and `jank`, the twenty pages,
   `--check-processes memory`, the quick mode and CI running every
   benchmark once (M35.1, T4.2).
2. *Text*: faces and plans kept, measures without a `String`, a run's
   metrics from its measure, the measure cache sharded, justification
   by character in one call (M35.8).
3. *Compiled bindings*: the bootstrap a module, its bytecode written
   once a process and read by every realm, the reviewed `unsafe`, the
   renderer's warm-up (M35.12).
4. *The rule index*: buckets by the rightmost compound, the rules in
   force kept, classes split once, the Bloom filter, the memo of
   failed compounds a cascade long (M35.2).
5. *Style sharing* and *matched rules kept for what animates* (M35.2,
   M35.11).
6. *Shared styles*: `Arc<ComputedStyle>` from the cascade to the
   fragments, shared text, the size tests (M35.9, M35.5).
7. *Fragments relative and shared*: offsets from the parent,
   `Arc<[Fragment]>` children, the cache handing back what it kept,
   absolute places from the walk, paint-only changes without layout
   (M35.9).
8. *Items and cells kept by the cache* (M35.9).
9. *The box tree kept and changed in place* (M35.9).
10. *Parallel layout* of items and cells (M35.3).
11. *Parallel raster* and the display list kept (M35.4).
12. *Compositing v2*: opaque tiles composited as copies, layers above
    the tiles (M35.10).
13. *Memory*: `Engine::memory`, the back-forward cache by bytes, kept
    documents let go of their pixels, the idle trim, the eviction
    without formatting (M35.5).
14. *Startup*: `--startup-trace`, fonts scanned once in the
    background, no borrowed engine, the profile after the window, the
    spare after the first frame (M35.6).
15. *Acceptance*: the budgets in `docs/perf-budgets.md`, T4.3 against
    the base, the fuzz runs (M35.7).
16. *The documents*.

## 4. Corrected to the code

Where the milestone's code came out otherwise than the sections above
planned, *(corrected)*, by section. The step notes say how each was
found, and `docs/perf-budgets.md` "Performance v1 (M35)" has the
numbers.

*The causes.* *(corrected)* `docs/perf-budgets.md` gave a computed
style as 376 bytes and a box and a fragment as 448 and 464; at the
base a style was 704 bytes, and a box and a fragment each held one,
so each was over 704. A box is now 160 bytes and a fragment 128,
pinned by a test.

*What is measured.* *(corrected)* The page-load benchmark's twenty
pages are nineteen of the web's classes and `about:silver`, as
planned. T4.3's base tree is step 1's commit (348f075), which is the
base's engine with the new benchmarks, so that they are timed before
and after too.

*The rule.* *(corrected)* The budgets are a table in
`docs/perf-budgets.md`, each with what M35 measured; two are past (a
1,000-row table's cell edit and class toggle, G142) and written in
"Over budget, and why". T4.3 found two regressions of the milestone's
own, mended in step 15: the ancestor filters, in a map hashed at every
match of a selector with a combinator, are kept by slot index; and an
opacity on 1,000 paragraphs, whose style stage step 7b had doubled
again, each changed fragment's style copied since the lists are shared
with the cache, now gives a fragment that carries the element's very
style the cascade's new one. Three rows past 10% are written as
exceptions with their items (G139 to G141).

*Style.* *(corrected)* The ancestor filter is not a counting filter
kept along the walk: a restyle does not walk in order. It is a
4,096-bit filter (`ANCESTOR_FILTER_BITS`, not `ANCESTOR_FILTER_SLOTS`),
two bits a feature, made for each parent the first
time one of its children is matched, from its parent's and its own,
and kept for the matcher's life (`MAX_ANCESTOR_FILTERS`, 16,384 of
them). The hashes a selector requires are made when it is parsed, so
`querySelector` uses the filter too. A subject that is a lone `:is()`
or `:where()` goes in the universal bucket; its members are not
bucketed. The memo of failures is kept for the matcher's life, keyed by
the selector's address, at most `MAX_FAILED` places (step 4).

Style sharing is not a sibling's style given when a list of conditions
holds, but a cascade's kept base styles, found by what each is made
of: the number of its parent's style among the distinct ones met (an
equal one among the last eight takes its number, so cousins share),
its matched declarations by address with their tier, layer,
specificity and order, its `dir`, whether it is the root, and its
`rem`; never one with an inline style's or a hint's declaration or one
that reads `attr()`; at most `MAX_KEPT_STYLES`, 4,096. Position among
siblings needs no condition: matching decided it. Matching is not
skipped for what animates, which was 0.8us of an element's 14us; an
animation of opacity, transform, translate, rotate or scale alone, none
important, is set on the base style rather than computing it again
(step 5). `STYLE_SHARING_CANDIDATES` is not a constant of the code.

*Styles shared, fragments shared.* *(corrected)* A fragment's text
stays a `String` of its own: with fragments shared between passes a
word's string is made once, at the pass that lays it out (step 6). A
fragment's children are a `Children`, an `Arc`'d `Vec`, none for none,
copied on write a level at a time, not an `Arc<[Fragment]>`, which
cannot be written in place while a layout builds it; a cache entry
holds its box's subtree whole, shared with the layout that made or
took it, and the places in tree order a taken subtree carries are
moved by a shift on its list rather than by copying it (step 7). What
only repaints is a difference in the colour, the cursor, the outline
or the custom properties alone, and in the background and border
colours but of a table part's box; `visibility` is not (`collapse`
lays a row out), and a box that draws a text decoration is laid out
(step 7). Items and cells are kept under their node, a few layouts
each (`MAX_ITEM_LAYOUTS`, 4), but not an anonymous one, a generated
one, or one an ancestor's pseudo-element styles; a box's content
widths are kept across passes too, which the note did not have (step
8). The box tree is kept by the layout cache, not the engine: an
element's own box is given back whole when nothing generated, no
marker, no control and no first-line style is in its subtree; the
numbering, marker and first-line walks still visit the whole tree, and
inline elements are built anew (step 9). The nightly fuzz run found two
holes in the tree kept, mended: a restyle written into the layout
without laying it out left the old styles in the boxes kept, and a node
moved took its boxes built where it was, without the decorations of
its new ancestors; the boxes of both are forgotten now (step 15).

*Text.* *(corrected)* Justification by character costs a fragment a
character, and its measures were cache hits already: nothing of text's
to do, and the "one shaping call" is dropped (step 2).

*Scripts.* *(corrected)* The bytecode keeps its source and debug lines,
so that nothing a page sees changes; a binding's `toString` therefore
shows its JavaScript, not `[native code]`, as it did (G135). `js`
denies rather than forbids `unsafe`, since a forbid cannot be allowed
under it (step 3).

*Threads.* *(corrected)* Layout's pool is the layout crate's own, made
the first time a container asks, threads with 8MiB stacks; nested
containers lay out on the thread they are on. Eight threads made the
flex grid's first layout 2.2 times faster, not eight; the cause is not
found (G134; step 10). Parallel raster was built, measured and
taken out: the tiles' drawing is not where a raster goes (the walk of
the list for the extents and the damage, and the glyph masks, are),
and four more glyph caches were not worth it. The display list is not
kept: a frame follows a change, and the list is 0.3 to 0.5ms of the
1,000-paragraph page's frame. G132 and G133 (step 11).

*Compositing v2.* *(corrected)* Not done: a turned card's frame is
1.9ms, a moved card's 0.3ms and a scroll's 0.7ms; opaque tiles would
save 0.2ms of a scroll, and layers above the tiles need overlap rules
in z-order and the composite split from the raster. G131 (step 12).

*Memory.* *(corrected)* A document put away lets go of its decoded
images; its layers needed nothing, since the backend is the engine's.
A document larger alone than `MAX_KEPT_BYTES` is not kept. The trim
lets go of every decoded image of an engine whose view is hidden, and
asks for no frame: a frame is wanted whether a view is hidden or not,
and one drawn would make again what the trim let go (step 13).

*Startup.* *(corrected)* The profile is read on a thread of its own
while the fonts are scanned, and given before the first navigation,
not opened after the window: the first navigation would go without
its cookies, and the first renderer starts with the session, before
the window. The browser's scan runs from `main` and is waited for when
the tab is made; the window is made after the session, whose toolbar
needs the fonts. A renderer does not scan at all: the browser sends it
the files of its fonts after the hello (`ToRenderer::Fonts`), a file
gone falling back to a scan (step 14).

*Bounds.* *(corrected)* The bounds are `ANCESTOR_FILTER_BITS` and
`MAX_ANCESTOR_FILTERS` for the filters, `MAX_FAILED` for the memo,
`MAX_KEPT_STYLES` for the kept styles, `MAX_ITEM_LAYOUTS` for an item's
layouts, `MAX_SHAPE_PLANS` and `MEASURE_SHARDS` beside `MAX_MEASURES`,
`MAX_WORKER_THREADS` and `PARALLEL_MIN_ITEMS` for the pool,
`MAX_KEPT_BYTES` and `IDLE_TRIM_AFTER` for what a renderer keeps, and
`MAX_STROKE_DASHES` for a canvas stroke's dashes, which the base's
nightly fuzz run found unbounded; `PARALLEL_MIN_PIXELS` went with the
parallel raster.

*Not in this milestone.* *(corrected)* Registered: layout without
recursion as G127, a block's children in parallel as G128, interning
names as G129, raster at the device transform as G130.
