# Silver Browser — Architecture

A hobby browser engine in Rust. Goal: a clean vertical slice now, with a
crate architecture shaped like a real engine (Chromium/Blink-style layered
components) so that growth never requires a restructure.

The milestone plan, parity tiers (committed target: T3, "replaces Chrome
daily"), decision register, and risk register live in
[`roadmap.md`](roadmap.md).

## The layering contract (the one rule that matters)

The workspace is a layered DAG. Every crate may depend ONLY on crates below
it in this stack:

```
shell          (bin "silver-browser": windowing, event loop, input, clipboard)
 ├ ui          (the browser's own interface, since M15: the toolbar and its
 │              address field, the status bubble, the window's title; draws
 │              with paint, render and text below, and touches no OS)
 └ engine      (session glue: orchestrates net → parse → style → layout →
                paint → render for each frame/navigation)
    ├ render  (rasterization backends: software today, wgpu GPU later)
    ├ paint   (fragment tree → display list)
    ├ layout  (box tree construction + layout algorithms)
    ├ style   (selector matching, cascade, computed values)
    ├ text    (font loading, shaping, glyph rasterization)
    | images  (image decoding, animation frames, decode bounds: a leaf
    |          crate, since M23, below layout, paint and render)
    | js      (scripts: the script engine behind traits of the engine's,
    |          the boundary scripts reach the document through, and
    |          their limits: a leaf crate, since M24, below engine;
    |          WebAssembly over wasmi, since M29)
    | profile (what the browsing keeps on disk, over redb: a leaf
    |          crate, since M28, below engine)
    ├ net     (URL parsing, resource fetching)
    ├ html    (bytes → DOM)
    ├ css     (stylesheet bytes → CSSOM)
    ├ dom     (document object model — a leaf crate)
    └ encoding (the Encoding Standard's labels, decoders, encoders — a
                leaf crate, since M14)
```

### Rules

1. **Dependencies point strictly downward.** A lower crate must never
   `use` a higher one. The workspace root `Cargo.toml` mirrors this graph;
   keep it honest.
2. **Only `shell` touches the OS**: window creation, input events, the
   event loop, the clipboard. `ui` draws the browser's own interface
   and touches no OS: the shell gives it input and shows its pixels
   (M15). Only `render` touches pixel buffers / the GPU. `engine`
   orchestrates but owns no platform state. Only `net` opens network
   connections and reads resources by URL (since M14): CI fails the
   build when `TcpStream`, `UdpSocket`, `ToSocketAddrs` or `rustls`
   appears in any other crate or in `fuzz/` (T3.4). Only `profile`
   keeps what the browsing keeps on disk (since M28), in the directory
   the shell chooses: it alone knows `redb`.
3. **Types are owned by their producer.** The display list lives in
   `paint`, the box tree in `layout`, the DOM in `dom`. No shared
   "common types" crate — if two layers need a type, it belongs to the
   lower one.
4. **Parsers never fetch.** `html` and `css` take bytes in and hand
   trees out; they never perform I/O. Asynchronous resource loading is
   `engine`'s job (that is where Chrome-style preload/scanner logic goes
   later).
5. **Library crates never panic on untrusted input.** All parsing entry
   points return `Result`. Web content is hostile by definition.
6. **Cross-platform only.** `std` plus crates that build on Windows,
   macOS, and Linux. No `#[cfg(windows)]` outside `shell` and `render`,
   but for one thing in `net`: turning a `file:` URL's path into a
   drive-letter path, and back in its tests (M14).
7. **Rendering is pluggable.** `render` exposes a backend trait. The
   software backend (tiny-skia) is the permanent fallback; since M19 a
   wgpu backend behind the `gpu` feature draws the same display lists
   behind the same trait, and the shell draws on it where the machine
   has an adapter (D3, D8). Both hand the shell a frame it presents
   through softbuffer (D9).
8. **`unsafe` only at the edges (since M24).** No crate of the
   workspace writes `unsafe` but `shell`, for the platform calls its
   windowing crates do not make, each block with a `// SAFETY:`
   argument. Code in another language is reached through a
   third-party crate's safe interface: QuickJS-ng through `rquickjs` in
   `js`, which says so with `#![forbid(unsafe_code)]`, and aws-lc
   through `rustls` in `net`. `profile` forbids it too, over `redb`,
   whose own (its file reads, locks and hashing) is third-party (M28).
   `js` runs WebAssembly over `wasmi` (M29), whose executor's own is
   third-party too. The unsafe code that leaves, third-party and
   pinned, is M33.5's audit.

## Pixels: where fractions become whole (M8.5)

Layout works in CSS pixels as `f32` and keeps its fractions, one CSS
pixel to a device pixel: the engine does not draw at a display's scale
factor yet (G9). The fractions become whole only where something is
drawn, by these rules, the reference browser's where it has one:

- **Boxes.** Paint snaps each edge of a box to the nearest device
  pixel on its own, a half rounded up (`paint::snapped`): the border
  box's edges and the padding box's apart, so that neighbouring boxes
  meet without a seam and border widths, which the cascade makes whole
  (M11), stay as they are. A text run's baseline (M17), a decoration
  line (M21.7) and a scrollbar are snapped the same way.
- **Text.** A glyph's advance is the font's hinted advance with the
  shaper's kerning, scaled and rounded to a whole pixel, a half up
  (M17, `text`), so that glyphs have no subpixel positions; a glyph's
  mask is put at the whole pixel at or before its origin, both ways,
  and the glyph cache knows a glyph by its font, its id and its size
  alone.
- **Transforms.** What a transform moves is not snapped: a group drawn
  through a matrix is rasterized in a layer of its own plane and
  resampled through the matrix, a pixel's colour taken at its centre
  (M19); a translation by whole pixels copies the layer's pixels.
- **Images.** An image fills its snapped box; its pixels are sampled at
  the destination pixels' centres, a nearest sample between two texels
  taking the lower as the reference browser's does (M23).

## Why this shape scales toward "Chrome-scale"

- **Crate = component.** Each crate compiles and tests independently,
  maps to a real engine subsystem (compare Blink modules), and can grow
  to hundreds of thousands of lines without naming or structure decay.
- **Crate boundary = future process boundary.** When the engine grows
  up, sandboxed renderer processes (Chrome's site isolation) replace
  direct calls with message passing across the same seams. The seams
  are chosen for that.
- **Parallelism fits the DAG.** Layout/paint/raster pipelines and
  multi-tab sessions extend naturally because no layer owns global
  state it didn't create.

## Current state

Milestone 3 (CSS parsing): `css` tokenizes per the css-syntax-3 token
model (idents, functions, at-keywords, hashes, strings with escapes,
urls, numbers/dimensions/percentages, CDO/CDC, comments skipped,
bad-string/bad-url recovery) and parses stylesheets with error
recovery — known at-rules become nested rules or declaration blocks,
unknown at-rules are kept as raw token blocks for forward
compatibility. `engine` collects `<style>` element text into
stylesheets (`stylesheets()`); `paint` skips style/script text.

Milestone 4 (Selectors v1): `style::selector_matching` parses selector
lists over `css::Token` slices (types, universal, id, class, all
attribute operators + `i`/`s` flags, all four combinators, structural
pseudo-classes including the full `An+B [of S]` grammar, `:not/:is/
:where`, dynamic pseudo-classes as parse-but-never-match stubs until
M26) and matches them right-to-left against a flattened DOM view
(`FlatDom`; since M13, against the live `dom::Document`). 121 tests
(unit + a table-driven harness over real markup,
`tests/selector_table.rs`); a hand-rolled baseline bench
(`benches/match.rs`) measures ~200ms matching 1k selectors against a
10k-element synthetic DOM — over the milestone's <100ms aspirational
budget, expected until M35.2's optimization pass (rule-hash bucketing,
style sharing) lands; M16.5 measured it again and found no call for a
Bloom filter.

