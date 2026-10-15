# Silver Browser — Master Roadmap

> Status: v1.0 · 2026-09-30 · Owner: the project (single-developer cadence unless noted)
> This is the canonical plan. `docs/architecture.md` is the binding layering
> contract; this document is the binding schedule, scope, and decision record.
> When work changes the plan, update this file in the same commit.

---

## 0. How to read this document

- **Work items are checkboxes with stable IDs** (`M17.3`, `T2.4`, `D1`).
  Never renumber existing IDs; retire them with `[x]` + `(done M17)` or
  `~~struck~~ (superseded by M18.2)`.
- **Every item is actionable**: it names a crate, an API, a spec section, or
  a measurable outcome. "Improve layout" is not an item; "implement margin
  collapsing between adjacent siblings per CSS2 §8.3.1" is.
- **Effort bands** (single developer, part-time cadence):
  - `S` — hours to a day
  - `M` — a few days to two weeks
  - `L` — two weeks to two months
  - `XL` — multiple months
  - `XR` — "year-scale": a quarter or more; usually split further before start
- **The vertical-slice rule** (already enforced since M1): every milestone
  ends demoable — `cargo build` green, `cargo test` green, `cargo clippy`
  clean, a smoke run of the window, and `docs/architecture.md` "Current
  state" updated. A milestone is not done until its Exit Criteria bullet
  list is literally checkable.
- **Spec citations are part of the item.** When an item says "per
  css-flexbox-1 §7", the acceptance test is conformance to that section,
  not to intuition.

---

## 1. Mission and non-negotiable goals

Build a from-scratch browser engine in Rust (no Chromium, no Gecko) that is:

1. **Secure** — memory-safe core by construction (Rust), hostile-input
   parsers that never panic, and a real isolation architecture before any
   untrusted scripting runs.
2. **Fast** — measurable frame budgets, incremental everything (style,
   layout, paint), GPU compositing when available.
3. **Lightweight** — small binary, small idle memory, no bundled
   telemetry or background services, fast cold start.
4. **Maximum compatibility** — spec-first parsing (WHATWG/W3C), graceful
   degradation everywhere, forward-compatible handling of unknown
   constructs (the M3 pattern: keep unknowns structurally, never drop).
5. **Chromium-level functionality** — see the parity tiers below; this is
   a ladder, not a boolean.

### 1.1 Priority order when goals conflict

**Security > Spec correctness > Performance > Compatibility > Feature count.**

Rationale: a browser that leaks memory or executes cross-origin data is a
liability regardless of features; spec correctness *is* the cheapest path
to compat in the long run; perf can be recovered later but a broken
security model cannot be patched in retroactively; and a smaller, correct
feature set always beats a large, wrong one.

### 1.2 Parity tiers — the operational meaning of "Chromium level"

