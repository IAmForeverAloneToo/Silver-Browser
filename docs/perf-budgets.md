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
  - `cargo bench -p engine --bench media` (M20).
  - `cargo bench -p engine --bench values` (M21).
  - `cargo bench -p engine --bench animation` (M22), on both backends.
  - `cargo bench -p engine --bench images` (M23), on both backends.
  - `cargo bench -p engine --bench scripts` (M24).
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
(`6cec3c1`) and M16's in turn, two runs of each, in release.

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
the code before M17 (`60f44c9`) and M17's in turn, in release. The
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
the code before M18 (`1988b4e`) and M18's built side by side in
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

**M19.8, on an integrated GPU.** 2026-10-08, three runs on another
machine, a laptop's Intel UHD Graphics (i9-11950H, Windows 10), with
the code at M22's end, where a turned card keeps its layout (M22.4).

| GPU, Intel UHD | A frame, median | Worst | Over 16.7ms |
| --- | --- | --- | --- |
| Scrolling | 4.6-6.1ms | 8.9-17.1ms | 0, 0, 1 of 120 |
| A card turned | 3.8-5.4ms | 19.3-29.6ms | 1 of 120 |
| The same, `will-change: transform` | 3.2-5.0ms | 15.3-24.3ms | 0-1 of 120 |
| A card moved | 3.4-5.3ms | 16.0-22.3ms | 0-1 of 120 |
| 200 faded cards, a scroll frame | 3.2-4.8ms | 28.0-51.0ms | 1 of 120 |

A card's frame over the budget is the one that lays it out and draws
its layer; the scroll's, in one run, a raster of 14.8ms against a
median of 3.2ms, no tile drawn. M19.8 is met but for that one frame.

**T4.3.** The benchmarks above, run on the same kind of machine as
M18's: the code before M19 (`d6f47ed`) and M19's built side by side
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

**T4.3.** The benchmarks above, the code before M20 (`edc17df`) and
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

### Custom properties, `calc()`, counters and generated content (M21)

**The values benchmark.** `benches/values.rs`, at 1280 by 800, in
release, two runs, each stage the median of ten: the mutation
benchmark's page cascaded with its colours, sizes and spacing in custom
properties on `:root` read by `var()`, against the values written out;
custom properties at the substitution bounds, with the memory a page
built to exhaust the cascade's bound takes; 1,000 boxes sized by math
functions of percentages and lengths against the lengths they come to;
an ordered list of 1,000 items, each holding a list of two numbered by
`counters()`, against the same elements as blocks; and 1,000 paragraphs
of six lines or so, justified, against the same left-aligned.

| | Two runs |
| --- | --- |
| The cascade of 1,000 paragraphs, the values written out | 3.96–4.06ms |
| The same under a theme of 8 custom properties | 4.33–4.46ms |
| A theme of 200, the rules reading 8 | 4.39–4.67ms |
| That, every paragraph declaring one of its own | 10.6–12.4ms |
| Thirty properties each doubling the last | 1.79–2.67ms, the fourteenth the last with a value, of 16,383 tokens |
| A chain of 10,000 each naming the next | 10.3–17.2ms |
| The same closed into a cycle | 4.8–8.1ms |
| 1,000 elements each declaring the doubling chain, cascaded once | 353–437ms, the chain whole in 135 of them |
| The memory that cascade takes at its peak | 225 MiB, over 25 MiB resident before |
| 1,000 boxes sized by lengths: cascade, box tree, layout | 3.4–3.8, 0.23–0.32 and 0.44–0.50ms |
| Sized by math functions | 7.8–8.0, 0.28–0.31 and 0.93–0.96ms |
| An ordered list of 1,000 items of two, as blocks | 14.9–18.7, 8.5–9.2 and 141–148ms |
| Numbered by counters | 21.0–26.6, 20.7–25.3 and 195–215ms |
| 1,000 paragraphs of six lines, left-aligned: their layout | 2.22–2.25s |
| Justified | 2.20–2.29s |
| Justified by `inter-character` | 5.49–5.89s |

A theme read by `var()` costs the cascade a tenth more than the values
written out, a theme of 200 properties no more than one of 8, the
paragraphs sharing the root's map; a property declared on every
paragraph, which copies the map on write and computes it for each,
triples it. The page built to exhaust `MAX_CASCADE_SUBSTITUTION` meets
it after 135 of its 1,000 elements, as the bound means it to, in under
half a second and 225 MiB. Math functions double the cascade of the
boxes they size and their layout, which stays under a millisecond. A
list's markers and counters cost its box tree two and a half times and
its layout two fifths more, the markers' lines. Justification by word
costs nothing measurable; by character it lays a fragment a character
out, each measured alone, as letter spacing does: 2.5 times as long,
M35.8's (text measurement off the hot path).

What the benchmark's step found, and mended before these runs: a math
function's infinite or NaN percentage, and a written one as large as an
`f32` holds, resolved to an infinite or NaN length, now censored where
every percentage is resolved; and, under T4.3, the box tree doing the
milestone's work for every document at every layout — the walk of the
counters and generated text run whether or not anything generates
text, the decorations and the generated texts kept in SipHash maps
grown from empty, and the fingerprint of a box's generated text
formatting its decorations' printed form for every decorated text,
every link's. The walk now runs only when a generated text is asked
for, the two maps are tables by node, and the decorations are hashed by
their fields: under callgrind, twenty appended paragraphs' frames at
1,000 paragraphs ran 804 million instructions, and 758 million now,
against 596 million before M21.

**T4.3.** The benchmarks above, the code before M21 (`4e93f37`) and
M21's built side by side in release, four rounds of each in turn. The
ranges are over the rounds. The machine is slower than M20's, which is
why the two tables are not to be read against each other.

