# Random Browser — Architecture

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
shell          (bin "random-browser": windowing, event loop, browser chrome UI)
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
    └ dom     (document object model — the leaf crate)
```

### Rules

1. **Dependencies point strictly downward.** A lower crate must never
   `use` a higher one. The workspace root `Cargo.toml` mirrors this graph;
   keep it honest.
2. **Only `shell` touches the OS**: window creation, input events, the
   event loop. Only `render` touches pixel buffers / the GPU. `engine`
   orchestrates but owns no platform state.
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
   macOS, and Linux. No `#[cfg(windows)]` outside `shell` and `render`.
7. **Rendering is pluggable.** `render` exposes a backend trait; the
   software backend (tiny-skia → softbuffer) is the default, a wgpu
   backend lands behind the same trait later.

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
(`FlatDom`). 121 tests (unit + a table-driven harness over real markup,
`tests/selector_table.rs`); a hand-rolled baseline bench
(`benches/match.rs`) measures ~200ms matching 1k selectors against a
10k-element synthetic DOM — over the milestone's <100ms aspirational
budget, expected until the M16.5/M35.2 optimization passes (bloom
filters, rule-hash bucketing) land.

Milestone 6 (Box tree + block layout v1): `layout::box_tree` turns a
`style::StyledDom` into a tree of block/inline/anonymous-text boxes
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
`hhea` for fonts without a (new-enough) OS/2 table. `letter-spacing`/
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
`dom::Node` plus its `css::Stylesheet`s into a `StyledDom` — every
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
