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
   appears in any other crate or in `fuzz/` (T3.4).
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