| | Before M21 | M21 |
| --- | --- | --- |
| Scrolling: the layout, once | 366–479ms, median 396 | 397–461ms, median 410 |
| Scrolling: a scroll frame, median | 0.96–1.02ms, median 0.98 | 1.00–1.11ms, median 1.09 |
| Its raster, median | 0.57–0.60ms, median 0.59 | 0.59–0.62ms, median 0.60 |
| Its paint, median | 0.37–0.39ms, median 0.37 | 0.42–0.44ms, median 0.43 |
| Changing the document: the first frame | 740–845ms, median 825 | 769–890ms, median 829 |
| An appended paragraph's frame, median | 10.5–12.3ms, median 11.2 | 13.5–14.2ms, median 13.9 |
| Its layout, median | 8.8–10.3ms, median 9.3 | 11.6–12.1ms, median 11.9 |
| Its paint, median | 1.30–1.43ms, median 1.37 | 1.40–1.49ms, median 1.46 |
| Those frames over 16.7ms | 16–90 of 1,000 | 256–384 of 1,000 |
| 1,000 appended in one frame | 745–856ms, median 774 | 790–923ms, median 840 |
| Loading: `load` to the first frame drawn | 831–870ms, median 856 | 848–902ms, median 869 |
| Going back to the page from the cache | 15.1–19.2ms, median 16.6 | 15.6–17.0ms, median 16.4 |
| Going back to the page fetched again | 798–891ms, median 874 | 842–895ms, median 872 |
| Matching: 1,000 selectors over 10,003 elements | 308–428ms, median 323 | 296–337ms, median 307 |
| Flex: a grid of 1,000 cards, the first frame | 2.57–2.89s, median 2.76 | 2.68–2.93s, median 2.88 |
| Flex: after a card's title changes, a frame, median | 2.63–2.76s, median 2.64 | 2.72–2.82s, median 2.79 |
| Flex: a chain of 64 nested containers, its layout | 20.5–21.6ms, median 21.2 | 22.6–37.4ms, median 24.6 |
| Grid: a gallery of 1,000 figures, the first frame | 1017–1105ms, median 1032 | 982–1107ms, median 1038 |
| Grid: after a caption changes, a frame, median | 967–1003ms, median 983 | 976–1024ms, median 988 |
| Grid: a dashboard of 200 cards, the first frame | 303–354ms, median 326 | 313–340ms, median 336 |
| Grid: a chain of 64 nested grids, its layout | 15.4–19.4ms, median 16.2 | 18.4–22.8ms, median 19.4 |
| Compositing: a scroll frame, median | 1.30–1.43ms, median 1.36 | 1.40–2.07ms, median 1.44 |
| Compositing: a card turned, a frame, median | 11.8–13.3ms, median 12.2 | 11.4–13.3ms, median 12.1 |
| Compositing: a card moved, a frame, median | 9.56–9.86ms, median 9.73 | 9.2–11.0ms, median 9.8 |
| Compositing: 200 faded cards, the first frame | 110–142ms, median 121 | 105–184ms, median 108 |
| Compositing: a scroll frame of them, median | 1.27–1.38ms, median 1.32 | 1.28–1.53ms, median 1.29 |
| Media: 1,000 lists answered again, median of 1,000 | 21.0–27.9µs, median 21.1 | 24.3–24.5µs, median 24.3 |
| Media: the page's cascade with the 1,000 rules | 9.0–11.1ms, median 9.9 | 10.4–23.0ms, median 11.9 |
| Media: a resize across the breakpoint, a frame | 779–819ms, median 799 | 769–794ms, median 783 |
| Media: a print, its layout, pagination and paint | 787–830ms, median 824 | 764–887ms, median 796 |
| Its pagination alone | 3.04–3.42ms, median 3.33 | 3.40–4.40ms, median 3.88 |
| The whole print | 8.08–8.84s, median 8.39 | 8.05–8.45s, median 8.19 |
| The process's peak memory | 129–130 MB | 145–147 MB |

The pseudo benchmark, the same four rounds:

| Sheet | The first frame, before | M21 | An appended paragraph's frame, median, before | M21 |
| --- | --- | --- | --- | --- |
| None | 2.37–2.65s | 2.35–2.62s | 19.5–22.1ms, median 21.7 | 30.2–32.0ms, median 30.7 |
| `::before` and `::after` | 2.39–2.53s | 2.33–2.61s | 25.6–29.3ms, median 27.0 | 37.0–42.7ms, median 39.6 |
| `::first-letter` | 2.33–2.45s | 2.26–2.41s | 23.2–25.8ms, median 24.8 | 32.6–36.9ms, median 34.6 |
| `::first-line`, colour and size | 2.69–2.94s | 2.67–3.04s | 26.7–29.8ms, median 28.4 | 42.4–44.9ms, median 43.3 |
| `::first-line`, a background | 2.25–2.34s | 2.29–2.39s | 23.1–24.2ms, median 23.7 | 34.2–40.0ms, median 35.4 |
| All of them | 2.90–3.42s | 3.08–3.29s | 52.9–58.1ms, median 54.3 | 76.5–82.7ms, median 77.6 |

Within T4.3's 10% by their medians: every first frame, loading and
going back, matching, the flex grid's frames, the grid's, the
compositing benchmark's and a resize. Over it, written down here with
the items that will undo them:

- **An appended paragraph's frame**, 25% longer by the medians, its
  layout 27%, and 256–384 of 1,000 over the budget against 16–90; the
  pseudo benchmark's appended paragraphs' frames, 40–53%; the chains of
  64 nested flex containers and grids, 16% and 20%; the scroll frame,
  11%, its paint 18%; a print's pagination alone, 17%; and the print's
  peak memory, 13%. Under callgrind, twenty appended paragraphs' frames
  run 27% more instructions than before M21 (758 million against 596),
  and the pseudo page's, with its links, 33% more; nearly all of it is
  the style's size. `ComputedStyle` is 648 bytes where it was 448, and
  its 22 fields that may now hold a `calc()` expression make its copy a
  field at a time where it was a copy of bytes: 363 instructions a copy
  where it was 142, besides the bytes moved, over the 482,424 copies of
  the twenty frames, as the values note chose and left to T4.3 to
  price.
  The layout cache copies every fragment it gives back, 19,000 a frame
  on the mutation page and 35,000 on the pseudo page, and each fragment
  its style. Fragments that refer to their styles, the pass copying
  only what it laid out, are M35.9's.