| Tier | Name | Measurable definition |
|------|------|----------------------|
| T0 | Renders | Any static page (HTML+CSS from this engine's supported subset) paints correctly: text, boxes, images, links visible. |
| T1 | Top-100 readable | The landing pages of the global top-100 sites load and render legibly (layout imperfections allowed, no JS-required content). |
| T2 | Top-100 usable | Core interaction on top-100 sites works: links, forms, scrolling, JS-driven DOM updates. |
| T3 | Daily driver | A developer can use it as their only browser: tabs, history, logins, cookies, cache, downloads, service-worker sites (banks, docs, mail) function. |
| T4 | Developer grade | DevTools parity on the big five (console, DOM inspector, network, sources/debugger, performance). |
| T5 | Chromium parity | The long tail: WebRTC, WebGPU, WebAudio, PWAs installed, autofill, extensions API. This is the 100–500 engineer-year tier — entered only if the project survives that long. |

**The project's committed goal is T3 — "replaces Chrome daily."** Everything
below T3 is a checkpoint on the way there; T4–T5 are optional
continuations the engine has to *earn* (by surviving real usage), not
scope commitments. When this document says "the goal", it means T3.

Current status: **T0 achieved at M3** (HTML + CSS parse), and since M9
painting is cascade-styled over a real layout that scrolls and clips;
images, the last item on T0's list, come with M23.

### 1.3 Non-goals (explicit)

- No Chromium/Gecko/WebKit code reuse. Third-party *Rust* crates for
  non-engine concerns (TLS, image codecs, windowing) are fine and
  encouraged; the engine itself (parse/style/layout/paint) stays ours.
- No telemetry. Crash reporting, if ever added, is opt-in and local-first.
- No extension ecosystem before T4 (and only if there's a reason).

---

## 2. Current state (M0-M24)

- **M0 — Scaffold** (done 2026-09-30): 11-crate workspace, layer DAG,
  `docs/architecture.md` contract.
- **M1 — Vertical slice** (done): window (winit+softbuffer), hardcoded DOM
  → display list → tiny-skia raster; real font stack (fontdb, rustybuzz,
  ab_glyph).
- **M2 — HTML parsing** (done): tokenizer + tree builder (entities, attrs,
  void elements, auto-close, EOF recovery; no script/style raw-text mode
  yet).
- **M3 — CSS parsing** (done): css-syntax-3 token model + stylesheet
  parser (unknown at-rules kept raw for forward compat; preludes stored as
  token vectors for the selector engine).
- **M4 — Selectors v1** (done 2026-09-30): full selectors-4 subset parsed
  and matched right-to-left against a flattened DOM view (see
  `docs/architecture.md` "Current state" for the detailed list). Shipped
  with real correctness bugs fixed post-hoc — this code had never actually
  been compiled or run before landing (a stale `pub mod selector_matching
  {}` inline stub shadowed the real module, plus a dangling `[[bench]]`
  target, blocked the whole workspace from building at all). Once wired
  in and run, ~half the matcher tests failed: an off-by-one in the
  right-to-left walk, a tokenizer-fusion case `parse_an_b` didn't handle
  (`-` fuses into the preceding ident/dimension, unlike `+`), a 0-based
  vs. 1-indexed mismatch in every `nth-*` pseudo-class, and a missing
  `of S` renumbering semantic. Lesson for future milestones: "compiles"
  and "the vertical-slice rule is satisfied" are not the same claim —
  run the tests before checking a milestone off.

- **M5 — Cascade + computed values** (done 2026-09-30): `ComputedStyle`
  v1 (color, background-color, font-size with `em` resolved correctly,
  font-family, `display`, margin/padding/border-width per side, border-
  color/border-style), cascaded from the built-in UA stylesheet
  (`style/assets/ua.css`) under author stylesheets and inline `style`
  attributes — origin + `!important` + specificity + document order, all
  six precedence tiers tested individually. `paint`/`engine` now consume
  it for real; the M5.9 acceptance scenario (a color rule visibly
  changing the rendered frame buffer) has its own test at the full-
  pipeline level. See `docs/architecture.md` "Current state" for the
  full property/value-syntax scope and its documented gaps (no `rgb()`,
  only ~18 named colors, no per-side border color/style, no backgrounds
  or borders painted yet — that needs M6's real box model).

- **M6 — Box tree + block layout v1** (done 2026-10-01): real CSS2.1
  §10.3.3 width/margin resolution and CSS2 §8.3.1 margin collapsing,
  producing a positioned `Fragment` tree; `paint` v2 (M6.8) paints real
  backgrounds and borders from it. `ComputedStyle` gained `width`/
  `height` and `margin` became `auto`-capable (M6.3 needs it) — a
  necessary extension of M5's model, not scope creep; its own tests were
  updated. One real bug found via a full-pipeline test and fixed at the
  UA-stylesheet layer: `<style>`'s text was rendering as visible content
  (no rule made it `display: none`). See `docs/architecture.md` for the
  fuller writeup, including the v1 simplifications (inline/text content
  gets a flattened stand-in, not real line boxes — that's M7).

- **M7 — Inline layout + line breaking** (done 2026-10-01): real text
  flow — words measured through `text::FontStore` (which gained
  `metrics()`/`measure()`), greedy line breaking at whitespace
  opportunities, baseline alignment across mixed font sizes sharing a
  line, `text-align` (left/right/center). Block-in-inline handled
  without restructuring the box tree (ends the line, lays the block out
  on its own, resumes fresh — same visible effect, and critically never
  panics on content real pages can produce). `overflow-wrap: break-word`
  is a documented gap, not a silent one. See `docs/architecture.md` for
  the fuller writeup, including why exact text-position test assertions
  now derive their expected values from the actual loaded system font's
  metrics rather than a fixed guess.

- **M8 — Text maturity** (done 2026-10-01): per-script font fallback
  (M8.1: `text::itemize` run segmentation + a new `FontCollection`
  holding a primary font plus resolved Han/Hiragana/Katakana/Hangul
  fallbacks — the roadmap's own "Latin → CJK" scope, not every script);
  ligatures/kerning (M8.2) turned out to need no new code — rustybuzz's
  default shaping already applied them since M1, now documented rather
  than left to be rediscovered; UAX#9 bidirectional text (M8.3, crate
  `unicode-bidi`), wired into `layout::inline` per literal line, base
  direction from the computed `direction` property; `letter-spacing`/
  `word-spacing`/`text-transform`/`white-space` (M8.4); OS/2-preferred
  vertical metrics (M8.5); the M8.6 mixed-LTR/RTL-paragraph acceptance
  case has its own test at the `layout::inline` level, not just unit
  tests on `text::bidi_reorder` in isolation. `FontCollection` replaced
  `FontStore` as what both `layout` (measurement) and
  `render::SoftwareBackend` (actual shaping/rasterizing) use — doing
  that properly meant fixing a glyph-mask cache that had no font-
  identity in its key at all (harmless with one font; a silent
  wrong-glyph bug waiting for a second one), caught by reasoning through
  the render pipeline before adding the second font, not after. Two
  more real bugs caught by hand-tracing before trusting a test: a
  leading-`Common`-run "claim" check in `itemize` had the comparison
  backwards; a `white-space: normal` text box's literal `\n` fed
  straight into `bidi_reorder` was silently truncated (`unicode-bidi`
  treats `\n` as its own paragraph separator). See `docs/architecture.md`
  for the fuller writeup, including the documented scope cuts (bidi
  "level 1"; CJK-only fallback).

- **Review of M0–M8** (done 2026-10-01): before M9, the finished
  milestones were read for defects crate by crate and the parsers were
  fuzzed. Real defects in every layer, all fixed with regression tests
  and none changing a milestone's scope: three ways for a page to abort
  the process (nesting depth in `html` and `dom`; glyph masks and frame
  buffers sized from a hostile `font-size` or window), exponential
  selector matching and quadratic paths in the tokenizers and the tree
  builder, HTML conformance gaps (entities, bogus comments, implied end
  tags, `<pre>`), CSS nesting that lost the rules after a block,
  `inherit`/`initial`/`unset` unimplemented, a UA sheet that did not
  make `<div>` a block, pairwise margin collapsing that was wrong for
  three margins or an empty box, and 66 of the 95 text-dependent tests
  passing vacuously on a machine without a system font (`text` now
  bundles DejaVu Sans for the tests). The bounds introduced and the
  items deferred to the milestones they belong to are listed in
  `docs/architecture.md` "Current state". The fuzz targets live under
  `fuzz/` and run in CI (T2.1 started, T2.3 in its short form).

- **M9 — Viewport, scrolling, clipping** (done 2026-10-01): the fragment
  tree in document coordinates with element indices and line boxes
  (`layout::DocumentLayout`); the scroll offset applied at paint, with
  out-of-view items culled; `overflow` clipping as clip rects in the
  display list and a clip stack in `render`; scrolling and a drawn
  scrollbar in the engine (fixed-tick lerp; wheel, keys and scrollbar
  in the shell); hit testing and `link_at`; the cascade and the layout
  cached so a scroll reruns paint and raster only; and the M9.6 bench,
  8.7ms a frame on the debug build (115fps), 2.0ms in release. The one
  real finding on the way: shaping was most of a frame (the face parsed
  per call, D7's item), fixed with a shaped-run cache in `render`;
  layout's own cost (160ms for the bench page) is next for the perf
  track. See `docs/architecture.md` for the fuller writeup and the
  stated gaps (`overflow-x`/`overflow-y`, elements that scroll — M10.5,
  horizontal scrolling, hover — M26).

- **M10 — Box model completion** (done 2026-10-01, the first milestone
  with a design note, `docs/design/box-model.md`, written before the
  code and corrected to it after): the implied `html`/`head`/`body` and
  the raw-text states in the parser (M2's debt, paid first); every unit
  through a cascade context, percentages kept for layout; `box-sizing`,
  `min-*`/`max-*`, percentage heights; borders per side and by style,
  the canvas from the root; anonymous block boxes and block formatting
  context roots; `inline-block` with intrinsic widths and the baseline
  (M7.7), the strut and css-text-3's white space with it; scroll
  containers with scroll chaining and overlay scrollbars; the form
  controls as boxes; and the acceptance harness — ten box-only pages
  captured from a headless browser, compared byte for byte, all
  matching. See `docs/architecture.md` for the writeup and the stated
  gaps (inline decorations and the two painting differences — M11,
  `overflow-x`/`-y`, `line-height`, `vertical-align`).

- **M11 — Floats, positioning, stacking** (done 2026-10-01, designed in
  `docs/design/floats-positioning.md` and built in its twelve steps):
  the properties with CSS2 §9.7's rules; one paint-order walk read by
  paint and by hit testing; floats in a context per block formatting
  context, placed by §9.5.1 with lines shortened beside them, clearance
  and the dodge of a context root; relative, absolute, fixed and sticky
  positioning; CSS2 Appendix E's stacking order with `z-index`; an
  inline element's box per line with its padding, border and
  background, and unbreakable pieces in the line breaker; box edges
  snapped to pixels, border widths whole, mitres between colours
  blended by exact area; and eleven more reference pages, twenty-one
  matching the headless browser, one within a declared tolerance of two
  at its mitres. See `docs/architecture.md` for the writeup and the
  stated gaps (`line-height`, `vertical-align`, nested sticky, the
  browser's last bit of mitre blending, a double border in two colours).

- **M12 — Tables** (done 2026-10-02, designed in `docs/design/tables.md`
  and built in its eight steps): CSS2 chapter 17's display values and
  properties with the HTML standard's table rules in the UA sheet; a
  box kind per table part with CSS2 §17.2.1's anonymous table objects;
  the grid of css-tables-3 §3.4 with `colspan` and `rowspan`; the fixed
  algorithm and the automatic one as css-tables-3 §3.9 writes the
  browsers' practice down; row heights with `vertical-align` in cells
  and the table's baseline; captions; the separated model with
  `empty-cells`, and the collapsed model's conflict resolution with
  exact half borders and segments painted by the table; and five more
  reference pages, twenty-six matching exactly, which corrected how a
  rowspan's excess and a table's height are shared out and where the
  collapsed lines lie. See `docs/architecture.md` for the writeup and
  the stated gaps (presentational attributes, `visibility: collapse`,
  percentage row heights, the parser's table insertion modes).

- **M13 — Incremental engine** (done 2026-10-02, designed in
  `docs/design/incremental.md` and built in its ten steps): the
  document as a live arena of slots named by generational `NodeId`s
  (D16), with the DOM's mutations, their checks and a change journal;
  every stage moved onto it; restyle of the elements a change can
  reach, by the features the selectors depend on; a layout cache by
  node that lays out only the boxes a change reaches and their
  ancestors; a raster that redraws only where the display list
  changed, byte for byte a whole raster's; frames drawn for a time and
  a deadline, timed by stage, with time-based scrolling and the shell
  drawing only when a frame is wanted; `docs/perf-budgets.md` (T4.1);
  and the acceptance benchmark, 1,000 paragraphs appended one a frame
  to a page of 1,000 without laying the document out whole, 7ms a
  frame at the median in release. Incremental is held to from scratch
  by a differential check over random mutations, in the tests and in
  the `mutation` fuzz target. See `docs/architecture.md` for the
  writeup and the stated gaps (the box tree rebuilt and every fragment
  copied each pass — M35.9, text shaping's cost — M35.8, a scroll
  redrawn whole — M19).

- **M14 — `net` v1, URLs and HTTP/1.1** (done 2026-10-02, designed in
  `docs/design/net.md` and built in its nine steps): the Encoding
  Standard as a leaf crate, `encoding` (D18); the URL Standard's parser
  of our own, with UTS #46 through `idna` (D17); MIME types, MIME
  Sniffing and `data:` URLs; documents and stylesheets decoded from
  bytes by HTML's and css-syntax-3's encoding rules; HTTP/1.1 of our
  own over any stream (D5); connections with Local Network Access's
  address spaces refused on the address connected to (D19), TLS through
  rustls over aws-lc-rs with post-quantum key exchange and Mozilla's
  roots, failing closed (D4), a pool of six an origin, and aborts that
  end a waiting connection at once on every OS; `net::Client::fetch`
  with Fetch's schemes, redirects, bad ports and mixed content; and the
  engine's loader: `Engine::load` on threads of its own, render-blocking
  stylesheets in tree order with their imports, `<base>`, media and the
  preferred set, kept current through the journal. The shell runs as
  `silver-browser [URL or path]`, and CI holds sockets and TLS to `net`
  (T3.4). A page of 1,000 paragraphs and three stylesheets from the
  local test server is shown 3.3ms after `load` in release, its first
  frame's layout taking the rest. See `docs/architecture.md` for the
  writeup and the stated gaps (parsing as bytes arrive — M24.8,
  images — M23, cookies and CORS — M27, the cache — M28.4, HTTP/2 —
  M38, revocation and Certificate Transparency — M33, the legacy CJK
  encodings — M42).

- **M15, navigation v1 and the address bar** (done 2026-10-02,
  designed in `docs/design/navigation.md` and built in its eight
  steps): the session history with a back-forward cache, HTML's
  navigate, traverse, reload and stop; the browser's own interface in
  a new `ui` crate (D9), its address field with its heuristics, the
  status bubble and the window's title; error pages, `about:silver`,
  and the page's icon.

- **M16, selectors v2 and pseudo-elements** (done 2026-10-03, designed
  in `docs/design/selectors-pseudo.md`): `::before` and `::after` with
  `content`, `::first-line` and `::first-letter`, `::marker`,
  `::placeholder` and `::selection` recognised, the attribute flags,
  `:lang()`, the control states, and an ancestor filter for matching.

- **M17, flexbox** (done 2026-10-03, designed in
  `docs/design/flexbox.md`): css-flexbox-1's layout, the intrinsic size
  keywords for every box, baselines, and the web-platform-tests
  harness that M17 started (T1.1).

- **M18, grid** (done 2026-10-05, designed in `docs/design/grid.md`):
  css-grid-2's placement, track sizing, alignment and named areas;
  subgrid and masonry are G1 and G2.

- **M19, compositing, transforms and accelerated scroll** (done
  2026-10-06, designed in `docs/design/compositing.md`): 2D and 3D
  transforms, opacity groups, layers, composited scrolling, and a GPU
  backend on wgpu beside the software one (D3, D8).

- **M20, media queries and printing** (done 2026-10-07, designed in
  `docs/design/media.md`): Media Queries 4 and 5's preferences,
  `@supports`, cascade layers, and printing to a paginated PDF.

- **M21, custom properties, `calc()`, counters and generated content**
  (done 2026-10-07, designed in `docs/design/values.md`): `var()`,
  `calc()` and its relatives, `env()`, counters, `attr()`, list
  markers and text decoration.

- **M22, transitions and animations** (done 2026-10-08, designed in
  `docs/design/animations.md`): CSS transitions and animations on the
  document timeline, composited where they can be, reduced motion.

- **M23, images** (done 2026-10-09, designed in
  `docs/design/images.md`): a new `images` crate decoding PNG, APNG,
  JPEG, GIF and WebP, bounded and isolated; `img`, backgrounds,
  gradients, border images and image markers; SVG images are M32.1's.

- **M24, the JS engine** (done 2026-10-09, designed in
  `docs/design/scripting.md`): a new `js` crate, QuickJS-ng behind the
  engine's traits (D1); the HTML parser rewritten to run as bytes
  arrive; classic scripts, inline and external, with `defer` and
  `async`; Content Security Policy's `script-src`; scripts off unless
  `--enable-scripts` asks, until the isolation of M33 and M34 (R2).

- **M25, DOM scripting v1** (done 2026-10-10, designed in
  `docs/design/dom-scripting.md`): the DOM for scripts, node objects
  over the arena's nodes made in a bootstrap of JavaScript; selectors,
  markup and `document.write`; `classList`, `style`, `dataset`;
  `location`, `navigator` by D12, `screen` and `getComputedStyle`; the
  nodes scripts let go of freed.
- **M26, events and input** (done 2026-10-10, designed in
  `docs/design/events.md`): HTML's event loop in the engine, timers,
  animation frames and the rendering steps; the DOM's events and
  handlers; the document's lifecycle; the pointer, focus and the
  keyboard delivered to the page, the dynamic pseudo-classes; form
  controls shown and edited, their submission in every encoding and
  their constraint validation.
- **M27, fetch, the same-origin policy, CORS and cookies** (done
  2026-10-11, designed in `docs/design/fetch.md`): origins kept and
  sites by the Public Suffix List; `fetch()`, its classes and the
  bodies scripts make; CORS with its preflight; cookies by RFC
  6265bis; mixed content for what scripts fetch; `XMLHttpRequest`; an
  adversarial page that learns nothing across origins, kept as a test;
  the web-platform tests' testharness.js tests run.
- **M28, storage** (done 2026-10-11, designed in
  `docs/design/storage.md`): a profile on disk over `redb` (D10);
  `localStorage` and `sessionStorage` with their quota and the
  `storage` event; the cookies with an expiry kept; an HTTP cache in
  memory by RFC 9111's freshness, kept apart by site; what one origin
  keeps no other reads, before and after the profile is opened again.
- **M29, WebAssembly** (done 2026-10-11, designed in
  `docs/design/wasm.md`): `wasmi` 2.0 bridged into the realm (D21);
  the `WebAssembly` namespace, `Module`, `Instance`, `Memory`, `Table`,
  `Global`, their errors and the promise-returning and streaming
  forms; a module's calls into a page's script and back, resumed by
  the bootstrap; a memory's buffer a mirror kept at the calls' edges;
  a trap an error, a loop stopped by the run's budget as a script's
  is, the realm's memories and tables bounded; a compiled Rust
  function given numbers and giving them back.
- **M30, Canvas 2D** (done 2026-10-11, designed in
  `docs/design/canvas.md`): a `canvas` crate drawing with `tiny-skia`
  0.12 (D22): the 2D context's state, paths, fills and strokes,
  gradients and patterns, every composite operator, shadows, clipping,
  text in the engine's face, images, pixels in and out, PNG out;
  `<canvas>` a replaced element in a layer of its own, shown again
  without a layout, its texture written in place on the GPU backend;
  `OffscreenCanvas` and `ImageBitmap`; `crossorigin` and the
  origin-clean flag; the web-platform tests of `html/canvas`; a chart
  and a particle demo drawn, 2,000 particles at 27 ms a frame (sixty a
  second up to 1,000; the rest G81's).
- **M31, Web Workers v1** (done 2026-10-11, designed in
  `docs/design/workers.md`): dedicated workers, each a realm of its own
  on an operating-system thread, its global what Web IDL exposes to a
  dedicated worker; the structured clone in the reference browser's
  order and words, with transfer; `postMessage`, `MessageChannel` and
  `MessagePort` across realms through one router; a worker's script
  fetched same-origin, `close`, `terminate` and errors through every
  step; timers, `fetch`, XHR, `importScripts` and WebAssembly in a
  worker; `transferControlToOffscreen` and frames drawn from a worker;
  every crossing a message on a channel, the seam M34's processes
  replace; the web-platform tests of workers and messaging and a
  worker's variant of every `.any.js` test; a Mandelbrot demo whose
  workers keep its frames 16.7 ms apart.
- **M32, SVG v1** (done 2026-10-13, designed in `docs/design/svg.md`):
  namespaces in the DOM, foreign content in the HTML parser and in CSS;
  an XML parser of the engine's own; SVG's properties and presentation
  attributes in the cascade; a new `svg` crate (D23) building a render
  tree of shapes, paths, gradients, `use` and `symbol`, nested
  viewports, clip paths and text v1 and drawing it through `canvas`;
  inline `<svg>` a replaced element drawn again without a layout, hit
  and asked its geometry by scripts; SVG images wherever an image goes,
  drawn at the pixels they land on; SVG documents; SMIL rejected; the
  engine and the shell drawing at a display's scale; an icon set and a
  chart the same pictures at every ratio, in the engine and in the
  shell at this machine's 125%.
- **M33, Security hardening sweep** (done 2026-10-13, designed in
  `docs/design/hardening.md`): frames, each an engine of its own under
  its parent's (D24), with `srcdoc`, sandboxing, `X-Frame-Options` and
  `frame-ancestors`; what a frame keeps partitioned by the top-level
  site (D25); Content Security Policy in full, its violations and
  reports; Referrer Policy, HSTS (D26) and `Upgrade-Insecure-Requests`;
  Local Network Access's prompt; the profile encrypted at rest (D27);
  `unsafe` in the shell's platform module alone; 29 fuzz targets with
  committed seeds; the web-platform tests of CSP, sandboxing, referrers
  and iframes; an adversarial site of 56 attacks; and
  `docs/security.md`, the threat model.
- **M34, Multi-process architecture** (done 2026-10-15, designed in
  `docs/design/processes.md`): each site's documents in a renderer
  process of its own, the same executable, locked to its site and
  speaking a versioned protocol (`engine::ipc`) with the browser, which
  keeps the network, the cookies, the storage, the profile and the
  frames' tree and checks every message (D28 to D31); navigations of
  another site handed over at their response's head, the session
  history in segments, 16 renderers a tab and a spare; frames of
  another site shown by their site's renderer and composited by the
  browser (D32, D33); on Windows a restricted token at low integrity in
  a job (D2); a renderer's crash a page of the browser's and a dump
  beside the profile (D34); one process a build feature; and a
  compromised renderer refused all it may not ask.

Each milestone's fuller writeup and stated gaps are in
`docs/architecture.md` "Current state", and the gaps no milestone holds
on the G list (M39+, below).

Verified per the vertical-slice rule at each milestone, by the gate:
the workspace's tests (2,606 at M33, two ignored), clippy clean (`-D
warnings`, every target), `cargo deny`, the fuzz targets, and a smoke
run of the shell. Toolchain: 1.98.1, pinned by `rust-toolchain.toml`.
The checkout lives outside OneDrive since 2026-10-08
(`D:\09-Projects\Silver-Suite\Silver-Browser`), its `target` inside it
and ignored.

---

## 3. Phase map

| Phase | Milestones | Outcome | Rough effort |
|-------|-----------|---------|--------------|
| A — Engine Core | M4–M13 | Pixels correct for the CSS subset; restyle→layout→paint pipeline | XR |
| B — Web Loader | M14–M15 | Type a URL, see a page | L |
| C — CSS Completeness | M16–M23 | Flexbox, grid, transforms, animations, images | XR |
| D — Scripting | M24–M32 | JS executes; DOM APIs; fetch; storage; wasm; canvas | XR |
| E — Hardening & Isolation | M33–M35 | Security sweep; multi-process; perf pass | XR |
| F — Chrome & Platform | M36–M38 | Tabs, downloads, settings, devtools v0, HTTP/2 | XL |
| G — Long Tail | M39+ | WPT-driven compat; service workers; the forever list | ∞ |

Phases B and F can interleave with A/C internals where dependencies allow
(e.g., M14 net work is independent of layout milestones and may be pulled
forward). Phases D and E are strictly ordered: **no untrusted script runs
before E-grade isolation exists** (see risk R2).

---

# 4. Milestones

---

## M4 — Selectors v1  ·  `style` crate  ·  Effort: L  ·  Depends: M3

Goal: parse selector preludes into an AST and match them against the DOM.

- [x] M4.1 `style::selector_matching` module skeleton with types:
       `Selector { compounds: Vec<CompoundSelector>, combinators: Vec<Combinator> }`
       (match order right-to-left per selectors-4), `CompoundSelector { simple: Vec<SimpleSelector> }`,
       `SimpleSelector` enum.
- [x] M4.2 Simple selector parser over `Vec<css::Token>` (M3 prelude
       format): type (`div`), universal (`*`), id (`#x` — `Hash{is_id:true}`),
       class (`.x` = Delim('.') + Ident), namespace-less.
- [x] M4.3 Attribute selectors: `[attr]`, `[attr=v]`, `^=`, `$=`, `*=`,
       `~=`, `|=`, and the `i`/`s` case-sensitivity flags (selectors-4 §6.1).
- [x] M4.4 Combinators: descendant (Whitespace), child (`>`), next-sibling
       (`+`), later-sibling (`~`); whitespace-vs-descendant disambiguation
       in the token stream.
- [x] M4.5 Structural pseudo-classes: `:first-child`, `:last-child`,
       `:only-child`, `:first-of-type`, `:last-of-type`, `:only-of-type`,
       `:empty`, `:root`, `:nth-child(An+B [of S])`, `:nth-last-child`,
       `:nth-of-type`, `:nth-last-of-type` (An+B parser: `odd`, `even`,
       `2n+1`, `-n+3`, and the fused-token forms `n-1`/`2n-1` — `-` is a
       name-continuation code point, so an unspaced minus tokenizes into
       the *same* ident/dimension rather than splitting like `+` does).
       `of S` support is scoped to `:nth-child` only (nth-last-child/
       nth-of-type/nth-last-of-type reject `of` as a parse error rather
       than silently drop it).
- [x] M4.6 Logical combinators: `:not()`, `:is()`, `:where()` with nested
       selector lists; specificity rules per selectors-4 (the `:where`
       zero-specificity case).
- [x] M4.7 Dynamic pseudo-classes as parseable-but-unmatched stubs for now
       (`:hover`, `:focus`, `:active`, `:visited`, `:link`) — matched in M26
       when input events exist.
- [x] M4.8 Specificity: three-column (ids, classes+attrs+pseudo-classes,
       types) computed per compound; document ordering tiebreak
       (css-cascade-5 §6).
- [x] M4.9 Matching engine: right-to-left evaluation; ancestor and
       sibling traversal against `dom::Node` (via a `FlatDom` flattened
       view); `matches_selector(&FlatDom, idx, &Selector) -> bool`.
- [x] M4.10 Unknown selector recovery: an unparseable selector invalidates
       its rule (per css-syntax-3) but never the stylesheet. True at the
       `style`-crate level today (`parse_selector_list` invalidates the
       whole list on any bad member; `css` never inspects selectors, so a
       bad one can't touch the stylesheet); wiring an actual `Stylesheet`
       through this (drop the one bad rule, keep the rest) is M5's job,
       once `style` starts consuming `css::Stylesheet` for real.
- [x] M4.11 Test battery: 121 tests (98 unit tests across `parse`/
       `matcher`/`flat_dom` + a 6-function, ~30-row table-driven harness
       over real markup, `tests/selector_table.rs`, each row a
       (selector, HTML, expected-match-set) triple). Fuzz target
       `selector_parse` deferred to the T2 fuzzing-infra setup (T2.1) —
       tracked there, not blocking this milestone's exit.
- [x] M4.12 Micro-benchmark: hand-rolled (`benches/match.rs`, no
       criterion dependency yet — see T4.2 for when that lands), no
       `#[bench]` harness. Baseline recorded: ~200ms (release) matching
       1k selectors against a ~10k-element synthetic DOM — over the
       <100ms aspirational budget; expected for a naive matcher with no
       bloom-filter rejection or rule bucketing (M16.5, M35.2), not a
       regression to chase now.

Exit criteria: `style` crate compiles with the above API; all M3
stylesheets' rule preludes can be parsed or safely dropped; benchmark
recorded; vertical-slice rule satisfied.

**Post-hoc note:** this milestone's code was originally written and
committed to the working tree without ever being compiled — a stale
`pub mod selector_matching {}` inline stub in `lib.rs` silently shadowed
the real module, and a `[[bench]] name = "match"` with no `benches/
match.rs` blocked `cargo build` for the *entire workspace*, not just this
crate. Once wired in and actually run, roughly half the matcher tests
failed on real logic bugs (see the M4 entry in §2 above) that plain
compilation could never have caught. Going forward: a milestone isn't
"done" until its tests have actually executed green in this tree, not
just been written.

---

## M5 — Cascade + computed values  ·  `style` crate  ·  Effort: L  ·  Depends: M4

Goal: every element gets a `ComputedStyle` derived from stylesheets, not
from its tag.

- [x] M5.1 `ComputedStyle` v1 fields: `color`, `font_size_px`,
       `font_family`, `background_color`, `display` (block/inline/none v1),
       `margin[4]`, `padding[4]`, `border_width[4]` (+ color, style v1).
- [x] M5.2 Declaration→property parsing for each v1 field with spec
       initial values and inherited flags (`color`, `font-*` inherit;
       `margin`/`padding`/`background` don't — css-cascade-5 §7.1).
       `border-color`'s initial value (`currentcolor`) is resolved too,
       once the rest of the element's cascade is final.
- [x] M5.3 Cascade order: author origin only at first (user/UA origins
       land with the UA stylesheet in M5.6); within origin — `!important`
       wins over normal; then specificity; then document order
       (css-cascade-5 §6). Implemented as six precedence tiers (UA
       normal/author normal/inline normal/author important/inline
       important/UA important) rather than "user" as a separate origin —
       there's no user-stylesheet feature planned, so that word in the
       original item text was aspirational; UA+author is the real scope.
- [x] M5.4 Inline `style` attribute via `css::parse_style_attribute`
       (already built in M3) as the highest-specificity author origin
       (its own tier, above every selector-matched author-normal rule
       regardless of specificity — and again above author-`!important`
       when the inline declaration is itself `!important`).
- [x] M5.5 Inheritance walk: relative units resolved at compute time
       (`em` in v1, against the right basis — parent's font-size for an
       element's own `font-size`, this element's own for everything
       else). Not a separate `StyledTree`/`StyledNode` type as first
       sketched: `cascade::StyledDom` instead pairs the M4 `FlatDom` with
       a parallel `Vec<ComputedStyle>` (`styles[i]` styles
       `flat.nodes[i]`) — one indexed tree, walked once, rather than two
       independently-walked trees that would risk desyncing (a real risk:
       `FlatDom` and a fresh `dom::Node` walk don't share an index space
       unless both are written to agree on traversal order by
       construction). `FlatDom` gained an ordered `children: Vec<FlatChild>`
       per node (text and element children interleaved, exactly as
       written) to make this possible — a genuinely general structural
       addition, not an M5-only hack.
- [x] M5.6 Default/UA stylesheet: ship `assets/ua.css` (html, body, h1–h6,
       p, ul/ol/li, b/strong, i/em, a, blockquote basics) loaded through
       the same cascade at UA origin (UA < author; !important inverts).
       Compiled in via `include_str!` (no runtime file I/O). `b/strong`/
       `i/em`'s rules parse but have no visible effect yet —
       `font-weight`/`font-style` aren't in the v1 `ComputedStyle` field
       set (M8/M21 add them); kept in the sheet now so it's complete per
       this item's own list.
- [x] M5.7 Engine integration: `Engine::styled_dom()` runs the cascade;
       `Engine::frame()` calls `paint::paint_styled_document` (real
       cascaded colors/font-sizes, real `display: none`, margin-driven
       vertical spacing — no backgrounds/borders/width resolution yet,
       that needs M6's fragment geometry, per M6.8). The old tag-based
       `paint::paint_document` is kept, unused by `Engine` now but still
       compiled and tested, as the "fallback path retained" this item
       asks for.
- [x] M5.8 Tests: cascade order, inheritance, UA-vs-author precedence —
       done, thoroughly (each of the six cascade tiers has its own
       dedicated test in `style::cascade::tests`, not just aggregate
       coverage). "Visual golden tests" (stored reference images) are
       *not* what got built here — that's T1.2's Golden-rendering harness,
       a bigger, separate piece of CI infrastructure that hasn't started.
       What this milestone's own M5.9 asks for (a same-run buffer-diff,
       "screenshot-diff test harness v0") is done, at both `paint` and
       `engine` level.
- [x] M5.9 Acceptance: both scenarios have dedicated tests —
       `engine::tests::deleting_the_style_block_still_yields_ua_styling`
       and `engine::tests::adding_a_color_rule_visibly_changes_rendered_pixels`.

Exit criteria: tag-based styling in `paint` fully replaced by the cascade
(this is the first milestone where user CSS visibly changes the screen).
True for `Engine`'s actual pipeline; `paint::paint_document` itself still
exists (see M5.7's note) rather than being deleted, which is what M5.7
asked for and M5.8's wording didn't quite anticipate — a small
inconsistency in the original item text, resolved in `Engine`'s favor
since that's the concrete, testable exit criterion.

---

## M6 — Box tree + block layout v1  ·  `layout` crate  ·  Effort: XL  ·  Depends: M5

- [x] M6.1 `layout::box_tree`: `LayoutBox { style: ComputedStyle, children: Vec<LayoutBox>, kind }`
       built from the styled DOM; text nodes become anonymous boxes.
       `style` is an owned `ComputedStyle`, not `&ComputedStyle` as first
       sketched — a borrowed field threads a lifetime parameter through
       every consumer for what's currently a small, cheap-to-clone
       struct (same reasoning as M5.5's `StyledDom` deviation).
- [x] M6.2 Block formatting context v1: vertical stacking of block boxes;
       width resolution (`auto`, fixed px, percentage of containing block).
- [x] M6.3 Horizontal margin resolution incl. `auto` centering.
- [x] M6.4 Height: content height v1 = sum of children; `height` property
       honored; percentage heights deferred to M10 (the *parser*, in
       `style`, already never produces a `Percent` height — see its own
       M6 extension note).
- [x] M6.5 Margin collapsing per CSS2 §8.3.1: adjacent siblings
       (including the mixed-sign formula), parent-first-child,
       parent-last-child, empty blocks; negative margins. One scope
       limit: parent/child escaping is one level — a margin collapses
       with its *direct* parent/child, but the implementation doesn't
       chase it transitively through multiple ancestors the way real
       engines do for deeply-nested border/padding-free wrappers. A
       double-counting bug here (an escaping margin materializing once
       inside a child's position *and again* when the parent's own
       escaped value got placed by its own parent) was caught by hand-
       tracing a parent/first-child scenario *before* running the code —
       worth the time; it would have been a very confusing one to debug
       from a failing pixel test alone.
- [x] M6.6 Containing block + viewport: initial containing block = window
       inner size; resize propagates. This fell out of the existing
       architecture for free — `shell` already re-queries
       `window.inner_size()` and calls `Engine::frame` on every redraw
       (M1's design), and `frame` now threads that width through to
       `layout::layout` as the initial containing block; nothing new to
       build here beyond wiring layout into the existing pull.
- [x] M6.7 Layout output: fragment tree (positioned rectangles) that
       `paint` consumes; `Fragment { x, y, w, h, style, kind }` — `kind`
       (`Box` or `Text(String)`) added beyond the original sketch so one
       fragment type serves both backgrounds/borders and text runs,
       which `paint` needs to tell apart.
- [x] M6.8 `paint` v2 (`paint::paint_fragments`): background rects and
       borders painted from fragments. Per the "fallback path retained"
       precedent M5.7 set, this doesn't delete the earlier hacks —
       `paint_document` (M1) and `paint_styled_document` (M5) still
       exist, compiled and tested, just no longer what `Engine` calls.
- [x] M6.9 Layout unit tests with fixed-size assertions: 18 tests in
       `layout`, each hand-computed before running (see M6.5's note on
       the bug that caught) — at an 800px viewport rather than literally
       10k×10k (the algorithms are scale-independent; 800px numbers are
       far easier to hand-verify and the exit criterion is "exact
       fragment geometry," which this is, regardless of scale).
- [x] M6.10 Acceptance: `layout::block::tests::
       two_level_document_with_margins_padding_and_borders` and
       `engine::tests::fragments_resolve_real_box_model_geometry` both
       hand-compute an exact expected geometry (margins, padding,
       borders, text baselines) and assert it to the pixel.

---

## M7 — Inline layout + line breaking  ·  `layout`  ·  Effort: XL  ·  Depends: M6

- [x] M7.1 Inline formatting context: inline-level boxes flow into line
       boxes; line height = max of font metrics (`ascent+descent+leading`
       v1, no `line-height` property yet). `text::FontStore::metrics`
       added for this (ascent/descent/line-gap, scaled per font size).
- [x] M7.2 Text measurement through `text` crate: word widths via
       shaping; whitespace collapsing and wrapping per CSS Text §3.
       `text::FontStore::measure` added (shares its shaping call with
       `shape`, returns just the total advance — layout only needs
       widths, not per-glyph positions). Whitespace collapsing is
       `str::split_whitespace`, which already implements css-text-3 §3's
       `white-space: normal` collapsing exactly.
- [x] M7.3 Greedy line breaking at soft-wrap opportunities (space
       separated). `overflow-wrap: break-word` v1 is **not** implemented
       — a lone word wider than its container overflows unbroken,
       documented as a known gap rather than silently wrong (mid-word
       breaking is secondary to greedy breaking itself; real text rarely
       needs it outside unbroken URLs/tokens at narrow viewports).
- [x] M7.4 Baseline alignment of mixed-size runs on a line: the shared
       baseline sits at the tallest ascent present on the line.
- [x] M7.5 `text-align` (left/right/center) — justification deferred to
       M21, as this item already said. `style::ComputedStyle` gained
       `text_align` (inherited) to carry it.
- [x] M7.6 Block-in-inline v1: a block child inside inline context splits
       the run (spec edge: the reverse — inline spread across blocks — is
       M10). First implemented without restructuring the box tree:
       `layout::inline` ended the current line, laid the block out on
       its own and resumed a fresh line after. Since M10.4 the box tree
       builder does it the specification's way (CSS2 §9.2.1.1): the
       block is hoisted out and the inline split around it, so its
       margins collapse with its siblings' through the anonymous blocks.
- [x] M7.7 Vertical alignment of images/inline-blocks on the text
       baseline (with M10's inline-block). Done 2026-10-01 with M10.4:
       an inline-block's baseline is its last line box's, or its bottom
       margin edge when it has none or clips (CSS2 §10.8.1); images are
       M23's. Other `vertical-align` values are not read.
- [x] M7.8 Acceptance: multi-paragraph text at a narrow viewport wraps
       onto more lines than the same content at a wide one, verified at
       the full `engine` pipeline level
       (`engine::tests::narrow_viewport_wraps_text_onto_more_lines_than_a_wide_one`).
       The literal ask — comparing against *stored Firefox reference
       screenshots* — isn't what got built: that's a golden-image
       harness (T1.2, separate CI infrastructure, unstarted), the same
       gap M5.8/M6.9 already flagged for their own "golden"/"reference"
       wording. What's actually verified instead: hand-computed exact
       geometry (word wrapping, baseline alignment, text-align), the
       same technique M6.10 used, now including font metrics read from
       the real loaded system font rather than a guess.

---

## M8 — Text maturity  ·  `text`  ·  Effort: L  ·  Depends: M7

- [x] M8.1 Run segmentation: font fallback per-script (Latin → CJK via
       fontdb family query), itemization into shape runs. Landed:
       `text::itemize` (maximal same-script runs, `Common`/`Inherited`
       merged into the adjacent run per HarfBuzz/ICU convention) +
       `FontCollection` (primary font plus resolved Han/Hiragana/
       Katakana/Hangul fallbacks, queried from the same loaded `fontdb`
       database). Scope cut, honestly: only those four CJK scripts get
       a real fallback list — the roadmap's own "Latin → CJK" framing —
       not a family-name matrix for every script `unicode-script` can
       report (Arabic, Devanagari, Thai, etc. all fall through to the
       primary font, same as before this item). Reaches further than
       just `text`: `layout` now measures through `FontCollection`
       (word widths and line metrics reflect the font a run will
       actually use) and `render::SoftwareBackend` now shapes/
       rasterizes through it too — doing the latter correctly required
       fixing the glyph-mask cache key (see M8's own current-state
       note and `docs/architecture.md`), since a `glyph_id` is only
       meaningful relative to the font that produced it.
- [x] M8.2 Ligatures and kerning via rustybuzz features (respect
       `font-feature-settings` later — M21). Turned out to need no new
       code: `FontStore::shape`'s rustybuzz call has applied the
       default per-script feature set (`kern`/`liga`/`clig`/`calt`)
       since M1 — passing `&[]` as the *features* argument means "no
       *additional* overrides," not "no features." Documented in
       `text`'s module doc so this doesn't get rediscovered as a gap
       or reimplemented by accident.
- [x] M8.3 Bidirectional text level 1: paragraph direction + the UAX#9
       two-level algorithm (crate: `unicode-bidi`); base direction from
       `dir` attribute + first-strong detection. Landed as
       `text::bidi_reorder`, wired into `layout::inline::
       collect_inline_items` — one bidi paragraph per *literal line*
       (a line already split by `white-space: pre-line`'s forced
       breaks, or by a plain `\n` under `normal`), applied before
       word-splitting, direction from the computed `direction` property
       (itself from `dir`, inherited — `dir="auto"` isn't recognized
       yet, so first-strong detection is reachable in `text::
       bidi_reorder`'s own API but `layout` always passes an explicit
       direction today, never `None`). Scope cut, matching the item's
       own "level 1": reordering is per literal line, not aware of
       *soft* line-breaking the way a fully spec-correct implementation
       would need to be (M42 territory per the roadmap's own note).
- [x] M8.4 Letter/word spacing, `text-transform` (uppercase/lowercase —
       full Unicode via `unicode-case`), `white-space` property
       (normal/pre/pre-wrap/pre-line). Landed in `style::ComputedStyle`
       + `layout::inline`. One deviation worth naming: uppercase/
       lowercase/capitalize use Rust std's `str`/`char` case-conversion
       (already full-Unicode default case mapping) rather than the
       named `unicode-case` crate — same substantive result, one fewer
       dependency, no locale-tailored casing (e.g. Turkish dotless-i)
       either way.
- [x] M8.5 Vertical metrics correctness: use OS/2 vs hhea per font
       platform heuristics; subpixel/skia `IntRect` quantization policy
       documented. Landed: `FontStore::from_bytes` prefers OS/2's
       typographic ascender/descender/line-gap, falling back to `hhea`
       for fonts without a (new-enough) OS/2 table. Subpixel/`IntRect`
       quantization policy is still undocumented — carried forward,
       not forgotten; it matters more once M19's compositing/transform
       work is in play than it does for today's whole-pixel software
       rasterizer. Corrected by M17: the reference browser takes
       `hhea`'s metrics unless the font sets OS/2's USE_TYPO_METRICS,
       rounds each to a whole pixel and places glyphs at hinted,
       whole-pixel advances, and `text` now does too (the flexbox
       note, "Vertical metrics, found at step 11"). The quantization
       policy is written down since 2026-10-09: `docs/architecture.md`,
       "Pixels: where fractions become whole".
- [x] M8.6 Acceptance: a mixed LTR/RTL paragraph with an embedded number
       and punctuation orders correctly. Landed as
       `layout::inline::tests::bidi_reorders_mixed_direction_text_
       before_layout` — exercises the full pipeline (`ComputedStyle`
       → `collect_inline_items` → `bidi_reorder` → line packing), not
       just `text::bidi_reorder` in isolation; checks the digits stay
       forward and the fragment order differs from naive logical-order
       splitting (proving the integration actually ran).

---

## M9 — Viewport, scrolling, clipping  ·  `shell`/`render`/`engine`  ·  Effort: M  ·  Depends: M6

- [x] M9.1 Document coordinate system; viewport offset (scroll position)
       applied at paint time. Done 2026-10-01: `layout::DocumentLayout`
       (a fragment tree in document coordinates, each fragment with its
       element's index, plus the content height), `paint::Viewport`
       with `scroll_y` subtracted at paint and out-of-view items culled.
- [x] M9.2 Wheel/scrollbar input → scroll offset; smooth scroll basic
       (fixed-tick lerp; compositor-smooth later M19). Done 2026-10-01:
       `Engine::scroll_by`/`scroll_to`/`tick`; the scrollbar is drawn by
       the engine (D9) and dragged through `Engine::scrollbar`; the keys
       too (arrows, Page Up/Down, Home/End, space).
- [x] M9.3 `overflow: hidden|visible` clipping (clip rects in the display
       list; scroll containers v1 = document root only). Done
       2026-10-01: `overflow` parsed (one value; `scroll`/`auto` clip
       like `hidden` until M10.5), `PushClip`/`PopClip` at the padding
       box, a clip stack in `render`.
- [x] M9.4 Hit testing v1: (x, y) → element for link clicks + hover
       ( groundwork for M26). Done 2026-10-01: `Engine::hit_test` and
       `link_at`; the shell prints the href on click (navigation is
       M15); hover is M26's to wire.
- [x] M9.5 Full-frame presentation pipeline: restyle → layout → paint →
       raster on scroll without re-layout (scroll-only invalidation).
       Done 2026-10-01: the cascade cached per document, the layout per
       (document, width), `Engine::layout_runs` proving it; shaped runs
       cached in `render` across frames.
- [x] M9.6 Acceptance: a 10× viewport-height document scrolls at 60fps on
       the debug build (measure; record in `benches/scroll`). Done
       2026-10-01, `crates/engine/benches/scroll.rs` (128 paragraphs,
       8061px, 120 frames of 40px at 1280×800): 8.7ms/frame (115fps) on
       the debug build with dependencies optimised (the dev profile now
       does), 15.7ms (64fps) with nothing optimised, 2.0ms (490fps) in
       release. Layout itself is 170ms (debug) / 160ms (release) for
       that page — a face parse per measured word, D7's and M35's item.

---

## M10 — Box model completion  ·  `style`/`layout`  ·  Effort: L  ·  Depends: M6, M7

Designed before the code in `docs/design/box-model.md` (the decisions,
the stated gaps, the order of work, and what the code taught), done
2026-10-01 in ten commits. The parser's implied `html`/`head`/`body`
and raw-text states came first, as M2's debt the milestone could not
do without.

- [x] M10.1 Units: `em`, `rem`, `%` (margins/paddings vs widths vs font
       sizes resolution rules), `ex`, `ch`, `vh`, `vw`, `vmin`, `vmax`.
       Done 2026-10-01: a `style::CascadeContext` (viewport, and a
       `FontMeasure` the engine lends for `ex`/`ch`); `rem` against the
       root; percentages kept for layout, which resolves margins and
       paddings against the containing block's width (CSS2 §8.3, §8.4)
       and heights against a definite height only; the engine
       recascades on a resize only when viewport units were used.
- [x] M10.2 `box-sizing` (content|border-box). Done 2026-10-01: the given
       sizes converted to content-box values before the CSS2 algorithms.
- [x] M10.3 `min-width/max-width/min-height/max-height` incl. interaction
       with `height:auto`. Done 2026-10-01: CSS2 §10.4 and §10.7, the
       margins re-resolved for the clamped width; a box with a
       `min-height` is never self-collapsing.
- [x] M10.4 `display: inline-block`, `display: none` (skip from box tree),
       anonymous block boxing (block-in-inline both directions). Done
       2026-10-01: anonymous block boxes and hoisting in the box tree
       builder (CSS2 §9.2.1.1), block formatting context roots;
       `inline-block` with intrinsic widths (`layout::intrinsic`),
       shrink-to-fit, the baseline (M7.7); the strut and css-text-3's
       white space, which the reference cases needed. `display: none`
       was skipped since M6.
- [x] M10.5 `overflow: auto|scroll` element-level scroll containers
       (nested scrolling; hit test through scroll chains). Done
       2026-10-01: the scrollable height on the fragment (css-overflow-3
       §2.2), offsets in the engine by element, `Engine::wheel` chaining
       from the pointer outward to the viewport, hit testing through the
       offsets, overlay scrollbars drawn by paint (indicators until
       M26). `hidden` scrolls by program only.
- [x] M10.6 Default form control styling v0 (`input`, `button`, `select`,
       `textarea` render as styled boxes; no behavior yet). Done
       2026-10-01 in the UA sheet; `size`/`rows`/`cols` not read, no
       value shown.
- [x] M10.7 Acceptance: CSS box model intro pages (custom built) render
       pixel-identical to Firefox on 10 reference cases. Done
       2026-10-01: `crates/engine/tests/reference/`, ten box-only pages
       captured by `capture.sh` from a headless browser (the headless
       shell of the Chromium this build environment ships; the harness
       accepts any browser's capture, Firefox's included — the two agree
       on CSS2 box geometry) into committed fixtures, compared byte for
       byte by `tests/reference.rs`, all ten matching. Nothing of the
       browser enters the engine.

---

## M11 — Floats, positioning, stacking  ·  `layout`/`paint`  ·  Effort: XL  ·  Depends: M10

Designed in `docs/design/floats-positioning.md`, written before the
code and corrected to it after; built in its twelve steps on
2026-10-01, one commit each.

- [x] M11.1 Floats: `float: left|right`, line shortening, `clear`
       (clearance computation per CSS2 §9.5). Done 2026-10-01: the
       properties with §9.7's computed-value rules; a float context
       per block formatting context (`layout::float`), placement by
       §9.5.1 in order, a float waiting while the margins above it
       collapse and placed level with what resolves the flow position;
       lines run between the floats overlapping them; a context root's
       `auto` height reaches its lowest float (§10.6.7); clearance per
       §9.5.2, the margins above a cleared box resolved and its own
       inside the clearance.
- [x] M11.2 Block formatting contexts around floats (the "BFC dodge").
       Done 2026-10-01: a context root takes the room between the floats
       at its top, its `auto` width that room's, and moves down when it
       does not fit.
- [x] M11.3 `position: relative` (offsets, paint-time only). Done
       2026-10-01: the fragment and its subtree moved as the box is
       placed, in layout rather than paint so that hit testing and an
       absolutely positioned descendant see the moved box; an inline
       element's line boxes move each with their content.
- [x] M11.4 `position: absolute` (containing block resolution, static
       position, shrink-to-fit width per CSS2 §10.3.7). Done 2026-10-01:
       the static position recorded in flow and carried up to the
       nearest positioned ancestor's padding box, §10.3.7 and §10.6.4
       solved there, the fragment under the containing block's after
       the in-flow content; the initial containing block takes the rest.
- [x] M11.5 `position: fixed|sticky` (sticky = scroll-driven constraints,
       v1 without nested scroll edge cases). Done 2026-10-01: a fixed
       box is a root of the document's fragments painted and hit with
       the viewport's offset left out; a sticky box is laid out in flow
       and shifted in the paint-order walk to keep its insets from the
       visible edge of its nearest scrollport within its containing
       block (the parent's padding box, the border box kept inside it).
- [x] M11.6 `z-index`, stacking contexts, paint order per CSS2 Appendix E
       (negative z, floats, inline, positioned). Done 2026-10-01: one
       walk (`paint::order`) read by paint and by hit testing; the root
       and each positioned box with a `z-index` not `auto` a stacking
       context; its positioned descendants gathered once, then the
       negative layer, three passes over the in-flow content, the
       `auto`/`0` layer and the positive layer; an inline element's box
       per line with its decorations in the inline pass; box edges
       snapped to pixels, border widths used whole, mitres between
       colours blended by the exact area each side covers.
- [x] M11.7 Acceptance: classic two-column float layout + absolutely
       positioned overlays render correctly; stacking order tests. Done
       2026-10-01: eleven more pages under `crates/engine/tests/reference/`
       (two columns, floats beside rows, placement, clear, the dodge,
       relative, absolute, fixed, stacking, inline decorations,
       fractional lengths with multi-colour borders), all twenty-one
       matching the headless browser's captures, the last within a
       declared tolerance of two at its mitres, every other exact;
       fixed under scroll, sticky and hits through the stacking order
       are engine tests. The pages found three defects, fixed.

---

## M12 — Tables  ·  `layout`  ·  Effort: XL  ·  Depends: M10

- [x] M12.1 Table formatting model: row/rowgroup/column/columngroup
       boxes, anonymous table boxes (missing tr/td wrapping). Done
       2026-10-01: a box kind per display value, CSS2 §17.2.1's
       anonymous rows, cells, row groups and tables in the builder, the
       first header and footer groups moved to their places, and the
       display values and table properties in the cascade and the UA
       sheet.
- [x] M12.2 Fixed layout algorithm (CSS2 §17.5.2.1). Done 2026-10-02:
       column boxes, then the first row's cells, percentages of the
       width left after the spacing, the rest shared equally; no cell
       measured.
- [x] M12.3 Automatic layout algorithm (min/max content widths,
       percentage column distribution — CSS2 §17.5.2.2). Done
       2026-10-02, as css-tables-3 §3.9 writes the browsers' practice
       down: cell and column measures, a length `width` a column's
       maximum and constraint, spanning cells, percentages cut past the
       whole, MIN, MAX and CAPMIN, §3.9.3's sizing guesses, `margin:
       auto`, and the table's intrinsic contribution.
- [x] M12.4 `colspan`/`rowspan` with cell grid construction. Done
       2026-10-02: css-tables-3 §3.4's slots, a `rowspan` cut at its
       group's end and 0 meaning that end, the HTML standard's clamps;
       a rowspan's excess shared out as the browsers do.
- [x] M12.5 `border-collapse` (collapse model conflict resolution per
       CSS2 §17.6.2), `border-spacing` for the separated model. Done
       2026-10-02: every grid line's segments resolved by §17.6.2.1,
       exact half borders and the table's border inside its border box
       (css-tables-3 §3.7.1.1), segments painted by the table with each
       joint its winner's; spacing and `empty-cells` in the separated
       model.
- [x] M12.6 `vertical-align` in cells; caption placement. Done
       2026-10-02: top, middle, bottom and the row's baseline, the
       table's baseline its first row's for an `inline-table`; a row
       group's and a table's `height` shared out by groups and rows;
       captions above and below the table box in the wrapper.
- [x] M12.7 Acceptance: 5 real-world table screenshots (including
       nested tables and colspan-heavy cases) match reference renderers.
       Done 2026-10-02: five more pages under
       `crates/engine/tests/reference/` (the grid with captions and
       alignment, automatic widths, spans, the collapsed model, nested
       and CSS tables), all twenty-six matching the headless browser's
       captures exactly; hits through a table and the collapsed lines'
       pixels are engine tests. The pages found two defects and
       measuring the browser a third, fixed.

---

## M13 — Incremental engine v1  ·  `engine`/`style`/`layout`  ·  Effort: L  ·  Depends: M5–M12

- [x] M13.1 Dirty flags: style dirty bits per element (class/attr change →
       subtree or sibling invalidation v1 = conservative subtree). Done
       2026-10-02: the change journal turned into the elements to
       restyle by the features the selectors depend on — named classes,
       ids and attributes, sibling combinators, positions from either
       end, `:empty` — restyled by depth, children only where a style
       changed; a `<style>` change or the root's font size cascades
       whole.
- [x] M13.2 Layout invalidation: damage region propagation (content-size
       changes → ancestor reflow; position-only changes → paint-only).
       Done 2026-10-02: a layout cache by node for elements' block and
       table boxes, forgotten for what a change reaches and its
       ancestors; a box that only moved is placed again whole, and one
       a float can reach is laid out again. The box tree is rebuilt
       each pass.
- [x] M13.3 Paint invalidation: display-list diffing v1 (item-equality
       rect collection). Done 2026-10-02: the last frame kept and the
       new list compared with the last; the rectangles where they
       differ redrawn, byte for byte a whole raster's.
- [x] M13.4 Frame scheduler: input → frame pipeline with a frame budget
       (16.7ms target, deadline-aware: skip layout polish when over
       budget — document the policy). Done 2026-10-02: frames drawn
       for a time and a deadline the caller passes, each stage timed,
       scrolling animated by time, the shell drawing only when a frame
       is wanted. The policy, in `docs/perf-budgets.md`: a late frame
       skips nothing that changes what is shown, there being no layout
       polish to defer, only the animation frames it could not draw.
- [x] M13.5 `Engine::set_document` public API + repaint triggering. Done
       2026-10-02: `set_document`, `mutate` over the document's own
       mutation API, `needs_frame`.
- [x] M13.6 Acceptance: a DOM-mutation benchmark (append 1k nodes) runs
       without full-document relayout; measure and record. Done
       2026-10-02: `crates/engine/benches/mutation.rs`; each appended
       paragraph's frame lays out three boxes, 7ms at the median in
       release, recorded in `docs/perf-budgets.md`.

---

## M14 — `net` v1: URLs + HTTP/1.1  ·  `net`  ·  Effort: XL  ·  Depends: none (parallelizable from M4+)

- [x] M14.1 WHATWG URL parsing: scheme/host/port/path/query/fragment,
       percent-encoding, IDNA (`idna` crate), IPv4/IPv6 hosts,
       relative-resolution. Full test suite from the WHATWG
       `urltestdata.json`. Done 2026-10-02: `net::url`, our own (D17),
       against `urltestdata.json`, `toascii.json`, `IdnaTestV2.json` and
       `percent-encoding.json`, 3,660 cases; the `url` crate is gone.
- [x] M14.2 Fetch core: `net::fetch(url) -> Response { status, headers, body }`;
       redirect following (max 20, cross-scheme rules); `data:` and
       `file:` schemes. Done 2026-10-02: `net::Client::fetch(&Request)`,
       Fetch's scheme fetch for `about:blank`, `data:`, `file:`, `http:`
       and `https:`, and its redirect fetch, with every check run again
       on every hop.
- [x] M14.3 HTTP/1.1 over `rustls` (pure-Rust TLS 1.2/1.3, webpki roots):
       chunked transfer, keep-alive, connection reuse pool (per-origin,
       max 6). Done 2026-10-02: `net::http` over any stream (D5), rustls
       over aws-lc-rs (D4), six connections an origin, an idle one looked
       at before reuse, gzip and deflate.
- [x] M14.4 Certificate validation: WebPKI rules; no user-configurable
       cert bypass in v1 (security posture: fail closed). Done
       2026-10-02: rustls's WebPKI verifier against `webpki-roots`; a
       failure names its fault and cannot be accepted.
- [x] M14.5 MIME sniffing (WHATWG mime sniffing algorithm) + charset
       detection (`charset` detection: BOM → declared → UTF-8 fallback).
       Done 2026-10-02: `net::mime`, and HTML's encoding sniffing with
       the prescan and a re-parse on a late `<meta>`; css-syntax-3's
       stylesheet encoding.
- [x] M14.6 Resource loading pipeline in `engine`: HTML → inline subresource
       set (stylesheets, images) → parallel fetch → incremental document
       assembly (stylesheet arrives → restyle). Done 2026-10-02:
       `Engine::load`, `poll_network` and `load_state`; stylesheets in
       tree order with their imports, render-blocking before `<body>`,
       restyling as they arrive, and kept current through the journal.
       Images are M23's; parsing as bytes arrive moved to M24.8.
- [x] M14.7 Security v1 in net: private-address blocking posture decision
       (PNA later, M33), `blob:`/`javascript:` scheme rejection in fetch.
       Done 2026-10-02: D19, refused outright by Local Network Access's
       address spaces on the address connected to; every scheme but the
       five a network error; mixed content blocked.
- [x] M14.8 Tests: URL suite (1000+ cases), mock HTTP server harness
       (`httptest`-style, local only), TLS via self-signed test CA. Done
       2026-10-02: 3,660 URL cases; `net::test_server`, scripted HTTP and
       HTTPS on 127.0.0.1 with a root made as the tests run, which the
       engine's tests fetch from too.
- [x] M14.9 Fuzz: URL parser (T2 priority target — URL parsers are the
       #1 historical CVE source in browsers). Done 2026-10-02:
       `url_parse`, with `encoding_decode`, `mime_sniff` and
       `http_response`, run in CI with the other targets.

---

## M15 — Navigation v1 + address bar  ·  `shell`/`engine`  ·  Effort: L  ·  Depends: M14

- [x] M15.1 Navigation state machine: `about:blank`, `about:silver`
       (built-in demo), user URL entry → fetch → parse → render; error
       pages (DNS fail, TLS fail, HTTP error classes). Done 2026-10-02:
       `Engine::navigate` and `follow_link`, HTML's history handling,
       responses dispatched by type, error, status and type pages built
       as trees, the initial `about:blank`.
- [x] M15.2 Session history: back/forward with in-memory bfcache v1
       (document snapshot, no JS state yet), same-document fragment
       navigation. Done 2026-10-02: 50 entries, the document's state as
       one value, six kept whole, fragments scrolled to through scroll
       containers, positions restored.
- [x] M15.3 Minimal chrome UI v0: a URL text box + back/forward buttons
       drawn in-engine (softbuffer UI; not native widgets — see D9).
       Done 2026-10-02: the `ui` crate's toolbar, address field and
       status bubble; the shell's keys, clipboard and input method.
- [x] M15.4 Address bar input heuristics v1: scheme-less input →
       `https://` prefix; "looks like a host" detection (single token,
       dot rules). Done 2026-10-02: `engine::address`, `http://` for
       local hosts, no search and no `http:` fallback.
- [x] M15.5 Page title → window title; favicon fetch (v0: no UI display).
       Done 2026-10-02: `Engine::title`, cleaned for the window; the
       icon fetched as an image and kept.
- [x] M15.6 Acceptance: navigate to a real static site over HTTPS
       (e.g., example.com) and read it. **This is the T1 gate-crasher
       milestone.** 2026-10-02: the engine's live test reads
       `https://example.com/`, run on a network whose TLS is intercepted
       by trusting its root; the browser pointed at the site from a
       direct connection waits for a run by hand. Done 2026-10-08: from
       a direct connection the browser shows the page, title and text,
       and both live tests pass; its background is `light-dark()` (G6).

---

## M16 — Selectors v2 + pseudo-elements  ·  `style`  ·  Effort: M  ·  Depends: M4, M26 (dynamic parts)

- [x] M16.1 `::before` / `::after` with `content` (strings, `attr()`,
       counters via M21; url() images via M23). Done 2026-10-02:
       `content` with strings, `attr()` and the quote keywords,
       `quotes`, HTML's `q` rules; generated boxes of every display at
       their quote depth, the element's to the pointer.
- [x] M16.2 `::first-line`, `::first-letter` (layout-aware pseudo styling).
       Done 2026-10-03: the first letter split by Pseudo-Elements 4's
       pattern into an inline box or a float; the first line's run
       restyled as the box tree is built and filled in those styles,
       through `style::Recascade`.
- [x] M16.3 `::selection`, `::placeholder`, `::marker` as recognized
       stubs. Done 2026-10-02: parsed, and their rules set aside
       unmatched until list items (M21.6) and selection (M26).
- [x] M16.4 Case-insensitive attribute flags end-to-end; `:lang()`;
       form pseudo-classes (`:disabled`, `:checked`, `:required` — stubs
       until M26 wires state). Done 2026-10-02: the `i` and `s` flags and
       HTML's case-insensitive attributes; `:lang()` by RFC 4647 against
       HTML's language; the form states from attributes, and `:link`.
- [x] M16.5 Selector matching performance pass: bloom-filter ancestor
       rejection (Blink-style) if micro-bench regresses >2× on 1k
       selectors × 10k DOM. Done 2026-10-03: 1.4–1.6 times the code as
       imported, so no filter; the pseudo-elements' cost measured and
       cut, and the computed style copied without allocating
       (`docs/perf-budgets.md`).
- [x] M16.6 Acceptance: pseudo-elements visible on the built-in demo page.
       Done 2026-10-03: `about:silver`'s drop cap, first line in
       capitals, labelled cards and quotations in two languages' marks.

---

## M17 — Flexbox  ·  `layout`  ·  Effort: XL  ·  Depends: M10

Per css-flexbox-1 (all items cite the spec section in code comments).
Designed in `docs/design/flexbox.md` before the code and corrected to it
after; built in its thirteen steps.

- [x] M17.1 Flex container/item box tree integration; `display: flex |
       inline-flex`; item creation order and `order` property. Done
       2026-10-03: a `Flex` box, block-level or an atomic inline by its
       display, floated or positioned as any box; its children
       blockified into items, absolutely positioned ones aside; the
       items in `order`, their fragments, painting and hit testing too.
- [x] M17.2 Axis setup: `flex-direction`, `flex-wrap`, main/cross axis
       resolution; anonymous flex items. Done 2026-10-03: the four
       directions, `wrap` and `wrap-reverse`, `flex-flow`; a run of text
       that is not all white space an anonymous item.
- [x] M17.3 Base size resolution: `flex-basis` (`auto` → content or
       width), content-based min/max sizes (`min-content`, `max-content`
       intrinsic sizing v1). Done 2026-10-03: every basis; the automatic
       minimum size (§4.5); `min-content`, `max-content` and
       `fit-content` as widths and limits of blocks, floats,
       inline-blocks, positioned boxes and tables, and in the block axis
       as the automatic size; `min-width` and `min-height: auto`.
- [x] M17.4 Flexible lengths algorithm (§9.7): grow/shrink factors,
       hypothetical main size, resolved flexible lengths, min/max
       violations, inflexible item handling. Done 2026-10-03: §9.7 in
       `f64`, per line, with gaps; definite sizes (§9.8) through a
       height imposed on the item; each item's layouts and each box's
       content widths kept for the pass.
- [x] M17.5 Main axis alignment: `justify-content` (all 6 values),
       margins auto-absorption. Done 2026-10-03: every value of
       css-align-3 with `safe` and `unsafe` and the reference browsers'
       fallbacks; auto margins on both axes.
- [x] M17.6 Cross axis: `align-items`/`align-self`, `align-content`,
       single-line stretch; `flex-wrap: wrap` multi-line packing. Done
       2026-10-03: stretch with the item laid out again at its line's
       height; lines packed and stretched by `align-content`.
- [x] M17.7 Baseline alignment of flex items (first-baseline v1). Done
       2026-10-03: first and last baseline groups; a flex container's
       own baselines, for `inline-flex` on its line, an outer flex
       container and a table cell.
- [x] M17.8 Absolute-positioned flex children (containing block rules §9.2).
       Done 2026-10-03: placed in the static-position rectangle as the
       sole item would be (§4.1), by `justify-content` and `align-self`.
- [x] M17.9 Test matrix: the WPT flexbox subset (adopt the harness in T1
       this milestone) + 20 hand-built golden cases. Done 2026-10-03:
       350 tests of `css/css-flexbox`, chosen by rule; 342 pass, and the
       8 left are named with their reasons, none of them this
       milestone's. Twenty reference pages, 30 to 49, match the
       reference browser byte for byte.
- [x] M17.10 Acceptance: 5 real flex layouts (nav bar, card grid, holy
       grail) render correctly. Done 2026-10-03: a navigation bar, a card
       grid, the holy grail, media objects and an application shell, with
       text, every element of each (143 boxes) within a pixel of where
       the reference browser puts it; `about:silver`'s cards laid out
       with flexbox.

---

## M18 — Grid  ·  `layout`  ·  Effort: XL  ·  Depends: M17

Per css-grid-1 (level 2 subgrid included):

- [x] M18.1 `display: grid`; explicit tracks: `grid-template-columns |
       rows` (`none`, fixed, `fr`, `minmax()`, `auto`, `repeat()` incl.
       `auto-fill`/`auto-fit`), line names, `gap`. Done 2026-10-03:
       `grid` and `inline-grid`, every track form with `fit-content()`,
       named areas, `grid-template` and `grid`, percentage gaps and the
       legacy gap names; the grid properties behind one pointer in the
       computed style, which grows by its eight bytes.
- [x] M18.2 Implicit tracks (`grid-auto-rows | columns`), auto-placement
       algorithm (sparse v1; dense v2). Done 2026-10-03: implicit
       tracks before and after the explicit grid, sized by patterns;
       §8.5 sparse and dense, by row and by column; lines limited to
       ±10,000.
- [x] M18.3 Item placement: line/area/span forms of
       `grid-column|row-start|end`, `grid-area`, z-order in grid. Done
       2026-10-03: every form, named and counted from either end, with
       §8.3.1's conflicts; items painted and hit in order-modified
       document order, `z-index` on items; absolutely positioned boxes
       in grid areas (§10).
- [x] M18.4 Track sizing algorithm (§12): base sizes, growth limits,
       intrinsic contributions, `fr` distribution, spanning items
       (single-span-only v1 for intrinsic sizing). Done 2026-10-03:
       §12 for one axis, spanning items of every span with §12.5.1's
       distribution, `fr` in definite and indefinite space, the
       automatic minimum (§6.6); where the reference browser parts from
       the draft is recorded in the design note.
- [x] M18.5 Alignment: `justify-items|self`, `align-items|self`,
       `place-*`, `justify|align-content`; safe/unsafe alignment. Done
       2026-10-03: every value with `safe` and `unsafe`, `auto`
       margins, distributed gaps, the arithmetic shared with flexbox in
       `layout::align`; first and last baseline groups in both axes,
       with shims, and a grid's own baselines.
- [x] M18.6 `subgrid` (grid-2) and `masonry` deferred — registered as
       G-track items, not blocking. Done 2026-10-03: G1 and G2 in the
       long tail.
- [x] M18.7 Acceptance: CSS grid garden-style suite + 5 real layouts
       (dashboard, photo gallery, form grid) render correctly. Done
       2026-10-03: a grid garden of twenty-four levels, a dashboard, a
       photo gallery, a form grid, a page of named areas and a calendar
       month, with text, every element of each (363 boxes) within a
       pixel of where the reference browser puts it; twenty reference
       pages, 50 to 69, byte for byte; 213 of the 217 web-platform
       tests of `css/css-grid` taken by the rule; `about:silver`'s
       tiles laid out by grid.

---

## M19 — Compositing + transforms + accelerated scroll  ·  `render`  ·  Effort: XL  ·  Depends: M11, M13

- [x] M19.1 `transform` 2D: matrix, translate/rotate/scale/skew;
       `transform-origin`; percentage resolution. Done 2026-10-06:
       every 2D and 3D function, `transform-origin`, `transform-box`
       and the individual `translate`, `rotate` and `scale`, behind one
       pointer in the computed style; `style::transform`'s 4x4 matrix,
       exact at the right angles; a translated or scaled box drawn
       through the raster, its text rasterized at its scale.
- [x] M19.2 `opacity` as a group; `transform`+`opacity` layers →
       composited subtrees (layer tree + layerization policy doc).
       Done 2026-10-06: groups in the display list, drawn through or
       from a scratch; a turned, skewed or projective group, or one
       whose `will-change` asks, kept as a layer under its key and
       composited by its matrix; the policy in
       `docs/design/compositing.md`, `Engine::layers` reporting it.
- [x] M19.3 GPU backend v1 via `wgpu` (decision D3): display list →
       render passes; tiny-skia retained as fallback (software path is
       first-class forever — lightweight machines are a stated goal).
       Done 2026-10-06: `render::gpu` behind the `gpu` feature, four
       pipelines, the same layers as textures, the frame read back for
       softbuffer; the shell draws on it where there is an adapter,
       `--software` otherwise; an equivalence suite holds the two
       backends together.
- [x] M19.4 Composited scrolling: scrolling as a layer transform, no
       repaint on scroll for composited content. Done 2026-10-06: the
       document's content as layers of tiles translated by the
       offset, rounded to whole pixels, a margin drawn ahead after the
       frame; a scroll moves the last frame's pixels and draws the
       strip it uncovers, no tile rasterized in a scroll's frames; an
       element scroller with `will-change: scroll-position` the same.
- [x] M19.5 3D transforms + perspective (matrix decomposition,
       preserve-3d v1); `backface-visibility`. Done 2026-10-06:
       perspective on a parent's children, `preserve-3d` contexts with
       accumulated matrices painted by depth, hidden backfaces neither
       painted nor hit, projective composites and hit testing through
       the planar inverse; no plane splitting.
- [x] M19.6 `will-change` hint; layer memory budget + eviction
       (lightweight goal: hard cap configurable, default modest). Done
       2026-10-06: 64 MiB by default, `--layer-budget <MiB>`, the
       least recently composited let go, never one a frame shows, a
       group the budget cannot hold drawn through.
- [x] M19.7 Acceptance: 60fps scrolling + animated transform demos on
       integrated GPU; software path still passes all golden tests
       (rendering equivalence test suite). Done 2026-10-06 but for the
       integrated GPU, which this machine has not got: on the software
       backend a scroll of 1,000 paragraphs at 2.0–2.6ms a frame and a
       turned card composited, its frames inside the budget but for
       those its layout makes long; on the GPU backend, on Mesa's
       software Vulkan driver, the scroll inside the budget
       (`docs/perf-budgets.md`); every golden page passes on the
       software path, and the equivalence suite holds the GPU backend
       to it over every reference page and layout. The run on an
       integrated GPU is M19.8.
- [x] M19.8 M19.7's GPU half measured on an integrated GPU: the
       compositing benchmark's scroll and turned card inside 16.7ms a
       frame on the GPU backend (found at M19: the machine the
       milestone was built on has a software Vulkan driver alone).
       Done 2026-10-08 on Intel UHD Graphics: inside the budget but for
       each card's first, layout frame and one scroll frame in 360
       (17.1ms; `docs/perf-budgets.md`).

---

## M20 — Media queries + printing  ·  `style`  ·  Effort: M  ·  Depends: M5, M9

- [x] M20.1 `@media` evaluation: width/height/aspect-ratio/orientation/
       resolution features (the M3 parser already nests these rules — now
       evaluate them); dynamic re-evaluation on resize. Done 2026-10-07,
       designed in `docs/design/media.md`: `style::media`, Media Queries
       4's grammar and three-valued evaluation with Level 5's
       preferences, against `style::Media`; conditional rules at any
       depth in the cascade, `@import`'s conditions and the `media`
       attribute in the loader, every sheet fetched and only those whose
       media hold blocking; the answers kept with the styles and asked
       again before a frame, a resize across a breakpoint one cascade.
- [x] M20.2 `@supports` (declaration-level v1: parseable = supported);
       `@layer` ordering semantics. Done 2026-10-07: supported when the
       cascade takes the declaration, `selector()` when the selector
       parser and matcher take it; cascade layers in their three forms,
       through `@import`, ordered by their first appearance among the
       rules in force, `!important` reversing them.
- [x] M20.3 `prefers-color-scheme`, `prefers-reduced-motion` (OS wiring;
       the built-in interface's dark theme does not exist, its toolbar
       having one light theme until the settings, M37.3, corrected
       2026-10-07). Done 2026-10-07: `Engine::set_preferences`; the
       shell's `--prefers-color-scheme` and `--prefers-reduced-motion`,
       the colour scheme from winit's window theme and its changes, and
       reduced motion from Windows and macOS, read at start and when the
       window regains the focus; on Linux the options (the desktop
       portal is G9).
- [x] M20.4 Print media: `@media print` stylesheets, print layout
       (paginated block fragmentation v1), print-to-PDF via tiny-skia
       surface → pdf-writer crate. Done 2026-10-07: `@page` with its
       selectors, `size` and margins; the break properties, `orphans` and
       `widows`; `layout::paginate`, a pass over the laid-out fragments
       that breaks them between blocks and lines by Fragmentation 3's
       rules and moves what follows each break; `Engine::print`, each
       page drawn at 300 dots per inch on the software backend and
       written by `pdf-writer`; the shell's `--print-to-pdf`; the
       pagination held word by word to the reference browser's PDF.
- [x] M20.5 Acceptance: the built-in demo prints to a sane multi-page
       PDF; a responsive page relayouts at breakpoint widths. Done
       2026-10-07: `about:silver` prints to three pages, every word
       where the reference browser's print puts it, and stacks its cards
       and widens its tiles at its breakpoints; a responsive page of
       layers and feature queries is laid out as the reference browser
       lays it out at 560, 800 and 1100 pixels.

---

## M21 — Custom properties, calc, counters, generated content  ·  `style`  ·  Effort: L  ·  Depends: M5

- [x] M21.1 Custom properties: `--x` parse (the M3 tokenizer already
       handles `--custom` idents), inheritance (computed-value = token
       stream), `var()` substitution with fallback + cycles = invalid at
       computed-value time (IACVT). Done 2026-10-07: `style::custom`, a
       copy-on-write trie shared with the parent where the declarations
       are the same; `var()` by Values 5's Appendix A on a stack of its
       own, its arguments `{}`-wrapped or not; substitution bounded by
       the memory its tokens take, a mebibyte a substitution and 256 MiB
       a cascade (`docs/design/values.md`).
- [x] M21.2 `calc()`: +, -, ×, ÷ with unit algebra (length/number/
       percentage type checking at compute time); `min()/max()/clamp()`.
       Done 2026-10-07: `style::calc`, typed by Typed OM's algebra, 32
       deep, an expression with a percentage kept for layout, in every
       property, shorthand, grid track and line, transform, `@page`
       value and media feature.
- [x] M21.3 `env()` stubs (safe-area, viewport). Done 2026-10-07: the
       safe-area insets 0, `preferred-text-scale` 1, the viewport
       segments undefined, as the reference browser has them; in
       properties (G18 for the rest).
- [x] M21.4 Counters: `counter-increment/reset`, `counter()`, `counters()`
       + `content` integration (M16). Done 2026-10-07: `counter-set` and
       `reversed()` too, a walk of the document ahead of the box tree by
       the reference browser's scoping where the tests hold it; the
       predefined counter styles of Counter Styles 3 §6
       (`style::counter_style`); the layout cache keyed on a fingerprint
       of the generated text in a box.
- [x] M21.5 Generated content: `content: url()` (needs M23), `attr()`
       (css-values-5's typed `attr()`; `content`'s string `attr()` is
       M16's, see `docs/design/selectors-pseudo.md`). Done 2026-10-07:
       the typed `attr()` in every property, its `<syntax>` subset;
       `content: url()` waits with images (M23.3).
- [x] M21.6 List styling: `list-style-type` (disc/decimal + 3 custom),
       `::marker` content. Done 2026-10-07: `display: list-item`,
       `::marker` styled for list items, every predefined style and a
       string, inside and outside markers, disc, circle and square drawn
       by the reference browser's measures, HTML's list rules and
       hints.
- [x] M21.7 `text-decoration` (underline position/thickness v1);
       `text-justify` + real justification (M7 debt). Done 2026-10-07:
       the properties of Text Decoration 3 and 4, propagated through the
       box tree, their lines placed as the reference browser places
       them; `text-align`'s values and longhands, `text-justify`, a
       line's slack shared across its gaps, or its characters too.
- [x] M21.8 Acceptance: theming demo (custom props drive colors),
       auto-numbered sections with counters. Done 2026-10-07:
       `about:silver` themed by custom properties under each scheme and
       in print, its sections numbered by counters, held word by word
       to the reference browser's print; the theming demo, reference
       pages 98 and 99, both ways.

---

## M22 — Transitions + animations  ·  `style`/`render`  ·  Effort: XL  ·  Depends: M19, M13

- [x] M22.1 CSS transitions: property interpolation (lengths, colors,
       transforms, opacity), easing functions (the standard set + cubic
       bezier), start/end edge cases per css-transitions-1. Done
       2026-10-08: every longhand interpolated by its animation type
       (`style::animate`), transforms decomposed as Transforms 1 and 2
       say; the easing functions with `steps()` and `linear()`
       (`style::easing`); Transitions 1 §3 to the letter, with
       Transitions 2's `transition-behavior` and `display`
       (`docs/design/animations.md`).
- [x] M22.2 CSS animations: `@keyframes` (the M3 parser keeps unknown
       at-rules raw — now parse `@keyframes` properly), iteration/direction/
       fill/delay/play-state, animation events. Done 2026-10-08:
       `@keyframes` and `@-webkit-keyframes` by name, layer and order;
       Web Animations' timing model (`style::animation`); the events of
       Animations 2 and Transitions 2 queued at each frame,
       `Engine::animation_events`, dispatched with the event loop
       (M26.1).
- [x] M22.3 `requestAnimationFrame`-aligned ticking (real rAF needs JS —
       M25; here: the frame scheduler drives animation timelines). Done
       2026-10-08: a document timeline on `frame_at`, `tick` and
       `frame`, an animation starting at the first frame after it is
       made; `Engine::needs_frame` and `next_frame_at`, the shell
       waiting out a delay.
- [x] M22.4 Compositor-only animations (transform/opacity) run off the
       main layout path; everything else animates via style recalc.
       Done 2026-10-08: a change of a transform or an opacity alone that
       leaves the box what it was to layout is written into the
       fragments of the layout and its cache, the layout kept
       (`layout::composite`); on the engine's one thread, a thread of
       their own being G29.
- [x] M22.5 Reduced-motion respect (M20.3 dependency). Done 2026-10-08:
       the engine's own scrolling jumps under `reduce`; CSS animations
       and transitions run as written, the page's own query being its
       means of honouring the user, as in every browser (a user's
       setting to stop them is G30).
- [x] M22.6 Acceptance: a spinner (rotate) and a color-fade demo hit
       60fps; jank budget test (no frame > 20ms during a 10s animation).
       Done 2026-10-08: `tests/demos/`, ten seconds sixty a second
       (`benches/animation.rs`): no frame over 16.7ms on the software
       backend, the worst 6.1ms; on this machine's GPU backend,
       llvmpipe, which rasterizes on the CPU's cores, the medians
       within the budget and a single frame of 20 to 130ms in its
       raster in some runs, at no frame in particular, the code before
       the milestone's last changes alike; a real GPU is unmeasured
       here (M19.8). `tests/demos.rs` holds the work their frames do.

---

## M23 — Images  ·  `render`/`net`  ·  Effort: L  ·  Depends: M14

- [x] M23.1 Decode pipeline: PNG (png crate), JPEG (jpeg-decoder), GIF
       (incl. animation), WebP (image-webp), AVIF (ravif) — behind a
       `ImageDecoder` trait; decode off-main-thread. Done 2026-10-09: a
       new leaf crate, `images` (`docs/design/images.md`), PNG and APNG
       by `png`, JPEG by `zune-jpeg` (jpeg-decoder being in maintenance
       for it), GIF by `gif`, WebP by `image-webp`, behind `read_header`
       and `decode` rather than a trait; EXIF orientation; frames
       composed; every decode bounded and isolated, on the engine's
       decode threads. AVIF is G34: `ravif` only encodes. Animated
       images on the document timeline (step 7).
- [x] M23.2 `<img>` element: intrinsic sizing, `width/height` attrs +
       aspect-ratio box, `alt` text rendering, `object-fit` + `object-position`.
       Done 2026-10-09: a replaced box or its alt text with an icon, as
       the reference browser draws them; CSS Images 3's default sizing
       in block, inline, flex, grid and intrinsic sizing;
       `aspect-ratio`, `object-fit`, `object-position`,
       `image-orientation` and `image-rendering`.
- [x] M23.3 CSS images: `background-image`, border-image v1, gradients
       (linear/radial — gradient rasterization in tiny-skia/wgpu paths),
       `image-set`; `content: url()`, `list-style-image` and `<image>` in
       a typed `attr()`, keeping `attr()`'s taint: an attribute's value
       never a URL (M21, `docs/design/values.md` section 2). Done
       2026-10-09: every background longhand and the shorthand, layers,
       the canvas; `border-image` in full; gradients rendered by paint's
       own code into an image both backends draw; image markers and
       `content` images; the taint per value.
- [x] M23.4 SVG-as-image v1 (render an SVG file to a surface via M32
       subset: static shapes/text only). Moved to M32.1, which it
       depends on: an SVG image is broken until then. Done there
       2026-10-13.
- [x] M23.5 Lazy decoding + downscale-on-decode for huge images (memory
       cap posture, `decoding=async`). Done 2026-10-09: headers read on
       arrival, pixels decoded on first draw; 512 MiB of decoded pixels
       kept, those drawn longest ago dropped; an image drawn under half
       its size decoded to a box-filtered copy at twice it, `zune-jpeg`
       having no scaled decode; the GPU's textures under 256 MiB.
- [x] M23.6 Acceptance: a photo-grid page with mixed formats + gradients
       + an animated GIF renders at 60fps scroll. Done 2026-10-09:
       `tests/demos/photo-grid.html` scrolled 40px a frame sixty a second
       (`benches/images.rs`): a frame's median 0.43-0.44ms on the
       software backend and 1.64-1.68ms on the GPU's, the worst 6.8ms,
       nothing laid out; `tests/demos.rs` holds what holds whatever the
       machine. T4.3 within 10% (`docs/perf-budgets.md`, "Images (M23)").
       The web-platform-tests subset's image directories (the note's
       step 9), done once the checkout was on the machine: `css-images`,
       `css-backgrounds` and the `img` element, 402 tests more, 2,140 in
       all; the 31 failures that were M23's mended, the 31 left listed
       with their reasons.

---

## M24 — JS engine integration  ·  new `js` crate  ·  Effort: XR  ·  Depends: M13, M15, **E-track pre-work (see R2)**

> Decision D1 must be made here. Working assumption: **quickjs-ng** behind
> a `ScriptEngine` trait. The trait is the real deliverable: engine swaps
> (SpiderMonkey later) must not cost a rewrite.

- [x] M24.1 `js` crate: `ScriptEngine` trait (compile/eval/call,
       promise/tick integration, GC integration points, error sinks).
       Done 2026-10-09: a new leaf crate, `js`
       (`docs/design/scripting.md`), its `ScriptEngine` making `Realm`s
       that run a classic script, run the microtask checkpoint, report
       their heap, collect it and say when they are dead; errors by kind
       with where they were thrown, muted for another origin's script.
       Calling a function the page gave comes with M26.
- [x] M24.2 quickjs-ng bindings (fork or `rquickjs`-style FFI, vendored +
       pinned; security branch: track upstream CVEs quarterly — T1).
       Done 2026-10-09: QuickJS-ng 0.16.2 through `rquickjs` pinned at
       `=0.14.0`, its C source vendored by the crate and its checksum,
       QuickJS's own allocator kept so that the heap limit holds; `js`
       writes no `unsafe`.
- [x] M24.3 Script execution pipeline: `<script>` parsing (M2 debt:
       raw-text mode for script/style elements), external scripts via
       `net`, execution order (sync/defer), `async` v1. Done 2026-10-09:
       HTML's "prepare the script element" for classic scripts the
       parser inserts; external ones fetched as scripts, held to Fetch's
       type checks and decoded as HTML says; parser-blocking, deferred
       and async in their order, a loading style sheet holding the
       scripts after it; `Loaded` waiting for them; `javascript:` links.
- [x] M24.4 Embedding boundary: NO engine object graph references DOM
       internals — a `ScriptingHost` with explicit conversion at the
       boundary (this is the security-critical seam; reviewed by R2 rules).
       Done 2026-10-09: `ScriptingHost`'s methods take and give plain
       data, the host moved into a run and back; `js` depends on no
       crate of the engine's.
- [x] M24.5 Memory/time limits: instruction budget, heap cap, OOM =
       script failure not engine failure. Done 2026-10-09: a step
       budget of interrupt checks a run, a 256 MiB heap, a 256 KiB
       stack, 32 MiB a fetched script; a realm whose heap stays full is
       dropped and the page's scripts stop, the page as drawn.
- [x] M24.6 CSP `script-src` enforcement at the loader (M33 does full CSP;
       this blocks the obvious foot-gun from day one). Done 2026-10-09:
       `net::csp`, from the response's fields and the head's `<meta>`
       elements; `script-src-elem`, `script-src` and `default-src` for
       script URLs at every hop, inline scripts and `javascript:` URLs;
       nonces, hashes and `'strict-dynamic'` match nothing until M33.1.
- [x] M24.7 Acceptance: `console.log("hello")`, `document.title = "x"`
       from an inline script; a script-error page state. Done
       2026-10-09: with `--enable-scripts` (scripts are off otherwise,
       R2) the shell prints the page's console and its errors to the
       standard error and takes the title the script set;
       `Engine::scripts` gives running or stopped with the errors
       counted, and a realm out of memory says so in the status bubble.
- [x] M24.8 Streamed parsing (from M14.6, see `docs/design/net.md`):
       the tokenizer as HTML §13.2's state machine, resumable at any
       byte; the tree builder writing into the `dom::Document` as bytes
       arrive, with frames drawn as the document grows; the input
       stream's insertion point and the parser paused by scripts. Done
       2026-10-09 but the insertion point, which comes with
       `document.write` at M25.1: the tokenizer fed in pieces and held
       to html5lib's tests; bodies handed over as they arrive, decoded
       and sniffed as a stream; a document parsed a slice a poll, shown
       once it has a body and no blocking sheet, and growing as a change
       like any other; the parser pausing at every script.

---

## M25 — DOM scripting v1  ·  `js`/`dom`/`engine`  ·  Effort: XR  ·  Depends: M24

- [x] M25.1 DOM Core bindings: `document.getElementById/ querySelector(
       All)` (uses the M4 selector engine!), `createElement`, `createTextNode`,
       `appendChild/removeChild/insertBefore`, `textContent`, `innerHTML`
       (reparse via html crate — the parser is already hostile-input-safe).
       And `document.write`, with the input stream's insertion point,
       which M24.8's parser leaves to it. Done 2026-10-10
       (`docs/design/dom-scripting.md`): node objects made in a bootstrap
       of JavaScript over handles, one object a node, the host answering
       one `Call` at a time; the DOM's mutations with its checks and the
       reference browser's messages; live and static collections; the
       selectors, `:scope` among them; an HTML serializer and fragment
       parser in `html` for `innerHTML`, `outerHTML` and
       `insertAdjacentHTML`; scripts that scripts insert, run nested;
       `document.write` at the parser's insertion point, written
       scripts inline and external; the native work charged to the step
       budget, the DOM's data bounded, and detached nodes freed.
- [x] M25.2 Node properties: `parentNode/children/childNodes/firstChild/
       nextSibling`, `classList`, `setAttribute/getAttribute`, `style`
       object (set->restyle). Done 2026-10-10: and `dataset`, the
       reflected attributes pages read most, and listeners kept for
       M26 to call; `style` over the attribute, values kept as written
       (G50).
- [x] M25.3 Mutation -> engine invalidation: DOM writes trigger M13
       incremental restyle/layout (the whole incremental engine pays off
       here; benchmark it). Done 2026-10-10: a script's changes go
       through the journal to the next frame, held to a fresh engine's
       by the `dom_api` fuzz target; `benches/dom.rs` measures a table
       built, a class toggled and a cell edited by script. Its first
       finding, text shaped again at every pass, gave half of M35.8: a
       1,000-row table's layout from 1,135 ms to 68 ms
       (`docs/perf-budgets.md`).
- [x] M25.4 `window` v1: `location` (read + navigate), `navigator`
       (fingerprint-minimal: no lies beyond a fixed UA string policy —
       see D12), `screen`, `getComputedStyle`. Done 2026-10-10: a
       script's navigations with HTML's handling, one entry for a page
       that redirects by script; `navigator` and `screen` by D12, with
       `--compat-user-agent`; `getComputedStyle` over a copy of the
       cascade, serialized by `style::serialize` (G46 for what needs
       layout); `noscript` computing `inline` and making no box.
- [x] M25.5 Acceptance: a page that builds a table via JS then styles it
       via a class toggle renders correctly; DOM-mutation benchmark under
       frame budget. Done 2026-10-10, but for the budget: the table
       renders in the shell's window and its PDF as the reference
       browser would, and pages 119 to 122 match it exactly; the frame
       after a cell's text is edited is 4 ms at 100 rows, within the
       budget, but 46-58 ms at 1,000 rows, and a class toggle restyling
       every cell takes 20 ms at 100 rows and 105-116 ms at 1,000, where
       the reference browser's style and layout alone take 33 ms at
       1,000. What keeps them over is M35.9's (the box tree, the
       fragments, and now a table's cells) and M35.11's.

---

## M26 — Event loop, events, input  ·  `engine`  ·  Effort: L  ·  Depends: M25

- [x] M26.1 HTML event loop: task queue, microtask checkpoint, rendering
       steps (rAF callbacks -> style -> layout -> paint), per spec
       ordering.
       Done 2026-10-10: `Engine::run_tasks` and `next_task_at` over
       tasks kept per document, on a clock the engine is given (the
       shell's, a hand's in tests); the realm entered by `js::Task`, a
       microtask checkpoint after each task and after each listener of
       the engine's dispatches; `frame_at`'s rendering steps (`resize`,
       `scroll`, the animation events, the callbacks); the document's
       lifecycle events and a hidden page's throttling.
- [x] M26.2 Event dispatch: capture/bubble/target phases, listeners,
       `stopPropagation/preventDefault`, synthetic event creation from
       JS (`dispatchEvent`). Done 2026-10-10: the event classes,
       `AbortController`, listeners' options, the dispatch as a
       generator the realm steps, handlers as IDL and content
       attributes, the `error` event and `reportError`, activation
       hooks.
- [x] M26.3 User input: mouse (click/dblclick/mousedown/up/move),
       keyboard (keydown/up/keypress), focus chain (focus/blur, tab order
       v1), wheel, `change`/`input` on form controls. Done 2026-10-10:
       pointer and mouse events in the reference browser's order, clicks
       and their counts; focus, sequential navigation and the address
       field, the window's focus, `autofocus`; keys with their default
       actions, the scrolling keys and the wheel moved into the engine;
       editing with `beforeinput`, `input` and `change`.
- [x] M26.4 Now-dynamic pseudo-classes: `:hover`, `:active`, `:focus`
       (M4.7 debt paid). Done 2026-10-10: `dom`'s state flags journaled
       and restyled by where the selectors read them; `:focus-visible`,
       `:focus-within`, `:target`; `outline` and `cursor`.
- [x] M26.5 Timers: `setTimeout/setInterval` (clamped, background
       throttling policy), `Promise` + microtask ordering. Done
       2026-10-10: nesting clamped to 4ms past five, `MAX_TIMERS`, a
       hidden document's at most once a second; checked against the
       reference browser by M26.12's golden page.
- [x] M26.6 Link handling: `<a href>` click -> navigation; middle-click
       -> new-tab-less v1 (tabs are M37: queue a note). (M15 follows a
       click on a link from the shell, `docs/design/navigation.md`; this
       item makes the click an event a listener can cancel.) Done
       2026-10-10: a link followed as its click's activation behaviour,
       by the engine with scripts off; a middle click's notice; a
       `javascript:` URL's string result made the document.
- [x] M26.7 Form controls v1: what each control shows, a button's label,
       an `input`'s value or placeholder, a checkbox's and a radio's
       mark, a `select`'s chosen option and a `textarea`'s text (M10
       draws them as empty boxes); and how each is edited: focus,
       typing with a caret and a selection in text fields and
       textareas, checkboxes and radios toggled, an option chosen;
       each control's value, dirty flag and checkedness as HTML keeps
       them. Found at M24, on Google's page without scripts. Done
       2026-10-10: `dom`'s control records and the IDL; the boxes, marks
       and list boxes, sized by the reference browser's figures and
       stretching as its do; editing with a caret and a selection, the
       clipboard and an input method's commits; the select's list.
- [x] M26.8 Form submission (HTML 4.10.21): a submit button, or Enter in
       a text field (implicit submission), submits its form; the form
       data set built from its controls, encoded by `enctype`
       (`application/x-www-form-urlencoded`, `multipart/form-data`,
       `text/plain`) and sent by `method` to `action`, a GET as the
       URL's query and a POST as the request's body, through a
       navigation (net has had `Method::Post` and a request body since
       M14); `formaction`, `formmethod`, `formenctype` and
       `formtarget` on the button; constraint validation v1
       (`required`, `pattern`, `min`, `max`, `minlength`,
       `maxlength`), skipped under `novalidate`. Found at M24: no
       milestone held it. Done 2026-10-10: the entry list and the three
       encodings in `net`, POST navigations with their history and the
       resubmission page, implicit submission, reset; validation with
       `js::pattern_matches`, the `invalid` event and the form
       pseudo-classes. `formtarget` navigates this tab (M37).
- [x] M26.9 Acceptance: interactive todo-list page (add/remove/style
       entries) works end-to-end; and a search form, a text field and
       a button, typed into and submitted, its results page loaded.
       Done 2026-10-10: both through the shell's window (session
       tests), the search form with scripts on and off, by Enter and by
       its button; the to-do page in the real shell; pages 123-125 and
       a page of controls' boxes against the reference browser. T4.3 in
       `docs/perf-budgets.md`; the fuzz runs found a form's `:invalid`
       left stale, mended.

---

## M27 — fetch, SOP, CORS, cookies  ·  `net`/`js`  ·  Effort: XL  ·  Depends: M25, M14

- [x] M27.1 `fetch()` API: promises, Request/Response/Headers objects,
       streaming body v1 (full-buffer first), `FormData` basic.
       Done 2026-10-11 (`docs/design/fetch.md`): `Headers`, `Request`,
       `Response` and the `Body` mixin in the bootstrap; `Blob`,
       `File`, `FormData` and `URLSearchParams`; a script's request
       checked again by the engine (`scripts::fetch::request`),
       fetched on its pool (`Target::Fetch`) and answered as a task
       (`js::Task::Fetched`); aborts by signal; bodies whole (streams
       are G67); why a fetch failed said on the console only.
- [x] M27.2 URL same-origin computation; SOP enforcement matrix (document
       → script, DOM access, network reads vs writes).
       Done 2026-10-11: a document keeps its origin, opaque ones once;
       sites by the Public Suffix List (D20, the `psl` crate);
       `self.origin`, `isSecureContext`, `document.domain`; the matrix
       stated vector by vector in the design note, each enforced in
       `net` (tainting, the CORS check, `Response::exposed`, CORP) or
       closed by construction (no frames).
- [x] M27.3 CORS: preflight (OPTIONS), `Access-Control-*` handling, error
       events, credentialed vs bare requests (fetch-cors spec).
       Done 2026-10-11: the CORS check on every hop, the preflight and
       its cache (5 s by default, 7,200 at most), `Origin` and its
       tainting across redirects, exposed headers, the `crossorigin`
       attribute for scripts and stylesheets (an image's waits for
       canvas, M30), a script's errors unmuted by its response's type.
- [x] M27.4 Cookies v1: RFC6265bis parse/store/send, `HttpOnly`,
       `SameSite=Lax|Strict|None` (default Lax), `Secure`, jar per-origin
       in-memory (persistence in M28).
       Done 2026-10-11: `net::cookies`, RFC 6265bis-22's user agent
       (parsing, storage, the prefixes, the limits of 180 a domain and
       3,000 in all, retrieval, same-site by site); one jar for the
       engine, its clients sharing it; `document.cookie` and
       `navigator.cookieEnabled`; a `cookies` fuzz target.
- [x] M27.5 Mixed content: block active http subresources on https pages.
       (Stylesheets are blocked since M14, `docs/design/net.md`; this
       item covers the rest.)
       Done 2026-10-11: `fetch()` and XHR from an `https:` page to
       `http:` blocked on every hop, tested over TLS; a CORS-mode image
       is not upgraded but blocked (in `net`; no image asks so until
       M30).
- [x] M27.6 `XMLHttpRequest` (legacy, minimal but correct).
       Done 2026-10-11: on `fetch()`'s path, its states and events in
       the standard's order, the upload's, timeouts, aborts, the
       response types but `document` (G70), synchronous requests that
       block the page; `ProgressEvent`.
- [x] M27.7 Security tests: an adversarial test page attempting
       cross-origin reads must fail in every documented vector (this test
       page becomes a permanent regression artifact).
       Done 2026-10-11: `tests/security/cross-origin.html` makes 26
       reads of another site holding a secret and learns nothing, and
       by exactly the 11 CORS reads once that site allows it, as the
       reference browser does (`loader/security_tests.rs`). The
       web-platform tests of Fetch, XHR, `URLSearchParams` and `Blob`
       run under the real testharness.js, 1,124 of 1,469 subtests
       passing, each failure named; pages 126-127 match the reference
       browser; a page signs in by `fetch` through the shell's window
       and reads another origin by CORS with credentials (session
       test, and in the real shell). T4.3 and `benches/fetch.rs` in
       `docs/perf-budgets.md`.

---

## M28 — Storage v1  ·  `net`/`engine`  ·  Effort: M  ·  Depends: M27

- [x] M28.1 Profile/persistence layer (decision D10): on-disk layout,
       atomic writes, per-origin namespacing.
       Done 2026-10-11 (`docs/design/storage.md`): a leaf crate,
       `profile`, over `redb` 4.3.0: one file, `profile.redb`, in the
       platform's place or `--profile`'s; each origin's items keyed by
       its serialization and read as one range; every write one
       transaction, durable when it returns; a profile in use, of a
       newer schema, or damaged refused, `redb`'s own panics on damage
       caught; the engine writes what changed at most once a second,
       when dropped and when the window closes.
- [x] M28.2 `localStorage`/`sessionStorage` bindings.
       Done 2026-10-11: areas in the engine, local ones from the
       profile and written by the item, a tab's session ones in memory,
       a quota of 5,242,880 UTF-16 code units each; `Storage` with Web
       IDL's named properties, `QuotaExceededError`, `StorageEvent` and
       the `storage` event between the document shown and the one on
       its way; opaque origins, `file:` included, refused.
- [x] M28.3 Cookie persistence in the profile.
       Done 2026-10-11: the cookies with an expiry written as records
       of `net`'s encoding and restored at startup, held to the storage
       model's rules, the jar's orders and limits kept.
- [x] M28.4 Cache v0: in-memory subresource cache with TTL + `Cache-Control`
       basics (full conditional caching in M38).
       Done 2026-10-11: `net::cache`, RFC 9111's private cache for the
       `GET`s fetched whole, by site and URL with `Vary`'s variants,
       fresh by `max-age`, `Expires`, `Age`, `Date` and the 10%
       heuristic; Fetch's cache modes and `Request.cache`; unsafe
       methods invalidate; 64 MiB, least recently used first.
- [x] M28.5 Storage isolation tests (origin A can't read origin B, incl.
       after profile reload).
       Done 2026-10-11: `storage/isolation_tests.rs`, four origins in
       memory and on disk, before and after the profile is opened
       again; the cache kept apart by site. The web-platform tests of
       `webstorage`, 1,239 of 1,251 subtests passing; page 128 matches
       the reference browser; a page counts its visits across windows
       and in the real shell. `benches/storage.rs` and T4.3 in
       `docs/perf-budgets.md`.

---

## M29 — WebAssembly MVP  ·  `js`  ·  Effort: L  ·  Depends: M24

- [x] M29.1 WASM instantiation via the chosen engine's WASM support
       (quickjs-ng: none — D1 contingency: `wasmtime`/`wasmi` bridge, or
       this milestone forces the SpiderMonkey upgrade decision D1).
       Done 2026-10-11 (`docs/design/wasm.md`): the contingency, D21,
       `wasmi` 2.0, an interpreter in Rust, in `js` beside QuickJS-ng:
       one store a realm, made at its first module; its portable
       dispatch; modules validated whole and translated lazily; a
       module's call into a script's function stopped by `wasmi`'s
       resumable call and resumed by the bootstrap, the start function
       too, taken out of the module's bytes to be called so.
- [x] M29.2 `WebAssembly.Module/Instance/Memory` JS API surface.
       Done 2026-10-11: the namespace with `validate`, `compile`,
       `instantiate` and their streaming forms; `Module` and its
       reflection, `Instance`, `Memory`, `Table`, `Global`,
       `CompileError`, `LinkError`, `RuntimeError`; the interface's
       conversions, exported functions the same object wherever they
       come from; a memory's buffer a copy synced at the calls' edges
       (G79 to share it). The web-platform tests of `wasm/jsapi`, 879
       of 925 subtests passing, the rest proposals (G80) and shared
       memories (M31).
- [x] M29.3 Engine-crash isolation: trap = error, never engine death.
       Done 2026-10-11: traps `RuntimeError`s in the reference
       browser's words, a recursion too deep a `RangeError`; a module's
       work charged to the run's budget as fuel, a loop stopped as a
       script's is; the realm's memories and tables bounded to 256 MiB,
       instances to 1,000, calls nested to 64; a panic of `wasmi`'s
       caught, the realm's WebAssembly given up. The `wasm` fuzz
       target, `wasm-smith`'s modules and any bytes.
- [x] M29.4 Acceptance: a small WASM module (e.g., compiled Rust fn)
       round-trips numbers.
       Done 2026-10-11: `tests/wasm/numbers`, plain Rust built for
       `wasm32-unknown-unknown`, fetched and instantiated by a page in
       the engine and in the real shell: numbers in and out, a panic a
       trap, a call that never returns stopped and the page running
       on. Page 129 matches the reference browser. `benches/wasm.rs`
       and T4.3 in `docs/perf-budgets.md`.

---

## M30 — Canvas 2D  ·  `render`/`js`  ·  Effort: L  ·  Depends: M25

- [x] M30.1 Canvas 2D API: paths, fills/strokes (colors + gradients),
       text (through the `text` stack), images, transforms,
       `save/restore`, `clearRect/fillRect`.
       Done 2026-10-11 (`docs/design/canvas.md`): a crate of its own,
       `canvas`, below `engine`, drawing with `tiny-skia` 0.12 (D22)
       into a premultiplied bitmap; the whole 2D context of the HTML
       Standard but `filter`, its 74 members and `Path2D`,
       `CanvasGradient`, `CanvasPattern`, `ImageData`, `TextMetrics`
       and the geometry interfaces in the bootstrap, each call a
       `Call::Canvas` the host answers; text as the glyphs' outlines;
       `drawImage` from `img`, canvases and bitmaps, `crossorigin` and
       the origin-clean flag. `hsl()` and `hwb()` in `style` (G16).
- [x] M30.2 Pixel formats, `toDataURL` (PNG), `OffscreenCanvas` v0
       (no worker transfer until M31).
       Done 2026-10-11: `getImageData`, `putImageData` and
       `createImageData` in 8-bit sRGB; `toDataURL` and `toBlob` in
       PNG through `png`'s encoder; `OffscreenCanvas` with its context
       and `convertToBlob`, `ImageBitmap` and `createImageBitmap` from
       every source. Formats past 8 bits and colour spaces past sRGB:
       G81.
- [x] M30.3 Rasterization shares the software/wgpu backend split (M19);
       canvas-in-composited-layer.
       Done 2026-10-11: `Replaced::Canvas` with scripts on; its image
       named by the node and drawn at its bitmap's generation, a frame
       wanted at a change and nothing laid out; every canvas a layer
       (`LayerReason::Canvas`); the GPU backend writing a new
       generation into the canvas's texture in place. Drawing stays on
       the processor (G81 for the GPU).
- [x] M30.4 Acceptance: a chart + a particle demo render correctly at
       60fps.
       Done 2026-10-11, the rate in part: a chart and 2,000 particles
       drawn correctly in the engine and the real shell, pages 130 and
       131 matching the reference browser within an edge's tolerance;
       the chart from its load to its first frame in 32 ms; the
       particles 26.9 ms a frame on the GPU backend and 31.3 on the
       software one (37 and 32 a second, against 116 and 120 ms as
       elements), sixty a second up to 1,000 and 700. The rest, the
       calls and tiny-skia's fills, G81. The web-platform tests of
       `html/canvas` (2,180), the `canvas` fuzz target,
       `benches/canvas.rs` and T4.3 in `docs/perf-budgets.md`.

---

## M31 — Web Workers v1  ·  `engine`  ·  Effort: L  ·  Depends: M25, M28

- [x] M31.1 Dedicated workers: dedicated-thread JS realms, `postMessage`
       + structured clone, `onmessage`.
       Done 2026-10-11 (`docs/design/workers.md`): each worker a thread
       of its own making its realm there (`RealmKind::DedicatedWorker`,
       one bootstrap putting on its global only what Web IDL exposes to
       a dedicated worker); the structured clone in the bootstrap, in
       the reference browser's order and words, crossing realms as a
       string; `structuredClone`, `MessageEvent`, `window.postMessage`
       to the window itself; `MessageChannel` and `MessagePort` across
       realms through one router a document and its workers share.
- [x] M31.2 Worker script fetch (SOP applies), termination semantics,
       error propagation.
       Done 2026-10-11: the script fetched as a worker's (destination
       `worker`, same-origin mode, `data:` taken), another origin a
       `SecurityError`; `close()` ending the task, `terminate()` the
       stop flag and nothing more heard; an uncaught error at the
       worker's global, then its `Worker` object, then the window, in
       the reference browser's words; a worker's scope: timers, fetch
       and XHR on threads of its own, `importScripts`, `location`,
       `navigator`, canvases, WebAssembly and nested workers.
- [x] M31.3 Thread policy: workers are OS threads now; process isolation
       happens in M34 (document the migration seam).
       Done 2026-10-11: every crossing a `WorkerMessage` on a channel
       and a wake, the data already strings, the seam M34 turns into
       its IPC; a document's workers bounded (16 live, 8 deep), its
       messages (256 MiB, 512 MiB waiting) and ports (65,536).
- [x] M31.4 `OffscreenCanvas` transfer + render-from-worker v1 (if wgpu
       path exists; else software-surface handoff).
       Done 2026-10-11, the software-surface handoff:
       `transferControlToOffscreen()` and placeholders; an
       `OffscreenCanvas` transferred to a worker, its frames committed
       after each of the worker's tasks and shown at the next frame;
       `requestAnimationFrame` in a worker; golden page 132 matching
       the reference browser.
- [x] M31.5 Acceptance: worker-driven Mandelbrot page stays responsive.
       Done 2026-10-11: `tests/demos/mandelbrot.html`, the set
       computed by four workers in bands sent back in transferred
       buffers while the page's own square turns: in
       `benches/workers.rs` every frame 16.7 ms after the last while
       they compute (the set in 463 ms), where on the page's own
       thread frames come 100 ms apart (1,548 ms); in the real shell
       the set in 515 ms over 30 frames, once the shell drew a wanted
       frame no sooner than a frame's budget after the last (it drew
       270 a second); the same set, pixel for pixel, as the page's own
       thread computes (`tests/demos.rs`). A worker's start 25.9 ms, a
       message's round trip 0.17 ms, 4 MB there and back 89 ms:
       `docs/perf-budgets.md`, with T4.3.

---

## M32 — SVG v1  ·  new `svg` crate (below `css`, above `dom`)  ·  Effort: XL  ·  Depends: M23, M9

*(corrected: the `svg` crate is below `layout`, over `canvas`, `style`,
`text` and `dom`: it draws through the canvas crate and reads the
cascade's computed styles, and layout reads its `viewBox` grammar.)*

- [x] M32.1 SVG-as-image (M23.4 completion): static shapes (rect, circle,
       ellipse, line, polyline, polygon, path), fill/stroke, gradients,
       transforms, `viewBox`, text v1.
       Done 2026-10-13: the `svg` crate (D23): microsyntaxes, a render
       tree of groups, shapes and text runs from a document and its
       computed styles, drawn through `canvas` (which gained a
       gradient's spread and transform and a clip of a union);
       gradients with `href` templates, `use` and `symbol`, nested
       viewports, `clipPath`, `switch`; text v1 in the engine's face.
       An XML parser in `html` (entities bounded, no recovery, libxml2's
       words). SVG images in `img`, backgrounds by CSS Images 3's
       default sizing, markers, border images, print and `drawImage`,
       drawn at the size they land on; SVG documents with their
       scripts; golden pages 133 to 138 the reference browser's.
- [x] M32.2 Inline `<svg>` in HTML: layout integration (replaced-element
       sizing), CSS styling of SVG presentation attributes.
       Done 2026-10-13: namespaces in the DOM, foreign content in the
       tree builder (html5lib's tests of it), `@namespace` and
       namespace selectors, the user agent sheet split; the SVG
       properties and presentation attributes as hints of the author's
       level; an outer `<svg>` a replaced box sized as the reference
       browser sizes it, drawn again without a layout when only its
       content changes, hit to its shapes; `createElementNS` and SVG's
       interfaces, `getBBox()` and the geometry questions.
- [x] M32.3 SMIL: explicitly rejected (chromium has it; cost/benefit
       documented — revisit only at T4).
       Done 2026-10-13: rejected in `docs/design/svg.md` section 2, its
       interfaces absent, its elements drawing nothing; the web-platform
       tests that need it left out.
- [x] M32.4 Acceptance: an icon set + a statistical chart render at all
       DPIs.
       Done 2026-10-13: the engine and the shell draw at a display's
       scale (`devicePixelRatio`, the `resolution` feature, the pointer
       in CSS pixels); `tests/demos/svg-icons.html` (thirty icons,
       stroked and filled, inline and as images, at 16, 24 and 48
       pixels) and `tests/demos/svg-chart.html` (a chart as d3 writes
       one) drawn at ratios 1.25, 1.5, 2 and 3 the same pixels as the
       pages drawn that many times larger at 1 (`tests/demos.rs`); in
       the real shell at 125% the same as software on the GPU backend,
       once its scrolled layer was kept whole at the scale and an svg's
       drawing snapped to device pixels. `benches/svg.rs` and T4.3 in
       `docs/perf-budgets.md`.

---

## M33 — Security hardening sweep  ·  all crates  ·  Effort: XL  ·  Depends: M27

- [x] M33.1 CSP: fetch directives (`default-src`, `script-src`,
       `style-src`, `img-src`, `frame-src`, `connect-src`, ...),
       report-only mode, violation events, `nonce`/`hash` sources.
       Done 2026-10-13 (`docs/design/hardening.md`): every document's
       policies from fields and `<meta>`, enforced and report-only
       (`net::csp`, 64 at most, failing closed past them); the fetch
       directives for every destination with their fallbacks; inline
       scripts, styles, handlers and `javascript:` URLs by
       `'unsafe-inline'`, nonces (hidden from selectors under a header
       policy), hashes with `'unsafe-hashes'`, `'strict-dynamic'`;
       `eval`, `Function` and string timers by `'unsafe-eval'`,
       WebAssembly by `'wasm-unsafe-eval'`; `base-uri`, `form-action`,
       `frame-ancestors`, `sandbox`, `upgrade-insecure-requests`;
       violations as `securitypolicyviolation` events, console lines and
       `report-uri` reports; workers and frames under their own (steps
       6 to 8).
- [x] M33.2 Frame isolation: `X-Frame-Options`, `frame-ancestors`,
       iframe `sandbox` attribute (all flags), `srcdoc` (M24's raw-text
       debt: iframes parse nested documents through the same pipeline).
       Done 2026-10-13: `iframe` as a replaced box of 300 by 150 with
       its inset border, its frame an engine of its own under its
       parent's (D24), drawn into it as a canvas is, with `src`,
       `srcdoc` and `about:blank`, its parent's load waiting for it,
       100 frames a tab and 16 deep; input, focus, scrolling, scale and
       printing through it; `WindowProxy`, `postMessage`, names and
       targets, navigating frames; the sandbox's flags as the reference
       browser has them, CSP's `sandbox`, `X-Frame-Options` and
       `frame-ancestors`; what a frame keeps kept apart by the
       top-level site: its cookies' same-site context, partitioned
       cookies (CHIPS), storage, the cache, the pools and TLS sessions
       (D25). Every frame is cross-origin to every other until M34
       (G90) (steps 1 to 5).
- [x] M33.3 `Referrer-Policy`, HSTS (max-age/includeSubDomains, preload
       list shipped read-only), `Upgrade-Insecure-Requests`.
       Done 2026-10-13: referrers by every policy, from the field,
       `<meta>`, `referrerpolicy` and the defaults (step 9); HSTS kept
       in the profile, 10,000 hosts and a year each at most, with
       `includeSubDomains`; the preload list read from
       `crates/net/data/hsts_preload.txt` (D26), the reference
       browser's 95,136 hosts as of 2026-10-13, read once at the first
       lookup (12.8ms, on a fetch thread); `Upgrade-Insecure-Requests`
       sent on navigations; a form
       from a secure page to `http:` held on a warning page (step 10).
- [x] M33.4 Private Network Access: block public->local requests
       (pre-flight posture v1: block outright). Begun at M14 (D19):
       public→local and local→loopback requests are refused outright
       from then on; this item adds Local Network Access's permission
       prompt.
       Done 2026-10-13: a secure page's request across spaces waits for
       the user's answer on a bar under the toolbar (60 seconds at
       most), kept in the profile for the site; an insecure page's is
       refused at once; a frame asks as its top-level site when its
       origin or its element's `allow` lets it, a worker as its
       creator; `fetch`'s `targetAddressSpace` (step 11).
- [x] M33.5 Memory-safety audit pass: `unsafe` inventory across the
       workspace (target: unsafe exists only in FFI crates, each block
       has a written safety argument — policy lives in architecture.md).
       Done 2026-10-13: `#![forbid(unsafe_code)]` in every crate, test,
       bench, example and fuzz target but the shell, whose platform
       module alone writes it, four blocks each argued; a test and CI
       hold it (T3.2); the third-party inventory (261 of 379 locked
       packages write some) is `docs/security.md`'s, its example
       `unsafe_inventory` (step 13). Besides: the profile encrypted at
       rest, AES-256-GCM under a key DPAPI wraps on Windows (D27, step
       12).
- [x] M33.6 Parser fuzzing graduation (T2): continuous fuzzing, corpus
       committed, OSS-Fuzz application (Rust targets).
       Done 2026-10-13: `csp` made whole, `referrer`, `hsts` and
       `frames` new, `dom_api` making frames: 29 targets; each one's
       corpus minimized into `fuzz/seeds/` and committed (50,653 files,
       14.6 MB);
       CI fuzzes every target nightly from the seeds. The OSS-Fuzz
       application waits for the repository to be public (G99) (step
       14).
- [x] M33.7 Threat-model doc `docs/security.md`: assets, boundaries,
       trust levels; updated per milestone from here on.
       Done 2026-10-13: assets, adversaries, trust levels, each
       boundary and what holds it, the unsafe code, the fuzzing, the
       adversarial site, the limits (133 constants) and the known gaps
       (step 18).
- [x] M33.8 Adversarial test-site v2: the M27.7 page + CSP violations +
       cookie exfil attempts, all must fail closed.

---
       Done 2026-10-13: four pages beside M27.7's, 56 attacks on frames,
       CSP, cookies and storage and the network, each failing with the
       defences on and succeeding with its defence taken away (but the
       nine that reach across frames, G90); they found a parsing
       document's form sent as a GET, an iframe deaf to its frame's
       later documents and a frame's fragment fetched again, all mended
       (step 17).

## M34 — Multi-process architecture  ·  `shell`/`engine`  ·  Effort: XR  ·  Depends: M33, M31

The site-isolation milestone — the architecture.md crate seams become
process seams.

- [x] M34.1 Process model: browser process (chrome, net, profile,
       policy) + renderer processes (documents/frames); process-per-site-
       instance v1 (not per-tab — lightweight goal: process count capped,
       LRU eviction).
       Done 2026-10-15 (`docs/design/processes.md`, D28 to D33): the
       browser keeps the window, the network, the jar, the storage, the
       cache, the profile, the permissions and the frames' tree; each
       site's documents are engines in a renderer process locked to the
       site at its first document; a navigation whose response is of
       another site is handed over at its head to that site's renderer;
       the history in segments, one a run of a site's entries; frames of
       another site in their site's renderer, drawn in their parent's
       hole and composited by the browser, their messages, loads and
       input through it; at most 16 renderers a tab, the least recently
       shown let go first, and a spare.
- [x] M34.2 IPC design: message-passing over the existing internal
       boundaries; capability-scoped handles (a renderer gets exactly the
       interfaces it needs — no god-channel); schema versioned from day
       one.
       Done 2026-10-15: `engine::ipc`, an own codec with versioned
       messages (`IPC_VERSION`), frames of at most 16 MiB read as their
       bytes come, a hello in 10 seconds; the browser's services (the
       network, the storage, the frames) answer each message on its
       renderer's reader thread, checked against the renderer's lock
       and the frames granted it, a renderer that sends what it may not
       killed. Fuzzed by `ipc_decode`, its seeds partly a session's
       traffic.
- [x] M34.3 Renderer sandbox per-OS (decision D2): Windows job objects +
       restricted token v1; document the macOS/Linux equivalents
       (sandbox-exec/seccomp) for portability.
       Done 2026-10-15: on Windows a restricted token of the browser's
       (no privilege but traversal, groups deny-only but Users, Everyone
       and the logon session's, Users, Everyone and RESTRICTED
       restricting), low integrity, a job of one process, 4 GiB and the
       desktop's state closed, three pipes and a crash channel
       inherited; probes inside a renderer for each refusal.
       `docs/security.md` says what macOS and Linux need (G113).
- [x] M34.4 Crash isolation: renderer death = sad-tab + respawn, browser
       process survives; crash reporting local-only (minidump written,
       user chooses to share — nothing phones home).
       Done 2026-10-15: a renderer gone leaves the browser's own page
       saying so, in the tab or in a frame's hole, its parent's load not
       held, until a reload or a navigation brings it back in a new
       renderer; on Windows the browser writes a minidump of a renderer
       that crashed by an exception or a panic, beside the profile, the
       newest eight kept, nothing sent.
- [x] M34.5 The `Engine` type becomes a session proxy in the browser
       process; the render pipeline state lives in the renderer.
       Done 2026-10-15: the session drives a `View`, an `Engine` in one
       process or a `RemoteTab` whose answers are what its renderers
       last reported; a view's frames go to the browser as the rows
       that changed (shared surfaces are G125).
- [x] M34.6 Migration plan executed incrementally: single-process remains
       a build feature (`--features single-process`) for debugging and
       constrained machines — tested in CI both ways.
       Done 2026-10-15: many processes by default, `--features
       single-process` the one-process browser, both linted and tested
       by the gate and CI.
- [x] M34.7 Acceptance: kill -9 a renderer mid-animation; the tab shows a
       crash placeholder and recovers; a renderer compromise cannot read
       another site's cookies (test via the adversarial suite).
       Done 2026-10-15: a renderer process killed in an animation leaves
       the page saying so and a reload brings it back in a new one
       (`--check-processes kill`); the adversarial site's pages run
       again in a tab of renderers with the same outcomes, and a
       renderer speaking the protocol itself is refused 21 requests,
       killed for each; it found requests of no origin or in a
       navigation's mode reading another site's HTML, fixed. The cap
       measured found the fallback fonts read into each renderer, fixed
       (120 MB of its own to 5 MB). T4.3 and the fuzz runs: in
       `docs/perf-budgets.md` and the notes.

---

## M35 — Performance v1  ·  all  ·  Effort: XL  ·  Depends: M13, M19, M25

- [ ] M35.1 Benchmarks committed: Speedometer-style interaction bench
       (subset), page-load bench on 20 saved pages, scroll/animation
       jank bench, memory footprints per page class. All recorded per
       milestone from now on (T4 track formalized).
- [ ] M35.2 Style: rule hash bucketing (rightmost compound), style
       sharing cache (sibling style reuse), state-bounded invalidation.
       At M26 the user agent sheet's 73 rules, 13 more for controls,
       `:any-link` and `:focus-visible`, are each tried on every
       element restyled: T4.3's restyles 9-16% slower.
- [ ] M35.3 Layout: parallel block layout (rayon subtrees where
       independent — the DAG shape from architecture.md pays off here).
- [ ] M35.4 Paint/raster: parallel raster tiles, display-list retention.
- [ ] M35.5 Memory: shared string/data arenas per pipeline stage;
       measure + cap the layer cache (M19.6) and the back-forward cache
       (M15, six documents for now); idle renderer memory trim.
- [ ] M35.6 Startup: cold start < 300ms to a blank tab on a mid laptop
       (measure; lazy-init everything non-critical).
- [ ] M35.7 Acceptance: numeric budgets published in
       `docs/perf-budgets.md`, regressions block milestone exit (T4).
- [ ] M35.8 Text measurement off layout's hot path: each font's
       shaping face kept rather than rebuilt per call, and a cache of
       word measures in layout (measured at M13: about 25µs a word,
       8µs of it the face, most of a 1,000-paragraph page's 700ms
       layout). The cache landed at M25, in `text::FontCollection`
       (`MAX_MEASURES` widths by size and text), when a table built by
       script spent a second a frame shaping its words again; the face
       kept is still to do.
- [ ] M35.9 Layout's work in proportion to the page on every pass
       (measured at M13): the box tree, rebuilt each pass and growing
       faster than the page (1.1ms at 1,000 paragraphs, 80ms at
       16,000), and the fragments, every one copied into each new
       layout since fragments hold absolute coordinates and copies of
       their styles (58ms of a 90ms pass at 10,000 paragraphs). A box
       tree updated in place, and fragments positioned relative to
       their parent that refer to their styles, so that a pass copies
       only what it laid out and a paint-only change only repaints.
       And a table's cells kept by the layout cache, which lays every
       cell out again with its table: at M25 a cell's text edited in a
       table of 1,000 rows cost 32 ms of cells measured and laid out,
       17 ms of box tree and 12 ms of fragments copied, a 46-58 ms
       frame.
       And the box tree and the layout made without recursion: a page
       nested to the parser's cap needs some 3.6 MiB of stack in
       release, 17 in a debug build and more under the address
       sanitizer, which overflowed Windows' 1 MiB main thread at M25; the
       shell's main thread has 32 MiB since (`engine::STACK_SIZE`).
- [ ] M35.10 Compositing v2 (found at M19, `docs/design/compositing.md`
       section 2): the document's tiles drawn opaque over the canvas
       colour, so that a frame without them draws the page through at
       a whole raster's cost and composites them as copies, to the
       same pixels; a layer tree beside the tiles, so that a moving
       layer inside the document composites over them rather than
       drawing them again; raster at the device transform for a layer
       that does not animate, and a layer's raster scale kept through
       an animation.
- [ ] M35.11 A restyle's cost an element and a whole viewport's raster
       (measured at M25, `benches/dom.rs`): a class toggled on a table
       restyles its elements at 7-8 us each, more than the whole
       cascade's 6, and rasters the viewport again in software in about
       10 ms; a 100-row table's toggle is a 20 ms frame.
- [ ] M35.12 A realm's bindings loaded compiled: every realm compiles
       the DOM's bootstrap, 3,300 lines of JavaScript, 4.5 ms of the
       5 ms a realm now takes to make (0.12 ms before M25); 9,158 lines
       and 15.6 ms at M27, whose `fetch`, XHR and bodies added 5 ms.
       QuickJS writes and reads compiled code, but `rquickjs` reads it
       only through an `unsafe` function, and `js` writes no `unsafe`:
       a reviewed exception, or a safe reader, first.

---

## M36 — DevTools v0 + remaining storage  ·  new `devtools` crate  ·  Effort: XL  ·  Depends: M25, M34

- [ ] M36.1 Console: log/warn/error objects, network + JS errors.
- [ ] M36.2 DOM inspector: live tree, style inspection (matched rules +
       computed), attribute editing.
- [ ] M36.3 Network panel: request log, headers, timing, body preview.
- [ ] M36.4 Storage UI + IndexedDB implementation (via D10 engine):
       `indexedDB.open/getAll/put/delete` + object stores + indexes,
       spec transactions v1.
- [ ] M36.5 Acceptance: debug the todo-list demo entirely in-engine.

---

## M37 — Browser chrome v1  ·  `shell`  ·  Effort: XL  ·  Depends: M15, M34

- [ ] M37.1 Tab system: multiple renderers, tab strip UI (in-engine
       drawing — D9), per-tab history, crash placeholders.
- [ ] M37.2 Downloads: download manager, progress, open-folder,
       `Content-Disposition` handling, safe default paths. (Since M15 a
       response that would be a download leaves the page shown, with a
       notice, `docs/design/navigation.md`.)
- [ ] M37.3 Settings: startup page, search-engine selection, JS/CSP
       toggles (no-scripts mode), dark mode, engine feature flags,
       languages (`Accept-Language`, fixed at `en-US,en;q=0.9` since
       M14).
- [ ] M37.4 Error/network pages polish; `about:` pages (`about:silver`
       = engine demo gallery — the M1 demo document graduates here).
- [ ] M37.5 Profile system: multi-profile v1 (per-profile cookie/storage
       jars).
- [ ] M37.6 Acceptance: daily-driving dogfood sessions begin (T1 gate:
       the developer uses it for 30 minutes of real browsing weekly and
       files the failures).

---

## M38 — HTTP/2 + real cache  ·  `net`  ·  Effort: L  ·  Depends: M27

- [ ] M38.1 HTTP/2: streams/priorities/flow-control, HPACK, connection
       coalescing (rustls ALPN).
- [ ] M38.2 Cache: `Cache-Control` (max-age, no-store, no-cache,
       immutable), conditional requests (`ETag`/`If-None-Match`,
       `Last-Modified`), `Vary`, 304 handling, disk-backed via profile.
- [ ] M38.3 Preload/scanner: speculative subresource discovery during
       HTML tokenization (the architecture doc predicted this — the
       tokenizer emits resource hints as it sees tags).
- [ ] M38.4 Acceptance: cached repeat-load of a 20-resource page makes
       zero network requests; HTTP/2 multiplexing verified against a
       test server.

---

## M39+ — The long tail (gated, in rough priority order)

Each gets a full milestone spec (per §4 template) **only when the
gate before it opens** — this document deliberately does not
gold-plate unstarted work.

- [ ] M39 Service workers + Cache API (offline; the single hardest web
       platform feature — gate: T3 usage demand).
- [ ] M40 WebAudio, Media elements (`<video>`: decode + basic controls;
       media keys; DRM explicitly never (broken by design)).
- [ ] M41 WebGL1/2 (on wgpu), WebGPU (gate: M19.3 maturity).
- [ ] M42 Full bidi + ICU4X integration (writing modes, vertical text);
       the Encoding Standard's legacy CJK encodings (GBK/gb18030, Big5,
       EUC-JP, ISO-2022-JP, Shift_JIS, EUC-KR), sooner if a T1 page
       needs one (R1; `docs/design/net.md`).
- [ ] M43 WebRTC (gate: T4).
- [ ] M44 Accessibility (accessibility tree, platform APIs — critical
       for real users, gate: T3 exit).
- [ ] M45 i18n UI, input methods (IME).
- [ ] M46 Password manager, autofill (gate: T4, security-reviewed).
- [ ] M47+ WPT-driven everything: the forever list (T1 track becomes the
       primary driver — see track below).

Deferred from the milestones above, each with what it waits on:

- [ ] G1 `subgrid` (css-grid-2 §9): a grid item's tracks taken from its
       parent grid's, with the parent's line names and gaps. Deferred
       from M18.6; `docs/design/grid.md`, section 2.
- [ ] G2 Masonry (css-grid-3, now `grid-lanes`): one axis of tracks, the
       other packed item by item. Deferred from M18.6, the draft still
       moving.
- [ ] G3 Self-alignment of an absolutely positioned box in its
       inset-modified containing block (css-align-3 §6.1,
       css-position-3 §4): `justify-self` and `align-self` place a box
       whose insets leave it room, as the reference browsers do, in a
       grid area or a padding box alike. Positioning's (M11), found at
       M18, which uses the two properties only for a static position.
- [ ] G4 A layout that knows its pages (CSS Fragmentation): content laid
       out again in the room each page leaves, a float and the lines
       beside it across a break, flex and grid containers, tables and
       inline-blocks broken inside, pages of other widths, named pages
       (`page`) and multi-column layout. Deferred from M20.4, whose
       pagination moves laid-out content; `docs/design/media.md`,
       section 2.
- [ ] G5 Printing's next steps: a PDF of text and paths drawn from the
       display list, its fonts embedded and its text selectable, which
       would make a page a few kilobytes rather than 420 KiB; the
       page-margin boxes, running headers and `counter(page)`; and
       `print-color-adjust: economy`. Deferred from M20.4.
- [ ] G6 The `color-scheme` property, the dark canvas and form controls
       a page asking for the dark scheme gets, `light-dark()` and the
       system colours (CSS Color Adjustment 1). Deferred from M20.3.
- [ ] G7 `revert` and `revert-layer`, which roll a property back to an
       origin or a layer. Deferred from M20.2.
- [ ] G8 `@container`, `@scope`, CSS nesting (with `@media` inside a
       style rule) and custom media queries (`@custom-media`), which the
       cascade passes over. Deferred from M20.1.
- [ ] G9 The user's colour scheme and reduced motion on Linux from the
       XDG desktop portal, over D-Bus; and the resolution media feature
       at the display's scale factor, once the engine draws at it.
       Deferred from M20.3. The resolution at the display's scale: done
       at M32.
- [ ] G10 Line breaking by UAX #14 (css-text-3 §5): soft wrap
       opportunities after hyphens and the rest of the line-breaking
       classes, where M7.3 wraps at white space only; and `white-space:
       nowrap`, which M8.4 did not take. Found at M20.5.
- [ ] G11 The root element's margins (CSS2 §8.3.1): a child's top margin
       collapses through the root's box here, which the root's box should
       not let it, so that the root's box starts below the margin. Found
       at M20.
- [ ] G12 What M20's web-platform-tests ask of the rest of the engine:
       the `visibility` property; `@font-face` and the page's fonts,
       which `font-format()` and `font-tech()` would answer; `:has()`;
       `::details-content` and the pseudo-elements of form controls; and
       the HTML parser's "in head noscript" insertion mode. Found at M20.
- [ ] G13 The print reftests of the web-platform-tests, which compare
       paginated renderings: the harness renders the screen alone.
       Deferred from M20.
- [ ] G14 Registered custom properties: `@property` and
       `CSS.registerProperty()`, a syntax, an initial value and
       inheritance of a property's own, computed and animated by its
       type (CSS Properties and Values API 1). Deferred from M21.1;
       `docs/design/values.md`, section 2.
- [ ] G15 The other math functions: `round()`, `mod()`, `rem()`,
       `abs()`, `sign()`, the trigonometric functions, `pow()`,
       `sqrt()`, `hypot()`, `log()` and `exp()`; Values 5's
       `progress()`, `random()`, `calc-size()`, `if()`, `inherit()`,
       `@function` and the spread syntax. Deferred from M21.2.
- [ ] G16 Colour functions, `rgb()`, `hsl()` and the rest, and
       `currentcolor` outside `text-decoration-color`; the engine takes
       hexadecimal and named colours alone. Found again at M21, and at
       M22, whose reference pages write theirs in hexadecimal. `rgb()`
       and `rgba()` landed at M25, which `getComputedStyle` gives a
       script to write back; `hsl()`, `hwb()`, `lab()`, `color()` and the
       rest remain.
- [ ] G17 `@counter-style` and `symbols()`, the complex predefined
       counter styles of Counter Styles 3 §7 (the longhand Chinese,
       Japanese and Korean styles, `ethiopic-numeric`) and `speak-as`.
       Deferred from M21.4.
- [ ] G18 `env()` and `var()` outside properties, in media queries and
       `@page`'s descriptors; author-defined environment variables.
       Deferred from M21.3.
- [ ] G19 Markers on `::before` and `::after`, `display: inline
       list-item` and `flow-root list-item`, `marker-side`, and a
       marker beside a table placed off its first row as the reference
       browser places it. Deferred from M21.6.
- [ ] G20 The rest of text decoration: `text-decoration-skip-ink` and
       `-skip`, emphasis marks, `text-shadow`, the spelling and grammar
       lines, and `text-decoration` on `::first-line` and
       `::first-letter`. Deferred from M21.7.
- [ ] G21 Justification beyond expanding gaps and characters:
       compressing a line, the rules of Chinese, Japanese, Korean and the
       South-East Asian scripts (with G10), cursive elongation,
       `text-align` with a string; and `match-parent` on a right-to-left
       root. Deferred from M21.7.
- [ ] G22 The fonts' remaining properties: `font-weight`, `font-style`,
       `font-feature-settings`, `line-height` and the `font` shorthand,
       which M5.6, M8.2 and M20's note pointed at M21 and none of its
       items holds. Registered at M21.
- [ ] G23 `display: flow-root`, which the cascade does not take. Found
       at M21.
- [ ] G24 Logical properties (`padding-inline-start` and the rest) and
       `:dir()`, which HTML's rules give lists, so that a right-to-left
       list is padded on its right. Found at M21.6.
- [ ] G25 A grid's rows sized a second time for an item's percentage of
       the first sizing's height (css-grid-2 §11), as the reference
       browser grows an `auto` row for an item's `min-height:
       calc(25px + 50%)`, where this engine takes the percentage of
       zero. Found at M21.
- [ ] G26 The Web Animations API: `Element.animate()`,
       `getAnimations()`, the `Animation`, `KeyframeEffect`,
       `CSSAnimation` and `CSSTransition` objects, playback rates,
       seeking and reversing, iteration start, and the `add` and
       `accumulate` composite operations with Animations 2's
       `animation-composition`. Deferred from M22;
       `docs/design/animations.md`, section 2.
- [ ] G27 Scroll-driven animations: `animation-timeline`, `scroll()`
       and `view()` timelines, `animation-range`, keyframe selectors of
       a timeline's ranges, and `animation-delay-start` and
       `-end`. Deferred from M22.
- [ ] G28 Entry transitions and sizes: `@starting-style`, `overlay`,
       and `interpolate-size` with `calc-size()` (G15), so that an
       element just shown transitions from a style and a size from
       `auto`. Deferred from M22.
- [ ] G29 A compositor thread: transform and opacity animations kept at
       their rate while a page's own work takes longer than a frame.
       Here they are composited without layout, on the engine's one
       thread. Deferred from M22.4.
- [ ] G30 A user's setting to stop or slow animations (Transitions 1
       §10.1), their events still sent. Deferred from M22.5.
- [ ] G31 What M22's web-platform-tests ask of the rest of the engine:
       shadow trees, declarative shadow roots among them, which no
       milestone holds; and the root's or the body's overflow carried
       to the viewport (css-overflow-3 §3.3), which tests since M17
       have asked for. Found at M22. *(M33: the viewport takes it for
       its scrollbar and the user's scrolling; the element still clips
       as its own box.)*
- [ ] G32 `border-radius`, `box-shadow` and `filter`, which no
       milestone holds: M22's spinner is a square ring for want of the
       first. Found at M22.
- [ ] G33 A CSS transition's side effect (Web Animations 1 §5.6): the
       box acting as if `will-change` named the property it transitions,
       so that a transition of a transform from `none`, or of an
       opacity from 1, is composited from its start rather than laid
       out where the box becomes a stacking context. Found at M22.
- [ ] G34 AVIF, once an AV1 decoder meets `deny.toml` and builds on
       Windows without NASM: `ravif`, which M23.1 named, only encodes.
       Until then an AVIF image is broken, and an `image-set()` option
       of its type is skipped. Found at M23.
- [ ] G35 `srcset`, `sizes` and `<picture>`, and EXIF density correction
       (HTML 4.8.4.3.6): an `img`'s `src` is its fallback. Found at M23.
- [ ] G36 `aspect-ratio` on boxes that are not replaced: it computes but
       sizes only replaced boxes. Found at M23.
- [ ] G37 Images 4: conic gradients, `<color-interpolation-method>`,
       `image()`, `cross-fade()`, `element()` and `image-resolution`.
       Found at M23.
- [ ] G38 Colour management: embedded ICC profiles, `gAMA`, `cHRM` and
       `cICP` are not applied; decoded pixels are taken as sRGB. Found
       at M23.
- [ ] G39 Progressive display of an image not yet completely loaded,
       which HTML allows; an image is shown whole or not at all. Found
       at M23.
- [ ] G40 Backgrounds 4's `background-position-x` and `-y`. Found at M23.
- [ ] G41 Module scripts, import maps and speculation rules: a `script`
       of those types is recognized and not run. Found at M24.
- [ ] G42 `Intl` and `Temporal`, which QuickJS-ng does not implement.
       Found at M24.
- [ ] G43 Subresource Integrity: a `script`'s `integrity` is not
       checked. Found at M24.
- [ ] G44 The tree builder's missing insertion modes: the table modes,
       the active formatting elements and the adoption agency, foster
       parenting, templates and foreign content beyond self-closing
       elements; comments and doctypes in the tree ("in head noscript" is
       G12's). Found at M24. Foreign content: done at M32 (html5lib's
       tests of it pass but for those of the table, select and frameset
       modes).
- [ ] G45 HTML's presentational attributes and elements (HTML section
       15, Rendering), as hints the cascade takes below the author's
       rules, as M21.6 does for lists and M23 for images: `<center>` and
       `align` on blocks, headings, tables and cells, with
       `text-align`'s `-webkit-center` and `match-parent` that they
       need; `bgcolor`, `background`, `text`, `link`, `vlink` and
       `alink` on `body`; the table attributes `border`, `cellpadding`,
       `cellspacing`, `width`, `height`, `bgcolor`, `nowrap` and
       `valign` (a gap stated at M12); `hspace`, `vspace` and `border`
       on images; and `<font>`'s `color`, `face` and `size`. Old pages
       and pages served to browsers without scripts lean on them:
       Google's basic page is laid out uncentred without them. Found at
       M24.
- [ ] G46 A style and layout flush for scripts: `getComputedStyle`
       that starts transitions, shows animations and sees the sheets a
       running script added; the resolved values of `width`, `height`
       and the rest, which are the used values; CSSOM View's layout
       queries (`offsetWidth`, `clientWidth`, `getBoundingClientRect`,
       `scrollTop`, `scrollTo`, `innerText`). Found at M25.
- [ ] G47 More kinds of node: `Comment`, `DocumentFragment`,
       `DocumentType`, `ProcessingInstruction`, `CDATASection`, `Attr`
       and `attributes`, with `createComment`, `createDocumentFragment`
       and `template`'s content, which the parser's comments and
       doctypes (G44) would make too. Found at M25.
- [ ] G48 `document.open` and `document.close`, and a `document.write`
       with no insertion point replacing the document (a `javascript:`
       URL's string result is M26.6's). Found at M25.
- [ ] G49 `MutationObserver`, which M13 expected of M24 or M25. Found
       at M25.
- [ ] G50 CSSOM's serialization of specified values and of shorthands
       in `style` and `cssText`, which keep the text they were given;
       `document.styleSheets` and the CSSOM's rule objects. Found at M25.
- [ ] G51 Namespaces: `createElementNS`, and MathML's elements as their
       own interfaces (SVG's are M32's). Found at M25. `createElementNS`,
       the namespaced attribute methods for XLink's and XML's, and
       `MathMLElement`: done at M32; attributes of other namespaces,
       `[ns|attr]` selectors and MathML's own interfaces remain.
- [ ] G52 Traversal and ranges: `TreeWalker`, `NodeIterator`, `Range`,
       `Selection`, `compareDocumentPosition`, `isEqualNode`,
       `normalize`. Found at M25.
- [ ] G53 Custom elements and shadow trees. Found at M25.
- [ ] G54 The window's named properties (HTML 7.2.2.3): an element by
       its id as a global (`toggler.className`), which most of the
       selector invalidation reftests use. Found at M25.
- [ ] G55 A viewport for `--print-to-pdf`'s load: the page's scripts
       read a 0 by 0 window, the shell's window giving the engine its
       size only when one is shown. Found at M25.
- [ ] G56 Dialogs the user answers: `alert`, `confirm` and `prompt`,
       which log and answer as suppressed dialogs do. Found at M26.
- [ ] G57 Leaving a page: `beforeunload`'s prompt, and asking before a
       form is sent again (the resubmission page stands in). Found at
       M26.
- [ ] G58 Pointer capture (`setPointerCapture` and its events); touch
       and pen input; drag and drop. Found at M26.
- [ ] G59 Editing hosts: `contenteditable`, `designMode`,
       `document.execCommand`, and selecting and copying a page's own
       text. Found at M26.
- [ ] G60 Input methods: composition events, and the text an input
       method composes shown before it commits, in the page as in the
       address field. Found at M26.
- [ ] G61 Pickers: the `file` control (choosing a file, the files in a
       form's body), `color`, `range`, the date and time types, and an
       image button's picture and the point clicked; text fields until
       then. Found at M26.
- [ ] G62 Undo and redo in text controls. Found at M26.
- [ ] G63 `dirname`, and the direction of a control's text. Found at
       M26.
- [ ] G64 A validation message beside its control, as the reference
       browser draws it; the status bubble says it now. Found at M26.
- [ ] G65 Observers and idle callbacks: `IntersectionObserver`,
       `ResizeObserver`, `requestIdleCallback` (`MutationObserver` is
       G49's); a `dialog` form's method. Found at M26.
- [ ] G66 DOM's `moveBefore`, a move that keeps a node's state. Found at
       M26, by a crashtest that threw on it.
- [ ] G67 Streams: `ReadableStream` and the rest of the Streams Standard,
       `Response.body` and `Request.body`, a request body streamed,
       `Blob.stream()`, progress events as bytes arrive. Named at M27
       (`docs/design/fetch.md`), 111 web-platform subtests.
- [ ] G68 The `URL` class and its setters, `URL.createObjectURL` and
       `blob:` URLs. Named at M27, 134 web-platform subtests.
- [ ] G69 `TextEncoder` and `TextDecoder`. Named at M27.
- [ ] G70 XHR's `document` response type and `responseXML`, a `Document`
       sent as a body, `DOMParser`: an XML parser and a second
       document. Named at M27.
- [ ] G71 HTTP authentication: a 401 or 407 answered with credentials,
       and a URL's credentials (refused by `fetch` and XHR until then).
       Named at M27.
- [ ] G72 The Cookie Store API (`cookieStore`). Named at M27.
- [ ] G73 Requests that outlive their page: `keepalive`,
       `navigator.sendBeacon`, `fetchLater`. Named at M27.
- [ ] G74 WebSockets and server-sent events (`EventSource`). Named at
       M27.
- [ ] G75 `FileReader`. Found at M27, by the web-platform tests of
       `FileAPI/blob` (39 subtests).
- [ ] G76 HTML's `formdata` event, and the constructing entry list's
       `InvalidStateError`. Found at M27.
- [ ] G77 A response whose header section ends at the connection's
       close, without its empty line, read as the reference browser
       reads it (`net` takes it as truncated). Found at M27, by XHR's
       `.asis` tests.
- [ ] G78 Lone surrogates kept in `localStorage` and `sessionStorage`
       keys and values, as the reference browser keeps them: here every
       string a script hands the engine is well formed, a lone
       surrogate U+FFFD. Named at M28, 12 web-platform subtests.
- [ ] G79 A WebAssembly memory's bytes shared with the script's
       `ArrayBuffer`, not copied at the calls' edges, and the memory's
       own fixed-length and resizable buffers
       (`toFixedLengthBuffer`, `toResizableBuffer`). Named at M29: a
       call with a megabyte's memory seen costs 1.6 ms, and the `js`
       crate writes no `unsafe` to lend the bytes.
- [ ] G80 WebAssembly's proposals past what `wasmi` has: SIMD and
       relaxed SIMD, memory64 and 64-bit tables, exception handling,
       GC and typed function references, JavaScript promise
       integration, the JS string builtins, the type reflection
       interface. Named at M29: the core specification's suite, Wasm
       3.0's, needs memory64 and GC to run; 46 subtests of
       `wasm/jsapi`.
- [ ] G81 The 2D canvas past M30: its calls batched in the bootstrap, a
       command buffer sent once a frame, and its drawing on the GPU,
       which 2,000 particles at sixty frames a second need (26.9 ms a
       frame at M30: 14 ms of calls at 0.8 to 1.8 us each, 5.3 ms of
       tiny-skia's fills); `filter`; colour spaces past sRGB, pixel
       formats past 8 bits and the HDR attributes; `TextMetrics`'s
       proposed members and `fillTextCluster`; a style given as a
       colour object. Named at M30 (`docs/design/canvas.md`, section
       2), with 191 of the web-platform subtests it leaves failing.
- [ ] G82 Shared workers (`SharedWorker`, `SharedWorkerGlobalScope`,
       `onconnect`) and service workers' own globals (M39's). Named at
       M31 (`docs/design/workers.md`, section 2).
- [ ] G83 `SharedArrayBuffer` and `Atomics.wait` between threads, with
       `crossOriginIsolated` and the headers that give it (COOP and
       COEP), and WebAssembly's shared memories with them. Named at M31.
- [ ] G84 `BroadcastChannel`. Named at M31.
- [ ] G85 Workers past M31: an `ImageBitmap` in a transfer list; a
       placeholder canvas's `toDataURL` and `toBlob` of the frame its
       worker sent; a cross-origin `importScripts` script's errors
       muted; `WorkerNavigator`'s `hardwareConcurrency` and the
       navigator members neither global has here; a page with workers
       kept by the back-forward cache, its workers suspended; messages'
       bytes crossing without base64 (MESSAGE_COST); a length-tracking
       view's clone tracking its buffer (QuickJS-ng keeps whether a
       view tracks from script); `MessageEvent`'s `userActivation`.
       Named at M31.
- [ ] G86 `matchMedia()` and `MediaQueryList`: `matches`, `media` and
       its `change` events as the viewport and the preferences change.
       M20's note left them to M24 and M25, which did not take them
       up. Found 2026-10-12 in Google's account bar, which calls
       `matchMedia` as it starts and fails without it (with `URL`,
       G68).
- [ ] G87 SVG past v1 (M32): filters, masks, markers, patterns, `<image>`
       and `foreignObject`'s HTML; `textPath`, `rotate`, `textLength`,
       vertical and bidirectional text, `lang`'s localized forms, text
       in a `clipPath`, a `tspan`'s own opacity; the SVG DOM's animated
       values (`SVGAnimatedLength`, `SVGTransformList`, `baseVal`),
       `getNumberOfChars()`, `pathLength`; `context-fill` and
       `context-stroke`; `vector-effect`'s stroke box; `<view>`,
       fragment identifiers and linked parameters of an SVG image;
       references into another document (`use`, paint servers,
       markers); 3D transforms of SVG content (drawn flattened); a
       gradient's `transform` property; a group's `fill-box`; a use's
       instance as a shadow tree to selectors; a template in a place
       nothing renders; the `svg` element's `load` event; an svg's tree
       kept per svg rather than per document version, its drawing
       counted in the image store's budget, an SVG image's drawings
       kept per size, its text drawn by print and `drawImage`; an SVG
       document's scripts with a `src` in order, its error page with
       the rendering up to the error, its encoding told. Named at M32.
- [ ] G88 The `CSS` namespace object: `CSS.supports()`, `CSS.escape()`
       and the rest. Found at M32 by the probes of SVG's properties.
- [ ] G89 What M32's web-platform tests found outside SVG: a border
       image and a list marker of an image of no natural size (an SVG
       of a ratio alone) sized by the border image area and the
       marker's default rather than 300 by 150; an SVG image's media
       queries told its page's color scheme; `attr()` with a type
       (css-values-5); a `::before` or `::after` of `display:
       list-item` a list item; a lazy `img` whose `src` changes in the
       load event; an element replaced by `content: url()` after its
       first frame; an image's `min-height` through its ratio in a
       container's min-content width; a scrolled layer's tiles at a
       display's scale (it is kept whole there, and repaints as it
       scrolls). Found at M32.
- [ ] G90 Same-origin scripting across frames: a parent reaching into
       its own origin's frame (`contentDocument`,
       `contentWindow.document`, `parent.someFunction()`),
       `document.domain` relaxing it, `opener`; `storage` events to
       other frames' documents of the same key. Every frame is
       cross-origin to every other until M34's site instances share a
       runtime (D24). Found at M33.
- [ ] G91 A frame's layers composited on the GPU: its pixels are drawn
       in software and uploaded. With M34's out-of-process frames.
       Found at M33.
- [ ] G92 The joint session history: a frame's navigations as entries
       of the tab's history. Found at M33.
- [ ] G93 Blocking third-party cookies as a setting (D25). With M37.3's
       settings. Found at M33.
- [ ] G94 Trusted Types (`require-trusted-types-for`,
       `trusted-types`). Found at M33.
- [ ] G95 `report-to` and the Reporting API (`Reporting-Endpoints`),
       which carries other reports too. Found at M33.
- [ ] G96 Permissions Policy past Local Network Access's features: the
       `Permissions-Policy` field, `allow` for the other features.
       Found at M33.
- [ ] G97 The Permissions API (`navigator.permissions.query`). Found at
       M33.
- [ ] G98 The profile's key on macOS and Linux (Keychain, Secret
       Service): unwrapped there (D27). Found at M33.
- [ ] G99 The OSS-Fuzz application and continuous fuzzing on its
       machines, for when the repository is public;
       `docs/security.md` says what it would hold. Found at M33.
- [ ] G100 Revocation and Certificate Transparency: the reference
       browser checks neither online; it enforces CT and revokes by
       lists it pushes to itself, which go stale without an update
       service, a CT check with a stale list to be switched off after
       70 days as its own is. With T6.3's auto-update. Found at M33.
- [ ] G101 Encrypted Client Hello, which needs a DNS client of the
       browser's own for HTTPS records (DNS over HTTPS). Found at M33.
- [ ] G102 `<frame>`, `<frameset>`, and HTML in `<object>` and
       `<embed>`. Found at M33.
- [ ] G103 `loading=lazy` frames, `credentialless`, the `csp` attribute
       (embedded enforcement), `allowfullscreen` and the Fullscreen
       API. Found at M33.
- [ ] G104 CSP's edges the web-platform tests found: a nonce hidden
       only when the parser inserted its element; a script whose
       attribute names `<link` still nonceable, and a nonce given twice
       honoured; a violation's line and column zero, a worker's firing
       no event, and an icon's named by a `<link>` reaching no
       listener; a report-only violation beside an enforced hash
       reported as enforced; a violation reported for a script
       `'strict-dynamic'`'s hash allows; a string a timer was refused
       still making a timer; U+000B in a directive's value; a frame's
       own subresources not upgraded by its parent's
       `upgrade-insecure-requests`, its navigations alone. Found at
       M33.
- [ ] G105 Interfaces the M33 pages and tests found missing:
       `document.scripts`, `document.links`, `document.styleSheets`
       and `cssRules`, `baseURI`; named access to an element by its id
       on `window`; `getComputedStyle` by a hyphenated name for
       `background-image`; an image `new Image()` makes, which fetches
       nothing (one `createElement` makes does). Found at M33.
- [ ] G106 Frames' edges: a `javascript:` URL as a frame's `src` is not
       run; a target naming a frame of a document still parsing finds
       none, and the navigation goes to the top; popups (`window.open`,
       `_blank` as a new tab, `opener`, `allow-popups-to-escape-sandbox`)
       wait for M37's tabs. Found at M33.
- [ ] G107 The web-platform tests M33 added that fail undiagnosed: 41
       entries across 35 tests, each named in `expectations.txt` as not
       diagnosed at M33. Found at M33.
- [ ] G108 Referrer Policy's and the mixed form's edges:
       `document.referrer` is the referrer the first hop sent; a
       stylesheet's own requests carry the document's URL, and a
       worker's its own by the default policy, its response's field not
       read; a form submitted into another frame by its target, or
       redirected to `http:`, is not held on the warning page. Found at
       M33.
- [ ] G109 The profile's encryption's edges: the pages `redb` freed when
       a profile of schema 1 was sealed keep their values in the clear
       until written again; a profile opened with a wrap that cannot
       unwrap its key starts over with a new one, its values lost.
       Found at M33.
- [ ] G110 A GPU process: renderers draw in software, the GPU's driver
       being a large surface to give a sandboxed process; the one
       process keeps the GPU backend. Found at M34.
- [ ] G111 The network denied to a renderer: a low-box (AppContainer)
       token without `internetClient`; a renderer taken over can open a
       socket of its own, though it holds no cookie to send. Found at
       M34.
- [ ] G112 The reference browser's lockdown: a token admitting nothing
       after a start under another, untrusted integrity, a desktop of
       the renderers' own, win32k closed, the mitigation policies.
       Found at M34.
- [ ] G113 The sandbox on macOS (a Seatbelt profile by `sandbox_init`)
       and Linux (seccomp-BPF in new user, PID and network namespaces),
       and the crash dumps there. Found at M34.
- [ ] G114 A sandboxed frame in a process of its own, as the reference
       browser isolates one. Found at M34.
- [x] G115 Input hit-tested by the browser: not needed, the browser
       hit-tests the pointer itself by the frames' holes (M34).
- [ ] G116 A remote frame under a transform, an opacity or a filter:
       its hole is found by its colour, which those change. Found at
       M34.
- [ ] G117 `history.state` and form state kept by the browser: lost
       with a discarded segment's renderer. Found at M34.
- [ ] G118 A soft cap: past `MAX_RENDERERS` showing something, a
       frame of a new site stays empty. Found at M34.
- [ ] G119 Cross-origin isolation: `Cross-Origin-Opener-Policy`,
       `Cross-Origin-Embedder-Policy`, `crossOriginIsolated`, and
       `Origin-Agent-Cluster`'s processes by origin. Found at M34.
- [ ] G120 A hung renderer, noticed and offered to be stopped; only a
       dead one is noticed, and a panic on a renderer's thread other
       than its main loop is not reported. Found at M34.
- [ ] G121 ORB's JavaScript parse, done by sniffing for what is not
       script. Found at M34.
- [ ] G122 Frames across renderers: a remote frame's window answers
       `postMessage` and `location` alone (its `length`, `frames` and
       document are not read across renderers), ports are not carried
       between renderers, the wheel is not given back to the parent
       when a frame does not scroll, sequential focus does not enter a
       remote frame. Found at M34.
- [ ] G123 The URL a renderer reports for its page is shown as it says:
       error pages and the browser's made pages commit under the URL
       they were asked for, in the renderer that asked, so a renderer
       taken over can show another site's URL over its own content.
       The browser's own error and made pages, then the URL held to the
       lock. Found at M34.
- [ ] G124 `--print-to-pdf` runs its page in the browser's own process,
       unsandboxed. Found at M34.
- [ ] G125 Shared surfaces (D32): a view's frames go through the pipe
       as rows; a click reaches its frame in 25 ms through renderers,
       1.5 ms in one engine. Found at M34.

---

# 5. Cross-cutting tracks (never "done", run through every phase)

## T1 — Compatibility & testing
- [x] T1.1 WPT runner harness (pull web-platform-tests, run subset,
       triage UI) — **start at M4**, becomes the main compat driver by M17.
       Done 2026-10-03 (M17): `examples/wpt.rs` pulls a directory's tests
       by a rule of what the engine has; `tests/wpt.rs` runs reftests,
       check-layout tests and crashtests against `expectations.txt`, and
       writes `target/wpt/report.html`, with a failing reftest's render
       beside its reference's.
- [x] T1.2 Golden-rendering harness: fixed test pages -> frame buffers ->
       compare vs reference (Firefox) with tolerance; runs in CI.
       Done 2026-10-01 (M10.7) as `tests/reference.rs`: pages captured
       from a headless reference browser, Chrome rather than Firefox
       (the box model's note), compared byte for byte or within a
       tolerance a page declares; 132 pages at M31, 123 to 127 and
       132 run on the reference browser's virtual time, M27's two
       waiting for their fetches, M28's served over `http:` for its
       storage, and M31's waiting for its worker.
       It is one of the workspace's tests, which CI runs (`cargo
       test`) and the gate runs on the machine while CI's runs are
       stopped by the account's billing.
- [ ] T1.3 Adversarial test-site (built in-repo): every attack pattern
       gets a page; the suite must fail closed. Extended at M27, M33.
       Started 2026-10-11 (M27.7): `tests/security/cross-origin.html`,
       26 cross-origin reads that must learn nothing, run against a
       victim that allows nothing and one that allows the page by
       CORS (`loader/security_tests.rs`).
- [ ] T1.4 Top-100 crawl fixture: saved HAR + DOM snapshots of the top
       sites (refreshed quarterly), render-diff reports.
- [ ] T1.5 CI matrix: Windows/macOS/Linux, debug + release, single-
       process + multi-process (M34 on).

## T2 — Fuzzing
- [ ] T2.1 cargo-fuzz targets: `url_parse` (M14), `css_tokenize` (M3 —
       start now), `css_parse` (M3), `selector_parse` (M4),
       `html_tokenizer` + `html_parse` (M2), `font_data` (M1).
       Started 2026-10-01: `fuzz/` has `html_parse`, `css_stylesheet`
       (tokenizer, stylesheet and style attribute), `selector_parse`,
       `style_document` and `pipeline`; `mutation` since M13,
       `url_parse`, `encoding_decode`, `mime_sniff` and `http_response`
       since M14, `media_query` since M20, when `pipeline` began to
       paginate and print, `values` since M21, which holds math
       functions, substitution, typed `attr()`, counters and generated
       text to their bounds, and `animations` since M22, which holds
       easing functions, timing, interpolation and frames drawn at a
       series of times to theirs; `dom_api` since M25 (T2.4), and
       `input` since M26, which uses pages of controls through the
       engine's input, the engine's clock moved by hand, both compared
       with a fresh engine given the state no markup holds; `cookies`
       since M27, which holds every cookie a store keeps to the rules,
       the jar to its limits and the header to the cookies that match;
       `font_data` waits for its milestone.
- [ ] T2.2 Seed corpora from WPT + real-site captures; corpus committed.
- [ ] T2.3 Coverage-guided runs in CI nightly; crash → reproducer test
       before anything else. Since 2026-10-01 CI runs every target for a
       minute a push and half an hour weekly, the corpus carried in the
       actions cache (`media_query`, left out of the list at M20, and
       `values` joined it at M21, `animations` at M22).
- [x] T2.4 Structured fuzzing for the JS boundary (M24+): API call
       sequences, not just strings. Done 2026-10-10 (M25): the
       `dom_api` target, calls drawn from M25's surface written out as
       scripts, run as the page is parsed and after frames, the
       document validated and the frame held to a fresh engine's; in
       CI's matrix. M26 gave it events, listeners, timers, animation
       frames and the controls' state among its calls; M27 `fetch()`
       and XHR of `data:` URLs, settled after each run, the bodies and
       `document.cookie`.

## T3 — Security cadence
- [ ] T3.1 Quarterly dependency audit (`cargo audit`), pinned Cargo.lock,
       `cargo deny` (advisories + licenses).
- [x] T3.2 `unsafe` review gate: any PR adding `unsafe` carries a safety
       argument in code (enforced by convention + grep in CI).
       Done 2026-10-13 (M33.5): `#![forbid(unsafe_code)]` everywhere but
       the shell's platform module, each block argued, held by the
       shell's test and CI's grep.
- [ ] T3.3 `docs/security.md` updated with every milestone (threat model
       lives there from M33 on; before that, in architecture.md notes).
       Begun 2026-10-13 (M33.7): each milestone's documents step updates
       it.
- [x] T3.4 Secret-handling rule: no network egress anywhere except the
       `net` crate's fetch path. CI grep-enforced. Done 2026-10-02
       (M14): CI's check job fails when `TcpStream`, `UdpSocket`,
       `ToSocketAddrs` or `rustls` appears under `crates/` or `fuzz/`
       outside `crates/net`.

## T4 — Performance
- [x] T4.1 `docs/perf-budgets.md` exists from M13; every milestone exit
       includes the numbers. Done: written at M13, a section of
       numbers for every milestone since.
- [ ] T4.2 Criterion benches in-repo (`benches/`), run in CI, tracked.
       Half done: the benches are in-repo, plain binaries
       (`harness = false`) run by hand, their numbers kept in
       `docs/perf-budgets.md`; Criterion and CI runs are not.
- [x] T4.3 Regression rule: >10% regression on any tracked bench blocks
       exit (unless a written exception with a follow-up item). Done:
       applied at every milestone's exit since M15, the code before and
       after it timed by turns (`t43.sh`), each exception written in
       `docs/perf-budgets.md` with its follow-up.

## T5 — Documentation
- [ ] T5.1 architecture.md "Current state" updated every milestone
       (already the rule).
- [ ] T5.2 Every crate's module docs cite the spec sections implemented
       (M3 pattern: deviations documented in the module header).
- [ ] T5.3 ADRs: every D-register decision gets a short written record
       (context → options → choice → consequences) in `docs/adr/`.

## T6 — Release engineering
- [ ] T6.1 Versioning: semver on the workspace, changelog per milestone.
- [ ] T6.2 Signed release artifacts (Windows: code-sign when a cert is
       available; unsigned + hash-published before).
- [ ] T6.3 Auto-update (opt-in, delta or full) — T3-gated design.

---

# 6. Decision register

| ID | Decision | Trigger | Options (working lean) | Default lean |
|----|----------|---------|------------------------|--------------|
| D1 | JS engine | M24 | quickjs-ng / SpiderMonkey (mozjs) / V8 (rusty_v8) / Boa | quickjs-ng behind a `ScriptEngine` trait; trait designed for swap; decided at M24: QuickJS-ng 0.16.2 through `rquickjs` 0.14.0 in a leaf crate, `js`, behind its `ScriptEngine` and `Realm` traits, for its interrupt handler and heap limit, which Boa has not (see `docs/design/scripting.md`) |
| D2 | Renderer sandbox mechanism | M34 | Win job objects+tokens / AppContainer / others per-OS | Job objects + restricted token v1 |
| D3 | GPU backend | M19 | wgpu / keep software only | wgpu, software path permanent fallback; decided at M19: `render::gpu` on wgpu behind the `gpu` feature, on by default, the shell drawing on it where there is an adapter and the software backend otherwise or with `--software`, both behind `RenderBackend` (see `docs/design/compositing.md`) |
| D4 | TLS | M14 | rustls / native-tls | rustls (pure Rust, no OpenSSL linkage); decided at M14: the `aws-lc-rs` provider for X25519MLKEM768, Mozilla's roots from `webpki-roots`, no bypass (see `docs/design/net.md`) |
| D5 | HTTP client | M14 | hyper / custom over rustls | custom-lite first (control + learning), hyper if maintenance bites; decided at M14: our own HTTP/1.1 on threads, no async runtime, each connection waiting in a `mio` poll of its own that an abort wakes (see `docs/design/net.md`) |
| D6 | Image codecs | M23 | image crate ensemble / per-format crates | per-format crates behind a trait (faster, auditable) |
| D7 | Text/i18n | M8/M42 | rustybuzz+unicode crates / harfbuzz FFI / ICU4X | rustybuzz now, ICU4X at M42 |
| D8 | 2D rasterizer | M19 | tiny-skia forever / vello (wgpu-native) / both | both behind the existing `RenderBackend` trait; decided at M19: tiny-skia for the software backend and four wgpu pipelines of our own for the GPU backend, not vello: the two share their glyph masks and polygon coverage, so that text and mitres come out the same in both and the equivalence suite can hold them within two levels inside a shape (see `docs/design/compositing.md`) |
| D9 | Chrome UI toolkit | M15/M37 | in-engine (softbuffer) / egui / native | in-engine (no extra toolkit; the engine is the toolkit); decided at M15: a `ui` crate beside `engine`, under `shell`, drawing with `paint`, `render` and `text`, and touching no OS (see `docs/design/navigation.md`) |
| D10 | Profile storage engine | M28 | redb / sled / SQLite / custom files | redb (pure Rust, embedded, single-file); decided at M28 as written: `redb` 4.3.0 behind a leaf crate of its own, `profile`, which forbids `unsafe`, each origin's `localStorage` items and the cookies with an expiry in one file, every write one transaction (see `docs/design/storage.md`) |
| D11 | DevTools protocol | M36 | custom in-engine UI only / CDP-compatible | custom first; CDP-compat only if external tools wanted |
| D12 | UA string & fingerprint policy | M25 | honest UA / Chrome-mimic | honest minimal UA + minimal API surface (compat escape hatch flag exists but is not default); decided at M25 as written: `Mozilla/5.0 (compatible; SilverBrowser/<version>)` in requests and `navigator.userAgent`, `navigator`'s values HTML's for the Gecko mode or fixed, `screen` the viewport; the shell's `--compat-user-agent` gives the reference browser's string and HTML's Chrome mode (see `docs/design/dom-scripting.md`) |
| D13 | Process granularity | M34 | per-tab / per-site-instance / hybrid | per-site-instance with LRU cap |
| D14 | Quirks mode | M40s | full quirks emulation / standards-only + limited quirk stylesheet | limited quirk stylesheet (uqlist-style) driven by T1 data |
| D15 | HTTP/3 (QUIC) | M47+ | quinn / skip | quinn when HTTP/2 coverage demonstrably costs compat |
| D16 | DOM storage and node identity | M13 | owned tree with ids on nodes / reference-counted nodes / preorder indices remapped per change / arena of slots with generational ids | arena with generational `NodeId`s; the owned `Node` kept as a tree literal (see `docs/design/incremental.md`) |
| D17 | URL parser | M14 | the `url` crate / our own | our own WHATWG URL parser in `net`, UTS #46 through the `idna` crate (see `docs/design/net.md`) |
| D18 | Character encodings | M14 | `encoding_rs` / our own | our own `encoding` crate, a leaf beside `dom`; `encoding_rs` is Gecko code (§1.3); CJK legacy encodings with M42 or a T1 page (see `docs/design/net.md`) |
| D19 | Requests from public pages to local addresses, before M33 | M14 | allow until M33.4 / refuse outright now | refuse outright now, by Local Network Access's address spaces, checked on the address connected to (see `docs/design/net.md`) |
| D20 | The Public Suffix List | M27 | the `psl` crate / the list vendored with a matcher of our own | the `psl` crate (2.1.241, `no_std`, `forbid(unsafe_code)`), a matcher generated from the list; a bump of the crate keeps it current (see `docs/design/fetch.md`) |
| D21 | WebAssembly | M29 | a `wasmi` bridge / a `wasmtime` bridge / the change of JavaScript engine D1 left open | `wasmi` 2.0 in `js` beside QuickJS-ng: an interpreter, validating every module, metered by fuel, bounded by a resource limiter, calls resumable at a host function; no JIT before M34's sandbox (see `docs/design/wasm.md`) |
| D22 | The 2D canvas's rasterizer | M30 | `tiny-skia` on the processor / the GPU through `wgpu` / a rasterizer of our own | `tiny-skia` 0.12 in a `canvas` crate below `engine`: a port of Skia, the reference browser's, on every machine and read back without a stall; the bitmap shown as an image by both backends; drawing on the GPU left to G81 (see `docs/design/canvas.md`) |
| D23 | SVG's renderer | M32 | `resvg` and `usvg` / an `svg` crate of our own drawing through `canvas` | an `svg` crate of our own: a render tree from the engine's DOM and its cascade's computed styles, drawn through the canvas crate's `tiny-skia` context, so that an inline `<svg>` is styled by the page's CSS and its shapes hit; `resvg` brings its own XML, CSS, fonts and a second `tiny-skia` (see `docs/design/svg.md`) |
| D24 | Frames | M33 | a tab of navigables in one engine / a process per frame now / an engine of its own for each frame, under its parent's | an engine for each frame, drawn into its parent as a canvas is and reached through a frame tree the tab shares, so that M34 can move it into a process of its own behind the same seam; its price, every frame cross-origin to every other until then, is G90 (see `docs/design/hardening.md`) |
| D25 | Third-party cookies | M33 | blocked / all partitioned / sent, the rest partitioned | the reference browser's: a frame of another site sends and reads its `SameSite=None; Secure` cookies, and its partitioned cookies, storage, cache, pools and TLS sessions are kept by the top-level site; blocking them as a setting is G93 |
| D26 | The HSTS preload list | M33 | none / fetched at run time / Chromium's, generated into a file of the repository and read-only | Chromium's list, its `force-https` entries generated by `crates/net/examples/hsts_preload.rs` into `crates/net/data/hsts_preload.txt` and read once; fetched 2026-10-13, 95,136 hosts, its notice kept with them, 11 left out at the maintainer's choice; a copy goes stale without an update service (T6.3) |
| D27 | The profile's encryption | M33 | none / a password of the user's / a data key the operating system wraps | AES-256-GCM under a data key the operating system wraps for the user (DPAPI on Windows); Keychain and Secret Service are G98 |

---

# 7. Risks (top-of-mind register — reviewed each milestone)

- **R1 — Infinite compat tail.** The web's surface area is unbounded;
  the tier ladder (§1.2) exists to keep scope honest. Mitigation: T1
  fixtures decide what "compat" means at each tier; no scope without a
  failing real page.
- **R2 — Scripting before isolation.** JS before M33/M34 hardening is
  the single most dangerous window. Mitigation: M24 runs only in a
  strict-CSP, no-cookie, in-memory default profile until M33 lands; the
  `ScriptEngine` boundary is reviewed as security code.
  Since M34 (2026-10-15, the maintainer's choice) scripts run by
  default in the many-process build's window, each page in a renderer
  process of its site, sandboxed on Windows; `--disable-scripts` turns
  them off. The one-process build and `--print-to-pdf`, whose pages are
  the browser's own process's, run them only when `--enable-scripts`
  asks.
- **R3 — Single-developer throughput.** Phases marked XR are
  year-scale at hobby cadence. Mitigation: milestones stay vertical and
  demoable (motivation + integration risk control); parallel tracks
  (net vs layout) allow context switching.
- **R4 — FFI rot.** quickjs-ng / wgpu / rustls move; pinned versions
  rot. Mitigation: T3.1 quarterly audit; upgrade windows scheduled, not
  surprise.
- **R5 — GPU variance.** wgpu across Windows drivers is historically
  uneven. Mitigation: software path is permanent and CI-tested (D3/D8);
  GPU is an accelerator, never a requirement.
- **R6 — OneDrive workspace.** Builds already go to a target dir outside
  the sync root; keep it that way; consider relocating the repo off
  OneDrive entirely when M34 work starts (process spawning under a
  synced folder is a known annoyance).
- **R7 — Scope capture by this document.** The roadmap is a map, not a
  contract for its own sake: amend it whenever reality argues. Every
  amendment updates §9.

---

# 8. Spec reference library

| Area | Spec / source |
|------|--------------|
| HTML parsing | WHATWG HTML Standard §13 (tree construction) |
| DOM | WHATWG DOM |
| URL | WHATWG URL + `urltestdata.json` |
| MIME sniffing | WHATWG MIME Sniffing |
| CSS syntax | css-syntax-3 |
| Selectors | selectors-4 (+CSS2 §5 pseudo-classes) |
| Cascade | css-cascade-5 (+css-values-4 for units) |
| Box model | CSS2 §8 |
| Visual formatting | CSS2 §9–10 (floats, positioning, sizing) |
| Tables | CSS2 §17 |
| Flexbox | css-flexbox-1 |
| Grid | css-grid-1/-2 |
| Transforms | css-transforms-1/-2 |
| Compositing | compositing-1, filter-effects-1 |
| Text | css-text-3/-4, css-writing-modes-4, UAX#9 |
| Custom props | css-variables-1 (+IACVT, css-cascade-5 §7) |
| Animations | css-transitions-1, web-animations-1, css-animations-1 |
| Media queries | mediaqueries-4, css-conditional-3/5 |
| Images | css-images-3/4 |
| Fonts | css-fonts-4/5, OpenType spec |
| Fetch/CORS | fetch (+CORS section), RFC6265bis (cookies) |
| CSP | CSP3 |
| Sandbox/process | Chromium security architecture docs (reference, not code) |
| ECMAScript | ECMA-262; WebIDL for bindings; webassembly spec |
| Storage | storage, IndexedDB |
| Tests | web-platform-tests, csswg test suites |
| PDF output | pdf-writer crate docs (ISO 32000 reference only as needed) |

---

# 9. Document maintenance

- One roadmap edit per milestone commit (check the boxes you finished,
  update §2 Current state, and amend anything reality disproved).
- New work items get the next free ID in their section; never reuse IDs.
- New decisions append to §6 with an ADR note (T5.3).
- A milestone may be inserted (e.g., "M17.5 — interactive forms") without
  renumbering; gaps in numbering are acceptable, renumbering is not.

*v1.0 — initial full ladder, written at M3+ (CSS parsing complete,
selectors next).*
