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

## 2. Current state (M0–M9)

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

Verified per the vertical-slice rule at each milestone: 388 tests
workspace-wide (178 in `style`, 44 in `layout`, 42 in `html`, 30 in
`text`, 25 in `engine`), clippy clean (`-D warnings`, every target),
smoke run.
Toolchain: 1.98.1, pinned by `rust-toolchain.toml`.
Build target dir outside OneDrive (`%LOCALAPPDATA%\silver-browser\target`)
— currently only by passing `CARGO_TARGET_DIR` manually each session; no
`.cargo/config.toml` enforces this yet (worth adding before it's
forgotten once, inside OneDrive).

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
       M10). Implemented without restructuring the box tree into real
       anonymous boxes: `layout::inline` ends the current line, lays the
       block out on its own (calling back into `block::layout_block`,
       now `pub(crate)` for this), and resumes a fresh line after — the
       same observable splitting effect, and critically it doesn't panic
       on this shape of input (non-conforming HTML content-model-wise,
       but this engine's lenient tree builder constructs it anyway, so
       layout has to survive it regardless of what real pages "should"
       contain).
- [ ] M7.7 Vertical alignment of images/inline-blocks on the text
       baseline (with M10's inline-block). Not started — correctly: this
       item is explicitly gated on M10's `inline-block`, which doesn't
       exist yet (M10 hasn't happened). Nothing to vertically-align yet
       besides text, which M7.4 already baseline-aligns.
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
       rasterizer.
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

- [ ] M10.1 Units: `em`, `rem`, `%` (margins/paddings vs widths vs font
       sizes resolution rules), `ex`, `ch`, `vh`, `vw`, `vmin`, `vmax`.
- [ ] M10.2 `box-sizing` (content|border-box).
- [ ] M10.3 `min-width/max-width/min-height/max-height` incl. interaction
       with `height:auto`.
- [ ] M10.4 `display: inline-block`, `display: none` (skip from box tree),
       anonymous block boxing (block-in-inline both directions).
- [ ] M10.5 `overflow: auto|scroll` element-level scroll containers
       (nested scrolling; hit test through scroll chains).
- [ ] M10.6 Default form control styling v0 (`input`, `button`, `select`,
       `textarea` render as styled boxes; no behavior yet).
- [ ] M10.7 Acceptance: CSS box model intro pages (custom built) render
       pixel-identical to Firefox on 10 reference cases.

---

## M11 — Floats, positioning, stacking  ·  `layout`/`paint`  ·  Effort: XL  ·  Depends: M10

- [ ] M11.1 Floats: `float: left|right`, line shortening, `clear`
       (clearance computation per CSS2 §9.5).
- [ ] M11.2 Block formatting contexts around floats (the "BFC dodge").
- [ ] M11.3 `position: relative` (offsets, paint-time only).
- [ ] M11.4 `position: absolute` (containing block resolution, static
       position, shrink-to-fit width per CSS2 §10.3.7).
- [ ] M11.5 `position: fixed|sticky` (sticky = scroll-driven constraints,
       v1 without nested scroll edge cases).
- [ ] M11.6 `z-index`, stacking contexts, paint order per CSS2 Appendix E
       (negative z, floats, inline, positioned).
- [ ] M11.7 Acceptance: classic two-column float layout + absolutely
       positioned overlays render correctly; stacking order tests.

---

## M12 — Tables  ·  `layout`  ·  Effort: XL  ·  Depends: M10

- [ ] M12.1 Table formatting model: row/rowgroup/column/columngroup
       boxes, anonymous table boxes (missing tr/td wrapping).
- [ ] M12.2 Fixed layout algorithm (CSS2 §17.5.2.1).
- [ ] M12.3 Automatic layout algorithm (min/max content widths,
       percentage column distribution — CSS2 §17.5.2.2).
- [ ] M12.4 `colspan`/`rowspan` with cell grid construction.
- [ ] M12.5 `border-collapse` (collapse model conflict resolution per
       CSS2 §17.6.2), `border-spacing` for the separated model.
- [ ] M12.6 `vertical-align` in cells; caption placement.
- [ ] M12.7 Acceptance: 5 real-world table screenshots (including
       nested tables and colspan-heavy cases) match reference renderers.

---

## M13 — Incremental engine v1  ·  `engine`/`style`/`layout`  ·  Effort: L  ·  Depends: M5–M12

- [ ] M13.1 Dirty flags: style dirty bits per element (class/attr change →
       subtree or sibling invalidation v1 = conservative subtree).
- [ ] M13.2 Layout invalidation: damage region propagation (content-size
       changes → ancestor reflow; position-only changes → paint-only).
- [ ] M13.3 Paint invalidation: display-list diffing v1 (item-equality
       rect collection).
- [ ] M13.4 Frame scheduler: input → frame pipeline with a frame budget
       (16.7ms target, deadline-aware: skip layout polish when over
       budget — document the policy).
- [ ] M13.5 `Engine::set_document` public API + repaint triggering.
- [ ] M13.6 Acceptance: a DOM-mutation benchmark (append 1k nodes) runs
       without full-document relayout; measure and record.

---

## M14 — `net` v1: URLs + HTTP/1.1  ·  `net`  ·  Effort: XL  ·  Depends: none (parallelizable from M4+)

- [ ] M14.1 WHATWG URL parsing: scheme/host/port/path/query/fragment,
       percent-encoding, IDNA (`idna` crate), IPv4/IPv6 hosts,
       relative-resolution. Full test suite from the WHATWG
       `urltestdata.json`.
- [ ] M14.2 Fetch core: `net::fetch(url) -> Response { status, headers, body }`;
       redirect following (max 20, cross-scheme rules); `data:` and
       `file:` schemes.
- [ ] M14.3 HTTP/1.1 over `rustls` (pure-Rust TLS 1.2/1.3, webpki roots):
       chunked transfer, keep-alive, connection reuse pool (per-origin,
       max 6).
- [ ] M14.4 Certificate validation: WebPKI rules; no user-configurable
       cert bypass in v1 (security posture: fail closed).
- [ ] M14.5 MIME sniffing (WHATWG mime sniffing algorithm) + charset
       detection (`charset` detection: BOM → declared → UTF-8 fallback).
- [ ] M14.6 Resource loading pipeline in `engine`: HTML → inline subresource
       set (stylesheets, images) → parallel fetch → incremental document
       assembly (stylesheet arrives → restyle).
- [ ] M14.7 Security v1 in net: private-address blocking posture decision
       (PNA later, M33), `blob:`/`javascript:` scheme rejection in fetch.
- [ ] M14.8 Tests: URL suite (1000+ cases), mock HTTP server harness
       (`httptest`-style, local only), TLS via self-signed test CA.
- [ ] M14.9 Fuzz: URL parser (T2 priority target — URL parsers are the
       #1 historical CVE source in browsers).

---

## M15 — Navigation v1 + address bar  ·  `shell`/`engine`  ·  Effort: L  ·  Depends: M14

- [ ] M15.1 Navigation state machine: `about:blank`, `about:silver`
       (built-in demo), user URL entry → fetch → parse → render; error
       pages (DNS fail, TLS fail, HTTP error classes).
- [ ] M15.2 Session history: back/forward with in-memory bfcache v1
       (document snapshot, no JS state yet), same-document fragment
       navigation.
- [ ] M15.3 Minimal chrome UI v0: a URL text box + back/forward buttons
       drawn in-engine (softbuffer UI; not native widgets — see D9).
- [ ] M15.4 Address bar input heuristics v1: scheme-less input →
       `https://` prefix; "looks like a host" detection (single token,
       dot rules).
- [ ] M15.5 Page title → window title; favicon fetch (v0: no UI display).
- [ ] M15.6 Acceptance: navigate to a real static site over HTTPS
       (e.g., example.com) and read it. **This is the T1 gate-crasher
       milestone.**

---

## M16 — Selectors v2 + pseudo-elements  ·  `style`  ·  Effort: M  ·  Depends: M4, M26 (dynamic parts)

- [ ] M16.1 `::before` / `::after` with `content` (strings, `attr()`,
       counters via M21; url() images via M23).
- [ ] M16.2 `::first-line`, `::first-letter` (layout-aware pseudo styling).
- [ ] M16.3 `::selection`, `::placeholder`, `::marker` as recognized
       stubs.
- [ ] M16.4 Case-insensitive attribute flags end-to-end; `:lang()`;
       form pseudo-classes (`:disabled`, `:checked`, `:required` — stubs
       until M26 wires state).
- [ ] M16.5 Selector matching performance pass: bloom-filter ancestor
       rejection (Blink-style) if micro-bench regresses >2× on 1k
       selectors × 10k DOM.
- [ ] M16.6 Acceptance: pseudo-elements visible on the built-in demo page.

---

## M17 — Flexbox  ·  `layout`  ·  Effort: XL  ·  Depends: M10

Per css-flexbox-1 (all items cite the spec section in code comments):

- [ ] M17.1 Flex container/item box tree integration; `display: flex |
       inline-flex`; item creation order and `order` property.
- [ ] M17.2 Axis setup: `flex-direction`, `flex-wrap`, main/cross axis
       resolution; anonymous flex items.
- [ ] M17.3 Base size resolution: `flex-basis` (`auto` → content or
       width), content-based min/max sizes (`min-content`, `max-content`
       intrinsic sizing v1).
- [ ] M17.4 Flexible lengths algorithm (§9.7): grow/shrink factors,
       hypothetical main size, resolved flexible lengths, min/max
       violations, inflexible item handling.
- [ ] M17.5 Main axis alignment: `justify-content` (all 6 values),
       margins auto-absorption.
- [ ] M17.6 Cross axis: `align-items`/`align-self`, `align-content`,
       single-line stretch; `flex-wrap: wrap` multi-line packing.
- [ ] M17.7 Baseline alignment of flex items (first-baseline v1).
- [ ] M17.8 Absolute-positioned flex children (containing block rules §9.2).
- [ ] M17.9 Test matrix: the WPT flexbox subset (adopt the harness in T1
       this milestone) + 20 hand-built golden cases.
- [ ] M17.10 Acceptance: 5 real flex layouts (nav bar, card grid, holy
       grail) render correctly.

---

## M18 — Grid  ·  `layout`  ·  Effort: XL  ·  Depends: M17

Per css-grid-1 (level 2 subgrid included):

- [ ] M18.1 `display: grid`; explicit tracks: `grid-template-columns |
       rows` (`none`, fixed, `fr`, `minmax()`, `auto`, `repeat()` incl.
       `auto-fill`/`auto-fit`), line names, `gap`.
- [ ] M18.2 Implicit tracks (`grid-auto-rows | columns`), auto-placement
       algorithm (sparse v1; dense v2).
- [ ] M18.3 Item placement: line/area/span forms of
       `grid-column|row-start|end`, `grid-area`, z-order in grid.
- [ ] M18.4 Track sizing algorithm (§12): base sizes, growth limits,
       intrinsic contributions, `fr` distribution, spanning items
       (single-span-only v1 for intrinsic sizing).
- [ ] M18.5 Alignment: `justify-items|self`, `align-items|self`,
       `place-*`, `justify|align-content`; safe/unsafe alignment.
- [ ] M18.6 `subgrid` (grid-2) and `masonry` deferred — registered as
       G-track items, not blocking.
- [ ] M18.7 Acceptance: CSS grid garden-style suite + 5 real layouts
       (dashboard, photo gallery, form grid) render correctly.

---

## M19 — Compositing + transforms + accelerated scroll  ·  `render`  ·  Effort: XL  ·  Depends: M11, M13

- [ ] M19.1 `transform` 2D: matrix, translate/rotate/scale/skew;
       `transform-origin`; percentage resolution.
- [ ] M19.2 `opacity` as a group; `transform`+`opacity` layers →
       composited subtrees (layer tree + layerization policy doc).
- [ ] M19.3 GPU backend v1 via `wgpu` (decision D3): display list →
       render passes; tiny-skia retained as fallback (software path is
       first-class forever — lightweight machines are a stated goal).
- [ ] M19.4 Composited scrolling: scrolling as a layer transform, no
       repaint on scroll for composited content.
- [ ] M19.5 3D transforms + perspective (matrix decomposition,
       preserve-3d v1); `backface-visibility`.
- [ ] M19.6 `will-change` hint; layer memory budget + eviction
       (lightweight goal: hard cap configurable, default modest).
- [ ] M19.7 Acceptance: 60fps scrolling + animated transform demos on
       integrated GPU; software path still passes all golden tests
       (rendering equivalence test suite).

---

## M20 — Media queries + printing  ·  `style`  ·  Effort: M  ·  Depends: M5, M9

- [ ] M20.1 `@media` evaluation: width/height/aspect-ratio/orientation/
       resolution features (the M3 parser already nests these rules — now
       evaluate them); dynamic re-evaluation on resize.
- [ ] M20.2 `@supports` (declaration-level v1: parseable = supported);
       `@layer` ordering semantics.
- [ ] M20.3 `prefers-color-scheme`, `prefers-reduced-motion` (OS wiring;
       `dark` theme for the built-in UI exists already).
- [ ] M20.4 Print media: `@media print` stylesheets, print layout
       (paginated block fragmentation v1), print-to-PDF via tiny-skia
       surface → pdf-writer crate.
- [ ] M20.5 Acceptance: the built-in demo prints to a sane multi-page
       PDF; a responsive page relayouts at breakpoint widths.

---

## M21 — Custom properties, calc, counters, generated content  ·  `style`  ·  Effort: L  ·  Depends: M5

- [ ] M21.1 Custom properties: `--x` parse (the M3 tokenizer already
       handles `--custom` idents), inheritance (computed-value = token
       stream), `var()` substitution with fallback + cycles = invalid at
       computed-value time (IACVT).
- [ ] M21.2 `calc()`: +, -, ×, ÷ with unit algebra (length/number/
       percentage type checking at compute time); `min()/max()/clamp()`.
- [ ] M21.3 `env()` stubs (safe-area, viewport).
- [ ] M21.4 Counters: `counter-increment/reset`, `counter()`, `counters()`
       + `content` integration (M16).
- [ ] M21.5 Generated content: `content: url()` (needs M23), `attr()`.
- [ ] M21.6 List styling: `list-style-type` (disc/decimal + 3 custom),
       `::marker` content.
- [ ] M21.7 `text-decoration` (underline position/thickness v1);
       `text-justify` + real justification (M7 debt).
- [ ] M21.8 Acceptance: theming demo (custom props drive colors),
       auto-numbered sections with counters.

---

## M22 — Transitions + animations  ·  `style`/`render`  ·  Effort: XL  ·  Depends: M19, M13

- [ ] M22.1 CSS transitions: property interpolation (lengths, colors,
       transforms, opacity), easing functions (the standard set + cubic
       bezier), start/end edge cases per css-transitions-1.
- [ ] M22.2 CSS animations: `@keyframes` (the M3 parser keeps unknown
       at-rules raw — now parse `@keyframes` properly), iteration/direction/
       fill/delay/play-state, animation events.
- [ ] M22.3 `requestAnimationFrame`-aligned ticking (real rAF needs JS —
       M25; here: the frame scheduler drives animation timelines).
- [ ] M22.4 Compositor-only animations (transform/opacity) run off the
       main layout path; everything else animates via style recalc.
- [ ] M22.5 Reduced-motion respect (M20.3 dependency).
- [ ] M22.6 Acceptance: a spinner (rotate) and a color-fade demo hit
       60fps; jank budget test (no frame > 20ms during a 10s animation).

---

## M23 — Images  ·  `render`/`net`  ·  Effort: L  ·  Depends: M14

- [ ] M23.1 Decode pipeline: PNG (png crate), JPEG (jpeg-decoder), GIF
       (incl. animation), WebP (image-webp), AVIF (ravif) — behind a
       `ImageDecoder` trait; decode off-main-thread.
- [ ] M23.2 `<img>` element: intrinsic sizing, `width/height` attrs +
       aspect-ratio box, `alt` text rendering, `object-fit` + `object-position`.
- [ ] M23.3 CSS images: `background-image`, border-image v1, gradients
       (linear/radial — gradient rasterization in tiny-skia/wgpu paths),
       `image-set`.
- [ ] M23.4 SVG-as-image v1 (render an SVG file to a surface via M32
       subset: static shapes/text only).
- [ ] M23.5 Lazy decoding + downscale-on-decode for huge images (memory
       cap posture, `decoding=async`).
- [ ] M23.6 Acceptance: a photo-grid page with mixed formats + gradients
       + an animated GIF renders at 60fps scroll.

---

## M24 — JS engine integration  ·  new `js` crate  ·  Effort: XR  ·  Depends: M13, M15, **E-track pre-work (see R2)**

> Decision D1 must be made here. Working assumption: **quickjs-ng** behind
> a `ScriptEngine` trait. The trait is the real deliverable: engine swaps
> (SpiderMonkey later) must not cost a rewrite.

- [ ] M24.1 `js` crate: `ScriptEngine` trait (compile/eval/call,
       promise/tick integration, GC integration points, error sinks).
- [ ] M24.2 quickjs-ng bindings (fork or `rquickjs`-style FFI, vendored +
       pinned; security branch: track upstream CVEs quarterly — T1).
- [ ] M24.3 Script execution pipeline: `<script>` parsing (M2 debt:
       raw-text mode for script/style elements), external scripts via
       `net`, execution order (sync/defer), `async` v1.
- [ ] M24.4 Embedding boundary: NO engine object graph references DOM
       internals — a `ScriptingHost` with explicit conversion at the
       boundary (this is the security-critical seam; reviewed by R2 rules).
- [ ] M24.5 Memory/time limits: instruction budget, heap cap, OOM =
       script failure not engine failure.
- [ ] M24.6 CSP `script-src` enforcement at the loader (M33 does full CSP;
       this blocks the obvious foot-gun from day one).
- [ ] M24.7 Acceptance: `console.log("hello")`, `document.title = "x"`
       from an inline script; a script-error page state.

---

## M25 — DOM scripting v1  ·  `js`/`dom`/`engine`  ·  Effort: XR  ·  Depends: M24

- [ ] M25.1 DOM Core bindings: `document.getElementById/ querySelector(
       All)` (uses the M4 selector engine!), `createElement`, `createTextNode`,
       `appendChild/removeChild/insertBefore`, `textContent`, `innerHTML`
       (reparse via html crate — the parser is already hostile-input-safe).
- [ ] M25.2 Node properties: `parentNode/children/childNodes/firstChild/
       nextSibling`, `classList`, `setAttribute/getAttribute`, `style`
       object (set→restyle).
- [ ] M25.3 Mutation → engine invalidation: DOM writes trigger M13
       incremental restyle/layout (the whole incremental engine pays off
       here; benchmark it).
- [ ] M25.4 `window` v1: `location` (read + navigate), `navigator`
       (fingerprint-minimal: no lies beyond a fixed UA string policy —
       see D12), `screen`, `getComputedStyle`.
- [ ] M25.5 Acceptance: a page that builds a table via JS then styles it
       via a class toggle renders correctly; DOM-mutation benchmark under
       frame budget.

---

## M26 — Event loop, events, input  ·  `engine`  ·  Effort: L  ·  Depends: M25

- [ ] M26.1 HTML event loop: task queue, microtask checkpoint, rendering
       steps (rAF callbacks → style → layout → paint), per spec ordering.
- [ ] M26.2 Event dispatch: capture/bubble/target phases, listeners,
       `stopPropagation/preventDefault`, synthetic event creation from
       JS (`dispatchEvent`).
- [ ] M26.3 User input: mouse (click/dblclick/mousedown/up/move),
       keyboard (keydown/up/keypress), focus chain (focus/blur, tab order
       v1), wheel, `change`/`input` on form controls.
- [ ] M26.4 Now-dynamic pseudo-classes: `:hover`, `:active`, `:focus`
       (M4.7 debt paid).
- [ ] M26.5 Timers: `setTimeout/setInterval` (clamped, background
       throttling policy), `Promise` + microtask ordering.
- [ ] M26.6 Link handling: `<a href>` click → navigation; middle-click
       → new-tab-less v1 (tabs are M37 — queue a note).
- [ ] M26.7 Acceptance: interactive todo-list page (add/remove/style
       entries) works end-to-end.

---

## M27 — fetch, SOP, CORS, cookies  ·  `net`/`js`  ·  Effort: XL  ·  Depends: M25, M14

- [ ] M27.1 `fetch()` API: promises, Request/Response/Headers objects,
       streaming body v1 (full-buffer first), `FormData` basic.
- [ ] M27.2 URL same-origin computation; SOP enforcement matrix (document
       → script, DOM access, network reads vs writes).
- [ ] M27.3 CORS: preflight (OPTIONS), `Access-Control-*` handling, error
       events, credentialed vs bare requests (fetch-cors spec).
- [ ] M27.4 Cookies v1: RFC6265bis parse/store/send, `HttpOnly`,
       `SameSite=Lax|Strict|None` (default Lax), `Secure`, jar per-origin
       in-memory (persistence in M28).
- [ ] M27.5 Mixed content: block active http subresources on https pages.
- [ ] M27.6 `XMLHttpRequest` (legacy, minimal but correct).
- [ ] M27.7 Security tests: an adversarial test page attempting
       cross-origin reads must fail in every documented vector (this test
       page becomes a permanent regression artifact).

---

## M28 — Storage v1  ·  `net`/`engine`  ·  Effort: M  ·  Depends: M27

- [ ] M28.1 Profile/persistence layer (decision D10): on-disk layout,
       atomic writes, per-origin namespacing.
- [ ] M28.2 `localStorage`/`sessionStorage` bindings.
- [ ] M28.3 Cookie persistence in the profile.
- [ ] M28.4 Cache v0: in-memory subresource cache with TTL + `Cache-Control`
       basics (full conditional caching in M38).
- [ ] M28.5 Storage isolation tests (origin A can't read origin B, incl.
       after profile reload).

---

## M29 — WebAssembly MVP  ·  `js`  ·  Effort: L  ·  Depends: M24

- [ ] M29.1 WASM instantiation via the chosen engine's WASM support
       (quickjs-ng: none — D1 contingency: `wasmtime`/`wasmi` bridge, or
       this milestone forces the SpiderMonkey upgrade decision D1).
- [ ] M29.2 `WebAssembly.Module/Instance/Memory` JS API surface.
- [ ] M29.3 Engine-crash isolation: trap = error, never engine death.
- [ ] M29.4 Acceptance: a small WASM module (e.g., compiled Rust fn)
       round-trips numbers.

---

## M30 — Canvas 2D  ·  `render`/`js`  ·  Effort: L  ·  Depends: M25

- [ ] M30.1 Canvas 2D API: paths, fills/strokes (colors + gradients),
       text (through the `text` stack), images, transforms,
       `save/restore`, `clearRect/fillRect`.
- [ ] M30.2 Pixel formats, `toDataURL` (PNG), `OffscreenCanvas` v0
       (no worker transfer until M31).
- [ ] M30.3 Rasterization shares the software/wgpu backend split (M19);
       canvas-in-composited-layer.
- [ ] M30.4 Acceptance: a chart + a particle demo render correctly at
       60fps.

---

## M31 — Web Workers v1  ·  `engine`  ·  Effort: L  ·  Depends: M25, M28

- [ ] M31.1 Dedicated workers: dedicated-thread JS realms, `postMessage`
       + structured clone, `onmessage`.
- [ ] M31.2 Worker script fetch (SOP applies), termination semantics,
       error propagation.
- [ ] M31.3 Thread policy: workers are OS threads now; process isolation
       happens in M34 (document the migration seam).
- [ ] M31.4 `OffscreenCanvas` transfer + render-from-worker v1 (if wgpu
       path exists; else software-surface handoff).
- [ ] M31.5 Acceptance: worker-driven Mandelbrot page stays responsive.

---

## M32 — SVG v1  ·  new `svg` crate (below `css`, above `dom`)  ·  Effort: XL  ·  Depends: M23, M9

- [ ] M32.1 SVG-as-image (M23.4 completion): static shapes (rect, circle,
       ellipse, line, polyline, polygon, path), fill/stroke, gradients,
       transforms, `viewBox`, text v1.
- [ ] M32.2 Inline `<svg>` in HTML: layout integration (replaced-element
       sizing), CSS styling of SVG presentation attributes.
- [ ] M32.3 SMIL: explicitly rejected (chromium has it; cost/benefit
       documented — revisit only at T4).
- [ ] M32.4 Acceptance: an icon set + a statistical chart render at all
       DPIs.

---

## M33 — Security hardening sweep  ·  all crates  ·  Effort: XL  ·  Depends: M27

- [ ] M33.1 CSP: fetch directives (`default-src`, `script-src`,
       `style-src`, `img-src`, `frame-src`, `connect-src`, ...),
       report-only mode, violation events, `nonce`/`hash` sources.
- [ ] M33.2 Frame isolation: `X-Frame-Options`, `frame-ancestors`,
       iframe `sandbox` attribute (all flags), `srcdoc` (M24's raw-text
       debt: iframes parse nested documents through the same pipeline).
- [ ] M33.3 `Referrer-Policy`, HSTS (max-age/includeSubDomains, preload
       list shipped read-only), `Upgrade-Insecure-Requests`.
- [ ] M33.4 Private Network Access: block public→local requests
       (pre-flight posture v1: block outright).
- [ ] M33.5 Memory-safety audit pass: `unsafe` inventory across the
       workspace (target: unsafe exists only in FFI crates, each block
       has a written safety argument — policy lives in architecture.md).
- [ ] M33.6 Parser fuzzing graduation (T2): continuous fuzzing, corpus
       committed, OSS-Fuzz application (Rust targets).
- [ ] M33.7 Threat-model doc `docs/security.md`: assets, boundaries,
       trust levels; updated per milestone from here on.
- [ ] M33.8 Adversarial test-site v2: the M27.7 page + CSP violations +
       cookie exfil attempts, all must fail closed.

---

## M34 — Multi-process architecture  ·  `shell`/`engine`  ·  Effort: XR  ·  Depends: M33, M31

The site-isolation milestone — the architecture.md crate seams become
process seams.

- [ ] M34.1 Process model: browser process (chrome, net, profile,
       policy) + renderer processes (documents/frames); process-per-site-
       instance v1 (not per-tab — lightweight goal: process count capped,
       LRU eviction).
- [ ] M34.2 IPC design: message-passing over the existing internal
       boundaries; capability-scoped handles (a renderer gets exactly the
       interfaces it needs — no god-channel); schema versioned from day
       one.
- [ ] M34.3 Renderer sandbox per-OS (decision D2): Windows job objects +
       restricted token v1; document the macOS/Linux equivalents
       (sandbox-exec/seccomp) for portability.
- [ ] M34.4 Crash isolation: renderer death = sad-tab + respawn, browser
       process survives; crash reporting local-only (minidump written,
       user chooses to share — nothing phones home).
- [ ] M34.5 The `Engine` type becomes a session proxy in the browser
       process; the render pipeline state lives in the renderer.
- [ ] M34.6 Migration plan executed incrementally: single-process remains
       a build feature (`--features single-process`) for debugging and
       constrained machines — tested in CI both ways.
- [ ] M34.7 Acceptance: kill -9 a renderer mid-animation; the tab shows a
       crash placeholder and recovers; a renderer compromise cannot read
       another site's cookies (test via the adversarial suite).

---

## M35 — Performance v1  ·  all  ·  Effort: XL  ·  Depends: M13, M19, M25

- [ ] M35.1 Benchmarks committed: Speedometer-style interaction bench
       (subset), page-load bench on 20 saved pages, scroll/animation
       jank bench, memory footprints per page class. All recorded per
       milestone from now on (T4 track formalized).
- [ ] M35.2 Style: rule hash bucketing (rightmost compound), style
       sharing cache (sibling style reuse), state-bounded invalidation.
- [ ] M35.3 Layout: parallel block layout (rayon subtrees where
       independent — the DAG shape from architecture.md pays off here).
- [ ] M35.4 Paint/raster: parallel raster tiles, display-list retention.
- [ ] M35.5 Memory: shared string/data arenas per pipeline stage;
       measure + cap the layer cache (M19.6); idle renderer memory trim.
- [ ] M35.6 Startup: cold start < 300ms to a blank tab on a mid laptop
       (measure; lazy-init everything non-critical).
- [ ] M35.7 Acceptance: numeric budgets published in
       `docs/perf-budgets.md`, regressions block milestone exit (T4).

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
       `Content-Disposition` handling, safe default paths.
- [ ] M37.3 Settings: startup page, search-engine selection, JS/CSP
       toggles (no-scripts mode), dark mode, engine feature flags.
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
- [ ] M42 Full bidi + ICU4X integration (writing modes, vertical text).
- [ ] M43 WebRTC (gate: T4).
- [ ] M44 Accessibility (accessibility tree, platform APIs — critical
       for real users, gate: T3 exit).
- [ ] M45 i18n UI, input methods (IME).
- [ ] M46 Password manager, autofill (gate: T4, security-reviewed).
- [ ] M47+ WPT-driven everything: the forever list (T1 track becomes the
       primary driver — see track below).

---

# 5. Cross-cutting tracks (never "done", run through every phase)

## T1 — Compatibility & testing
- [ ] T1.1 WPT runner harness (pull web-platform-tests, run subset,
       triage UI) — **start at M4**, becomes the main compat driver by M17.
- [ ] T1.2 Golden-rendering harness: fixed test pages → frame buffers →
       compare vs reference (Firefox) with tolerance; runs in CI.
- [ ] T1.3 Adversarial test-site (built in-repo): every attack pattern
       gets a page; the suite must fail closed. Extended at M27, M33.
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
       `style_document` and `pipeline`; `url_parse` and `font_data`
       wait for their milestones.
- [ ] T2.2 Seed corpora from WPT + real-site captures; corpus committed.
- [ ] T2.3 Coverage-guided runs in CI nightly; crash → reproducer test
       before anything else. Since 2026-10-01 CI runs every target for a
       minute a push and half an hour weekly, the corpus carried in the
       actions cache.
- [ ] T2.4 Structured fuzzing for the JS boundary (M24+): API call
       sequences, not just strings.

## T3 — Security cadence
- [ ] T3.1 Quarterly dependency audit (`cargo audit`), pinned Cargo.lock,
       `cargo deny` (advisories + licenses).
- [ ] T3.2 `unsafe` review gate: any PR adding `unsafe` carries a safety
       argument in code (enforced by convention + grep in CI).
- [ ] T3.3 `docs/security.md` updated with every milestone (threat model
       lives there from M33 on; before that, in architecture.md notes).
- [ ] T3.4 Secret-handling rule: no network egress anywhere except the
       `net` crate's fetch path. CI grep-enforced.

## T4 — Performance
- [ ] T4.1 `docs/perf-budgets.md` exists from M13; every milestone exit
       includes the numbers.
- [ ] T4.2 Criterion benches in-repo (`benches/`), run in CI, tracked.
- [ ] T4.3 Regression rule: >10% regression on any tracked bench blocks
       exit (unless a written exception with a follow-up item).

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
| D1 | JS engine | M24 | quickjs-ng / SpiderMonkey (mozjs) / V8 (rusty_v8) / Boa | quickjs-ng behind a `ScriptEngine` trait; trait designed for swap |
| D2 | Renderer sandbox mechanism | M34 | Win job objects+tokens / AppContainer / others per-OS | Job objects + restricted token v1 |
| D3 | GPU backend | M19 | wgpu / keep software only | wgpu, software path permanent fallback |
| D4 | TLS | M14 | rustls / native-tls | rustls (pure Rust, no OpenSSL linkage) |
| D5 | HTTP client | M14 | hyper / custom over rustls | custom-lite first (control + learning), hyper if maintenance bites |
| D6 | Image codecs | M23 | image crate ensemble / per-format crates | per-format crates behind a trait (faster, auditable) |
| D7 | Text/i18n | M8/M42 | rustybuzz+unicode crates / harfbuzz FFI / ICU4X | rustybuzz now, ICU4X at M42 |
| D8 | 2D rasterizer | M19 | tiny-skia forever / vello (wgpu-native) / both | both behind the existing `RenderBackend` trait |
| D9 | Chrome UI toolkit | M15/M37 | in-engine (softbuffer) / egui / native | in-engine (no extra toolkit; the engine is the toolkit) |
| D10 | Profile storage engine | M28 | redb / sled / SQLite / custom files | redb (pure Rust, embedded, single-file) |
| D11 | DevTools protocol | M36 | custom in-engine UI only / CDP-compatible | custom first; CDP-compat only if external tools wanted |
| D12 | UA string & fingerprint policy | M25 | honest UA / Chrome-mimic | honest minimal UA + minimal API surface (compat escape hatch flag exists but is not default) |
| D13 | Process granularity | M34 | per-tab / per-site-instance / hybrid | per-site-instance with LRU cap |
| D14 | Quirks mode | M40s | full quirks emulation / standards-only + limited quirk stylesheet | limited quirk stylesheet (uqlist-style) driven by T1 data |
| D15 | HTTP/3 (QUIC) | M47+ | quinn / skip | quinn when HTTP/2 coverage demonstrably costs compat |

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