- **The cascade of the media benchmark's page with its 1,000 rules**,
  20% longer, and the pseudo page's `::first-line` cascading its
  paragraphs' inline elements again at every build: the user agent
  sheet has twelve more rules, 60 where it had 48, for HTML's lists and
  decorated phrasing elements, and every rule is tried against every
  element. Rule hash bucketing is M35.2's.
- **1,000 media lists answered again**, 15% longer by the medians, 3µs,
  the ranges of the two columns overlapping.

### Transitions and animations (M22)

**The acceptance.** `benches/animation.rs` draws the milestone's two
demos (`tests/demos/`) at 1280 by 800 for ten seconds of frames sixty a
second through `Engine::frame_at`, the time a frame leaves before its
deadline spent drawing ahead, and records every frame: a spinner, a
ring turned by an infinite animation of `rotate`, and a colour fade, a
card alternating between two themes. Three runs, in release, on the
code as M22 left it:

| 600 frames | The software backend | The GPU backend, llvmpipe |
| --- | --- | --- |
| The spinner, a frame's median | 0.69–0.71ms | 6.39–6.97ms |
| Its worst frame | 1.38–5.49ms | 30.9–56.5ms |
| Its frames over 16.7ms, over 20ms | none | 1–2, 1–2 |
| Its layouts | none | none |
| The colour fade, a frame's median | 1.70–1.76ms | 8.49–9.05ms |
| Its worst frame | 3.63–6.06ms | 15.6–25.6ms |
| Its frames over 16.7ms, over 20ms | none | 0–2, 0–1 |
| Its boxes laid out a frame | 7 | 7 |

On the software backend both demos keep sixty frames a second with
room to spare, no frame of either near the budget. The spinner's frames
lay nothing out and composite its layer; the fade's lay out the card,
the three boxes inside it and the three around it, and take the rest
from the layout cache. This machine's GPU backend is llvmpipe, which
rasterizes on the same cores: its medians keep the budget, and a single
frame now and then takes 20 to 130ms in its raster, at no frame in
particular. A logger of every slow frame, built from the code before
step 12 and from the code after it and run by turns six times each,
found such frames in both, 57ms the worst before and 26ms after: the
machine, not the change. On a real GPU the demos are unmeasured, there
being none here (M19.8).

**The costs.** The same benchmark, on the software backend, two
seconds of frames each, three runs:

| | Three runs |
| --- | --- |
| A transform animated on one of 1,000 paragraphs, a frame's median | 2.65–3.41ms, no layout |
| A colour animated there | 8.50–9.58ms, of it layout 6.4–7.2ms, three boxes a frame |
| An opacity animated on every paragraph | 23.5–26.8ms, of it style 15.5–17.2ms and raster 6.85–7.65ms |
| The body's class changed under 1,000 paragraphs, the frame of the change | 824–864ms, of it layout 798–837ms |
| The same with `transition: all` on every paragraph | 19.9–21.2ms, of it style 18.2–19.6ms and no layout |
| `composite.rs`'s turned card, a frame | 2.30–2.88ms, no layout (10.8ms before M22) |

A transform or an opacity animated is a composited change: its frames
restyle the element and lay nothing out. A colour lays out the
paragraph it is on and the two boxes around it, the paint-only change
being M35.9's. A thousand animations at once keep no budget: each
frame restyles 1,000 elements, their selectors matched again (M35.2),
and draws their groups. A class change that starts a transition on
every paragraph costs its cascade twice what it costs without, the
before-change and after-change styles of every element, and lays
nothing out at its frame, every paragraph showing its old value; the
frames after lay out the paragraphs as their paddings move.

What the step found, and mended before these runs: `layout::composite`
walked the layout and the cache once for each change, a thousand
opacities taking 406ms a frame, now one walk for all of a restyle's
changes, 16ms; the restyle counted an element changed when only the
after-change style the cascade keeps beside its style changed, so that
the frame starting a transition laid the page out though nothing on it
moved; and, under T4.3, three costs every page paid. The previous
frame's layout was freed in the next frame's style stage, where it
waited for the restyle to say whether a composited change kept it,
7 million instructions a frame of the mutation benchmark that were
paid between frames before: a change layout reads whatever the styles
drops it when it is made again. Every computed style built a default
`Motion`, thirteen lists, to compare itself with, and the older grid
properties theirs; the initial values are now made only for a style
that holds a copy. And each element's unstyled pseudo-elements hashed
their targets to look them up in the empty tables of animations and
transitions, which are now not asked. Under callgrind the cascade of
the flex benchmark's grid did 23% more work than before M22 and does 4%
more, the copies of the larger style; twenty appended paragraphs'
frames 0.3%.

**T4.3.** The benchmarks above, the code before M22 (`0f1ce3a`) and
M22's built side by side in release, four rounds of each in turn. The
ranges are over the rounds.

