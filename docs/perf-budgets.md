# Performance budgets

Roadmap T4.1, begun at M13 (M13.4). What a frame may cost, what a frame
that costs more gives up, how the numbers are taken, and the numbers.
Every milestone's exit adds its numbers here (T4.1). A regression of
more than 10% on a number recorded here blocks the exit, unless an
exception is written here with the item that will undo it (T4.3).

## The frame budget

A frame has 16.7ms: sixty frames a second, `engine::FRAME_BUDGET`.

The shell draws a frame only when the engine wants one
(`Engine::needs_frame`): the document changed, a scroll offset was set
or is still on its way to its target, or the window changed size. It
passes each frame the time it is drawn for and a deadline, by default
that time plus the budget (`engine::FrameTime`). The engine reads the
clock only to time its stages. What a frame shows is a function of the
document, the viewport, the scroll inputs and the times passed in,
which is what lets tests drive it.

Changes between frames are applied as they come and their work is left
to the frame. A burst of mutations costs one restyle, one layout, one
paint and one raster.

Each frame records what its stages took and whether it ended past its
deadline (`Engine::frame_stats`): style, layout, paint, raster, and the
whole frame.

## What a late frame gives up

The roadmap asks what a frame over budget skips (M13.4). The answer is
nothing that changes what is shown.

- Nothing visible can be skipped. What is visible can depend on content
  anywhere before it in the tree, and on positioned boxes anywhere, so
  laying out only what is in the viewport is not safe in this engine.
  There is no other layout work that could wait: no "layout polish"
  exists here to defer.
- A late frame gives up only the animation frames it could not draw.
  The scroll animation is timed by the clock, not counted in frames: a
  third of the remaining distance goes every sixtieth of a second,
  however many frames share it. So the next frame makes up the time in
  one step. The first frame after a scroll begins moves one sixtieth's
  step, however long the engine was idle.
- The deadline is passed into every frame for the work to come that
  can wait. Since M19 that work is done after the frame is presented,
  in the time before the deadline: the shell calls
  `Engine::draw_ahead`, which draws the tiles of the margin around
  the viewport, nearest first, and leaves what it has no time for to
  a later call or to a frame that shows it. It is not part of the
  frame or its stages. M22's animations will be timed the same way.

## How the numbers are taken

- **Build:** release unless stated; the debug build where the budget
  names it.
- **Font:** the bundled test font (`text::TEST_FONT`), so every machine
  shapes the same text.
