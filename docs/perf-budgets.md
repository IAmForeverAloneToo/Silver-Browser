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
  can wait: M19's re-raster of layers and M22's animations. That work
  is to stop at the deadline and finish in a later frame.

## How the numbers are taken

- **Build:** release unless stated; the debug build where the budget
  names it.
- **Font:** the bundled test font (`text::TEST_FONT`), so every machine
  shapes the same text.
- **Machine:** a cloud container with four cores and 15GB of memory:
  an Intel Xeon at 2.1GHz for the numbers up to M15, and one at 2.8GHz
  from M16, which measured its base again ("Selectors and
  pseudo-elements", below). Numbers from different machines are not
  compared; a milestone that changes machine measures its base again.
- **Benchmarks:** plain binaries, `harness = false`, run by hand:
  - `cargo bench -p engine --bench scroll` (M9.6; stages since M13.4).
  - `cargo bench -p engine --bench mutation` (M13.6).
  - `cargo bench -p engine --bench load` (M14; going back, M15).
  - `cargo bench -p engine --bench pseudo` (M16.5).
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
Redrawing a shifted frame instead is M19's.

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

### Over budget, and why

These are known and recorded rather than hidden. Each has the item that
is to take it on.

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
  times in 1,000, the median at 12.6ms. M35.9 takes it on.
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