| | Before M22 | M22 |
| --- | --- | --- |
| Scrolling: the layout, once | 342–416ms, median 364 | 330–488ms, median 418 |
| Scrolling: a scroll frame, median | 0.98–1.32ms, median 1.00 | 0.89–1.30ms, median 1.14 |
| Its raster, median | 0.54–0.72ms, median 0.56 | 0.51–0.73ms, median 0.63 |
| Its paint, median | 0.40–0.57ms, median 0.41 | 0.37–0.58ms, median 0.48 |
| Changing the document: the first frame | 682–733ms, median 697 | 747–878ms, median 781 |
| An appended paragraph's frame, median | 12.2–14.5ms, median 12.6 | 12.7–14.4ms, median 13.1 |
| Its layout, median | 10.3–12.4ms, median 10.7 | 10.8–12.5ms, median 11.3 |
| Its paint, median | 1.33–1.50ms, median 1.39 | 1.31–1.40ms, median 1.38 |
| Those frames over 16.7ms | 103–446 of 1,000 | 58–299 of 1,000 |
| 1,000 appended in one frame | 742–951ms, median 780 | 695–820ms, median 771 |
| Loading: `load` to the first frame drawn | 750–835ms, median 778 | 752–882ms, median 814 |
| Going back to the page from the cache | 14.9–15.6ms, median 15.4 | 14.3–15.7ms, median 15.1 |
| Going back to the page fetched again | 808–872ms, median 836 | 748–842ms, median 796 |
| Matching: 1,000 selectors over 10,003 elements | 260–320ms, median 289 | 273–330ms, median 286 |
| Flex: a grid of 1,000 cards, the first frame | 2.48–2.79s, median 2.74 | 2.31–2.68s, median 2.57 |
| Flex: after a card's title changes, a frame, median | 2.45–2.66s, median 2.51 | 2.46–2.55s, median 2.50 |
| Flex: a chain of 64 nested containers, its layout | 20.5–27.1ms, median 21.8 | 20.6–24.0ms, median 21.9 |
| Grid: a gallery of 1,000 figures, the first frame | 934–1241ms, median 1025 | 856–1093ms, median 941 |
| Grid: after a caption changes, a frame, median | 883–933ms, median 909 | 864–942ms, median 924 |
| Grid: a dashboard of 200 cards, the first frame | 276–319ms, median 298 | 274–317ms, median 284 |
| Grid: a chain of 64 nested grids, its layout | 13.3–18.4ms, median 16.5 | 14.8–19.5ms, median 17.2 |
| Compositing: a scroll frame, median | 1.28–1.53ms, median 1.50 | 1.30–1.40ms, median 1.31 |
| Compositing: a card turned, a frame, median | 10.7–12.0ms, median 10.8 | 2.29–3.86ms, median 2.40 |
| Compositing: a card moved, a frame, median | 8.30–9.26ms, median 8.92 | 0.35–0.62ms, median 0.36 |
| Compositing: 200 faded cards, the first frame | 97–132ms, median 98 | 100–147ms, median 118 |
| Compositing: a scroll frame of them, median | 1.15–1.24ms, median 1.21 | 1.24–1.65ms, median 1.38 |
| Media: 1,000 lists answered again, median of 1,000 | 23.1–30.9µs, median 27.1 | 21.1–22.5µs, median 21.8 |
| Media: the page's cascade with the 1,000 rules | 8.9–17.1ms, median 12.8 | 8.4–10.3ms, median 9.8 |
| Media: a resize across the breakpoint, a frame | 723–763ms, median 736 | 704–760ms, median 743 |
| Media: a print, its layout, pagination and paint | 721–850ms, median 754 | 671–802ms, median 733 |
| Its pagination alone | 3.33–3.79ms, median 3.59 | 3.15–3.86ms, median 3.37 |
| The whole print | 7.63–8.01s, median 7.93 | 7.49–7.81s, median 7.77 |
| The process's peak memory | 145–146 MB | 146–147 MB |
| Values: the cascade of 1,000 paragraphs, written out | 3.11–4.98ms, median 4.23 | 3.51–5.82ms, median 3.54 |
| Under a theme of 8 custom properties | 3.70–4.43ms, median 4.30 | 3.84–5.63ms, median 4.66 |
| A theme of 200, the rules reading 8 | 4.00–6.65ms, median 4.46 | 4.06–7.01ms, median 4.56 |
| That, every paragraph declaring one of its own | 10.4–15.5ms, median 10.9 | 9.3–16.7ms, median 12.1 |
| 1,000 elements each declaring the doubling chain | 290–369ms, median 353 | 290–309ms, median 297 |
| 1,000 boxes sized by lengths: cascade | 3.13–3.51ms, median 3.25 | 3.15–4.68ms, median 3.38 |
| Sized by math functions: cascade | 7.1–11.7ms, median 7.6 | 7.04–7.94ms, median 7.63 |
| An ordered list of 1,000 items, as blocks: cascade | 13.1–14.2ms, median 13.8 | 14.2–18.3ms, median 15.4 |
| Its box tree | 7.15–8.03ms, median 7.85 | 7.43–8.38ms, median 8.21 |
| Numbered by counters: cascade | 18.1–20.3ms, median 19.2 | 19.3–21.9ms, median 20.4 |
| Its box tree | 17.4–19.4ms, median 18.2 | 18.1–20.9ms, median 18.7 |
| 1,000 paragraphs of six lines, left-aligned: their layout | 2.02–2.08s, median 2.05 | 1.96–2.21s, median 2.09 |
| Justified | 2.00–2.18s, median 2.14 | 1.96–2.04s, median 2.00 |

The pseudo benchmark, the same four rounds:

| Sheet | The first frame, before | M22 | An appended paragraph's frame, median, before | M22 |
| --- | --- | --- | --- | --- |
| None | 2.01–2.41s | 2.03–2.37s | 24.7–36.1ms, median 27.7 | 26.1–29.3ms, median 26.9 |
| `::before` and `::after` | 2.12–2.62s | 2.12–2.71s | 33.6–36.4ms, median 34.5 | 34.2–41.1ms, median 36.6 |
| `::first-letter` | 1.92–2.37s | 2.12–2.25s | 30.8–33.6ms, median 31.8 | 32.2–35.6ms, median 34.2 |
| `::first-line`, colour and size | 2.43–2.94s | 2.42–3.07s | 37.0–44.9ms, median 38.8 | 39.1–41.2ms, median 39.8 |
| `::first-line`, a background | 2.04–2.24s | 2.09–2.32s | 30.8–38.4ms, median 31.6 | 29.5–40.0ms, median 32.0 |
| All of them | 2.78–3.23s | 2.61–3.42s | 66.7–68.8ms, median 67.0 | 67.7–75.5ms, median 69.9 |