- **Machine:** a cloud container with four cores and 15GB of memory:
  an Intel Xeon at 2.1GHz for the numbers up to M15, and one at 2.8GHz
  from M16, which measured its base again ("Selectors and
  pseudo-elements", below), as M17 did ("Flexbox", below). Numbers
  from different machines are not compared; a milestone that changes
  machine measures its base again.
- **Benchmarks:** plain binaries, `harness = false`, run by hand:
  - `cargo bench -p engine --bench scroll` (M9.6; stages since M13.4).
  - `cargo bench -p engine --bench mutation` (M13.6).
  - `cargo bench -p engine --bench load` (M14; going back, M15).
  - `cargo bench -p engine --bench pseudo` (M16.5).
  - `cargo bench -p engine --bench flex` (M17).
  - `cargo bench -p engine --bench grid` (M18).
  - `cargo bench -p engine --bench composite` (M19), on both backends.
  - `cargo bench -p style --bench match` (M4.12; recorded since M16).
  - Criterion-tracked benchmarks run in CI are T4.2, for M35.
- **Counters** prove work that is not done: `Engine::style_runs`,
  `restyled_elements`, `layout_runs`, `boxes_laid_out`, `boxes_reused`
  and `redrawn`.

## The numbers

### Scrolling

The scroll benchmark: a page of 128 paragraphs, 8,061px tall, ten
viewports of 1280 by 800. It is laid out once, then scrolled 40px a
frame for 120 frames drawn sixty a second. Two runs of each build:

| | Release | Debug |
| --- | --- | --- |
| Layout, once | 158–169ms | 185–186ms |
| First frame, the caches filling | 7.8–8.5ms | 14–15ms |
| A scroll frame, median | 2.0–2.1ms | 8.2–9.2ms |
| Its raster, median | 1.9–2.0ms | 7.8–8.7ms |
| Its paint, median | 0.1ms | 0.4–0.5ms |
| The worst frame | 3.7–13ms | 15–26ms |
| Frames over 16.7ms | 0 of 120 | 0–2 of 120 |

A scroll redraws the whole frame (M13.3), so raster is most of it.
Redrawing a shifted frame instead is M19's, which takes a scroll
frame's raster from 3.0ms to 0.85ms ("Compositing", below).

### Changing the document

The mutation benchmark (M13.6) draws a page of 1,000 paragraphs at
1280 by 800. It then appends 1,000 paragraphs one a frame, and 1,000
more in a single frame. Two runs in release, one in debug:

| | Release | Debug |
| --- | --- | --- |
| The first frame | 642–687ms | 749ms |
| Its layout | 634–676ms | 722ms |
| An appended paragraph's frame, median | 7.0–7.1ms | 16.0ms |
| Its layout, median | 5.7–5.8ms | 12.8ms |
| Its paint, median | 1.0ms | 2.8ms |
| Its style and raster, median | 0.3ms and 0.03ms | 0.4ms and 0.09ms |
| The worst of those frames | 15–24ms | 30ms |
| Those frames over 16.7ms | 0–2 of 1,000 | 448 of 1,000 |
| 1,000 appended in one frame | 581–685ms | 720ms |

No frame after the first lays the document out whole: the claim of
M13.6. Each appended paragraph's frame restyles one element and lays
out three boxes: the new paragraph, the body and the root. Every other
box comes from the layout cache, 1,499,500 of them over the thousand
frames. The thousand appended at once cost 1,000 element restyles and
1,002 boxes laid out, the 2,000 paragraphs already there taken from
the cache. The new paragraphs are below the viewport, so raster redraws
little more than the scrollbar's thumb.

Measured as each step of M13 landed, in release, against the step
before it:

| What | Before | After |
| --- | --- | --- |
| Restyle after appending a paragraph to 10,000 (M13.1) | a cascade, 35ms | about 2µs, one element |
| Layout after appending a paragraph to 1,000 (M13.2) | 490ms | 4ms |
| The same with 10,000 paragraphs | 4.6s | 90ms |
| A frame with nothing changed, 1,000 paragraphs at 1280 by 800 (M13.3) | 1.9ms | 0.65ms |
| A frame after recolouring a visible paragraph (M13.3) | 2.0–3.4ms | 0.7–0.9ms |

The cascade of the whole page costs what it did before M13.1: 3ms for
1,000 paragraphs, 35–37ms for 10,000.

### Loading a page

The load benchmark (M14) serves a page of 1,000 paragraphs, 111KB,
and three stylesheets its `<head>` links from `net`'s test server on
127.0.0.1. Each run starts a new engine, so its connections are new
too, calls `Engine::navigate`, takes results as the waker says they come,
and draws the first frame at 1280 by 800 once the page is shown. The
sheets block the page, so it is shown styled, with all three in. Ten
runs a row, medians, and in release the range over two rows:

| | Release | Debug |
| --- | --- | --- |
| The document fetched alone, a client of its own | 0.6–0.8ms | 0.9ms |
| Parsed alone | 1.5ms | 11.3ms |
| `load` to the page shown, its sheets in | 3.3–3.4ms | 13.8ms |
| The first frame | 693–711ms | 891ms |
| Its layout | 686–704ms | 857ms |
| Its style, paint and raster | 2.4–2.7, 0.6–0.7 and 3.7–3.9ms | 18.8, 2.7 and 10.7ms |
| `load` to the first frame drawn | 697–714ms | 906ms |

Fetching, parsing and putting the page together with its sheets take
half a percent of it. The rest is the first layout of 1,000
paragraphs, which costs what it costs when the page is given as
source (the mutation benchmark's first frame, above): text shaping,
M35.8's.

The numbers recorded before M14 were measured again with it. The
mutation benchmark's are within their ranges or at most 2% past them
(its first frame 650–688ms, the thousand appended at once 640–699ms),
inside T4.3's 10%, and the scroll benchmark's layout is 156–170ms over
three runs on a quiet machine, as recorded.

### Going back

The same benchmark (M15) shows the page, leaves it, and goes back to
it, timing the traversal to the end of the first frame drawn after it.
Kept in the back-forward cache, the page comes back whole, its cascade,
layout and scroll offsets with it, and the frame only paints and
rasterizes. Left for seven pages, so that the cache, which keeps six,
lets it go, it is fetched, parsed and laid out again. Before each page
is left, the benchmark waits for its icon, a fetch of its own that
would otherwise keep it from the cache. Ten runs a row, medians, and
in release the range over two rows:

| | Release | Debug |
| --- | --- | --- |
| Back to the page from the cache, to its first frame drawn | 5.2–6.4ms | 14.6ms |
| Back to the page fetched again, to its first frame drawn | 648–656ms | 796ms |

The cached page is back within the frame budget; the one fetched again
costs what its first load did, the first layout's text shaping again.
The rows for loading the page, measured again in the same runs: 3.7ms
from `navigate` to the page shown, against 3.3–3.4ms recorded, and
633–635ms for the first frame, against 693–711ms; inside T4.3's 10%
the one way, and faster the other.

### Selectors and pseudo-elements (M16)

M16 was measured on a new machine, with an Intel Xeon at 2.8GHz, which
ran the benchmarks above again for T4.3: the code before M16
(`770049c`) and M16's in turn, two runs of each, in release.

| | Before M16 | M16 |
| --- | --- | --- |
| Scrolling: the layout, once | 175–180ms | 177–181ms |
| Scrolling: a scroll frame, median | 2.8–2.9ms | 2.7–2.8ms |
| Changing the document: the first frame | 678–741ms | 692–732ms |
| An appended paragraph's frame, median | 13.1–13.4ms | 12.6–12.7ms |
| Its layout, median | 11.3–11.5ms | 11.0ms |
| Those frames over 16.7ms | 151–247 of 1,000 | 115–240 of 1,000 |
| 1,000 appended in one frame | 680–682ms | 701–706ms |
| Loading: `load` to the first frame drawn | 720–809ms | 742–759ms |
| Going back to the page from the cache | 5.7–5.8ms | 5.7–5.8ms |
| Going back to the page fetched again | 748–751ms | 785–787ms |

All are inside T4.3's 10%: the thousand paragraphs appended at once
take 3–4% longer, going back to a page fetched again 5%, and the rest
are within the spread of the runs. That holds since a change M16.5 made
for it. M16's `content` and `quotes` made a computed style 48 bytes
larger and dearer to copy, and a layout pass copies every fragment's
style (below): measured first, an appended paragraph's frame took
15.6–16.0ms against 14.0–14.2ms before M16, 12% longer, and 8% more
instructions under callgrind. Every copy of a style also allocated its
`font-family` again. The family is now shared between the styles that
have it, and the frame takes 17% fewer instructions than before M16.

The machine is no faster than the one before it at this work. The
appended paragraph's frame is past the budget in release too, which
"Over budget, and why" records.

**Selector matching (M16.5).** The match benchmark, 1,000 selectors over
10,003 elements, three runs of each in turn:

| The code as imported (M8) | Before M16 | M16 |
| --- | --- | --- |
| 202–209ms | 291–307ms | 292–328ms |

The roadmap asks for a Bloom filter of ancestors if matching has slowed
by more than twice. It takes 1.4–1.6 times what the code as imported
did, and the growth came before M16, so there is no filter. M35.2's
rule hashing and style sharing remain where the larger win is.

**What pseudo-elements cost (M16.5).** The pseudo benchmark draws a page
of 1,000 paragraphs of three lines at 1280 by 800, each with a link and
an emphasis on its first line, under each of six sheets. It then
appends a paragraph a frame, a hundred times. The box tree is built
again at each frame, and its pseudo-elements with it, while the cascade
and the layout are incremental. Two runs, in release, of the code
before M16.5's changes and of M16's:

| Sheet | The first frame, before | The first frame | An appended paragraph's frame, median, before | An appended paragraph's frame, median |
| --- | --- | --- | --- | --- |
| None | 2.13–2.17s | 2.02–2.18s | 26.6ms | 24.3–24.9ms |
| `::before` and `::after` | 2.16–2.18s | 2.14–2.23s | 33.9ms | 31.0–32.0ms |
| `::first-letter` | 2.13–2.14s | 2.02–2.24s | 34.5–35.4ms | 26.9–29.0ms |
| `::first-line`, colour and size | 2.79–3.04s | 2.54–2.60s | 40.4–43.1ms | 31.9–32.4ms |
| `::first-line`, a background | 2.78–2.86s | 2.06–2.11s | 32.1–34.2ms | 26.4–26.7ms |
| All of them | 2.97–3.07s | 2.59–2.68s | 70.6–71.7ms | 55.3–58.9ms |

Generated boxes are built again with the box tree: `::before` and
`::after` on every paragraph add 6–8ms to an appended paragraph's
frame, the first letter's split 2–5ms. A `::first-line` that sets
inherited properties has every inline element on a first line's run
cascaded again at each build, 7–8ms here for 2,000 links and emphases.
With all the sheets, the generated boxes and the first letters are on
the first line too, and are cascaded again as well: 30–35ms. Besides
the shared `font-family` above, which takes 2ms off every row's
appended paragraph, M16.5 changed three things:

- A pseudo-element's style asked of `Recascade` with its element's own
  style as the parent is the style the cascade kept, so the first
  letter of a paragraph's own text is not cascaded again.
- A first line's run is collected a second time, in its styles there,
  only where a box in the run has a style of its own there. A
  `::first-line` with a background alone now costs a first frame what
  none does, and 2ms of an appended paragraph's, for the boxes it
  paints.
- That second collection stops where the first line has surely ended,
  rather than at the end of the paragraph.

### Flexbox (M17)

**T4.3.** The benchmarks above, run on the same kind of machine as M16's:
the code before M17 (`f04fcc6`) and M17's in turn, in release. The
scrolling and loading rows are four rounds of each, and the matching
row eight runs; the mutation and pseudo rows are two more rounds, run
once the layout cache's change below was in. The ranges are over the
rounds.

| | Before M17 | M17 |
| --- | --- | --- |
| Scrolling: the layout, once | 177–228ms, median 180 | 186–205ms, median 187 |
| Scrolling: a scroll frame, median | 2.8–3.2ms | 2.8–3.2ms |
| Changing the document: the first frame | 720–722ms | 729–730ms |
| An appended paragraph's frame, median | 13.7–14.6ms | 15.1–15.5ms |
| Its layout, median | 12.0–12.6ms | 13.2–13.5ms |
| Those frames over 16.7ms | 259–291 of 1,000 | 394–442 of 1,000 |
| 1,000 appended in one frame | 707–781ms | 710–768ms |
| Loading: `load` to the first frame drawn | 752–792ms | 760–815ms |
| Going back to the page from the cache | 6.2–8.2ms | 6.4–7.3ms |
| Going back to the page fetched again | 752–785ms | 766–821ms |
| Matching: 1,000 selectors over 10,003 elements | 294–335ms, median 311 | 293–372ms, median 319 |

The scrolling rows are for the same page on both sides, of 128
paragraphs. The benchmark grows its page until it is ten viewports
tall, and with M17's text metrics (the flexbox note, "Vertical
metrics") 128 paragraphs are 7,988px rather than 8,061, so the
benchmark as it stands takes 256 and reports twice the layout:
377–402ms, which is 1.47ms a paragraph against 1.37.

The appended paragraph's frame is the row nearest T4.3's line: 8%
longer by the clock, and 5% more instructions under callgrind, 37.1
million a frame against 35.2. It was 11% more, 39.1 million, before
one change. M17's layout cache moves the places of a reused subtree's
boxes along in tree order when something was inserted before them, and
it rebuilt the subtree a fragment at a time to do so even when nothing
had moved, as when a paragraph is appended after it. It now copies
such a subtree whole, as it did before. What remains is the computed
style, 48 bytes larger for the flex properties (424 bytes, from 376),
and the fragments and boxes that carry copies of it: a `Fragment` is
512 bytes and a `LayoutBox` 504. Fragments that refer to their styles
rather than copy them are M35.9's, and take this with the rest.

The pseudo benchmark, the same two rounds:

| Sheet | The first frame, before | M17 | An appended paragraph's frame, median, before | M17 |
| --- | --- | --- | --- | --- |
| None | 1.99–2.06s | 2.09–2.14s | 26.4–27.1ms | 27.8–28.8ms |
| `::before` and `::after` | 2.21–2.24s | 2.26–2.28s | 32.8–34.6ms | 35.4–36.2ms |
| `::first-letter` | 2.03–2.43s | 2.13–2.23s | 33.6–34.0ms | 34.7–35.4ms |
| `::first-line`, colour and size | 2.69–2.77s | 2.60–2.68s | 34.2–36.9ms | 40.2–40.5ms |
| `::first-line`, a background | 2.06–2.08s | 2.16–2.28s | 30.7–31.2ms | 31.9–33.3ms |
| All of them | 2.63–2.65s | 2.74–2.92s | 63.3–64.9ms | 67.9–68.6ms |

`::first-line` with inherited properties reads 14% over in these two
rounds, while the four before read 4% (37.8–41.8ms before, 40.0–43.0ms
with M17). Counted under callgrind, its appended paragraph's frame
takes 129.1 million instructions against 122.0, 6% more, which is the
style's growth again; the rounds' clock moved more than the code did.

**What flex layout costs.** The flex benchmark (`benches/flex.rs`), at
1280 by 800, in release:

| | Release |
| --- | --- |
| A grid of 1,000 cards, the first frame | 2.56s |
| Its layout | 2.53s |
| The grid after a card's title changes, a frame, median of 20 | 2.58s |
| Its layout | 2.57s |
| A chain of 8 nested flex containers, its layout | 1.9ms, 0.24ms a level |
| 16 | 4.2ms, 0.26ms a level |
| 32 | 10.5ms, 0.33ms a level |
| 64 | 25.2ms, 0.39ms a level |

The grid's card is a column of a title, a paragraph that wraps and a row
of two spans, in a wrapping row with gaps. Its time is text measurement,
the font's face parsed and its shaping plan built on every call (M35.8):
a trial that kept both for each font laid 200 cards out in 95ms rather
than 676ms. A change inside a flex container lays all its items out
again, by design (the flexbox note, "The layout cache"), so the frame
after a title changes costs what the first one did.

The chain's time is to grow linearly with its depth. It first took
210ms at 64 levels, 4.1ms a level against 0.6 at 8: each container
measured its item's content widths afresh, down to the foot of the
chain. The pass keeps them now (the flexbox note, "The layout pass").
In instructions under callgrind the layout grows 2.1 and 2.0 times for
each doubling from 16 to 64 levels, and 2.4 from 64 to 128. What grows
faster than the depth is the kept item layouts copied into each layout
of the container above them, 12% of the instructions at 64 levels and
21% at 128 (M35.9).

### Grid (M18)

**T4.3.** The benchmarks above, run on the same kind of machine as M17's:
the code before M18 (`ef22f07`) and M18's built side by side in
release, four rounds of each in turn. The ranges are over the rounds.

| | Before M18 | M18 |
| --- | --- | --- |
| Scrolling: the layout, once | 370–406ms, median 377 | 373–420ms, median 393 |
| Scrolling: a scroll frame, median | 2.9–3.0ms | 2.9–3.2ms |
| Changing the document: the first frame | 691–779ms | 696–790ms |
| An appended paragraph's frame, median | 11.4–15.3ms, median 13.2 | 13.3–15.3ms, median 13.5 |
| Its layout, median | 10.0–13.3ms | 11.6–13.4ms |
| Those frames over 16.7ms | 114–345 of 1,000 | 211–378 of 1,000 |
| 1,000 appended in one frame | 729–768ms | 721–776ms |
| Loading: `load` to the first frame drawn | 763–773ms | 766–777ms |
| Going back to the page from the cache | 5.7–5.9ms | 5.8–6.2ms |
| Going back to the page fetched again | 738–783ms | 754–784ms |
| Matching: 1,000 selectors over 10,003 elements | 295–414ms, median 345 | 303–322ms, median 309 |
| Flex: a grid of 1,000 cards, the first frame | 2.44–2.51s | 2.44–2.73s, median 2.46 |
| Flex: after a card's title changes, a frame, median | 2.48–2.52s | 2.42–2.47s |
| Flex: a chain of 64 nested containers, its layout | 21.1–22.2ms | 22.7–27.3ms, median 24.4 |

The pseudo benchmark, the same four rounds:

| Sheet | The first frame, before | M18 | An appended paragraph's frame, median, before | M18 |
| --- | --- | --- | --- | --- |
| None | 2.13–2.28s | 2.03–2.15s | 25.3–27.3ms | 25.3–28.8ms |
| `::before` and `::after` | 2.18–2.28s | 2.13–2.31s | 32.7–34.8ms | 33.6–35.9ms |
| `::first-letter` | 2.14–2.29s | 2.08–2.28s | 30.2–33.0ms | 30.2–31.6ms |
| `::first-line`, colour and size | 2.54–2.60s | 2.64–2.86s | 35.6–38.0ms | 35.8–41.3ms |
| `::first-line`, a background | 2.16–2.29s | 2.10–2.25s | 28.8–34.4ms | 28.5–33.9ms |
| All of them | 2.62–2.83s | 2.64–2.78s | 64.2–67.2ms | 62.8–73.7ms |

All are inside T4.3's 10%. The appended paragraph's frame, the row
nearest the line at M17, is 2% longer by its median; the computed style
grew by the grid pointer's eight bytes (432 bytes, from 424), and a
`Fragment` and a `LayoutBox` with it (520 and 512, from 512 and 504),
as M17's row grew by 48. The flex chain of 64 reads 12% over by the
rounds' medians, and that is the clock, not the code: the two sides
run again in turn, twenty engines a run, take 17.1–18.1ms and
17.1–18.9ms; and under cachegrind the layout is 139.8 million
instructions before M18 against 140.1 million, 0.2% apart, with 1%
more first-level data cache misses for the larger structures. The
`::first-line` row's first frame reads 4% over by its median, with its
appended paragraph's frame unchanged, where M17's rounds moved as much
for no change.

**What grid layout costs.** The grid benchmark (`benches/grid.rs`), at
1280 by 800, in release:

| | Release |
| --- | --- |
| A gallery of 1,000 figures, the first frame | 875ms |
| Its layout | 862ms |
| The gallery after a caption changes, a frame, median of 20 | 848ms |
| A dashboard of 200 spanning cards, the first frame, median of 5 | 314ms |
| A chain of 8 nested grids, its layout | 1.4ms, 0.18ms a level |
| 16 | 2.9ms, 0.18ms a level |
| 32 | 5.8ms, 0.18ms a level |
| 64 | 15.8ms, 0.25ms a level |

The gallery's figure is a grid of a picture and a caption that wraps,
auto-placed in `repeat(auto-fill, minmax(160px, 1fr))` columns. Its time
is text measurement, as the flex benchmark's is (M35.8). A change inside
a grid container lays all its items out again, since they are kept only
with the container, as a flex container's are, so the frame after a
caption changes costs what the first one did. The dashboard's cards span
one to three columns of `auto` and `fr` tracks and one or two rows, and
go through §12.5's spanning steps. The chain nests each grid in the
`auto` column of the one above, beside a paragraph: an item there is
measured for its contributions, laid out for the rows and laid out
again in its area, and the pass keeps each, so a level costs about the
same at 64 as at 8.

Grid lets a page ask for a lot of work in a few bytes. A unit test, in
the unoptimised build, holds three such pages to the limit of ±10,000
lines and to a time: an item at line 10,000 of both axes and one at
-10,000, 19,999 tracks between them, lays out in 21ms; an `auto-fill`
of one-pixel tracks in a container a million pixels wide stops at
10,000 tracks, 11ms; a thousand items each spanning a thousand of two
thousand `auto` tracks take 0.7s, growing with the items times their
spans (two thousand over two thousand, 2.8s).

### Compositing (M19)

**The compositing benchmark.** `benches/composite.rs`, at 1280 by 800,
in release, two runs, on the software backend and on the GPU backend.
The one adapter here is Mesa's software Vulkan driver, llvmpipe (LLVM
20.1.2, 256 bits), which runs the shaders on the CPU: its column says
what the GPU backend costs on such a driver, not on a GPU. The rows
are the mutation benchmark's page of 1,000 paragraphs scrolled 40px a
frame for 120 frames drawn sixty a second, each frame followed by
drawing ahead until its budget is spent, as the shell does; a card of
twelve, inline blocks, turned two degrees or moved two pixels a frame
through `Engine::mutate` on its `style` attribute, with and without
`will-change: transform`; and 200 translated and faded cards, drawn
through, scrolled.

| | Software | GPU on llvmpipe |
| --- | --- | --- |
| Scrolling: a frame, median | 1.98–2.57ms | 11.0–12.6ms |
| Its paint and raster, median | 1.18–1.52 and 0.79–0.99ms | 2.3–2.5 and 8.8–10.0ms |
| The worst frame; frames over 16.7ms | 3.4–3.8ms; 0 of 120 | 16.1–16.2ms; 0 of 120 |
| A card turned: a frame, median | 11.4–12.1ms | 20.6–20.7ms |
| Its layout and raster, median | 8.5–9.0 and 2.7–2.8ms | 9.3–9.5 and 10.8–10.9ms |
| Those frames over 16.7ms | 2–5 of 120 | 120 of 120 |
| The same, `will-change: transform` | 11.7–12.0ms; 4–17 over | 20.8–21.4ms; 120 over |
| A card moved: a frame, median | 9.3–9.4ms | 21.2–21.7ms |
| Its raster, median | 0.43–0.44ms | 11.1–11.4ms |
| The same, `will-change: transform` | 9.2–9.4ms, raster 0.38–0.39ms | 21.2–21.5ms |
| 200 faded cards: the first frame | 109–147ms | 187–206ms |
| A scroll frame of them, median; over 16.7ms | 1.8–1.9ms; 0 of 120 | 10.6–10.7ms; 18 of 120 |

What the counters say, the same in both runs: on both backends every
frame of the scroll is a composite of the document's tiles, and no
frame rasterizes a tile, the 24 the scroll brings in having been drawn
ahead after the frames before them; the layer holds 12.5 MiB. On the
software backend a scroll frame moves the last frame's pixels by the
scroll and draws again only what the move uncovers and what does not
move with it. The turned card is a layer with or without
`will-change`, as the policy says of a turned box: drawn once and
composited at each of the 120 frames. It is inside the document's
tiles, so a frame draws the tiles it reaches again, two a frame on the
software backend, 56,000 pixels, before compositing them (M35.10).
The moved card is drawn through without `will-change` and composited
with it, both a partial redraw of 33,000 pixels a frame. Most of a
card's frame is its layout: a change to the `style` attribute lays
the card out again though only its transform changed, 8.5–9.0ms of
inline layout (M35.9, which a transition will need, M22). The GPU
backend draws every frame whole and reads it back (D9); on llvmpipe
that is 9–11ms of raster a frame, which a scroll stays inside the
budget with and a card's frame does not.

M19.7's acceptance, sixty frames a second for the scroll and the
turned card on both backends: on the software backend, met by the
scroll in every frame, and by the turned card in 103–118 frames of
120, the frames over the budget long by their layout, not by the
composite; on the GPU backend, met by the scroll on llvmpipe, and to
be measured on an integrated GPU, which this machine has not got.

**T4.3.** The benchmarks above, run on the same kind of machine as
M18's: the code before M19 (`82b6c49`) and M19's built side by side
in release, four rounds of each in turn. The ranges are over the
rounds.

| | Before M19 | M19 |
| --- | --- | --- |
| Scrolling: the layout, once | 363–415ms, median 390 | 371–415ms, median 374 |
| Scrolling: a scroll frame, median | 3.5–4.1ms, median 3.6 | 1.3–1.8ms, median 1.6 |
| Its raster, median | 3.0–3.5ms | 0.7–1.0ms |
| Its paint, median | 0.48–0.60ms, median 0.53 | 0.50–0.84ms, median 0.65 |
| Changing the document: the first frame | 746–833ms | 733–803ms |
| An appended paragraph's frame, median | 16.6–20.4ms, median 19.4 | 20.1–23.4ms, median 21.5 |
| Its layout, median | 14.5–17.6ms, median 16.8 | 16.9–19.5ms, median 18.0 |
| Its paint, median | 1.7–2.2ms, median 2.0 | 2.6–3.1ms, median 2.8 |
| Those frames over 16.7ms | 495–729 of 1,000 | 720–851 of 1,000 |
| 1,000 appended in one frame | 754–868ms | 738–788ms |
| Loading: `load` to the first frame drawn | 784–868ms | 776–837ms |
| Going back to the page from the cache | 6.5–6.9ms, median 6.7 | 16.2–18.5ms, median 16.5 |
| Going back to the page fetched again | 765–838ms | 778–821ms |
| Matching: 1,000 selectors over 10,003 elements | 303–386ms, median 353 | 312–354ms, median 327 |
| Flex: a grid of 1,000 cards, the first frame | 2.49–2.61s | 2.53–2.74s |
| Flex: after a card's title changes, a frame, median | 2.51–2.54s | 2.64–2.67s |
| Flex: a chain of 64 nested containers, its layout | 27.2–29.8ms, median 28.7 | 26.1–35.1ms, median 29.7 |
| Grid: a gallery of 1,000 figures, the first frame | 939–1027ms | 939–1058ms |
| Grid: after a caption changes, a frame, median | 911–974ms | 926–970ms |
| Grid: a dashboard of 200 cards, the first frame | 327–351ms | 331–366ms |
| Grid: a chain of 64 nested grids, its layout | 19.0–22.4ms | 19.5–21.7ms |

The pseudo benchmark, the same four rounds:

| Sheet | The first frame, before | M19 | An appended paragraph's frame, median, before | M19 |
| --- | --- | --- | --- | --- |
| None | 2.05–2.29s | 2.14–2.38s | 31.4–34.9ms, median 33.3 | 35.8–38.0ms, median 37.3 |
| `::before` and `::after` | 2.22–2.44s | 2.19–2.32s | 41.5–48.5ms, median 42.7 | 44.9–51.0ms, median 48.0 |
| `::first-letter` | 2.18–2.32s | 2.20–2.45s | 37.8–41.9ms, median 39.3 | 41.1–46.7ms, median 43.6 |
| `::first-line`, colour and size | 2.58–2.86s | 2.56–2.99s | 42.5–50.4ms, median 47.5 | 47.8–50.4ms, median 49.0 |
| `::first-line`, a background | 2.17–2.30s | 2.16–2.38s | 37.0–41.5ms, median 39.4 | 41.0–42.5ms, median 41.6 |
| All of them | 2.74–2.98s | 2.58–2.88s | 76.4–82.6ms, median 77.2 | 79.5–84.8ms, median 84.1 |

What T4.3 found on the way, and mended before these rounds: the
frame drawing ahead until its deadline, which made going back to a
page 20ms, now done after the frame; the paint walk projecting every
fragment's corners through the document's group, now testing it
against what is listed mapped into the group once; layout walking
every fragment of the page a second time for its extent, now
measured on the walk that makes them absolute; the box tree recording
for every element whether it holds a 3D context; a layer copying its
unchanged items, and the frame's list, every frame; and every tile
drawing every item of the list. Under callgrind those took the
appended paragraph's frames from 34% more instructions than before
M19 to 8% more.

Over T4.3's line, each written down with the item that will undo it:

- **Going back to the page from the cache**, 2.5 times as long. The
  page's first frame draws the four rows of tiles its viewport meets,
  1,024 rows for 800, finding the extents of the margin's content to
  draw each tile only what reaches it, and composites them, where
  before M19 the frame was drawn once. Drawing the page through until
  its tiles are drawn ahead would let what a frame shows depend on
  how much time there was to draw ahead, since a group drawn into a
  transparent tile and laid over the frame can differ at an
  anti-aliased edge by a level from one drawn on it (the compositing
  note, section 2). Tiles drawn opaque over the canvas colour,
  composited as copies, draw the same pixels either way: M35.10.
- **An appended paragraph's frame**, 11% longer by the medians, its
  paint 38% and its layout 7%; and the pseudo benchmark's appended
  paragraph's frame with no sheet, with `::before` and `::after` and
  with `::first-letter`, 11–12% longer, the others within 10%. Under
  callgrind, sixty such frames at 1,000 paragraphs run 8% more
  instructions than before M19 and miss the last-level cache 2.8 times
  as often, all but a few of the misses in the layout cache's copies
  of the page's twenty thousand fragments and their computed styles:
  a fragment is 32 bytes larger, 552, for the stacking context a
  positioned box belongs to, its parent in the document and whether
  it is in a 3D context, and its style 8, 440, and the copies now
  pass the cache. Fragments that refer to their styles, a pass
  copying only what it laid out, are M35.9. The paint walk, which
  visits every fragment of the page at each frame, tests each through
  the groups it is in and reads the stacking and containment rules of
  the effects, and lists the margin's content, 2.2 times the
  viewport's, which costs a tenth of a millisecond of it; a display
  list kept from frame to frame, the walk visiting what changed, is
  M35.4.
- **The scroll frame's paint**, 23% longer by the medians, in a frame
  57% shorter: the margin's content is listed, so that its tiles can
  be drawn ahead and the raster be a composite.

The flex chain's row moved more between rounds than between the two
builds, as at M18; its medians are 3% apart.

### Media queries and printing (M20)

**The media benchmark.** `benches/media.rs`, at 1280 by 800, in release,
two runs. The rows are a sheet of 1,000 `@media` rules, their lists in
every form the milestone takes, over the mutation benchmark's page of
1,000 paragraphs: the sheets prepared, the page's cascade with them and
without, and every list answered again and compared with the answers
the styles keep, which is what a resize or a change of the user's
preferences asks before it reuses the page's styles; that page with a
breakpoint at 1,200 pixels resized across it and back, which cascades
it again, and within it, which does not; and that page printed to PDF
on US Letter at 300 dots per inch.

| | Two runs |
| --- | --- |
| 1,000 lists answered again and compared, median of 1,000 | 14.4–19.7µs |
| The sheets prepared | 1.2–1.9ms |
| The page's cascade with the 1,000 rules, and without | 6.5–11.2ms, and 1.9–3.2ms |
| A resize across the breakpoint: a frame, median of 20 | 520–526ms |
| Its cascade and its layout | 5.8–6.0ms and 506ms |
| A resize within it: a frame, median of 20 | 513–516ms, its layout 505–509ms, no cascade |
| A print of the page: its 53 pages laid out, paginated and painted | 523–561ms |
| Its pagination alone | 2.4–2.7ms |
| Its rasters, each compressed as it is drawn | 5.39–5.40s, 102ms a page |
| The PDF written from them | 4.2ms |
| The whole print | 5.88–5.92s |
| The file | 21.8 MiB, 421 KiB a page |
| The process's peak memory | 130 MB |

A breakpoint costs a resize the cascade, 6ms of a frame whose layout at
the new width, 506ms, is the resize's own and M35.8's (the first layout
of a large page, below): evaluating the page's lists is a few hundredths
of a millisecond. Of a page of the print, three quarters of the
instructions are the compression of its raster, 21 MiB of RGB at the
default level, its raster 6% and its layout 6%; a PDF of text and paths
rather than rasters (the media note, section 2) would make a page a few
kilobytes and the compression go.

What the benchmark found, and mended before these runs: the print drew
each page through the document's scroll group, a layer on the screen
(M19), into the layer's tiles at a dot a pixel, resampling them to 300
dots per inch — its text blurred, and 80% of its time a tile lookup a
dot — so that the 53 pages took 33.5s and made a file of 77.3 MiB; a
page's groups are now drawn through. And the print held every page's raw
raster, 21 MiB, until the file was written, 1.28 GB at its peak for the
53; a page's raster is now compressed when it is drawn.

**T4.3.** The benchmarks above, the code before M20 (`8edc19d`) and
M20's built side by side in release, four rounds of each in turn. The
ranges are over the rounds. The machine is faster than M19's: the same
rows are shorter in both columns than in M19's table, which is why the
two tables are not to be read against each other.

| | Before M20 | M20 |
| --- | --- | --- |
| Scrolling: the layout, once | 259–276ms, median 263 | 256–266ms, median 264 |
| Scrolling: a scroll frame, median | 0.83–0.93ms, median 0.86 | 0.83–0.91ms, median 0.84 |
| Its raster, median | 0.46–0.50ms, median 0.48 | 0.47–0.49ms, median 0.48 |
| Its paint, median | 0.34–0.39ms, median 0.38 | 0.35–0.37ms, median 0.36 |
| Changing the document: the first frame | 518–575ms | 506–545ms |
| An appended paragraph's frame, median | 9.0–10.1ms, median 9.8 | 9.8–9.9ms, median 9.8 |
| Its layout, median | 7.2–8.2ms, median 7.8 | 7.8–8.0ms, median 7.9 |
| Its paint, median | 1.51–1.61ms, median 1.57 | 1.55–1.59ms, median 1.55 |
| Those frames over 16.7ms | 6–13 of 1,000 | 1–13 of 1,000 |
| 1,000 appended in one frame | 520–588ms, median 536 | 546–571ms, median 567 |
| Loading: `load` to the first frame drawn | 558–595ms | 542–577ms |
| Going back to the page from the cache | 12.2–13.7ms, median 13.0 | 12.3–12.5ms, median 12.4 |
| Going back to the page fetched again | 550–585ms | 549–584ms |
| Matching: 1,000 selectors over 10,003 elements | 245–274ms, median 251 | 240–281ms, median 252 |
| Flex: a grid of 1,000 cards, the first frame | 1.76–1.93s | 1.79–1.90s |
| Flex: after a card's title changes, a frame, median | 1.81–1.83s | 1.78–1.84s |
| Flex: a chain of 64 nested containers, its layout | 14.4–15.5ms, median 15.0 | 15.2–16.2ms, median 15.8 |
| Grid: a gallery of 1,000 figures, the first frame | 646–713ms, median 669 | 664–735ms, median 710 |
| Grid: after a caption changes, a frame, median | 636–669ms | 646–665ms |
| Grid: a dashboard of 200 cards, the first frame | 233–247ms | 220–244ms |
| Grid: a chain of 64 nested grids, its layout | 11.3–11.9ms | 10.6–13.8ms |
| Compositing: a scroll frame, median | 1.19–1.46ms | 1.21–1.29ms |
| Compositing: a card turned, a frame, median | 7.92–8.29ms | 7.85–8.37ms |
| Compositing: a card moved, a frame, median | 6.25–6.94ms | 6.34–6.96ms |
| Compositing: 200 faded cards, the first frame | 77.6–82.6ms | 74.8–82.5ms |
| Compositing: a scroll frame of them, median | 1.04–1.07ms | 1.01–1.06ms |

The pseudo benchmark, the same four rounds:

| Sheet | The first frame, before | M20 | An appended paragraph's frame, median, before | M20 |
| --- | --- | --- | --- | --- |
| None | 1.55–1.63s | 1.53–1.67s | 17.4–18.7ms, median 17.7 | 16.5–18.6ms, median 17.7 |
| `::before` and `::after` | 1.54–1.72s | 1.56–1.62s | 21.8–25.5ms, median 22.9 | 19.9–24.8ms, median 23.0 |
| `::first-letter` | 1.55–1.67s | 1.51–1.65s | 19.6–22.2ms, median 21.7 | 18.6–23.3ms, median 21.1 |
| `::first-line`, colour and size | 1.79–1.95s | 1.86–1.89s | 23.5–25.9ms, median 25.2 | 22.7–26.8ms, median 25.1 |
| `::first-line`, a background | 1.50–1.58s | 1.50–1.58s | 19.3–21.5ms, median 20.5 | 18.8–22.0ms, median 21.7 |
| All of them | 1.89–2.05s | 1.92–2.02s | 41.5–46.8ms, median 44.8 | 40.0–46.7ms, median 44.4 |

All are inside T4.3's 10% by their medians. The rows nearest the line
are 1,000 paragraphs appended in one frame and the gallery's first
frame, each 6% longer by the medians, the ranges of the two columns
overlapping; of the stages, the gallery's first paint is 13% longer,
1.40ms against 1.24, a sixth of a millisecond, and a card's paint in
the compositing benchmark a hundredth. The compositing benchmark's GPU
rows on llvmpipe, recorded in M19's table, moved as much between rounds
as between the two builds.

### Over budget, and why

These are known and recorded rather than hidden. Each has the item that
is to take it on.

- **A selector that fails at its leftmost compound, after descendant
  combinators.** The matcher keeps what it has failed on for one
  element's match only, so every element tries each pair of its
  ancestors: the square of the tree's depth, for every element. M17's
  fuzz runs found a 3,118-byte input, unknown elements nested hundreds
  deep and one such rule, whose cascade takes 0.9s in release (0.8s
  before M17). M35.2.
- **A large flex layout, and a change inside one.** A grid of 1,000
  cards takes 2.5s to lay out, nearly all of it text measurement
  (M35.8). A change anywhere inside a flex container lays all its items
  out again, since they are kept only with the container (the flexbox
  note, "The layout cache"), so a change to one card costs the grid's
  whole layout.
- **A large grid layout, and a change inside one.** A gallery of 1,000
  figures takes 0.86s to lay out, text measurement again, and a change
  to one caption costs the gallery's whole layout, since a grid's items
  are kept only with the container, as a flex container's are (the grid
  note, "The layout cache"). M35.8 and M35.9.