Milestone 6 (Box tree + block layout v1): `layout::box_tree` turns a
`style::StyledDom` (since M13, the document and its `style::Styles`)
into a tree of block/inline/anonymous-text boxes
(`display: none` skips the whole subtree); `layout::block` lays normal-
flow block boxes out for real — CSS2.1 §10.3.3 width/margin resolution
(`auto`/px/percent width, auto-centering, the over-constrained ltr rule)
and CSS2 §8.3.1 margin collapsing (adjacent siblings, parent/first-child,
parent/last-child, empty-block self-collapse — one level of
parent/child escaping, not transitively through multiple ancestors) —
into a positioned `Fragment` tree. Inline-level content (`display:
inline`, and text) still gets only a stand-in (every text run is its own
line; no real line boxes, wrapping, or bidi — that's M7). `paint::
paint_fragments` ("paint v2", M6.8) consumes the fragment tree: real
backgrounds and borders, painted from actual box geometry. `ComputedStyle`
gained `width`/`height` (both `LengthPercentageAuto`: `auto`, px/em, and
— width only — percentage) and `margin` changed type to allow `auto`
per side, needed for M6.3's centering — M5's own tests were updated for
the new type where that touched them. One real cross-cutting bug found
and fixed along the way: `<style>`/`<script>`/`<head>` had no UA rule
setting `display: none`, so a `<style>` element's own CSS source text
was computing as ordinary visible inline content (M1's hardcoded paint
functions had special-cased this directly; the new box-tree pipeline
had nothing analogous) — fixed at the correct layer, the UA stylesheet,
matching how real browsers solve it.

Milestone 7 (Inline layout + line breaking): `layout::inline` replaces
M6's "every text run is its own line" stand-in with real text flow.
`text::FontStore` gained vertical metrics (`metrics()`: ascent/descent/
line-gap, scaled per font size) and a lean `measure()` for word widths,
alongside the pre-existing `shape()`. Words are measured and packed onto
lines greedily at whitespace break opportunities (css-text-3 §3's
collapsing — `str::split_whitespace` already implements exactly that),
baseline-aligned across mixed font sizes sharing one line (the shared
baseline sits at the tallest ascent present; line height is tallest
ascent+descent+line-gap), and aligned per the new `text-align` property
(left/right/center — `justify` deferred to M21 per that item's own
note). `style::ComputedStyle` gained `text_align` (inherited). Block-in-
inline (a block box nested in inline content) doesn't restructure the
box tree into real anonymous boxes the way browsers do; it ends the
current line, lays the block out on its own via `block::layout_block`
(now `pub(crate)`, called back into from `inline`), and resumes a fresh
line after — same observable "splits the run" effect, without the
anonymous-box machinery, and critically without ever panicking on
input a real page can produce (`<span>` containing a `<div>` is
non-conforming HTML content-model-wise but this engine's lenient parser
builds the tree anyway, so layout has to survive it). `overflow-wrap:
break-word` (a lone word wider than its container) isn't implemented —
documented gap, not silently wrong. 279 tests workspace-wide. One
consequence worth knowing: exact text-position assertions throughout
the test suite now derive their expected values from the *actual*
loaded system font's own metrics (via the same `FontStore` the
production code uses) rather than a fixed guess — portable across
machines with different fonts installed, at the cost of a test needing
to call `FontStore::metrics`/`measure` itself to know what to expect,
rather than asserting a literal number. Still stub: `net`. Next
milestone: text maturity (M8) — per-script font fallback, ligatures/
kerning, bidi level 1, letter/word spacing, `text-transform`,
`white-space`.

Milestone 8 (Text maturity): `text` gained four real capabilities.
Vertical metrics (M8.5) now prefer the OS/2 table's typographic
ascender/descender/line-gap over `hhea`'s (real browsers do the same —
`hhea`'s are historically tuned for Windows clipping-avoidance, often
taller than comfortable line spacing actually needs), falling back to
`hhea` for fonts without a (new-enough) OS/2 table. (Corrected by
M17, whose acceptance pages put text beside the reference browser's:
that browser takes `hhea`'s unless the font sets OS/2's
USE_TYPO_METRICS, rounds each metric to a whole pixel, and places each
glyph at a hinted, whole-pixel advance; `text` now does the same. The
flexbox note, "Vertical metrics, found at step 11", has the
measurements.) `letter-spacing`/
`word-spacing`/`text-transform` (uppercase/lowercase/capitalize)/
`white-space` (normal/pre/pre-wrap/pre-line) (M8.4) landed in
`style::ComputedStyle` and `layout::inline`: `text-transform` runs
first (logical order, position-independent); `white-space` selects a
different newline-handling/tokenization strategy; `letter-spacing`
measures and renders character-by-character, so the width
line-breaking decides on never drifts from what's actually drawn, the
way measuring a whole word at once then rendering it per-character
would via kerning. Bidirectional text (M8.3) reorders each literal
line into visual order per UAX#9 (crate: `unicode-bidi`), base
direction coming from the computed `direction` property (itself from
the `dir` attribute, inherited) — wired into
`layout::inline::collect_inline_items`, one bidi paragraph per literal
line, before word-splitting. Per-script font fallback (M8.1):
`text::itemize` splits a string into maximal same-script runs
(`Common`/`Inherited` characters merge into whichever run they're
adjacent to, matching HarfBuzz/ICU convention), and a new
`FontCollection` type holds a primary font plus eagerly-resolved
fallbacks for Han/Hiragana/Katakana/Hangul (the roadmap's own
"Latin → CJK" scope — other non-Latin scripts fall through to the
primary font, same as before M8.1); `FontCollection` replaced
`FontStore` as what `layout` measures with (`measure`,
`metrics_for_text`) and what `render::SoftwareBackend` shapes/
rasterizes with (`shape`, now returning each glyph tagged with the
script whose font produced it). That last part mattered more than it
might look: a `glyph_id` is only meaningful relative to the specific
font that produced it, and `SoftwareBackend`'s glyph-mask cache had no
font-identity component in its key at all — introducing a second live
font without fixing that would have silently misrendered the moment
two fonts' glyph ids collided at the same `(id, size)`. Fixed as part
of landing the second font, not left as a known gap: the cache key is
now `(Script, glyph_id, size)`, with a dedicated regression test
(`mask_cache_does_not_collide_across_scripts`). M8.2 (ligatures/
kerning) needed no new code — rustybuzz's default shaping already
applies them, true since M1; documented in `text`'s module doc so it
doesn't get rediscovered as a gap or reimplemented by accident. Two
real bugs caught by hand-tracing *before* trusting a test result, not
by the tests themselves: `itemize`'s rule for a leading `Common` run
(e.g. digits) retroactively "claiming" the real script that follows it
checked the wrong side of the comparison and never actually claimed
anything; and routing a `white-space: normal` text box's content
straight into `bidi_reorder` fed it a string that could contain a
literal, un-split `\n` — which `unicode-bidi` treats as its own
paragraph separator, so `bidi_reorder`'s existing (and correct, for
its stated scope) "return the first paragraph" behavior silently
dropped everything after the first line. Scope cuts, documented rather
than silent: bidi is "level 1" (reordering is per *literal line*, not
aware of soft line-breaking the way a fully spec-correct
implementation would be — the roadmap's own M42 defers that); CJK is
the only fallback scope landed, not a script-to-family matrix for
every script `unicode-script` can name. 314 tests workspace-wide (163
in `style`, 33 in `layout`, 27 in `text`). Still stub: `net`. Next
milestone: viewport, scrolling, clipping (M9), per the roadmap's phase
map.

Milestone 5 (Cascade + computed values): `style::cascade` turns a
`dom::Node` plus its `css::Stylesheet`s into a `StyledDom` (since M13,
a `dom::Document` into `style::Styles`, keyed by `NodeId`) — every
element's `ComputedStyle` (`color`, `background-color`, `font-size`
with `em` resolved against the right basis, `font-family`, `display`,
`margin`/`padding`/`border-width` per side, `border-color`/`border-
style`), cascaded from the built-in UA stylesheet (`assets/ua.css`,
compiled in) under author stylesheets and inline `style` attributes,
`!important` correctly reversing UA-vs-author precedence, inheritance
resolved (`color`/`font-size`/`font-family` inherit; the box-model
properties don't), `border-color` defaulting to `currentcolor`, margin/
padding/border-width shorthands and the `border` compound shorthand
expanded, unrecognized properties/values ignored rather than erroring.
`paint::paint_styled_document` consumes it: real cascaded colors and
font sizes, real `display: none`, margin-driven vertical spacing — no
real box model yet (no backgrounds, no borders, no width resolution;
that needs the fragment geometry `layout` builds in M6, per M6.8).
150 tests in `style` (144 unit + the M4 table-driven harness); `paint`
and `engine` each gained their own M5-specific tests, including the
M5.9 acceptance scenario at the full-pipeline level (a color-changing
rule visibly changes the rendered frame buffer). Still stubs: `layout`,
`net`. Next milestone: box tree + block layout v1 (M6).

Review of M0–M8 (2026-10-01): before M9 started, the finished
milestones were read for defects crate by crate and the parsers were
fuzzed. Real defects turned up in every layer; all are fixed with
regression tests, and no milestone's scope changed. The bounds this
introduced are the ones hostile input now meets. `html` caps nesting
at `MAX_DEPTH` (512; an element opened past it closes the innermost
one first, so the tree keeps its shape and a page of 100,000 nested
`<div>`s no longer aborts the process), and `dom` drops and walks
trees iteratively. `css` bounds at-rule nesting (`MAX_NESTING`, 128;
deeper blocks are kept raw), `style` bounds selector nesting
(`MAX_NESTING`, 32), and the cascade parses every selector once per
stylesheet rather than once per element. `FlatDom::build` (retired in
M13), the cascade's walk and the matcher are iterative; the matcher memoises
the `(compound, element)` pairs that lead nowhere, so a chain of
descendant combinators over a deep tree is polynomial rather than
exponential, and `:nth-child(… of S)` ranks a parent's children once
(`Matcher`). Lengths are finite and clamped to `MAX_LENGTH_PX`
(2^25 − 1), and are dropped where the spec forbids a negative one.
`text::rasterize_glyph` refuses a mask over `MAX_MASK_PIXELS` and a
non-finite or non-positive size; `render` refuses a frame over
`MAX_FRAME_PIXELS` and bounds its glyph-mask cache (64 MiB, cleared per
document); the shell presents nothing rather than an empty buffer.
Conformance: the HTML tokenizer handles numeric references without a
`;`, the windows-1252 remap, the legacy named references and the
attribute exception, `<?…>` as a bogus comment, abrupt comments, CR/LF
normalisation and `<pre>`'s leading newline; the tree builder implies
end tags for `li`, `dd`/`dt`, `option`, table rows and cells and `p`
with the spec's scopes, and honours `/>` only on void and foreign
elements. The CSS tokenizer preprocesses its input (css-syntax-3
§3.3); the parser keeps the rules after a nested block and recovers
from a `{}` block inside a declaration. Selectors reject a trailing
comma and a numeric attribute value, treat `:is()` and `:where()` as
forgiving, and apply selectors-4 §6.2 to an empty substring value;
`An+B` survives multibyte input, handles signs and the `n-` form. The
cascade resolves `inherit`, `initial` and `unset`, ignores an empty
`border`, and rejects negative padding, border width and font size.
The UA sheet makes `div` and the other block-level HTML elements
blocks (a page of `<div>`s laid out as inline content before). Margin
collapsing carries adjoining margins as a set (`AdjoiningMargins`,
largest positive and largest negative), so three margins of mixed sign
and a self-collapsing box resolve per CSS2 §8.3.1 instead of pairwise;
an empty inline no longer separates margins; over-constrained `auto`
margins are zero; an overflowing line is start-aligned whatever
`text-align` says; blank lines in `pre` take a line each; a no-break
space is part of its word; `bidi_reorder` keeps every paragraph.
Tests: 66 of the 95 tests in `layout`, `paint`, `render`, `engine` and
`text` returned early on a machine without a system font and so passed
vacuously. `text` now bundles DejaVu Sans (`fonts/`, under the
Bitstream Vera licence beside it) as `text::TEST_FONT`; every test
measures the same font on every machine, and a missing font is a
failure. 370 tests workspace-wide. Fuzzing: five cargo-fuzz targets
under `fuzz/` (`html_parse`, `css_stylesheet`, `selector_parse`,
`style_document`, `pipeline`; M13 adds `mutation`), run on every push
and weekly in CI,
clean after the fixes. Noted for the milestones they belong to, not
fixed here: `rustybuzz` re-parses the face on every shape or measure
call (M9.6's bench decides whether that matters); a text fragment's
`y` is its baseline and `h` the font size, which M9.4's hit testing
will need as ascent plus descent; layout recursion is bounded only by
the parser's depth cap (a tree built by script is M25's concern);
`rustybuzz` and `ttf-parser` are unmaintained (RUSTSEC-2026-0192 and
RUSTSEC-2026-0206, acknowledged in `deny.toml`), which decision D7
takes up.

Milestone 9 (Viewport, scrolling, clipping): layout's output is a tree
(M9.1): a block box's `Fragment` holds its content's fragments as
`children`, every fragment carries the `FlatDom` index of its element
(`node`, a `NodeId` since M13; a text fragment names its containing
element), and a text
fragment's box is its line (`y` stays the baseline, `ascent` says where
the top is, `h` is ascent plus descent). `layout::layout_document`
returns `DocumentLayout { roots, height }` in document coordinates (the
initial containing block's top-left is `(0, 0)`), the height counting
the leading and trailing margins the viewport materialises; the flat
`layout` view remains for inspection. `paint::paint_fragments` paints
for a `Viewport { width, height, scroll_y }`: items are in viewport
coordinates, a fragment the viewport does not meet produces none (its
content is still walked, since content overflows), and a box whose
`overflow` clips wraps its content in `PushClip`/`PopClip` at its
padding box (M9.3) — `hidden`, `scroll` and `auto` all clip (CSS2
§11.1.1) until an element can scroll on its own (M10.5), the two-value
form is not parsed yet, and a clipping box out of view is skipped with
its content. `render::SoftwareBackend` keeps a clip stack: rects are
intersected, glyph blits touch only pixels whose centre is inside, an
unmatched pop is ignored. Scrolling lives in `Engine` (M9.2): a target
offset clamped to the document, approached a third of the remaining
distance per `tick` (the roadmap's fixed-tick lerp; no clock involved,
until M13.4 timed it by the clock the shell passes each frame),
through `scroll_by`, `scroll_to`, `set_scroll` and `scroll_to_fraction`,
vertical only (documents are laid out to the viewport width; a wider
word overflows to the right, M7's stated gap). The scrollbar is
in-engine chrome (decision D9): drawn into the frame after the document
when it is taller than the viewport, and its geometry
(`Engine::scrollbar`) is what the shell maps a press or a drag through.
The shell turns the wheel (a notch is 40px), the arrow keys, Page
Up/Down, Home/End and the space bar into scrolls — keys were not in
M9.2's words, but cost twenty lines and are the only scrolling without
a mouse — and a left click into `Engine::link_at` (M9.4), which prints
the href (following it is M15). Hit testing is `Engine::hit_test`: the
last fragment in paint order whose box contains the point, descending
past a clipping box only when the point is inside its padding box; the
groundwork for M26's hover. The cascade is computed once per document
and the layout once per (document, viewport width) (M9.5), and
`Engine::layout_runs` proves that a scroll, a hit test and a taller
viewport rerun paint and raster only. Measured (M9.6,
`crates/engine/benches/scroll.rs`: 128 paragraphs, 8061px, ten
viewports of 800px, 120 frames of 40px at 1280×800): the first profile
of a debug frame was 670ms, almost all of it shaping the same 624 text
runs again every frame (the text stack parses the face on every call,
decision D7's item), so `SoftwareBackend` now caches shaped runs by
(text, size) beside its glyph masks, bounded at a million glyphs and
emptied with them. With that: 2.0ms a frame in release (490fps), 8.7ms
on the debug build (115fps) with dependencies optimised, 15.7ms (64fps)
with nothing optimised at all. The workspace's dev profile now builds
dependencies at `opt-level = 3`: not for the scroll budget, which the
cache meets either way, but for layout, which shapes every word to
measure it and takes 170ms for that page with optimised dependencies
and eight seconds without — a debug build has to be usable. Layout's
own cost (160ms in release: a face parse per measured word) is the next
thing the perf track meets (M13's incremental engine, M35, D7). Not in
this milestone, said rather than hidden: `overflow-x`/`overflow-y` and
the two-value `overflow`; elements that scroll (M10.5); horizontal
scrolling; hover tracking (M26); anti-aliased clip edges (a clip is
pixel-aligned at pixel centres). 388 tests workspace-wide.

Milestone 10 (Box model completion), designed in
`docs/design/box-model.md` and built in its ten steps, one commit each.
The parser first (M2's debt, paid here because `rem`, the UA sheet's
`body { margin: 8px }` and every comparison with a browser need the
same tree): the tree builder implies `html`, `head` and `body` as the
HTML standard's insertion modes do, and the tokenizer has the raw-text
state for `style`, `script` and the others and the escapable one for
`textarea` and `title`. Units (M10.1): the cascade takes a
`CascadeContext` — the viewport for `vw`/`vh`/`vmin`/`vmax`, and a
`FontMeasure` the engine lends over its font for `ex` and `ch`, so
`style` keeps no dependency on `text` — resolves `rem` against the root
and all the absolute units, records whether viewport units were used
so the engine recascades on a resize only then, and keeps percentages
for `height`, the margins and the paddings as written; layout resolves
margins and paddings against the containing block's width (CSS2 §8.3,
§8.4) and heights against its height only when definite (§10.5,
§10.7). `box-sizing: border-box` is converted to content-box values
before the CSS2 algorithms run (M10.2); `min-*`/`max-*` clamp the
tentative size with the margins re-resolved (M10.3), and a box with a
`min-height` is never self-collapsing. Borders are complete: each side
its own width, style and colour, the ten styles (`double` exact,
`dashed`/`dotted`/`inset`/`outset`/`groove`/`ridge` approximations,
said so in the code), sides as mitred trapezoids through a new
`DisplayItem::Quad` the rasterizer fills without anti-aliasing so
neighbours tile; the root element's background paints the canvas (CSS2
§14.2, the body's when the root's is transparent), white otherwise.
The box tree builder makes every block container's children
block-level (CSS2 §9.2.1.1, M10.4): inline runs in anonymous blocks
that inherit from the container and have no fragment of their own,
blocks inside inlines hoisted out with the inline split around them;
block layout only stacks, and M7.6's block-in-inline path in the
inline layout is gone. A clipping box and an inline-block are block
formatting context roots (their margins never adjoin their children's,
never self-collapsing). `display: inline-block`: shrink-to-fit through
a new `layout::intrinsic` pass (min-content and max-content
contributions measured by the same item collection the line breaker
uses), laid out as a block container, placed on the line as one item
as wide as its margin box with its baseline the last line box's or its
bottom margin edge (CSS2 §10.8.1, which is M7.7). Two things the
reference cases forced on the inline layout: the strut (a line is at
least its container's font tall, css-inline-3's root inline box; the
line gap sits wholly below the baseline), and css-text-3's white space
(one gap and one wrap opportunity where collapsible white space was,
none where it was not — `a<b>b</b>` is set flush — an atomic inline a
wrap opportunity on both sides; the box tree keeps white-space-only
text where it is a gap and drops it at a run's ends). Elements scroll
(M10.5): layout records on a clipping box's fragment the height of its
scrollable overflow area (css-overflow-3 §2.2: the content as if
`auto`, or a descendant reaching lower, then the bottom padding, at
least the padding box) and `Fragment::max_scroll` is what it allows;
the engine keeps each container's offset by element index, reset on
load and clamped every frame, animated by the same fixed-tick lerp
(`tick` advances all of them), and hands the wheel from the pointer to
the innermost container among the clipping boxes around the element
hit that can still move that way, then outward, then to the viewport
(`Engine::wheel`; the shell passes the cursor); hit testing adds a
container's offset on the way into its content; `paint_fragments_in`
takes the offsets through a `ScrollOffsets` trait, moves a container's
content inside its clip and draws an eight-pixel overlay scrollbar over
the padding box's right edge when it can scroll (`scroll` always,
`auto` with overflow, `hidden` never; the thumb shaped by the one
function the viewport's uses, whose colours moved to `paint`). An
overlay rather than a reserved gutter is the engine's own choice,
recorded in the design note because it is observable. The UA sheet
styles the form controls as inline-block boxes (M10.6): 13.33px text,
an input `20ch` by one line (`1.2em` standing in for the line height),
a textarea two lines and a scroll container, a button as wide as its
label, a select the size of an input with its options hidden, a
checkbox or radio 13px, `type=hidden` nothing; no value shown, no
behaviour. Acceptance (M10.7): ten box-only pages under
`crates/engine/tests/reference/`, captured by `capture.sh` at 800×600
from a headless browser (the headless shell of the Chromium this build
environment ships, whose viewport is exactly the window where the full
browser's headless window includes its toolbar — which two cases
caught) into committed PNG fixtures, and `tests/reference.rs` renders
each with the engine and compares byte for byte, writing the render and
a difference image under `target/reference/` on a mismatch; all ten
match; nothing of the browser enters the engine, and the harness
accepts any browser's capture (the roadmap names Firefox). The cases
avoid sides of different colours (a browser anti-aliases the mitre;
this engine does not) and fractional edges (a browser snaps them; this
engine anti-aliases them), both M11's painting work. Not in this
milestone, said rather than hidden: inline boxes' own backgrounds,
padding and borders (M11); `overflow-x`/`overflow-y`, horizontal
scrolling and the two-value `overflow`; `vertical-align` beyond the
baseline; `line-height`; the `size`/`rows`/`cols` attributes; classic
scrollbar gutters; HTML's case-insensitive attribute values in the
selector matcher (the UA sheet says `i`). 448 tests workspace-wide.
Still stub: `net`. Next milestone: floats, positioning, stacking (M11).

Milestone 11 (Floats, positioning, stacking), designed in
`docs/design/floats-positioning.md` and built in its twelve steps, one
commit each. The properties first (M11's step 1): `float`, `clear`,
`position`, `top`/`right`/`bottom`/`left` and `z-index` in the cascade
with CSS2 §9.7's computed-value rules. Then the paint-order walk
(`paint::order`, step 2): one walk over the fragment tree yields the
steps — a box's background and border, a text run, a clip, a
scrollbar — with every rectangle in viewport coordinates, and both
`paint_fragments_in` and the engine's hit test read it, the last step
under the pointer being the element hit, so paint order and hit order
have one definition. Floats (M11.1): an out-of-flow box keeps its
place in the box tree (`BoxKind::Float`, `BoxKind::Absolute`), a
`layout::float::FloatContext` per block formatting context holds the
floats placed in it as margin boxes in the root's coordinates, and
`At` carries each box's position in that context down the recursion;
placement is CSS2 §9.5.1's rules in order, a float met while the
margins above it are still collapsing waits and is placed level with
what resolves the flow position (the first line, the top of the next
box that is not self-collapsing or whose top edge separates its
margins, the base of a box that clears it, or the context's end), a
float in a run is placed beside its line when it fits and below it
otherwise, lines run between the floats overlapping them, and a
context root's `auto` height reaches its lowest float's bottom
(§10.6.7). Float fragments stay under their parent and
`attach_floats` moves them to where the context put them. Clearance
(§9.5.2) puts a cleared box's top border edge level with the lowest
relevant float's bottom margin edge, the margins above it resolved and
its own inside the clearance, and a context root beside a float takes
the room between the floats at its top, moving down when it does not
fit (M11.2). `position: relative` (M11.3) moves the fragment and its
subtree as it is placed. `position: absolute` and `fixed` (M11.4):
layout records the static position where it meets the box and carries
it up (`Deferred`) until the nearest positioned ancestor's padding box
is known, lays it out there by §10.3.7 and §10.6.4 (shrink-to-fit, one
`auto` solved, over-constrained ignoring `right`/`bottom`), and hangs
its fragment under the containing block's after the in-flow content,
which is what makes an `overflow` box between the two neither clip nor
scroll it; the initial containing block takes the rest, fixed boxes
among them as the document's last roots, which the walk paints with
the viewport's offset left out. `position: sticky` (M11.5) is laid
out in flow and shifted in the walk by `paint::order::sticky_shift`,
keeping each inset that is not `auto` from the visible edge of its
nearest scrollport within its containing block (the parent's padding
box, the border box kept inside it: two stated approximations), the
subtree moving with it. Stacking (M11.6): the walk is CSS2 Appendix E
— the root element and each positioned box with a `z-index` not
`auto` are stacking contexts; one silent pass gathers a context's
positioned descendants with the clips and scroll offsets of their
ancestors in the fragment tree; the negative layer, three passes over
the in-flow content (block-level backgrounds, floats, inline-level
content), the `auto`/`0` layer and the positive layer follow, floats,
inline-blocks and `auto` boxes as units; a box painted out of tree
order has the clips in force closed down to the ones it shares and
its own opened around it, and a clip with nothing inside is left out.
Inline elements get a `FragmentKind::InlineBox` per line holding that
line's items (step 9): the content area from the baseline by the
font's ascent and descent (§10.6.1) with padding and border around it,
the vertical ones not changing the line; the start and end edges take
room on the lines the element starts and ends on, sliced in between
(`box-decoration-break: slice`), the start edge taking a word's gap
and wrap opportunity and gluing what follows, and the line breaker
moves an unbreakable piece whole to the next line, back to the last
wrap opportunity — before, a glued word overflowed. The two pixel
rules (step 10): a box's edges are snapped to whole pixels where its
background, border and clip are painted (each edge on its own, half
up, layout keeping its fractions); a border's width is used as whole
pixels, rounded down and never below one where the style gives any
(css-backgrounds-3 §4.1, as the reference browser has it), a used
value and so layout's; and a mitre between two colours blends the two
sides by the exact area each covers of the pixel — a run of
neighbouring sides in one colour is one `DisplayItem::Polygon`, the
first run painted with its corners whole and the runs after it cut
along the mitre, and `render` fills a polygon by signed-area
accumulation (`fill_polygon_by_area`), tiny-skia's supersampling
putting five eighths where the browser has one half; four sides
sharing a band stay plain quads. Acceptance (M11.7): eleven more
pages under `crates/engine/tests/reference/`, captured by the same
headless shell (which reproduces the M10 fixtures byte for byte), all
twenty-one matching; `capture.sh` takes page names; a page may declare
a per-channel tolerance in a `meta` element, which the fractional and
multi-colour page alone does (two, for the last bit of the browser's
blending at mitres, reported by the test) and every other page is
exact. The pages found three things, fixed: floats before a box with
collapsing margins waited for the context's end; fractional border
widths were rounded to edges rather than floored; and the mitre's
coverage. Fixed under scroll, sticky and hit testing through the
stacking order are engine tests. Not in this milestone, said rather
than hidden: `line-height` and `vertical-align` beyond the baseline;
a line that grows into a float after it started is not relaid; `inset`,
`float: inline-start`/`inline-end` and `box-decoration-break: clone`
are not parsed; `direction: rtl` does not change the over-constrained
rules; sticky in nested scrollers and `fixed` inside a transformed
ancestor (M19); opacity and transforms as stacking contexts (M19);
text positions are not snapped across, only their baselines (M17);
absolutely positioned boxes do not shrink-to-fit against their
containing block's height; floats do not
affect margin collapsing beyond clearance; collapsible white space at
the end of an inline element is rendered after its end edge; a scroll
container that is not a stacking context has its scrollbar painted
under its positioned descendants; a double border in two colours blends
its mitres from the bands' colours alone where the browser lets the
background through; the browser's last bit of mitre blending. 497
tests workspace-wide. Still stub: `net`. Next milestone: tables (M12).

Milestone 12 (Tables), designed in `docs/design/tables.md` and built in
its eight steps, one commit each. The properties first (step 1): CSS2
§17.2's `display` values, `table-layout`, `border-collapse`,
`border-spacing`, `caption-side`, `empty-cells` and `vertical-align` in
the cascade, the four the specification says inherited, and the HTML
standard's table rules in the UA sheet in place of M10's blocks, which
is why an HTML cell is centred and a CSS one sits on the baseline. The
box tree (M12.1, step 2): a box kind per display value (`Table`,
`RowGroup` with its header, body or footer, `Row`, `ColumnGroup`,
`Column`, `Cell`, `Caption`), `colspan`, `rowspan` and `span` read as
the HTML standard clamps them, CSS2 §17.2.1's anonymous rows, cells,
row groups and tables supplied in the builder, and a table's children
ordered top captions, columns, the first header group, the rest, the
first footer group, bottom captions; a floated or absolutely positioned
table is the `Float` or `Absolute` box with a table's children, laid
out as a table by its display. `layout::table` lays a table out (steps
3 to 6): the grid of css-tables-3 §3.4 (M12.4), a `rowspan` cut at its
group's end and 0 meaning that end; the fixed algorithm of CSS2
§17.5.2.1 (M12.2) and, for every other table, the automatic one as
css-tables-3 §3.9 writes the browsers' practice down (M12.3) — cell and
column measures, a length `width` its column's maximum and its
constraint, a spanning cell's excess by maximums, percentages cut past
the whole, MIN, MAX and CAPMIN, the used width shrink-to-fit with MIN
as its floor, and §3.9.3's sizing guesses interpolated; row heights
(M12.6), `vertical-align` placing a cell's content at its top, middle
or bottom or on the row's baseline, a cell's first baseline its first
line box's or a nested table's first row's, for which layout now
records a box's first baseline beside its last; and a rowspan's excess,
a row group's `height` and the table's `height` shared out as the
browsers share them. The separated model spaces the cells,
`empty-cells: hide` leaving an empty cell unpainted; the collapsed
model (M12.5) resolves every segment of every grid line by CSS2
§17.6.2.1's conflict rules, each cell holding exactly half of each line
on its side and the table half the widest of each outer line, inside
its border box, as css-tables-3 §3.7.1.1 and the browsers have it, and
paints the resolved borders as the table box's own fragments, one per
segment, each joint its winner's. The fragments: the table's wrapper,
holding the captions and the table box, painting nothing and carrying
the table's positioning, and the table box, painting its background and
border; column groups, columns, row groups and rows paint their
backgrounds as one fragment per cell, under it, in §17.5.1's order, and
the paint-order walk needs nothing new. Hit testing gives the cell, the
row in the spacing between its cells, the row group between its rows
and the table on a collapsed line. A table is a block formatting
context root, `margin: auto` centres it, its intrinsic contribution is
MIN and MAX, and an `inline-table` aligns by its first row. Acceptance
(M12.7, step 7): five more pages, twenty-six in all, every one exact.
They found two defects, and measuring the reference browser to pin them
down a third, all fixed: a rowspan's excess goes first, equally, to the
rows after its first where another rowspan starts, else to the
unconstrained rows by height, else to the constrained ones, else to the
last row, the cells ending together taken shortest first; a table's
height goes to its body groups by height before their rows, the header
and footer groups taking none, and a row group's `height` is a minimum;
and the collapsed model is css-tables-3's — exact halves and the outer
lines inside the border box, not CSS2's outer halves in the margin —
with a joint its winner's, a losing segment stopping at its edge. Not
in this milestone, said rather than hidden: the presentational
attributes (`border`, `cellpadding`, `cellspacing`, `width`, `height`,
`bgcolor`, `align`); `visibility: collapse`; the parser's table
insertion modes; `overflow` on a table; `direction: rtl` in tables;
percentage heights on rows, row groups and cells, which count as
`auto`; `min-height` and `max-height` on rows and row groups; a row
group with no rows taking a share of the table's height; `th`'s bold
(M21). 551 tests workspace-wide. Still stub: `net`. Next milestone: the
incremental engine (M13).

Milestone 13 (The incremental engine), designed in
`docs/design/incremental.md` and built in its ten steps, one commit
each. The document became a live value (step 1): `dom::Document` holds
every node in an arena of slots and names each by a generational
`NodeId` (decision D16), so a stale or forged id resolves to nothing
and no id makes the crate panic. It offers the DOM's mutations, each
checking what the standard checks and this engine's two limits, 512
levels of nesting and some four million live nodes, and records every
change in a journal in the DOM's terms, which collapses into one record
saying everything changed past 65,536 unread. `Document::from_node`
builds it from the parser's tree and `to_node` takes a snapshot back.
Every stage then moved onto it (steps 2 and 3): the matcher and the
cascade read the arena, the cascade's result is `style::Styles` by
`NodeId`, boxes and fragments carry `NodeId`s, `hit_test` answers with
one and element scroll offsets are kept by one; `FlatDom` and
`StyledDom` are gone, and the UA sheet is parsed once per process. The
engine's API (M13.5, step 4): `set_document`, `load_document`,
`mutate`, which lends the document to a closure and reads the journal
after, `needs_frame`, and `styles`, the cascade the frames use. Each
stage then learned to redo only what a change reaches. Style (M13.1,
step 5): a document's sheets are prepared once as `style::Stylesheets`
with the features their selectors depend on, and the journal turns
into the elements to restyle — a named class or attribute, siblings
where sibling combinators or positions are in use, the parent where
`:empty` is — restyled by depth, each element's children only when its
style changed. Layout (M13.2, step 6): the box tree is rebuilt each
pass, and `layout::LayoutCache` keeps an element's own block or table
box's layout by node, taken again when its subtree, its containing
block and its float context are as before and nothing was carried out
of it; its entries name their children's rather than copy them.
Raster (M13.3, step 7): `render::SoftwareBackend` keeps the last frame
and redraws only the rectangles where the display list changed, every
item kept inside them by means that cannot change its pixels there, so
a frame's bytes are a whole raster's. The frame scheduler (M13.4, step
8): `Engine::frame_at` draws a frame for a time and a deadline the
caller passes and lends the pixels; `Engine::frame_stats` times each
stage; scrolling animates by that time; the shell draws only when the
engine wants a frame. `docs/perf-budgets.md` records the budget, the
policy for a frame over it — it skips nothing that changes what is
shown — and the numbers (T4.1). Acceptance (M13.6, step 9):
`crates/engine/benches/mutation.rs` appends 1,000 paragraphs to a page
of 1,000 one a frame, and no frame after the first lays the document
out whole; each lays out three boxes, at a median of 7ms in release.
Incremental equals from scratch: `engine::differential` drives the
engine with random mutations and holds every style, fragment and pixel
to a fresh engine's, in the engine's tests on two pages and in the new
`mutation` fuzz target; the pipeline target holds every frame drawn in
part to a whole raster; the reference pages, mutated and changed back,
still match their captures. The counters that prove work not done:
`style_runs`, `restyled_elements`, `layout_runs`, `boxes_laid_out`,
`boxes_reused` and `redrawn`. Not in this milestone, said rather than
hidden: building the box tree incrementally; a paint-only change still
lays its element out again, and every pass copies every fragment,
since fragments hold copies of styles and absolute coordinates (M35.9);
text shaping's cost (M35.8); a scroll as a shifted frame (M19);
parallel layout (M35.3); rule hashing and style sharing (M35.2);
parsing into the arena as bytes arrive (M24.8); scripting, the
collector that frees detached nodes, and mutation observers (M24,
M25); input events (M26). 593 tests workspace-wide. Still stub: `net`.
Phase A is done. Next milestone: `net` v1, URLs and HTTP/1.1 (M14).

Milestone 14 (`net` v1, URLs and HTTP/1.1), designed in
`docs/design/net.md` and built in its nine steps. A new leaf crate,
`encoding` (D18, step 1): the Encoding Standard's 40 encodings and
228 labels, BOM sniffing, and decoders fed bytes in pieces of any size
with the replacement and fatal modes, for UTF-8, UTF-16, the 28
single-byte encodings, x-user-defined and replacement; encoders for
UTF-8 and the single-byte encodings; the single-byte tables generated
from the standard's index files. The legacy CJK encodings are
recognised and, until a page or M42 needs them, decoded as UTF-8,
`Engine::undecodable_encoding` saying so. `net::url` (D17, step 2) is
the URL Standard's parser written as its state machine, with IPv4,
IPv6 and opaque hosts, domains through `idna` only when they are not
ASCII, the serializer and origins; the `url` crate is gone.
`net::mime` (step 3): MIME types parsed, serialized and extracted from
`Content-Type` as Fetch does, the MIME Sniffing algorithm, and `data:`
URLs with forgiving-base64. `html::parse_document_from_bytes` and
`css::decode_stylesheet` (step 4) find a document's and a sheet's
encoding as HTML and css-syntax-3 say, a late `<meta>` having the
bytes parsed again. `net::http` (step 5) writes requests and reads
responses over any byte stream, per RFC 9112: interim responses
skipped, chunked and length-delimited bodies, 256 KiB of header and
64 MiB of body at most. `net::transport` (step 6): the system's
resolver; connection attempts alternating address families 250ms
apart; Local Network Access's address spaces, checked on every address
connected to (D19); TLS 1.2 and 1.3 through rustls over aws-lc-rs,
with X25519MLKEM768 and Mozilla's roots, failing closed (D4); six
pooled connections an origin, an idle one looked at before it is
taken and a `GET` or `HEAD` sent once more when a reused connection
failed before answering; gzip and deflate; 60-second waits; and aborts
that wake a waiting connection at once on every OS, each connection
waiting in a `mio` poll of its own. `net::Client::fetch` (step 7) is
Fetch's scheme fetch and HTTP-redirect fetch over them: `about:blank`,
`data:`, `file:`, `http:` and `https:`, twenty redirects with their
method rules, the bad ports, mixed content, and every check run again
on every hop. The engine's loader (M14.6, step 8): `Engine::load`
fetches a document on threads of the engine's own, at most 32 at
once, each result waking the shell through the waker it gave, and
`poll_network` applies what arrived. The page shown stays until the
new document and the stylesheets before its `<body>` have arrived or
failed, so a page first appears styled. Stylesheets come from `<link>`
and `<style>` in tree order, each with its `@import`s before it, with
`<base>`, media, types and the preferred set as HTML and CSSOM have
them, and follow the journal as elements come, go and change;
`load_state` says loading, loaded or failed. A document given as
source or as a tree fetches nothing. The shell runs as
`silver-browser [URL or path]`, an existing file first, and reports a
load that fails. Tests: the web-platform-tests files for URLs (3,660
cases), encodings, MIME types and `data:` URLs, kept with their
BSD-3-Clause notice; the HTTP reader over in-memory streams;
`net::test_server`, scripted HTTP and HTTPS on 127.0.0.1 with a root
made as the tests run, which `net`'s tests and the engine's fetch
from; and a page whose sheets come from it, drawing the frame the same
page draws with them inline. Four new fuzz targets, `url_parse`,
`encoding_decode`, `mime_sniff` and `http_response`, run in CI with
the others. Numbers (step 9, `docs/perf-budgets.md`): a page of 1,000
paragraphs and three stylesheets from the local server is shown 3.3ms
after `load` in release; its first frame takes 650–820ms more, nearly
all of it the first layout's text shaping (M35.8). Not in this
milestone, said rather than hidden: parsing as bytes arrive (M24.8);
images (M23); cookies, CORS and the same-origin policy (M27); Referer
and HSTS (M33.3); the cache (M28.4, M38.2); HTTP/2 (M38); revocation,
Certificate Transparency, ECH and pools partitioned by site (M33);
proxies; the legacy CJK encodings (M42, or a T1 page that needs one);
media queries beyond `all` and `screen` (M20); navigation and error
pages (M15). 835 tests workspace-wide. Next milestone: navigation and
the address bar (M15).

Milestone 15 (navigation v1 and the address bar), designed in
`docs/design/navigation.md` and built in its eight steps.
`engine::navigation` (step 1) keeps the session history: at most 50
entries, each with its URL, a document state that the entries a
fragment navigation made from one another share, and a scroll
position; a new engine shows the initial `about:blank`. Everything
made from the document shown — the document, its page and stylesheets,
the cascade, the layout and its cache, the scroll offsets, the icon —
is one value, which the back-forward cache keeps whole for up to six
documents left, unless leaving one aborted a fetch or the browser made
it. `Engine::navigate` and `follow_link` push an entry, or replace it
when the URL is the one shown or the page is the initial
`about:blank`; `go_back`, `go_forward`, `traverse`, `reload` and
`stop` are HTML's; `Engine::load` is gone. A response becomes (step
2), in HTML's order, a download refused with a notice (`attachment`,
PDF, a type nothing shows), nothing (204, 205), an HTML document,
XHTML through the HTML parser, a text document of one `<pre>` for
text, script, JSON and other XML types, a page saying an image, audio
or video cannot be shown yet, or, for an error status with an empty
body, a page naming the status; a network error becomes an error page
under the URL that failed, naming what failed, with no way past a TLS
failure. The browser's pages are built as trees, so nothing from
outside becomes markup; `about:silver` is the built-in page, with a
link to one of its fragments. Links (step 3): `link_at` gives the link
with its URL, parsed against the document's base URL and encoding when
it is followed, under the browsers' scheme rules (no `data:` from a
page, `file:` only from a `file:` page, `about:silver` only from the
browser's own); fragment navigations, HTML's indicated part, scrolling
into view through scroll containers, and scroll goals applied before a
frame is painted and restored on traversal. `Engine::title`, and the
icon, fetched with `net`'s new image destination and kept up to 1 MiB
(step 4). `engine::address` (step 5) makes what is typed a URL: a
domain gets `https://`, a local host `http://`, an absolute path its
`file:` URL; there is no search and no `http:` fallback. A new crate,
`ui` (D9, step 6), draws the toolbar — back, forward, reload or stop,
the address field — and the status bubble with `paint`, `render` and
`text`, which gained the caret's places in shaped text; the field is a
model of its own, tested by a seeded run of 20,000 commands. The shell
(step 7) puts the toolbar above the page and the bubble over it, and
its input is a session of its own types, driven in tests with an
engine and the test server: the focus, the keys of Linux, Windows and
macOS, the clipboard through `arboard`, double and triple clicks, the
input method, the pointer's shapes and the window's title. The
acceptance (step 8): an ignored live test reads `https://example.com/`
through the engine, run here on a network whose TLS is intercepted by
trusting that network's root; the browser itself, which trusts
Mozilla's roots alone, shows the interception as an error with no way
past, and pointing it at the site from a direct connection is left to
a run by hand. Going back to a page of 1,000 paragraphs kept in the
cache takes 5–6ms to its first frame, and to one fetched again 650ms
(`docs/perf-budgets.md`). Not in this milestone, said rather than
hidden: tabs and named targets (M37.1); downloads (M37.2); search and
suggestions (M37.3); scripts' part in navigation (M24–M26); form
submission; an XML parser; images and media as documents (M23, M40);
showing the icon (M23); grapheme clusters and bidirectional text in
the field (M42); an input method's preedit text (M45); scaling for
high-density displays; the toolbar's accessibility (M44). 940 tests
workspace-wide. Next milestone: selectors v2 and pseudo-elements
(M16).

Milestone 16 (selectors v2 and pseudo-elements), designed in
`docs/design/selectors-pseudo.md` and built in its eight steps. A
selector may end in a pseudo-element (step 1): `::before`, `::after`,
`::first-letter` and `::first-line`, also with CSS 2's one colon;
`::marker`, `::placeholder` and `::selection`, recognised and set
aside; and any `::-webkit-` name, which matches nothing. Names compare
in ASCII lowercase; the attribute selector's `i` and `s` flags work,
and without one the values of the 46 attributes HTML lists compare
case-insensitively; `:nth-child(An+B of S)` counts S's specificity.
`:lang()` (step 2) filters by RFC 4647 against the language HTML gives
an element, from `lang`, a `content-language` pragma or the response's
header; `:link` and `:any-link` match links, and `:disabled`,
`:enabled`, `:checked`, `:required` and `:optional` the states HTML
gives controls from their attributes, each with the restyles its
inputs call for. `Styles` keeps an element's pseudo-element styles
beside its own (step 3), cascaded with its style as their parent;
`content` (strings, `attr()`, the quote keywords) and `quotes` join
the computed style, and the UA sheet gains HTML's rules for `q`. The
box tree builder walks items (step 4): a `::before` or `::after` box is
built for every display, at the quote depth the build has reached,
names its originating element and is marked as generated, and hit
testing answers with the element. `::first-letter` (step 5) splits the
text holding the first formatted line's first letter, by
Pseudo-Elements 4's pattern, into an inline box or a float.
`::first-line` (step 6) gives the run holding a block's first line its
chain of styles, and every inline box in the run its style on the line,
in which the inline layout fills the line; layout asks the cascade for
these styles through `style::Recascade`. Step 7 is M16.5: matching
takes 1.4–1.6 times what the code as imported did, so there is no
Bloom filter. A benchmark of what the pseudo-elements cost led to three
changes, and a computed style's `font-family`, shared rather than
copied, undid the 12% that M16's new properties had cost an appended
paragraph's frame (`docs/perf-budgets.md`, measured again on a new
machine). `about:silver` (step 8) shows a floated drop cap, a first
line in capitals, cards labelled by `::before` from an attribute and
closed by `::after`, and quotations in English and in French marks.
Not in this milestone, said rather than hidden: `::marker` and list
items (M21.6); counters in `content` (M21.4); images in `content`
(M23); `::placeholder` and `::selection` drawn, and the user-action
and form states (M26); `:has()`, `:dir()` and `:scope`; quotes chosen
by language and language tags canonicalised (M42); an atomic inline
on the first line, which keeps its own style; the first line's boxes
cascaded again at every build of the box tree, until the box tree is
kept (M35.9); and two gaps older than this milestone that it found,
`br`, which makes no box and breaks no line, and white space alone at
the ends of a run under `pre`, which is dropped. 1,060 tests
workspace-wide. Next milestone: flexbox (M17).