Within T4.3's 10% by their medians but for the rows below, each of
which does the work it did before M22. Scrolling's layout and frames,
12–16% by the medians, have ranges that all but cover each other's. The
mutation page's first frame, 697 against 781ms, and the 200 faded
cards, whose first frame and scroll frames were before M22's in two of
the four rounds, were run again under callgrind: the first frame does
6,523.1 million instructions before M22 and 6,529.4 million now, its
scroll frames 1.1% fewer, the faded cards' first frame 0.15% more and
their scroll frames 0.1% fewer; the simulated cache misses are within
1% and the mispredicted branches 4% fewer; the page faults 1% more. The
first frame timed alone, five runs of each by turns, has medians of 749
and 753ms. Two of the values benchmark's cascades, 11% and 12% by their
medians, are within each other's ranges, and its plain cascade is 17%
shorter. The rest is the machine's variance, which moves a measure by
as much as a third from one round to the next: the faded cards' first
frame before M22 took 97ms in one round and 132ms in another.

### Images (M23)

**The acceptance.** `benches/images.rs` loads the photo grid
(`tests/demos/photo-grid.html`: 48 figures, photos in PNG, JPEG, GIF and
WebP, an animated GIF, gradients on the canvas, the header and some
figures) by its `file:` URL at 1280 by 800, draws frames until its
images are decoded, then scrolls it 40px a frame down and back up, 116
frames sixty a second through `Engine::frame_at`, the time a frame
leaves before its deadline spent drawing ahead. Three runs, in release,
on this laptop (Intel UHD Graphics for the GPU backend):

| 116 scroll frames | The software backend | The GPU backend, Intel UHD Graphics |
| --- | --- | --- |
| Loaded, the navigation to `Loaded` | 3.3-4.8ms | 2.5-3.4ms |
| The frames until its images are decoded | 2, in 68.8-77.1ms, the first 51.0-59.5ms | 2, in 61.7-64.4ms, the first 53.2-55.5ms |
| A scroll frame's median | 0.43-0.44ms | 1.64-1.68ms |
| Its 95th percentile | 2.64-2.65ms | 4.70-4.87ms |
| Its worst | 2.71-2.84ms | 6.09-6.79ms |
| Its raster, median | 0.23ms | 1.43-1.46ms |
| Frames over 16.7ms, over 20ms | none | none |
| Layouts | none | none |

Both backends keep sixty frames a second scrolling the grid, no frame
near the budget, and lay nothing out. The first frame asks for every
listed image's pixels and draws the page without them; the second,
once they are decoded on their threads, draws them.

What the benchmark found, and what was mended before these runs: the
canvas's images, the grid's repeating gradient, covered the viewport
and a viewport above and below it, so their display item changed at
every scroll and the document layer's tiles were drawn again each
frame, 37ms a frame on the software backend; they now cover the
document, the same item whatever the scroll. And the gradient renderer
took 54-67ms for a gradient the size of the viewport, which made the
grid's first frame 310ms; it now computes each column's position once,
rounds by a saturating cast, and renders a large gradient a band of
rows a thread.

**The costs.** The same benchmark, medians of their runs:

| | Three runs |
| --- | --- |
| Decoding `hills-1.jpg`, 480 by 320 | 1.27-1.30ms |
| `valley-1.png`, 480 by 320 | 1.44-1.57ms |
| `hills-4.webp`, 480 by 320, lossy | 4.62-4.71ms |
| `bands-1.gif`, 240 by 160 | 0.22-0.23ms |
| `spinner.gif`, 160 by 160, eight frames | 1.41-1.44ms |
| A 4096 by 2732 image made a copy for 256 by 171 drawn (512 by 342) | 34.6-35.1ms |
| A linear gradient rendered, 1280 by 800 | 4.35-4.77ms |
| A radial one | 4.65-4.86ms |
| A repeating linear one | 6.02-6.15ms |

The grid's first frame, 51-59ms, renders the canvas's repeating
gradient for the whole document, 1280 by 3082, and lays the page out;
the frame after it draws the decoded images. A downscaled copy is made
on a decode thread, so its 35ms never falls in a frame.

**T4.3.** The benchmarks above, the code before M23 (`e920942`) and
M23's (`1a40055`) built side by side in release, four rounds of each
in turn. The ranges are over the rounds.