- **The first layout of a large page.** 490ms for the 1,000 paragraphs
  M13.2 was measured on, 630–680ms for the mutation benchmark's page,
  and 690–700ms for the load benchmark's, with its sheets. Most of it is text shaping, at about 25µs a word. Caching word
  measures and fonts' faces is M35.8. Laying out 1,000 new paragraphs
  at once costs the same, for the same reason.
- **Work that grows faster than the page.** The box tree, rebuilt on
  every layout pass, takes 1.1, 2.4, 5.2, 18.7 and 80ms at 1,000,
  2,000, 4,000, 8,000 and 16,000 paragraphs. Its boxes, fragments and
  computed styles are large: a `LayoutBox` is 448 bytes, a `Fragment`
  464 and a `ComputedStyle` 376, each 40 or 48 more since M16. M35.9.
- **A layout pass copies every fragment.** Fragments hold absolute
  coordinates and copies of their styles, so even a pass that reuses
  every box copies them all into the new layout: 58ms of the 90ms
  above. Fragments that refer to styles, and relative coordinates, are
  M35.9's.
- **An appended paragraph's frame.** The work above that grows with the
  page, the box tree and the fragments copied, takes an appended
  paragraph's frame past the budget almost half the time between 1,000
  and 2,000 paragraphs in the debug build. In release it stayed within
  it on the machine measured to M15; on M16's it is past it 115–240
  times in 1,000, the median at 12.6ms. M17's larger computed style
  takes it further: 394–442 times in 1,000, the median at 15.1–15.5ms,
  against 259–291 and 13.7–14.6ms for the code before M17 measured in
  the same rounds. M35.9 takes it on.
- **A first line's boxes cascaded again at every build.** A
  `::first-line` that sets an inherited property has every inline
  element, `::before`, `::after` and `::first-letter` on a first line's
  run cascaded again whenever the box tree is built, which is at every
  layout pass. On the pseudo benchmark's page that is 7–8ms of an
  appended paragraph's frame with links and emphases alone, and 30–35ms
  with generated boxes and first letters as well. A box tree kept and
  updated in place, M35.9, keeps those styles with it.
- **Positions counted from the end.** With `:last-child` or
  `:nth-last-child` in a stylesheet, inserting an element restyles every
  sibling before it: 20,015 elements and 42ms at 10,000 paragraphs. That
  is the conservative rule of M13.1.