Milestone 17 (flexbox), designed in `docs/design/flexbox.md` and built
in its thirteen steps. The cascade (step 1) knows `display: flex` and
`inline-flex` and their `-webkit-` forms, the flex and alignment
properties, the gaps and the `place-*` shorthands; it blockifies a flex
container's children, takes CSS Color 4's 148 colour names, and keeps
`min-content`, `max-content` and `fit-content` in two new size types,
`min-width` and `min-height` becoming `auto`. Layout resolves those
keywords for every kind of box (step 2), and its functions share one
context per pass, `layout::pass::Pass` (step 3): the fonts, each flex
item's layouts by box and inputs, and each box's content widths, so a
chain of nested containers costs a few layouts a level rather than two
to the power of its depth. A formatting context can impose a height on
the box it lays out. `layout::flex` (step 4) follows css-flexbox-1 §9
in the abstract axes: items in `order`, base and hypothetical sizes,
the automatic minimum, lines with gaps, the flexible lengths in `f64`,
definite sizes, a flex container's intrinsic widths, and `inline-flex`
as an atomic inline. The web-platform-tests runner (step 5, T1.1)
chooses `css/css-flexbox`'s tests by a rule of what the engine has,
copies them with their references, and runs reftests, check-layout
tests and crashtests against `expectations.txt`, writing a report page
with a failing reftest's render beside its reference's. Then came
alignment on both axes with `safe` and `unsafe` (step 6), first and
last baseline groups and a container's own baselines (step 7),
absolutely positioned children in the static-position rectangle (step
8), and items painted as inline-blocks in order-modified document
order, with `z-index`, hit testing and the layout cache (step 9). 342
of the subset's 350 tests pass, and the 8 left wait on nothing in this
milestone. Twenty reference pages, box-only, match the reference
browser byte for byte (step 10); they found a cleared box measuring its
clearance against its own floats. Five real layouts with text, a
navigation bar, a card grid, the holy grail, media objects and an
application shell, match it within a pixel in every element (step 11),
once the text crate took the reference browser's vertical metrics,
`hhea`'s rounded to whole pixels, and its glyph advances, hinted as
FreeType hints them and placed on whole pixels; `about:silver` lays its
cards out with flexbox. The flex benchmark (step 12) found the pass
measuring content widths afresh at every level of a nested chain, the
square of its depth; it keeps them now, and 64 levels take 25ms rather
than 210. The benchmarks before M17 stay within T4.3's 10%
(`docs/perf-budgets.md`), and the three fuzz targets that lay pages out
ran fifteen minutes each with flex pages among their seeds, without a
crash; `style_document` kept an input whose cascade costs the square of
the tree's depth for every element, since the selector matcher forgets
its failures between elements (0.9s for 3KB, 0.8s before M17). Not
in this milestone, said rather than hidden: writing modes and
`direction: rtl` (M42); `visibility: collapse`; aspect ratios and
replaced items (M23); css-sizing-4's functions and keywords; the
multi-line column container's max-content width; fragmentation (M20);
and what the milestone found outside flexbox: the body's `overflow` not
carried to the viewport, classic scrollbars, two tables' unbounded
max-content widths, quirks mode (D14), `vertical-align` other than
`baseline` (M7.7), `font-weight`, and the stack that layout's
recursion needs: 2.3MB in release for the 511 levels the parser
allows, more than the 1MB of a Windows main thread, where the shell
lays out (some 200 levels fit). A grid of 1,000 cards takes 2.5s to lay out, nearly all
of it text measurement that parses the font's face on every call
(M35.8), and an appended paragraph's frame is 8% slower than before
the milestone for the larger computed style that every fragment copies
(M35.9). 1,155 tests workspace-wide. Next milestone: grid (M18).