| | Before M23 | M23 |
| --- | --- | --- |
| Scrolling: the layout, once | 381-421ms, median 392 | 375-439ms, median 422 |
| Scrolling: a scroll frame, median | 0.62-0.66ms, median 0.62 | 0.61-0.66ms, median 0.62 |
| Its raster, median | 0.35-0.37ms, median 0.36 | 0.35-0.38ms, median 0.36 |
| Its paint, median | 0.26-0.29ms, median 0.27 | 0.26-0.28ms, median 0.26 |
| Changing the document: the first frame | 733-797ms, median 751 | 741-811ms, median 805 |
| An appended paragraph's frame, median | 10.8-11.9ms, median 11.1 | 10.9-12.6ms, median 11.6 |
| Its layout, median | 9.8-10.7ms, median 10.0 | 9.9-11.5ms, median 10.6 |
| Its paint, median | 0.75-0.88ms, median 0.77 | 0.74-0.88ms, median 0.76 |
| Those frames over 16.7ms | 10-26 of 1,000 | 24-37 of 1,000 |
| 1,000 appended in one frame | 730-809ms, median 750 | 729-820ms, median 816 |
| Loading: `load` to the first frame drawn | 787-809ms, median 800 | 774-864ms, median 802 |
| Going back to the page from the cache | 11.4-11.7ms, median 11.5 | 11.4-11.7ms, median 11.6 |
| Going back to the page fetched again | 764-839ms, median 813 | 769-943ms, median 813 |
| Matching: 1,000 selectors over 10,003 elements | 248-258ms, median 254 | 246-256ms, median 252 |
| Flex: a grid of 1,000 cards, the first frame | 2.42-2.74s, median 2.69 | 2.68-2.70s, median 2.69 |
| Flex: after a card's title changes, a frame, median | 2.40-2.67s, median 2.65 | 2.63-2.67s, median 2.64 |
| Flex: a chain of 64 nested containers, its layout | 15.8-16.2ms, median 16.0 | 15.8-16.0ms, median 15.9 |
| Grid: a gallery of 1,000 figures, the first frame | 908-1055ms, median 917 | 902-1059ms, median 1027 |
| Grid: after a caption changes, a frame, median | 874-1031ms, median 897 | 890-1042ms, median 1006 |
| Grid: a dashboard of 200 cards, the first frame | 307-341ms, median 312 | 281-387ms, median 327 |
| Grid: a chain of 64 nested grids, its layout | 14.0-15.7ms, median 15.2 | 14.3-16.1ms, median 14.6 |
| Compositing: a scroll frame, median | 0.77-0.80ms, median 0.78 | 0.76-0.78ms, median 0.78 |
| Compositing: a card turned, a frame, median | 1.72-1.78ms, median 1.74 | 1.73-1.76ms, median 1.74 |
| Compositing: a card moved, a frame, median | 0.29-0.30ms, median 0.29 | 0.29-0.31ms, median 0.29 |
| Compositing: 200 faded cards, the first frame | 103-104ms, median 103 | 104-110ms, median 104 |
| Compositing: a scroll frame of them, median | 0.87-0.89ms, median 0.88 | 0.87-0.88ms, median 0.88 |
| Media: 1,000 lists answered again, median of 1,000 | 15.6-16.3us, median 15.8 | 15.8-16.5us, median 15.9 |
| Media: the page's cascade with the 1,000 rules | 9.53-9.95ms, median 9.86 | 9.6-10.0ms, median 9.6 |
| Media: a resize across the breakpoint, a frame | 712-717ms, median 715 | 704-729ms, median 717 |
| Media: a print, its layout, pagination and paint | 689-707ms, median 698 | 696-710ms, median 708 |
| Its pagination alone | 2.98-3.37ms, median 3.07 | 2.94-3.15ms, median 3.04 |
| The whole print | 5.26-5.32s, median 5.31 | 5.26-5.32s, median 5.28 |
| Values: the cascade of 1,000 paragraphs, written out | 4.35-4.66ms, median 4.44 | 4.66-4.81ms, median 4.71 |
| Under a theme of 8 custom properties | 5.21-5.45ms, median 5.36 | 5.37-5.59ms, median 5.51 |
| A theme of 200, the rules reading 8 | 5.45-5.66ms, median 5.54 | 5.53-5.76ms, median 5.71 |
| That, every paragraph declaring one of its own | 12.0-12.6ms, median 12.1 | 11.9-12.2ms, median 12.0 |
| 1,000 elements each declaring the doubling chain | 255-264ms, median 258 | 248-264ms, median 256 |
| 1,000 boxes sized by lengths: cascade | 4.30-4.33ms, median 4.31 | 4.29-4.53ms, median 4.43 |
| Sized by math functions: cascade | 10.8-10.9ms, median 10.9 | 10.9-11.2ms, median 11.0 |
| An ordered list of 1,000 items, as blocks: cascade | 14.8-15.2ms, median 15.0 | 15.1-15.2ms, median 15.1 |
| Its box tree | 8.32-8.54ms, median 8.50 | 8.94-9.09ms, median 9.00 |
| Numbered by counters: cascade | 21.8-22.3ms, median 22.0 | 22.5-22.7ms, median 22.6 |
| Its box tree | 16.9-17.4ms, median 17.2 | 17.8-18.1ms, median 18.0 |
| 1,000 paragraphs of six lines, left-aligned: their layout | 2.26-2.30s, median 2.29 | 2.20-2.43s, median 2.27 |
| Justified | 2.25-2.31s, median 2.28 | 2.20-2.44s, median 2.27 |

The pseudo benchmark, the same four rounds:

| Sheet | The first frame, before | M23 | An appended paragraph's frame, median, before | M23 |
| --- | --- | --- | --- | --- |
| no pseudo-elements | 2.15-2.38s | 2.19-2.41s | 19.3-20.4ms, median 20.3 | 18.4-20.5ms, median 18.6 |
| ::before and ::after | 2.25-2.45s | 2.24-2.51s | 23.8-25.5ms, median 24.6 | 23.6-25.5ms, median 23.9 |
| ::first-letter | 2.34-2.43s | 2.39-2.44s | 20.6-22.2ms, median 21.0 | 20.8-22.5ms, median 21.1 |
| ::first-line, inherited properties | 2.74-2.86s | 2.79-2.94s | 26.6-28.9ms, median 27.3 | 27.4-29.8ms, median 27.6 |
| ::first-line, a background | 2.34-2.38s | 2.34-2.49s | 22.1-24.1ms, median 22.5 | 21.6-23.1ms, median 21.9 |
| all of them | 2.95-3.02s | 3.05-3.18s | 44.7-49.5ms, median 45.0 | 45.8-46.5ms, median 45.9 |

The animation benchmark, the same four rounds, a frame's median:

| | Before M23 | M23 |
| --- | --- | --- |
| The spinner, software | 0.50-0.51ms, median 0.51 | 0.50-0.52ms, median 0.51 |
| The colour fade, software | 1.56-1.75ms, median 1.67 | 1.70-1.78ms, median 1.71 |
| The spinner, GPU | 1.69-1.75ms, median 1.70 | 1.69-1.76ms, median 1.71 |
| The colour fade, GPU | 2.90-3.04ms, median 2.91 | 2.88-3.02ms, median 2.90 |
| A transform on one of 1,000 paragraphs | 1.50-1.53ms, median 1.51 | 1.50-1.55ms, median 1.53 |
| A colour there | 13.62-13.83ms, median 13.68 | 13.91-14.47ms, median 14.20 |
| An opacity on every paragraph | 20.32-20.52ms, median 20.41 | 20.23-20.87ms, median 20.58 |

Within T4.3's 10% by their medians but for the gallery's two rows of
the grid benchmark, 12% by the medians of four rounds, whose ranges
cover each other's. Its layouts fall at about 890ms or about 1,020ms a
round, whichever tree runs (the code before M23 took 1,037ms in one
round and M23's 884ms in another). Six more rounds of each by turns
give medians of 983ms before and 955ms after for the first frame, and
1,059ms before and 1,043ms after for a caption's frame: the machine,
not the change. The mutation page's appended paragraphs went over the
budget 24-37 times in 1,000 against 10-26, their median frame 4%
longer, frames close to the budget either way. The process's peak
memory is not measured on Windows.

### Scripts (M24)

**What scripts cost.** `benches/scripts.rs`: a realm made, as a
document's first script makes it; two million additions in one script
and a thousand console calls, each run in one realm twenty times; and a
page of 1,000 paragraphs, 71KB, with an inline script after every tenth
paragraph and an external one after every hundredth, served by `net`'s
test server, from `Engine::navigate` to its first frame at 1280 by 800,
streamed and parsed as it arrives either way. In release, on this
laptop:

| | Median | Range |
| --- | --- | --- |
| A realm made | 0.12ms | 0.11-0.49ms |
| Two million additions | 83.1ms | 79.7-87.2ms |
| A thousand console calls | 1.15ms | 1.10-1.17ms |
| The page, scripts off | 492ms | 465-497ms |
| The page, its 110 scripts run | 494ms | 490-497ms |

A page's scripts cost what they do: 110 of them, ten fetched, load
within the page's own spread without them, the first layout's text
shaping most of either, as on the load benchmark's page.

**T4.3.** The benchmarks of the code before M24 (dc3d768) and of M24, by
turns, four rounds: every benchmark within 10% but four cascades of the
values benchmark; the rows past 5%:

| | Before M24 | M24 | |
| --- | --- | --- | --- |
| Loading: `load` to the first frame drawn | 807-897ms, median 842 | 769-835ms, median 778 | x0.92 |
| Going back to the page fetched again | 778-933ms, median 875 | 750-837ms, median 761 | x0.87 |
| Flex: a grid of 1,000 cards, the first frame | 2.71-2.76s, median 2.73 | 2.43-2.51s, median 2.46 | x0.90 |
| Grid: a gallery of 1,000 figures, the first frame | 1,029-1,060ms, median 1,043 | 914-924ms, median 915 | x0.88 |
| Values: the cascade of 1,000 paragraphs, written out | 4.74-5.37ms, median 4.99 | 5.95-6.31ms, median 6.20 | x1.24 |
| 1,000 boxes sized by lengths: cascade | 4.53-4.68ms, median 4.56 | 5.77-5.84ms, median 5.79 | x1.27 |
| Sized by math functions: cascade | 10.0-11.3ms, median 11.2 | 12.3-12.6ms, median 12.5 | x1.12 |
| An ordered list of 1,000 items, as blocks: cascade | 15.3-17.3ms, median 15.5 | 19.6-20.0ms, median 19.8 | x1.28 |
| The colour fade on the GPU, a frame's median | 2.85-2.94ms, median 2.92 | 2.92-3.39ms, median 3.17 | x1.08 |
| The photo grid on the GPU, its frames until decoded | 63.1-70.9ms, median 67.9 | 67.9-75.6ms, median 71.4 | x1.05 |

The four cascades are not the change's. Without the UA sheet's new
`noscript` rule they are back at M23's times, but that rule is never in
force on those pages (scripts are off), the rules in force and the work
for each element are the same with it and without, and builds that
leave the same rules in force are fast or slow by the sheet's contents
alone: the rule moved to the sheet's end, after an empty rule, with
another property or another tag, 5.8-6.1ms for the boxes sized by
lengths; the rule without `!important`, or an unrelated `!important`
rule in `@media print` instead, 4.5-4.8ms. Where the cascade's memory
falls decides it; why the fast layout is a quarter faster is for a
change of its own. The streamed parse leaves the loads faster, if
anything. The process's peak memory is not measured on Windows.

### DOM scripting (M25)

**A script's changes, a frame after each.** `benches/dom.rs`: a page
served by `net`'s test server, scripts on, whose links run its
functions as a click does (`javascript:` URLs): one builds a table of
rows of four cells, one toggles a class on it that changes every cell's
padding and colour, one edits the text of a cell in its middle row.
Each is followed by a frame at 1280 by 800; the build five times, each
change forty. In release, on this laptop, the frames' medians:

| | 100 rows | 1,000 rows |
| --- | --- | --- |
| The table built: the script | 7.5ms | 86-98ms |
| The frame after it | 22.5ms | 117-131ms |
| Its style, layout and raster | 3.9, 7.1 and 11.2ms | 32-34, 73-84 and 9-11ms |
| The class toggled: the script | 0.4ms | 2.3ms |
| The frame after it | 20.5ms, 40 of 40 over | 105-116ms, 40 of 40 over |
| Its style, layout and raster | 4.0, 6.2 and 10.0ms | 36-43, 58 and 8-9ms |
| A cell's text edited: the script | 0.4ms | 4-5ms |
| The frame after it | 4.2ms, 0 of 40 over | 46-58ms, 40 of 40 over |
| Its layout | 3.5ms | 41-51ms |

The incremental engine does what M13 built it to: no change cascades
the document whole, the toggle restyles the table's 502 or 5,002
elements and the edit none, and each lays out three boxes, the table and
its block ancestors, the rest taken from the cache. Within the budget:
the edit at 100 rows. Past it, and why:

- *Text shaped again at every pass* was the benchmark's first finding:
  the 1,000-row table's layout took 1,135ms, its words measured anew
  each pass. A cache of widths in `text::FontCollection`, by font size
  and text, up to 65,536 (`MAX_MEASURES`), half of M35.8, brought it to
  68ms; every page with text gains from it (T4.3 below).
- *A table laid out whole*: the layout cache keeps no cell, so an edit
  lays every cell out again, 23ms, and measures them for the table's
  width, 9ms, at 1,000 rows; the box tree built again, 17ms, and the
  fragments copied, about 12ms, are the M35.9 costs below. M35.9.
- *A restyle's cost an element*, 7-8us, more than the whole cascade's
  6us an element on this page, and a full raster of the viewport in
  software, about 10ms, when every visible cell changes colour. M35.11.

The reference browser (Chrome 154.0.8037.99) on the same table at
1,000 rows, style and layout alone forced by `offsetHeight`, no paint:
the first layout 49-60ms, the toggle 33ms, the edit 2ms. A change that
restyles and lays out every cell is past 16.7ms there too.

**A realm's bindings.** `benches/scripts.rs`'s realm takes 5.0-5.2ms
to make where it took 0.12ms: the DOM's bindings are a bootstrap of
3,300 lines of JavaScript every realm compiles and runs, 4.5ms of it
the compiling. A page's scripts cost it once, at their first; the page
of 1,000 paragraphs and 110 scripts now loads 11ms slower with them
than without, where it was 2ms. Loading the bootstrap compiled, which
`rquickjs` offers only as `unsafe`, is M35.12's.

**T4.3.** The benchmarks of the code before M25 (1f8d9b5) and of M25,
by turns, four rounds. Every first layout of a page of text is 7 to 25
times faster for the cache of widths; the rows past 10%, but those of
the same first frames on other benchmarks:

| | Before M25 | M25 | |
| --- | --- | --- | --- |
| Changing the document: the first frame | 721-739ms, median 726 | 56-58ms, median 57 | x0.08 |
| An appended paragraph's frames over 16.7ms | 22-34 of 1,000 | 16-22 of 1,000 | x0.70 |
| 1,000 appended in one frame | 738-833ms, median 781 | 65-70ms, median 67 | x0.09 |
| Its style | 4.31-4.74ms, median 4.45 | 4.76-7.45ms, median 7.05 | x1.58 |
| Loading: `load` to the first frame drawn | 782-955ms, median 812 | 59.9-62.0ms, median 60.5 | x0.07 |
| Going back to the page fetched again | 777-929ms, median 800 | 41.5-44.0ms, median 43.5 | x0.05 |
| Scrolling: the layout, once | 377-416ms, median 402 | 15.0-15.4ms, median 15.2 | x0.04 |
| Flex: a grid of 1,000 cards, the first frame | 2.39-2.69s, median 2.54 | 230-236ms, median 234 | x0.09 |
| Grid: a gallery of 1,000 figures, the first frame | 915-924ms, median 918 | 115-118ms, median 117 | x0.13 |
| Pseudo-elements, all of them: the first frame | 2.95-3.03s, median 2.97 | 159-162ms, median 161 | x0.05 |
| Animation: the body's class changed, its style | 8.91-11.13ms, median 9.41 | 10.85-11.46ms, median 11.09 | x1.18 |
| Values: the cascade of 1,000 paragraphs, written out | 6.12-6.34ms, median 6.19 | 4.50-4.86ms, median 4.81 | x0.78 |
| Values: 1,000 boxes sized by lengths, cascade | 5.73-5.85ms, median 5.76 | 4.38-4.42ms, median 4.41 | x0.77 |
| Scripts: a realm made | 0.12-0.13ms | 4.96-5.16ms | x40 |
| Scripts: the page, its 110 scripts run | 467-477ms, median 472 | 55.4-56.4ms, median 56.1 | x0.12 |

The restyles past 10% and the cascades now faster are the one effect
M24's T4.3 found: the cascade's speed follows where its memory falls,
and the user agent sheet lost its `noscript` rule at M25. The values
cascades are back at M23's times, as they were before that rule came,
and the two restyles slower by as much; the 1,000 appended at once
take 4.8ms in one round of four and 7.0-7.5ms in the others. The
realm is the bindings' compiling, above. The photo grid's rows
`t43sum.py` flags pair the software backend's section with the GPU
backend's; each backend's frames are as they were.

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
  at once costs the same, for the same reason. M25 cached the widths
  (above): the mutation benchmark's first frame is 56-58ms, the load
  benchmark's 50-54ms, past the budget still; the faces are M35.8's.
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
  the same rounds. M21's larger style again: 256–384 times in 1,000,
  the median at 13.9ms, against 16–90 and 11.2ms for the code before
  M21 in the same rounds (T4.3, M21). M22 leaves it as it was: 58–299
  times in 1,000, the median at 13.1ms, against 103–446 and 12.6ms
  before M22 (T4.3, M22). M35.9 takes it on.
- **Justification by character.** `text-justify: inter-character`, a
  word laid out a fragment a character and each measured alone, as
  letter spacing is: 1,000 paragraphs of six lines take 5.5–5.9s to lay
  out against 2.2s justified by word. M35.8.
- **A first line's boxes cascaded again at every build.** A
  `::first-line` that sets an inherited property has every inline
  element, `::before`, `::after` and `::first-letter` on a first line's
  run cascaded again whenever the box tree is built, which is at every
  layout pass. On the pseudo benchmark's page that is 7–8ms of an
  appended paragraph's frame with links and emphases alone, and 30–35ms
  with generated boxes and first letters as well. A box tree kept and
  updated in place, M35.9, keeps those styles with it.
- **Many animations at once.** An opacity animated on each of 1,000
  paragraphs takes 23.5–26.8ms a frame: every frame restyles the 1,000
  elements, their selectors matched again, 15.5–17.2ms, and draws their
  groups, 7ms (M22). Keeping an animated element's matched rules
  between frames is M35.2's.
- **A colour's animation lays out what it reaches.** A colour animated
  on one of 1,000 paragraphs lays out that paragraph and the boxes
  around it at every frame, 6.4–7.2ms of an 8.5–9.6ms frame, though it
  only repaints (M22). Fragments that refer to their styles, so that a
  paint-only change lays nothing out, are M35.9's.
- **Positions counted from the end.** With `:last-child` or
  `:nth-last-child` in a stylesheet, inserting an element restyles every
  sibling before it: 20,015 elements and 42ms at 10,000 paragraphs. That
  is the conservative rule of M13.1.