Milestone 18 (grid), designed in `docs/design/grid.md` and built in its
twelve steps. The cascade (step 1) knows `display: grid` and
`inline-grid`, every grid property and shorthand, `justify-items` and
`justify-self` with `legacy`, the `place-*` shorthands' justify halves
and the legacy gap names; the grid properties live behind one pointer in
the computed style, `GridStyle`, so the style grows by eight bytes. The
box tree (step 2) has a `Grid` kind whose items the builder makes as it
makes a flex container's; `layout::grid::placement` resolves every
placement form, runs css-grid-2 §8.5's auto-placement sparse and dense,
by row and by column, grows the implicit grid, and keeps the lines
inside ±10,000. `layout::grid::tracks` (step 3) is §12 for one axis,
sizing columns and then rows with the items' contributions, the spanning
items' distribution of §12.5.1, `fr` in definite and indefinite space,
`fit-content()`, the automatic minimum of §6.6 and `auto-fit`'s
collapse; `layout::grid` lays the items out in their areas through the
pass's kept layouts, sizes the container, resolves percentage tracks and
gaps against the size they help find, and gives the intrinsic sizes that
`inline-grid`, floats and cells need. The web-platform-tests subset
(step 4) took 217 of `css/css-grid`'s tests by M17's rule. Then came
alignment (step 5), its arithmetic moved into `layout::align` for flex
and grid alike; baseline groups in both axes with their shims, and a
grid's own baselines (step 6); the grid area as the containing block of
absolutely positioned boxes, and a child's static position in its area
(step 7); and items painted as inline-blocks in order-modified document
order, with `z-index`, hit testing and the layout cache (step 8). 213 of
the 217 grid tests pass, 559 of the 571 with flexbox's; of the 12 left,
two found `vertical-align` on an inline box (M7.7), and none waits on
grid. Twenty box-only reference pages, 50 to 69, match the reference
browser byte for byte (step 9), each page's declarations checked against
its computed values so that agreement is not two engines ignoring the
same rule. Six real layouts with text, a grid garden of twenty-four
levels, a dashboard, a photo gallery, a form, a page of named areas and
a calendar month, match it within a pixel in every one of their 363
elements (step 10); `about:silver` lays its tiles out by grid. The grid
benchmark (step 11) shows a chain of 64 nested grids costing what 8 do a
level, and the benchmarks before M18 stay within T4.3's 10%
(`docs/perf-budgets.md`), the one that read over under cachegrind within
0.2%; the three fuzz targets that lay pages out ran fifteen minutes each
with grid pages among their seeds and the grid keywords in a dictionary,
and found no crash, hang or input over the memory limit in 8,311, 6,180
and 259,818 runs. Where the reference browser parts from the drafts,
this engine follows the drafts and the note records each with the
reference browser's numbers: four corners of §12's track sizing, a grid
item's margin box in the scroll extent, an item whose percentage size
depends on an intrinsic track taking no part in baseline alignment, the
max-content shim across, and a fixed child's static rectangle. Not in
this milestone, said rather than hidden: subgrid and masonry (G1, G2);
self-alignment of an absolutely positioned box in its inset-modified
containing block (G3, found here, positioning's); writing modes and
`direction: rtl` (M42); aspect ratios and replaced items (M23);
fragmentation (M20); `calc()` in tracks (M21); the resolved track values
of `getComputedStyle` (M25); the line limit, where the reference browser
allows far more; and what the milestone found outside grid: the `inset`
shorthand not parsed, `vertical-align` other than `baseline` (M7.7), the
layout unit's rounding against the reference browser's 64ths of a pixel,
and absolutely positioned boxes inside floats placed before the float
was, fixed here with tests. 1,272 tests workspace-wide. Next milestone:
compositing, transforms and accelerated scroll (M19).

Milestone 19 (compositing, transforms and accelerated scroll), designed
in `docs/design/compositing.md` and built in its eleven steps. The
cascade (step 1) takes `transform` with its 2D and 3D functions,
`transform-origin`, `transform-box`, the individual `translate`,
`rotate` and `scale`, `transform-style`, `perspective`,
`perspective-origin`, `backface-visibility`, `opacity` and
`will-change`, behind one pointer in the computed style, which grows by
its eight bytes; `style::transform` owns the four-by-four matrix, exact
at the right angles as the reference browser's rotation is, its numbers
clamped so that every matrix stays finite. Layout (step 2) makes a
transformed, perspective, `preserve-3d` or `will-change` box a
containing block for its fixed descendants, measures the scroll extent
by whatever content reaches, a transformed box by its projected bounds,
and shares one stacking-context predicate with paint. The display list
(step 3) gains groups, `PushGroup` with a matrix, an opacity, a key and
a layer reason, and `PopGroup`: a group that only translates and scales
is drawn through the raster, text rasterized at the size the scale
makes, and any other from a scratch, resampled bilinearly by the
backend's own inverse mapping, the partial redraw mapping damage through
the groups and kept exact. Hit testing (step 4) maps the point through
the inverse matrices. Layers (step 5): a group whose matrix turns, skews
or is projective, or whose `will-change` asks, is kept as a pixmap under
its key and composited by its matrix, drawn again only where its own
items changed, under a budget of 64 MiB (`--layer-budget`) with the
least recently composited let go; the document's content is groups of
its own, translated by the scroll offset rounded to whole pixels, kept
as tiles 256 pixels tall, a margin a viewport's height either side
listed and drawn ahead, and the diff of two lists became a longest
common subsequence so that content entering the margin damages its tiles
alone. 3D (step 6): `preserve-3d` contexts whose members carry
accumulated matrices, painted by depth, furthest first, perspective on a
parent's children, hidden backfaces neither painted nor hit, projective
composites resampled through the inverse. The web-platform-tests subset
(step 7) took 325 tests of `css/css-transforms` and
`css/css-will-change` by M17's rule; 866 of the subset's 896 pass. The
GPU backend (step 8), `render::gpu` on wgpu behind the `gpu` feature,
draws the same lists with four pipelines, the glyph masks and polygon
coverage shared with the software backend, keeps the same layers as
textures and reads the frame back for softbuffer; the shell draws on it
unless `--software` is given or there is no adapter, and an equivalence
suite holds it to the software backend within two levels inside shapes
and sixty-four at edges over every reference page, every layout and
twelve pages of its own, on Mesa's software Vulkan driver here and on
CI. Ten reference pages, 70 to 79, and four acceptance layouts (step 9)
found a border's corners covered twice under a scale, now one `Border`
band, a perspective lost through an untransformed `preserve-3d` root, a
plane past its horizon not drawn, and a scaled layer drawn soft, now
rasterized at its scale. The compositing benchmark (step 10) measures a
scroll of the mutation page at 2.0–2.6ms a frame on the software
backend, where a frame moves the last one's pixels by the scroll and
draws the strip it uncovers, the tiles of the margin drawn ahead after
the frame is presented (`Engine::draw_ahead`), no tile rasterized in a
frame; a turned card at 11–12ms, its layer composited, most of the frame
the card's layout again; and on the GPU backend, on the software driver,
11–13ms and 21ms. T4.3 found the frame drawing ahead to its deadline,
the paint walk projecting every fragment's corners, layout walking every
fragment twice, and tiles drawing every item of the margin, mended; what
remains over its line, the first frame of a page drawing its rows of
tiles, and an appended paragraph's frame, its layout passing the
last-level cache with fragments 32 bytes larger and its paint visiting
every fragment, is written down in `docs/perf-budgets.md` with M35.10,
M35.9 and M35.4 to undo it. The three fuzz targets that lay pages out
ran fifteen minutes each with the transform keywords in a dictionary and
found no crash, hang or input over the memory limit in 3,780, 3,320 and
229,750 runs. Not in this milestone, said rather than hidden: animations
(M22), and a change of `transform` alone laying the box out again
(M35.9); raster at the device transform and layers above the document's
tiles (M35.10); presenting the GPU frame without the readback (M37);
plane splitting; `filter` and the other grouping properties; the
horizontal scroll extent; and sixty frames a second measured on an
integrated GPU, there being none here (M19.8). 1,356 tests
workspace-wide. Next milestone: media queries and printing (M20).

Milestone 20 (media queries and printing), designed in
`docs/design/media.md` and built in its twelve steps. `style::media`
(step 1) parses Media Queries 4's lists, with Level 5's preferences and
the Compatibility Standard's `-webkit-device-pixel-ratio`, and evaluates
them three-valued against `style::Media`, the medium, the viewport's or
page's size, the resolution and the user's preferences;
`style::supports_condition` decides `@supports`, declarations by what
the cascade takes and `selector()` by what the selector parser and
matcher take; parentheses nest 32 deep in both. The cascade (step 2)
reads its sheets into a rule set, `@supports` decided once and every
`@media` list answered at each cascade, the rules in force those inside
no false list; cascade layers in their three forms order the author's
rules by their first appearance among the rules in force, before
specificity, `!important` reversing them. The loader gives an
`@import`'s rules and a `media` attribute's sheet to the cascade wrapped
in their conditions and layer, fetches every sheet and lets only those
whose media hold block rendering; the answers are kept with the styles,
and a frame whose environment answers otherwise cascades again
(`Stylesheets::media_changed`). The user's preferences (step 3) come
from the shell: `--prefers-color-scheme` and
`--prefers-reduced-motion`, winit's window theme and its changes, and
reduced motion from Windows and macOS through `windows-sys` and
`objc2-app-kit`, to `Engine::set_preferences`. The web-platform-tests
subset (step 4) took 68 tests of `css/mediaqueries`,
`css/css-conditional` and `css/css-cascade`; 927 of its 974 pass, the
milestone's seventeen failures each for a part of the engine the
milestone leaves alone. Printing: `@page` with its selectors, `size`
and margins, the break properties, `orphans` and `widows` behind one
pointer in the computed style (step 5); `layout::paginate` (step 6), a
pass over the laid-out fragments, iterative, that reads them as lines,
monolithic boxes and empty space and the places between, takes the
latest break that breaks none of Fragmentation 3's rules, letting them
go in its order, and moves what follows each break by its strut, a
forced break keeping its margin and a side adding a blank page, at most
`layout::MAX_PAGES` pages; and `Engine::print` (step 7), a cascade,
layout and pagination of its own under the print medium, each page
painted with its fixed boxes, drawn at 300 dots per inch through a
scaling group on a software backend of its own and compressed at once,
and the PDF written by `pdf-writer`, the same bytes each time; the
shell prints with `--print-to-pdf`. A reader of the reference browser's
PDFs (step 8), shared by `examples/print.rs capture` and
`tests/print.rs`, holds the engine's pagination of seven print pages,
word by word, to the reference browser's, and found its page area
rounded, up for layout and pagination and down for the viewport units,
and a box avoiding breaks broken after its padding, mended. Eleven
reference pages, 80 to 90 (step 9), cover the breakpoints a pixel
either side, orientation and aspect ratio, combined and unknown
queries, `@supports`, layers with and without `!important`, the print
medium on the screen (`Engine::set_media_type`) and the preferences,
which a page now asks for in its head. `about:silver` (step 10), now a
file of its own, stacks its cards and widens its tiles at its
breakpoints and prints to three pages, every word where the reference
browser's print puts it; it found the user agent sheet's pixel
approximations of the HTML standard's `em` margins and heading sizes,
now the standard's; and a responsive page of layers and feature
queries is laid out as the reference browser lays it out at 560, 800
and 1100 pixels. The media benchmark (step 11) answers 1,000 lists
again in 15–20µs, prints 1,000 paragraphs to 53 pages in 5.9s and 21.8
MiB, and found the print resampling the document's layer, its text
blurred and 33.5s spent, and holding every page's raster, 1.28 GB, both
mended; T4.3 found every other benchmark within 10%; the `media_query`
target and the three that lay pages out, `pipeline` now printing, ran
fifteen minutes each and found no failure. Not in this milestone, said
rather than hidden and registered (G4 to G13): a layout that knows its
pages, flex, grid and tables broken inside; a PDF of text and paths and
the page-margin boxes; `color-scheme`; `revert-layer`; `@container`,
`@scope` and nesting; the preferences on Linux; line breaking by UAX
#14 and `white-space: nowrap`; the root's margins collapsing through
it; and what the subset asks of the rest of the engine. 1,429 tests
pass workspace-wide, two ignored. Next milestone: custom properties,
`calc()`, counters and generated content (M21).

Milestone 21 (custom properties, `calc()`, counters and generated
content), designed in `docs/design/values.md` and built in its twelve
steps. Declarations keep a function's block whole and a custom
property's name and `{}` blocks as written, and the cascade's parsers
take component values, so a shorthand splits at nesting. `style::calc`
(step 1) parses `calc()`, `min()`, `max()` and `clamp()` 32 deep, types
them by Typed OM's algebra, simplifies them where a value is computed
and keeps an expression with a percentage in it, `Calc(Arc<Calc>)` in
each of the 22 percentage-holding fields, for layout to resolve and
clamp; every property, shorthand, grid track and line, transform,
`@page` value and media feature takes them. `style::custom` (step 2)
computes custom properties into a copy-on-write trie by a keyed hash,
shared with the parent where the winning declarations are the same, and
substitutes `var()`, `env()` and (step 3) the typed `attr()` by Values
5's Appendix A, on a stack of its own with cycles marked on it, a
cascade keeping what it substituted by declaration and map; a value
invalid once substituted is `unset`. Counters (step 4) are the box
tree's: `box_tree::generated` walks the document ahead of the build,
when the build first asks for a generated text, writing quote marks,
`counter()` and `counters()` and markers in `style::counter_style`'s
predefined styles, by the reference browser's scoping where the
`counters-scope` tests hold it with the browser, and the layout cache
keys a box's results on a fingerprint of the generated text and
decorations in it. List items (step 5): `display: list-item`,
`::marker` styled for list items, HTML's rules and hints, inside and
outside markers, text or shapes drawn by the reference browser's
measures, an outside marker moved into the first line or beside
content with none. Text decoration (step 6) is propagated through the
box tree by ancestry and its lines resolved by layout into the
fragments it decorates, painted under and over the text by Text
Decoration 4's order; `text-align`'s values and longhands and
`text-justify` (step 7) justify a line's slack across its gaps, or its
characters too. The web-platform-tests subset (step 8) took eleven
directories, 729 tests, 1,579 of its 1,703 passing, and found
percentages of zero, a `<declaration-value>` check, counter overflow,
a space after an empty inline box, markers beside floats, a block's
decoration in its own font and the user agent sheet's `body` rule,
mended. Reference pages 91 to 99 and print pages of counters, lists and
justified text (step 9), held to the reference browser's PDFs by a
reader that now joins text objects that touch, found the decorations'
second lines and rounding, the width of a gap and a right-to-left
marker's order. `about:silver` (step 10) is themed by custom properties
under each scheme and in print and numbered by counters, and a theming
demo is drawn both ways against the reference browser. The values
benchmark (step 11) cascades a theme of 200 properties in 4.4–4.7ms
against 4.0 written out, and a page built to exhaust the cascade's
bound in 0.35–0.44s and 225 MiB; writing the `values` fuzz target found
infinite and NaN percentages, which every resolution of a percentage
now censors, and the `mutation` target a marker beside its content lost
by the layout cache, mended; T4.3 found the box tree doing the
milestone's work for every document, mended, and the larger style
copied with every fragment the layout cache gives back, an appended
paragraph's frame 25% longer and the pseudo benchmark's 40–53%, the
exception written in `docs/perf-budgets.md` with M35.9. Security, until
`docs/security.md` (M33): a math function nests 32 deep; a substitution
weighs a mebibyte at most (`style::custom::MAX_SUBSTITUTION`) and a
cascade's substitutions 256 MiB (`MAX_CASCADE_SUBSTITUTION`), by the
memory their tokens take, so thirty properties each doubling the last
stop at the fourteenth and a page of 1,000 such elements at 225 MiB; a
chain of custom properties is resolved without recursion, a cycle found
at the same cost however long; a counter keeps to 32 bits, a set to
1,024 counters and a walk to 2²⁰; a pseudo-element's text is a mebibyte
at most (`layout::MAX_GENERATED_TEXT`) and a walk's 256 MiB; a counter
style's representation is 60 code points
(`style::counter_style::MAX_REPRESENTATION`) and eight fallbacks; every
length layout resolves from a percentage is within `MAX_LENGTH_PX`;
`attr()` can put an attribute on the screen and never in a URL, which
M23 must keep so; and the `values` fuzz target holds the engine to all
of these from arbitrary bytes. Not in this milestone, said rather than
hidden and registered (G14 to G25): registered custom properties; the
other math functions; colour functions; `@counter-style`; `env()` and
`var()` outside properties; markers on pseudo-elements; ink skipping and
the rest of text decoration; justification beyond expanding; the fonts'
remaining properties; `display: flow-root`; logical properties and
`:dir()`; and a grid's second sizing for percentages. Images in
`content` and lists are M23's. 1,529 tests pass workspace-wide, two
ignored. Next milestone: transitions and animations (M22).

Milestone 22 (transitions and animations), designed in
`docs/design/animations.md` and built in its thirteen steps. The cascade
(step 1) takes the five transition and eight animation longhands, their
shorthands and the Compatibility Standard's `-webkit-` names, computed
behind one pointer, `ComputedStyle::motion`, the style 656 bytes;
`style::easing` evaluates the easing functions, cubic Béziers solved for
x and extended along their tangents, steps with the before flag and
`linear()` canonicalized, an output kept within 10⁹ of zero
(`MAX_PROGRESS`); the css crate parses `@keyframes` and
`@-webkit-keyframes` blocks. `style::animate` (step 2) holds the
longhands' animation types and interpolates each computed value, colours
premultiplied, integers rounded a half up, a length and a percentage as
their sum, everything clamped to its property's range, `display` by
Display 4's rule; transforms by Transforms 1 §9, pairs of a primitive
and the rest decomposed as matrices, a rest with a translation's
percentage kept as an interpolated transform function for its box,
nested eight deep at most (`MAX_MIX_DEPTH`). The rule set keeps
keyframes by name, the winner by layer, media and order, processed once
for every element that names them, and `style::animation` (steps 3 and
4) holds Web Animations' timing model and a document's animations: the
cascade applies their values between the normal and the important
declarations in each of its passes, each keyframe computed against the
element, an implicit keyframe its underlying value; `::before`,
`::after` and `::marker` animate; a custom property an animation sets is
tainted; and while one runs, the element acts as if `will-change` named
what it animates (`Effects::animated`). The engine runs the document
timeline on `frame_at`, `tick` and `frame`, an animation starting at the
first frame after it is made; each frame restyles the targets whose
animations moved; `needs_frame` and `next_frame_at` say when a frame is
wanted, and the shell waits out a delay with `WaitUntil`. Transitions
(step 5) follow Transitions 1 §3 to the letter, each element's
after-change style inheriting from its parent's, with Transitions 2's
`transition-behavior`; a whole cascade is a change of style from the
styles it replaces; an element shown by a transition of `display` alone
is inert, and hit testing passes over it. The events of both drafts'
tables (step 6) are queued at each frame, `Engine::animation_events`
giving them until the event loop dispatches them (M26.1). A change of
`transform`, `translate`, `rotate`, `scale` or `opacity` alone that
leaves the box what it was to layout (step 7) is composited:
`layout::composite` writes a restyle's changes into the fragments of the
layout and of the layout cache in one walk, measuring the scrollable
overflow again where a transformed box reached its scroll container's
bottom or the document's, and the layout is kept, as it is through a
frame whose changes reach nothing layout reads (`Engine::layouts_kept`);
writing it found two M19 raster bugs, a group's opacity and its layer
missing from the damage state, mended. Under
`prefers-reduced-motion: reduce` (step 8) the engine's own scrolling
jumps; CSS animations and transitions run as written, the page's own
query being its means of honouring the user. The web-platform-tests
subset (step 9) took `css/css-animations`, `css/css-transitions` and
`css/css-easing`, 36 tests joining, 1,610 of 1,738 passing; the four new
failures are gaps outside the milestone. Reference pages 100 to 106
(step 10) hold the values of each type, every easing, the timing,
transforms, keyframes and transitions to the reference browser's, a page
asking for changes after its first frame with
`<meta name="reference-change">` and `<meta name="reference-time">`; 107
to 109 are the acceptance's demos (step 11), a spinner and a colour
fade, whose frames on the software backend take at most 6.1ms, none near
the budget, and on this machine's GPU backend, llvmpipe, which
rasterizes on the CPU's cores, keep it by their medians with a single
frame of 20 to 130ms in its raster in some runs, the code before the
milestone's last changes alike; `tests/demos.rs` holds that the
spinner's frames lay nothing out and draw its layer once, and the fade's
lay out the card alone. The animation benchmark (step 12) measures a
transform animated on one of 1,000 paragraphs at 2.7–3.4ms a frame,
laying nothing out, a colour there at 8.5–9.6ms, an opacity on every
paragraph at 23.5–26.8ms, over the budget (M35.2), and a class change
under `transition: all` at 19.9–21.2ms; it found `layout::composite`
walking the layout once for each change, a thousand opacities 406ms a
frame, and the restyle counting an element changed when only the
after-change style it keeps beside its style did, so that a transition's
start laid the page out, both mended. T4.3 found the previous frame's
layout freed inside the next frame, a default `Motion` built to compare
every computed style with, and the empty tables of animations hashed
into for every element, mended: the cascade does 4% more work than
before M22, and every benchmark is within 10% or does the same work, by
callgrind's counts. The `animations` fuzz target holds easing functions,
timing, interpolation and frames drawn at a series of times to their
bounds; the `mutation` target's styles take transitions and animations,
and its comparison finishes the engine's transitions as `finish()` would
(`Engine::finish_transitions`), which found a change no frame had drawn;
the targets ran fifteen minutes each and found nothing else. Security,
until `docs/security.md` (M33): a document runs at most 65,536
animations and transitions together
(`style::animation::MAX_ANIMATIONS`), past which a new animation does
not start and a new transition's property takes its value at once;
keyframes are kept once for each rule and a sample computes the two
keyframes around its progress for each property, so its work is bounded
by the sheets and that bound; an interpolated transform nests eight
deep; times are `f64` seconds, infinities clamped and `NaN` zero, and an
easing's output is finite and within 10⁹ of zero; a frame's events are a
few for each animation; the layers animations make keep to M19's layer
budget; and the `animations` fuzz target holds the engine to these from
arbitrary bytes. Not in this milestone, said rather than hidden and
registered (G26 to G33): the Web Animations API; scroll-driven
animations; `@starting-style`, `overlay` and `interpolate-size`; a
compositor thread; a user's setting to stop or slow animations; what the
subset asks of the rest of the engine, shadow trees and the root's
overflow carried to the viewport; `border-radius`, `box-shadow` and
`filter`; and a transition's side effect. Dispatching the events
(M26.1), hover starting transitions (M26.4), a layer's raster scale kept
through an animation (M35.10) and a colour's animation laying nothing
out (M35.9) are at their milestones. 1,628 tests pass workspace-wide,
two ignored. Next milestone: images (M23).

Milestone 23 (images; `docs/design/images.md`): a new leaf crate,
`images` (step 1), reads PNG and APNG (`png`), JPEG (`zune-jpeg`,
without its SIMD and with it every `unsafe` block), GIF (`gif`) and WebP
(`image-webp`), a header apart from the pixels, EXIF orientation by a
reader of its own, an animation's frames composed with each format's
disposal and blending; every decode is bounded (`MAX_IMAGE_BYTES`,
`MAX_IMAGE_SIDE`, `MAX_IMAGE_PIXELS`, `MAX_FRAMES`,
`MAX_ANIMATION_BYTES`) and runs inside `catch_unwind`, so a hostile
image is a broken one. The loader fetches images (step 2) with the
destination "image", sniffing their type, an `http:` image of an
`https:` page upgraded, the body bounded, one URL once a document; the
engine's image store reads headers and decodes pixels on threads of its
own (`MAX_DECODE_THREADS`), and `Loaded` waits for the document's
images. An `img` (step 3) is a replaced box, or its alt text after a 16
by 16 icon, as the reference browser draws them, sized by CSS Images 3's
default sizing in block, inline, flex, grid and intrinsic sizing, with
`aspect-ratio`, `object-fit`, `object-position`, `image-orientation`
(`none` for an image of the document's origin alone) and
`image-rendering`. Paint's `DisplayItem::Image` and, for repeated images
(step 4), `DisplayItem::Pattern`, one item however many tiles, carry an
`ImageRef`, the image's number, frame, turn and crop, and both backends
draw them, the GPU's through its composite pipeline with its textures
under 256 MiB, a nearest sample between two texels taking the lower as
the reference browser's does. Backgrounds take every longhand and the
shorthand, layers, `image-set()`, `fixed` and `local`, and the canvas
takes the root's or the body's images over the whole document, so that a
scroll composites them; gradients (step 5), linear, radial and
repeating, are rendered by paint's own code into an image both backends
draw, a large one on several threads, hints as the reference browser's
nine stops. Border images (step 6) cut the image in nine by the crops of
their display items; markers and `content` take images, an element whose
`content` is an image alone being replaced. An animated image (step 7)
has one timeline a document, from its arrival on the document timeline;
its frame is chosen as a frame is drawn, and its next one, while it is
in view, is when the next frame is wanted. Pixels (step 8) are decoded
on first draw and kept under 512 MiB (`MAX_DECODED_BYTES`), those drawn
longest ago dropped; an image drawn at under half its size is decoded to
a box-filtered copy at twice it; `Engine::decoding` says a decode is
under way. Reference pages 110 to 115 (step 10), loaded by their `file:`
URLs, match the reference browser byte for byte but the gradients',
within 2. The photo grid (step 11, `tests/demos/photo-grid.html`)
scrolls at sixty frames a second on both backends, a frame's median
0.44ms on the software backend and 1.7ms on the GPU's, nothing laid out,
and `benches/images.rs` measures it and the costs; it found the canvas's
images drawn again at every scroll, 37ms a frame, and gradients taking
54-67ms to render for a viewport, both mended. T4.3 holds every
benchmark within 10%, the grid's gallery once 12% by four rounds'
medians and faster over six more. The `image_decode` fuzz target found a
frame of no area cut into rows zero bytes apart, a panic the decode
thread caught, mended; the targets that lay pages out ran with the new
properties and found nothing. The web-platform-tests subset's image
directories (step 9), pulled once the checkout was on the machine, add
402 tests, 2,140 in all, with the images they use; the runner waits for
decodes. They found 31 failures that were this milestone's, all mended:
replaced boxes' content sizes, which now leave their own axis's limits
out; flexbox's case B and a replaced item's width from the height its
container gives it, in flex and grid alike; `image-orientation` on
backgrounds and border images; repeating gradients of no period;
`fixed` backgrounds in transformed boxes and on a fixed root; a
declaration with white space before its colon. The 31 left are each
outside it and listed with their reasons. Not in this milestone, said rather than hidden and registered
(G34 to G40): AVIF; `srcset`, `sizes` and `<picture>`; `aspect-ratio` on
boxes that are not replaced; Images 4's conic gradients and the rest;
colour management; progressive display; `background-position-x` and
`-y`. SVG as an image moves to M32.1. 1,750 tests pass workspace-wide,
two ignored. Next milestone: JS engine integration (M24).

Milestone 24 (JS engine integration; `docs/design/scripting.md`): a new
leaf crate, `js` (step 1), holds the `ScriptEngine` and `Realm` traits,
the plain data that crosses the boundary, the limits and the QuickJS-ng
backend, 0.16.2 through `rquickjs` pinned at `=0.14.0` (D1), writing no
`unsafe` itself; a script reaches the document only through
`ScriptingHost`, moved into a run and back. A realm runs a classic
script with a step budget of interrupt checks, under a 256 MiB heap and
a 256 KiB stack, and the microtask checkpoint after it; a realm whose
heap stays full after a failed run is dead, and the document runs no
script again. The HTML parser is rewritten to run as bytes arrive: the
tokenizer (step 2) is HTML 13.2.5's state machine, fed in pieces, with
the full named reference table, held to html5lib's tokenizer tests; the
tree builder (step 3) writes into the `dom::Document` as it goes and
stops at every `</script>`; `net` hands bodies over in pieces as they
are read (step 4); and the engine (step 5) sniffs, decodes and parses a
navigation's document a slice a poll as it arrives, starting again for a
late `<meta>` charset, shows it once it has a body and no blocking
sheet, and lets it grow as a change like any other. Scripts are off
unless asked (risk R2): `Engine::set_scripting`, from the shell's
`--enable-scripts` (step 6), turns on the parser's scripting flag, the
`scripting` media feature and the UA rule that hides `noscript`; a
document gets a realm at its first script; inline classic scripts run at
their end tags, errors and console output going to the document's
console, which the shell prints to the standard error; a realm out of
memory posts a notice. External scripts (step 7) are fetched as no-cors
scripts, held to an ok status and Fetch's type checks, decoded by BOM,
type, `charset` or the document's encoding, their errors muted unless
every URL was the document's origin; the parser waits for a plain one
and for an inline one a loading style sheet holds, deferred ones run in
order once parsed and async ones as they come, and `Loaded` waits for
them; `javascript:` links run in the page. `net::csp` (step 8) enforces
a page's `script-src`, from its header and its head's `<meta>`, on
script URLs at every hop, inline scripts and `javascript:` URLs, failing
closed on what it does not understand. The web-platform-tests subset
(step 9) runs scripts, and takes tests whose scripts use only what M24
gives: 2,154 tests, the 14 it adds all passing, and the two `scripting`
media query tests passing now. Reference pages 116 to 118 (step 10) run
their scripts and match the reference browser exactly. The acceptance
(step 11): `console.log("hello")` and `document.title = "x"` from an
inline script, a script error and a realm out of memory, in the shell
and its window; `benches/scripts.rs` makes a realm in 0.12ms, runs two
million additions in 83ms, and loads a page of 1,000 paragraphs with 110
scripts in what it takes with scripts off. T4.3 holds every benchmark
within 10% but four cascades of the values benchmark, 12-28% slower,
which follow where memory falls rather than the work: the new UA rule is
never in force there, and builds that leave the same rules in force are
fast or slow by the sheet's contents alone; the loads, flex and grid are
8-13% faster. Fuzz runs of `script`, `csp`, `html_parse` and the targets
that lay pages out found nothing: the parser 2.7 million inputs, the
policies 5.6 million, scripts 660,000. Not in this milestone, said
rather than hidden and registered (G41 to G44): module scripts, import
maps and speculation rules; `Intl` and `Temporal`; Subresource
Integrity; the tree builder's missing insertion modes. The DOM's
interfaces and `document.write` are M25's, events and timers M26's.
1,831 tests pass workspace-wide, two ignored. Next milestone: DOM
scripting v1 (M25).

Milestone 25 (DOM scripting v1; `docs/design/dom-scripting.md`): scripts
get the DOM. Node objects are made in a bootstrap of JavaScript that
`js` runs when a realm is made (step 1), over handles (a node's slot and
generation as one number, `dom` retiring a slot after 2^21 generations),
one object a node held weakly, or strongly while its node is in the
document, released to the host when collected; its natives, out of a
page's reach, call the host through one method, `ScriptingHost::dom`, a
`Call` and its `Answer`, and the host charges its work to the step
budget, a step for every 10,000 units, refusing once the budget is
spent. `dom` counts the bytes its nodes hold (256 MiB at most) and gains
a version for live collections and a journal the host reads. Detached
subtrees no object reaches are freed after each run. The tree read and
written (step 2) with the DOM's checks and the reference browser's
messages, live and static collections; the selectors (step 3), with
`:scope`; markup (step 4) through an HTML serializer and a fragment
parser in `html`; scripts that scripts insert, run nested before the
call that connected them returns, and `document.write` at the parser's
insertion point, a stack of positions in its input, written scripts
inline and external, six levels deep at most (step 5); `classList`,
`style` over the attribute, `dataset`, reflected attributes and
listeners kept for M26 (step 6); and `window` v1 (step 7): a script's
navigations with HTML's handling, `navigator` and `screen` by D12 with
the shell's `--compat-user-agent`, which net's requests follow too,
`getComputedStyle` over a copy of the cascade restyled by the journal
and serialized by `style::serialize`, and `noscript` computing `inline`
and making no box. `benches/dom.rs` (step 8) builds a table, toggles a
class on it and edits a cell by script, a frame after each: its first
finding, every word shaped again at every pass, was fixed by a cache of
widths in `text::FontCollection` (half of M35.8; a 1,000-row table's
layout from 1,135 ms to 68 ms, the load benchmark's first frame from 812
ms to 61). The frame after a cell's edit is 4 ms at 100 rows; the others
are past the budget, as the reference browser's own style and layout are
for a toggle at 1,000 rows, and what keeps them there is M35.9's and
M35.11's. A new fuzz target, `dom_api` (step 9, T2.4), writes calls
drawn from M25's surface out as scripts, some run as the page is parsed
and the rest after frames through `differential::run_script`, the
document validated and the frame held to a fresh engine's. The
web-platform tests' rule (step 10) takes scripts that use M25's surface
and nothing it leaves to others, with three more directories: 2,234
tests, 80 new, the 12 of them that fail each a gap outside M25; they
found `getComputedStyle` lacking `transform-box`. Pages 119 to 122 (step
11) match the reference browser exactly, and found two things, mended: a
page loaded before its first frame read a 0 by 0 window
(`Engine::set_viewport`), and the cascade read no `rgb()` (now `rgb()`
and `rgba()`, part of G16). The acceptance (step 12): a page that builds
a table and styles it by a class toggle, in the shell's window and its
PDF; T4.3 finds every first layout of a page of text 7 to 25 times
faster, the values cascades back at M23's times and two restyles 18% and
58% slower, the cascade following its memory as M24's found, and a realm
40 times slower to make, its bindings compiled each time (M35.12); fuzz
runs of `dom_api`, `script`, `html_parse` and the targets that lay pages
out found two things: the `script` target's stack too small for the
bootstrap, and, in `pipeline`, a page nested 31 levels deep overflowing
the box tree's recursion under the address sanitizer, which in release
overflowed Windows' 1 MiB main thread at 160 levels, in M24's tree as
well: the shell's main thread now has `engine::STACK_SIZE`, 32 MiB, and
a test draws pages at the parser's cap on a thread of that size;
building and laying out boxes without recursion is M35.9's. Not in this
milestone, said rather than hidden and registered (G46 to G55): a style
and layout flush for scripts; comments, fragments and the other kinds of
node; `document.open`; `MutationObserver`; CSSOM's serialization and
style sheets; namespaces; traversal and ranges; custom elements and
shadow trees; the window's named properties; a viewport for
`--print-to-pdf`. Events, timers and form controls' state are M26's.
1,902 tests pass workspace-wide, two ignored. Next milestone: event
loop, events, input (M26).

Milestone 26 (event loop, events, input; `docs/design/events.md`): pages
become interactive. HTML's event loop is the engine's (step 1):
`run_tasks` runs the due timers of the document shown and of one on its
way, a microtask checkpoint after each, within a 50 ms slice, and
`next_task_at` says when to wake; every time the engine reads comes from
one clock it is given, the shell's, a hand's in tests, so that no test
waits. The realm is entered by a `js::Task`, its callbacks kept in the
bootstrap, which hands `js` its entry points. The DOM's events (step 2):
the classes, `AbortController`, listeners with their options, a dispatch
the realm steps as a generator so that the engine's dispatches take a
checkpoint after each listener, handlers as IDL and content attributes
compiled when first needed, the `error` event and `reportError`. The
rendering steps run at a frame's start (step 3): `resize`, `scroll`, the
animation and transition events a frame queued, the animation frame
callbacks; and the document's lifecycle, elements' `load` and `error`,
`pagehide` and `unload`, a hidden page's throttling. `dom` keeps each
element's state flags, journaled and restyled by where the selectors
read them (step 4): `:hover`, `:active`, `:focus`, `:focus-visible`,
`:focus-within`, `:target`, with `outline` and `cursor`. The pointer
(step 5) and the keyboard and focus (step 6) reach the page as the
reference browser's events in its order, with their default actions:
links followed as their clicks' activation (the shell no longer follows
them), focus moved by presses and by Tab through the sequential order
and back to the address field, keys typing into controls, the scrolling
keys and the wheel moved into the engine. Form controls (step 7) keep
their state in `dom`, given the IDL, and show it: a field's value or
placeholder, a checkbox's and a radio's marks drawn by paint, a
drop-down's chosen option and arrow, a list box's rows; the UA sheet
takes the reference browser's computed values and the box tree its
sizes. Editing (step 8) with a caret and a selection, the clipboard and
an input method's commits, `beforeinput`, `input` and `change`, the
model `text::edit` shared with the address field; the select's list the
engine draws. Forms are submitted (step 9) in every encoding, by GET and
POST, with their history and a resubmission page, by a button, by Enter
or by script, reset, and validated: the validity states and messages in
the reference browser's words, `pattern` matched by
`js::pattern_matches` in a context of its own whether the page's scripts
run or not, the `invalid` event and the form pseudo-classes. Fuzzing
(step 10): `dom_api` gains M26's calls and runs on a hand clock, and a
new target, `input`, uses pages of controls through the engine's input;
both compare with a fresh engine, whose snapshot is now given the state
no markup holds. The web-platform tests (step 11) run `reftest-wait` and
`test-wait` pages on a hand clock as `wptrunner` does, the rule taking
M26's scripts and controls and two directories: 2,432 tests, 2,254
passing, each failure named with its reason; on the way they found an
animation event that waited for an unrelated frame, events sampled
before the frame's own restyle, `:focus-within` dropped while the focus
moved within it, `getElementsByName` missing, `overflow: clip` treated
as a scroll container, checkboxes that could not stretch, and options
outside a select hidden. Pages 123 to 125 (step 12) match the reference
browser exactly with timers, frames and events run on its virtual time,
and a page of controls' boxes found that a text field, a textarea and a
select took their sizes from CSS: their sizes are the box tree's now, so
they stretch in flex and grid as the reference browser's do. The
acceptance (step 13): the to-do list and the search form through the
shell's window, the search with scripts on and off, by Enter and by its
button (finding `document.forms` missing), and the to-do page in the
real shell; T4.3 finds the restyles 9% to 16% slower, the user agent
sheet's 13 new rules tried on every element (M35.2), and a frame that
starts 2,000 transitions 31%, sampled again for their events: each
transition's name is now made only for an event (step 13b), and the 2ms
left is their events, made a frame sooner; fuzz runs of `dom_api`,
`input`, `script`, `pipeline`, `style_document` and `mutation` found one
thing, a form's `:invalid` left stale when a control's attributes, place
or form changed, every form and fieldset now restyled with them. Not in
this milestone, said rather than hidden and registered (G56 to G66):
dialogs the user answers; `beforeunload`; pointer capture, touch and
drag and drop; editing hosts; composition; pickers; undo; `dirname`; a
validation bubble; observers; `moveBefore`. 2,063 tests pass
workspace-wide, two ignored. Next milestone: fetch, SOP, CORS, cookies
(M27).

Milestone 27 (fetch, the same-origin policy, CORS and cookies;
`docs/design/fetch.md`): a page's scripts reach the network, inside
walls. A document keeps its origin (step 1), an opaque one made once,
and sites come from the Public Suffix List (D20, the `psl` crate):
`self.origin`, `isSecureContext` and `document.domain`. `net`'s request
becomes Fetch's in the parts this milestone needs (step 2): any method
token, the author's headers by Fetch's lists (`net::headers`), the
modes, the credentials, the redirect modes, `Origin` and its tainting
across redirects, `Sec-Fetch-*`, the response's tainting and
`Response::exposed`, the only view a realm is given, and
`Cross-Origin-Resource-Policy` on opaque responses. CORS (step 3): the
check on every hop, the preflight and its cache, the `crossorigin`
attribute for scripts and stylesheets, a script's errors unmuted by its
response's type. Cookies (step 4): `net::cookies`, RFC 6265bis-22's
user agent, one jar for the engine that every client it makes shares,
`Set-Cookie` stored on every hop and a document's own before its first
script, `Cookie` by the credentials mode and same-site,
`document.cookie`. The bodies scripts make (step 5): `Blob`, `File`,
`FormData` and `URLSearchParams`. `fetch()` (step 6): `Headers`,
`Request`, `Response` and the `Body` mixin; the realm describes a
request (`Call::Fetch`), the engine checks it all again
(`scripts::fetch::request`), fetches it on its pool and answers it as
a task (`js::Task::Fetched`) due when it came, in the order asked among
those that came at once; aborts by signal; bodies whole, cross the
boundary as binary strings one way and `ArrayBuffer`s the other.
`XMLHttpRequest` (step 7) on the same path, a synchronous request a
call that blocks the page, its events in the standard's order. Mixed
content (step 8): a script's `http:` fetch from an `https:` page
blocked on every hop, tested over TLS; why a script's fetch failed goes
to the console, never to the page. The adversarial page (step 9),
`tests/security/cross-origin.html`, makes 26 reads of another site
holding a secret and learns nothing, and by exactly the 11 CORS reads
once that site allows it, as the reference browser does. Fuzzing (step
10): a `cookies` target holds every kept cookie to the rules and the
header to the cookies that match; `dom_api` gains `fetch()`, XHR, the
bodies and `document.cookie`, its network settled after each run. The
web-platform tests (step 11) gain testharness.js: the real harness
under a report of the runner's, the page wptserve makes of an `.any.js`
test, a clock that waits for the network, expectations by subtest;
156 tests of Fetch, XHR, `URLSearchParams` and `Blob`, 1,124 of 1,469
subtests passing, the rest named, and on the way they found Web IDL's
conversions taken in the wrong order, `Blob.slice`'s rounding,
`base.href` unreflected, multipart padding and `HEAD`'s body. Pages 126
and 127 (step 12) match the reference browser with their fetches and
XHRs on its virtual time. The acceptance (step 13): a page signs in by a
`fetch` `POST` through the shell's window, reads its user with the
`HttpOnly` session cookie and another site by CORS with credentials,
and does the same in the real shell; `benches/fetch.rs` times a
thousand fetches at 80.6 ms from navigate to the last promise (the page
alone 16.7) and the jar of 3,000 cookies at 15 us a header; T4.3
finds a realm 5 ms slower to make, the bootstrap 40% larger for M27's
classes (loading it compiled is M35.12's); fuzz runs of `cookies`,
`dom_api`, `script`, `pipeline`, `http_response` and `url_parse`
found one thing, the `cookies` target's own check of expiry, which
now reads the jar as of its last store or read (step 13b). Not in
this milestone, said rather than hidden and registered (G67 to G77):
streams; the `URL` class and `blob:` URLs; `TextEncoder`; XHR's
documents; HTTP authentication; the Cookie Store API; requests that
outlive their page; WebSockets; `FileReader`; the `formdata` event; a
header section ended by the connection's close. 2,155 tests pass
workspace-wide, two ignored. Next milestone: storage (M28).

Milestone 28 (storage; `docs/design/storage.md`): the browser keeps
what browsing keeps. A new leaf crate, `profile` (D10: `redb` 4.3.0,
pure Rust, one file), holds each origin's `localStorage` items, keyed
by the origin's serialization so that an origin reads one range, and
the cookies' records, every write one transaction, and refuses a
profile in use, of a newer schema, or damaged (step 1). The shell opens
the platform's profile directory, `--profile`'s, or none with
`--temporary-profile`, and writes it when its window closes; the engine
takes the cookies with an expiry back into its jar, records of `net`'s
own encoding (`Jar::export`, `Jar::restore`), and writes what changed
at most once a second from its tasks and when dropped (step 2).
`localStorage` and `sessionStorage` (step 3): areas in the engine,
lent to a run as the jar is, local ones read from the profile at their
first use and written by the item, a tab's session ones in memory, a
quota of 5,242,880 UTF-16 code units each; `Storage` a legacy platform
object, a proxy with Web IDL's named-property traps;
`QuotaExceededError`; `StorageEvent` and the `storage` event between
the document shown and the one on its way; opaque origins, `file:`
included, refused. The
HTTP cache, version 0 (step 4): `net::cache`, RFC 9111's private cache
in memory for the `GET`s fetched whole, by site and URL with `Vary`'s
variants, fresh by `max-age`, `Expires`, `Age`, `Date` and the 10%
heuristic, read and written by Fetch's cache modes (`Request.cache`
real now), invalidated by unsafe methods, within 64 MiB. Isolation
(step 5): four origins read their own storage and cookies and none of
the others', in the session and after the profile is opened again, in
memory and on disk; the cache kept apart by site. Fuzzing (step 6):
`storage`, the engine's areas over a profile written, opened again,
copied as a crash leaves it and damaged, against a model
(`engine::StorageDriver`), and `http_cache`, freshness checked
independently; the `cookies` target's records, `dom_api`'s storage
calls; they and their tests found `redb` panicking on a damaged file,
now a profile refused, and records to hold to the storage model's
rules. The web-platform tests (step 7): 25 tests of `webstorage`,
1,239 of 1,251 subtests passing, the rest lone surrogates (G78); the
pull rule's frame check now reads any case. Page 128 (step 8), served
over `http:` so that it has an origin, matches the reference browser
exactly. The acceptance (step 9): a page counts its visits in
`localStorage` and a cookie across windows on one profile, and in the
real shell across runs, a window killed outright included;
`benches/storage.rs`, which found the jar's restore quadratic (now
11.5 ms for 3,000 cookies and 100 origins); T4.3 within 10% but for
three rows of code M28 does not change, their ranges the base's; fuzz
runs of `storage`, `http_cache`, `cookies`, `dom_api`, `script` and
`pipeline`, fifteen minutes each on six workers, found nothing. Not
in this milestone, said rather than hidden: IndexedDB (M36); the
cache on disk, revalidation and documents cached (M38); the Cache API
(M39); a user's view of what is kept (M36); session restore and the
`storage` event across tabs (M37); partitioned cookies and the
profile's encryption (M33); a profile a browser process owns (M34);
lone surrogates kept (G78). 2,221 tests pass workspace-wide, two
ignored. Next milestone: WebAssembly (M29).

Milestone 29 (WebAssembly; `docs/design/wasm.md`): a page's scripts
run WebAssembly. QuickJS-ng has none; the roadmap's contingency is
taken (D21): `wasmi` 2.0, an interpreter in Rust, in `js` beside it,
one store a realm made at its first module, its portable dispatch (its
tail-call dispatch overflowed the native stack on Windows). Modules
(step 1): `validate`, `Module` and its reflection in the module's own
order (read again with `wasmparser`), the interface's three errors, 64
MiB at most, compiled as their bytes are charged to the run. Instances
(step 2): imports read in the module's order with the reference
browser's words; exported functions, bound functions named by their
index and the same object wherever a function comes from (`wasmi`'s
arena key, read from its `Debug` form); a module's call into a
script's function stopped by `wasmi`'s resumable call and resumed by
the bootstrap, so that no native calls the realm; the start function
taken out of the module's bytes and called so; traps `RuntimeError`s
in the reference browser's words. `Memory`, `Table` and `Global` (step
3), imported, exported and the same object each time; a memory's
buffer a copy, the native's copy beside it, synced whole on the way in
and by 4 KiB runs on the way out, detached when the memory grows.
Limits (step 4): a module's work charged to the run's budget as fuel,
a loop stopped as a script's is; the realm's memories and tables
within 256 MiB, 1,000 instances, calls nested 64 deep and a module's
own 10,000; a panic of `wasmi`'s caught, the realm's WebAssembly given
up. The promise-returning interface (step 5), settled in promise jobs,
and the streaming forms, a response's type, origin and status checked.
Fuzzing (step 6): the `wasm` target, `wasm-smith`'s modules and any
bytes driven through the interface, only its errors allowed;
`dom_api`'s calls of a small module. The web-platform tests (step 7):
`wasm/jsapi`, 42 tests, 879 of 925 subtests passing, the rest
proposals and shared memories, after four fixes they found; not
`wasm/core`, Wasm 3.0's suite, which needs memory64 and GC to start.
Page 129 (step 8) matches the reference browser. The acceptance (step
9): plain Rust built for `wasm32-unknown-unknown`
(`tests/wasm/numbers`), fetched and instantiated by a page in the
engine and in the real shell, gives numbers back, its panic a trap,
its endless call stopped with the page running on; `benches/wasm.rs`,
which found the mirror's decoding slow (a call with 1.1 MB of memory
seen 3.3 ms, now 1.9); T4.3 within 10% but for a realm made, 17.5 ms
from 15.7 (x1.11), the WebAssembly bindings every realm compiles; fuzz
runs of `wasm`, `dom_api` and `script`, fifteen minutes each on six
workers, found nothing. Not in this milestone, said rather than
hidden: the memory's bytes shared (G79); SIMD, memory64, exceptions,
GC, promise integration, string builtins and type reflection (G80);
threads and shared memories (M31); ES module integration (G41); a JIT
(M34); compilation off the realm's thread or while the bytes arrive
(M31, M38). 2,247 tests pass workspace-wide, two ignored. Next
milestone: Canvas 2D (M30).
