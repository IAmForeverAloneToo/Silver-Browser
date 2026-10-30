# Design note: text, the DOM and scripts (gap batch D)

The third batch of the G-list worked through by its priorities
(`silver-work/notes/gap-priorities.md`), the first of about twenty gaps
at once: the priorities' groups D (fonts and text), F (the HTML parser
and the DOM), C (modern script loading) and B (scripts that measure),
with two gaps batch E found. Written before the code, as the record of
the decisions, and corrected to the code when the batch ends: where they
disagree, the code wins, and section 4 lists the places. What ships is
described in `docs/architecture.md` "Current state", and the limits it
adds live in `docs/security.md`.

From this batch on, a step is checked by its crates' tests, clippy and
its sabotage cases, and committed signed but kept local; the whole gate,
the acceptance and the push come at the end of the batch.

Every page sets its fonts, and the engine draws one face at one weight:
`font-weight`, `font-style`, `font-feature-settings`, `line-height` and
the `font` shorthand are not properties here (G22); `@font-face` is
dropped and `document.fonts` missing (G12, part), so a page's own
typeface never shows and a script waiting on `document.fonts.ready`
waits forever. Lines break only at spaces (G10), so a long URL or a
hyphenated word overflows its box, and `white-space: nowrap` is invalid.
`visibility` is unknown (G12, part) and `text-shadow` too (G20, part).

Real HTML is malformed, and the tree builder has only the skeleton's
modes and "in body" (G44): a table's misnested cell, a stray `<b>`
across paragraphs and a `<template>` build the wrong tree. Comments,
doctypes, fragments and `template`'s content are not nodes (G47), which
frameworks build with. HTML's presentational attributes (`<center>`,
`bgcolor`, the table attributes, `<font>`) are ignored (G45).
`document.open` and `close` are missing (G48), and so are `Range`,
`Selection`, `TreeWalker`, `NodeIterator`, `compareDocumentPosition`,
`isEqualNode` and `normalize` (G52) and `moveBefore` (G66).

Module scripts and import maps are recognized and dropped (G41), so a
site that ships only modules runs no script at all; and since
`nomodule` is dropped too, the module/nomodule pattern runs neither.
`integrity` is not checked (G43). There is no `Intl` (G42), which
QuickJS-ng does not implement. Every realm runs the whole bootstrap,
the console, IndexedDB and Web Audio included (G163).

Scripts cannot measure: `offsetWidth`, `getBoundingClientRect`,
`scrollTop`, `innerText` and the rest do not exist, and
`getComputedStyle` sees neither the sheets a running script added nor
animations (G46). `document.styleSheets` and the CSSOM's rule objects
are missing, and `style` keeps text as written rather than serializing
it (G50). `getComputedStyle` lacks every shorthand and many longhands
(G159), gradients among them (G232); and `currentcolor` is resolved
when a colour computes, so a property inheriting it explicitly takes
the parent's colour (G226).

This batch gives the engine all of them but what section 2 names. Its
acceptance is WPT's directories for them (fetched for it, at the
checkout's commit, 30 October 2026: `html/syntax/parsing`, `dom/ranges`,
`dom/traversal`, `selection`, `html/semantics/scripting-1`,
`subresource-integrity`, `css/css-fonts`, `css/css-text`, `css/cssom`,
`css/cssom-view`), with expectations.

The documents this note follows, at their versions of 30 October 2026:
CSS Fonts 4 (the font properties, `@font-face`, the matching algorithm,
synthesis), CSS Font Loading 3 (`FontFace`, `document.fonts`); CSS Text
3 (line breaking, `white-space` and its longhands, `overflow-wrap`,
`word-break`, `text-shadow` in CSS Text Decoration 3), UAX #14; CSS 2's
`visibility` and `line-height` (section 10.8); HTML's tree construction
(13.2.6), its Rendering section 15 for the hints, `document.open` and
`close` (8.4.2), module scripts and import maps (8.1.3, 8.1.5);
DOM's node kinds, ranges, traversal and `moveBefore`; the Selection API;
Subresource Integrity; ECMA-402; CSSOM (`document.styleSheets`, the rule
objects, serialization) and CSSOM View (the layout queries).

## 1. Decisions

### D53: one font a box, chosen and named in the display list

The font properties computed: `font-weight` (1 to 1000, `bolder` and
`lighter` by the parent's), `font-style` (`normal`, `italic`,
`oblique <angle>`), `font-stretch`, `font-feature-settings`,
`font-variant` (its `small-caps`, by synthesis or the feature) and
`line-height` (`normal`, a number, a length or a percentage), with the
`font` shorthand, whose parser `canvas::font` already has.
`ComputedStyle::font_family: Arc<str>` becomes `font: Arc<FontSpec>`,
the family list and the rest of the inherited font properties together,
shared by every element that inherits it, so the style does not grow;
`line-height` stays beside `font_size_px`, small and often set.

The `text` crate chooses a face by CSS Fonts 4's matching (section 5):
the families in order, each by fontdb's query with the weight, style and
stretch, the generic families mapped to the system's, then the
collection's fallbacks by script. A face is known by a `FaceId`, a
number the collection hands out once a face is loaded and keeps for the
process. Measuring, shaping and the measure cache take a `FaceId` with
the size. Where the family has no bold, the face is emboldened as the
reference browser does (its outlines widened by a stroke of size / 24);
where it has no italic, slanted 14 degrees.

`DisplayItem::Text` carries its `FaceId` and synthesis, so that render
draws the face layout measured: render no longer chooses a font. A
renderer process ships the faces it is given as now (`FontFiles`), the
web fonts among them.

`line-height` puts half-leading above and below each inline box's
content (CSS 2 section 10.8.1), `normal` by the face's metrics as now.

### D54: web fonts, fetched and kept by the document

`@font-face` rules are gathered from the sheets in force (family,
`src`'s urls and `local()`, `font-weight`, `font-style`,
`font-stretch` ranges, `unicode-range`, `font-display`), at most
`MAX_FONT_FACES` a document. A face is fetched when a box first needs
its family, by the loader with the font destination and `cors`
anonymous mode, at most `MAX_FONT_BYTES`; its bytes are checked by
`ttf-parser` (WOFF2 is a later gap: its Brotli and transform need a
decoder; WOFF 1's zlib is in hand and taken). A document's faces lie
over the system collection in a `DocumentFonts` the cascade's families
resolve against first. Until a face arrives the fallback is drawn
(`font-display: swap`, the behaviour `auto` gets), and its arrival lays
the page out again.

`FontFace` (from a url or an `ArrayBuffer`), `FontFaceSet`
(`document.fonts`: `add`, `delete`, `load`, `check`, `ready`,
`status`, the loading events) and `self.fonts` in workers as an empty
set.

### D55: line breaking by ICU4X

UAX #14 by `icu_segmenter`'s line segmenter with compiled data, in the
`text` crate (`text::line_breaks`), the same major version as the ICU4X
crates `idna` already brings. Inline layout splits a word at its
opportunities; `white-space` becomes the shorthand of CSS Text 4's
`white-space-collapse` and `text-wrap-mode` longhands, so `nowrap`,
`pre`, `pre-wrap`, `pre-line` and `break-spaces` are all taken;
`overflow-wrap` (`anywhere`, `break-word`) and `word-break`
(`break-all`, `keep-all`) are taken; preserved spaces stop collapsing
under `pre` and `pre-wrap`. Hyphenation (`hyphens: auto`) is not here.

### D56: `visibility` and `text-shadow`

`visibility` inherited: a `hidden` box paints nothing of its own and is
not hit, its visible descendants still painted; `collapse` is `hidden`
but on a table's rows and columns, which take no room (CSS 2 section
17.5.5).

`text-shadow` inherited, a list of at most `MAX_TEXT_SHADOWS`: a
`DisplayItem::TextShadow` before its run, the run's glyph coverage
blurred by render's `shadow::gaussian` and tinted, the same in both
backends (the GPU's atlas caches it as box shadows are).

### D57: the tree builder by the standard's modes

The tree builder is rewritten as HTML 13.2.6's machine: every insertion
mode, the stack of template insertion modes, the list of active
formatting elements with its reconstruction and the adoption agency
algorithm (its limits of 8 outer and 3 inner loops), foster parenting,
"in head noscript", the frameset modes, and "after body". The open
stack's depth bound (`MAX_DEPTH`) and the foreign content of M32 stay.
Comments and doctypes become nodes (D58). The tests are WPT's
`html/syntax/parsing` (html5lib's tree tests among them) and html5lib's
own `.dat` files, which the harness now takes whole, with expectations.

### D58: every kind of node

`dom::NodeData` gains `Comment`, `DocumentType { name, public_id,
system_id }`, `ProcessingInstruction { target, data }` and
`DocumentFragment`; XML's `CDATASection` is a kind of text. A template's
content is a fragment the element owns, in the same arena, inert:
nothing in it is styled, laid out or run. `Attr` is a script object over
its element's attribute, made when asked for (`attributes`,
`getAttributeNode`), not an arena node. The cascade, layout, paint and
the journal's consumers skip the new kinds; `:empty` counts comments as
nothing. The bindings: `Comment`, `DocumentType`, `DocumentFragment`,
`ProcessingInstruction`, `Attr`, `NamedNodeMap`,
`createComment`, `createDocumentFragment`,
`createProcessingInstruction`, `template.content`,
`document.doctype`, and fragments in every "convert nodes" place.

### D59: live ranges in the DOM crate

A `Range` is a live boundary pair the `dom::Document` keeps (at most
`MAX_RANGES`, a range let go of by its script object dropped), moved by
every insertion, removal, text change and split as DOM section 5.5
says. `StaticRange`; `Selection`, one a document, its range the
document's (painted over text as M26's text fields paint theirs);
`TreeWalker` and `NodeIterator` in the bootstrap over the node methods,
an iterator's pre-removal steps by the journal; `compareDocumentPosition`,
`isEqualNode`, `normalize` natives. `moveBefore` moves a node without the
removal and insertion steps' side effects (focus, an `iframe`'s
document, animations and `MutationObserver` see a move).

`document.open` (HTML 8.4.2.1): the document's children removed, a new
parser at a script-created insertion point, the URL left; `close` ends
it; `write` with no insertion point opens the document first as the
standard says, its destructive-write counters kept.

### D60: presentational hints as the cascade's hints

HTML section 15's attributes join `hint_declarations` beside the list,
image and frame hints: `align` on blocks, headings, tables, rows and
cells (with `text-align: -webkit-center`, `-webkit-left`,
`-webkit-right` and `match-parent`), `<center>`; `body`'s `bgcolor`,
`background`, `text`, `link`, `vlink`, `alink`; the table attributes
`border`, `cellpadding`, `cellspacing`, `width`, `height`, `bgcolor`,
`nowrap`, `valign`, `rules`, `frame`; `hr`'s `align`, `color`,
`noshade`, `size`, `width`; images' `hspace`, `vspace`, `border`,
`align`; `<font>`'s `color`, `face` and `size`; and the UA sheet's
`[hidden]`. Every hint attribute is in the invalidation's list, the
frame hints too.

### D61: module scripts, a graph fetched by the engine

rquickjs's `loader` feature (no new crate: `relative-path` is in the
lock already): a resolver that refuses the bootstrap's own module names
and a loader that serves only sources the engine has fetched. The
engine fetches a module script's graph first (`cors` mode, the module
script destination, JavaScript MIME types only), parsing each module's
static imports by declaring it, at most `MAX_MODULES` a realm and
`MAX_MODULE_DEPTH` deep; then links and evaluates it, a top-level await
included. `import()` fetches its graph and settles its promise from a
task. `import.meta.url` and `resolve`. Import maps (`imports`,
`scopes`, `integrity`), one a document before the first module. Module
scripts are deferred by default, `async` taken; `nomodule` runs where
modules are not supported, so here it does not. Module workers and
module service workers; `modulepreload` links. Each module evaluated
once a realm, by its URL.

### D62: Subresource Integrity

`integrity` on `script`, `link rel=stylesheet`, `link rel=preload` and
`modulepreload`, and fetch's `integrity` option: the metadata parsed
(sha256, sha384, sha512, the strongest kept, options ignored), the body
hashed with `aws-lc-rs` as `net::csp` does, a mismatch an error as a
network error is; a cross-origin request without CORS fails it.

### D63: `Intl` by ICU4X, read at its first use

`Intl.NumberFormat`, `DateTimeFormat`, `Collator`, `PluralRules`,
`RelativeTimeFormat`, `ListFormat`, `Locale`, `getCanonicalLocales`
and `supportedValuesOf`, and the locale forms of `toLocaleString`,
`toLocaleDateString`, `toLocaleTimeString` and `localeCompare`, by
ICU4X's formatters with compiled data for its full locale set; options
as ECMA-402 resolves them, `formatToParts` and `resolvedOptions`
included. The time zone is the system's or UTC. `Temporal` is not here.
The bindings are a chunk read at their first use (D64).

### D64: binding chunks read at their first use

M41's lazy GPU bindings become the pattern for every chunk a page may
not touch: Intl, IndexedDB and `navigator.storage`, Web Audio,
WebAssembly, the 2D canvas and `OffscreenCanvas`, each its own module
compiled once a process, installed by accessors on the global and the
prototypes that read it, per realm kind's exposure, by a generalized
`chunkInstall(name)`; a test holds each chunk's names complete.

### D65: a style and layout flush for scripts

A script run borrows the shown page's styles, layout and animations,
not a copy: the run takes `scripts` out of `Shown` and lends the rest.
`getComputedStyle` is then the engine's own cascade, brought up to date
(the sheets a script added rescanned, transitions started, animations
sampled at the frame's time); the layout queries lay the page out when
it is stale (CSSOM View): `offsetTop`, `offsetLeft`, `offsetWidth`,
`offsetHeight`, `offsetParent`, `clientTop` to `clientHeight`,
`scrollTop`, `scrollLeft`, `scrollWidth`, `scrollHeight`,
`getBoundingClientRect`, `getClientRects`, `elementFromPoint`,
`elementsFromPoint`, `scrollTo`, `scrollBy`, `scrollIntoView`,
`window.scrollX`, `scrollY`, `scrollTo`, `innerText` (getter and
setter) and `outerText`; at most one layout a flush, and none when
nothing changed. `getComputedStyle`'s resolved values are the used ones
for the properties CSSOM names (`width`, `height`, the margins,
paddings and insets).

### D66: the CSSOM and every computed value

`getComputedStyle` gives every property the engine computes and every
shorthand of them (G159), gradients (G232) and calculations serialized
as CSS Values 4 section 10.13 says. A style sheet's rules become a list
the CSSOM can change: `document.styleSheets`, `CSSStyleSheet`
(`cssRules`, `insertRule`, `deleteRule`, `replace`, `replaceSync`,
`disabled`, `media`, `ownerNode`, `href`), `CSSStyleRule`,
`CSSMediaRule`, `CSSImportRule`, `CSSKeyframesRule`, `CSSKeyframeRule`,
`CSSFontFaceRule`, `CSSSupportsRule`, `CSSLayerBlockRule`, `CSSPageRule`,
and `<style>.sheet`, `<link>.sheet`; an edited sheet is kept by its
owner until the owner's text changes. Constructed and adopted sheets are
not here. `style` and `cssText` serialize specified values in their
canonical forms and set and read shorthands through their longhands, by
a token serializer in the `css` crate and one for specified values in
`style::serialize`.

`currentcolor` keeps its keyword (G226): a computed colour may be the
keyword, inherited as such and resolved to the element's own `color`
where it is used, in paint, the canvas and serialization; a
`color-mix()` holding it stays resolved where it computes (section 2).

## 2. Not in this batch, said rather than hidden

- WOFF2 web fonts (a Brotli decoder and its glyph transforms), variable
  fonts' axes past weight, width and slant (`font-variation-settings`),
  color fonts, `size-adjust` and the other metric descriptors.
- Hyphenation (`hyphens: auto`), `text-justify`'s inter-character mode,
  `text-wrap: balance` and `pretty`, and vertical writing modes.
- Constructed and adopted style sheets, `CSSStyleSheet.replace` of a
  sheet with `@import`, and the CSS Typed OM.
- `Temporal`, `Intl.DisplayNames`, `Intl.Segmenter` and
  `Intl.DurationFormat`.
- Speculation rules, import attributes (JSON and CSS modules), and
  `import.meta.resolve`'s import-map edge cases beyond the standard's
  tests.
- `color-mix()` holding `currentcolor` kept as a recipe (its value is
  the element's own colour mixed where it computes).
- A selection's rendering past text: `::selection` styling and
  selection across replaced elements.

## 3. Order of work

1. *Fonts* (D53): the font properties and shorthand, face matching and
   synthesis, `FaceId` in the display list, `line-height`.
2. *Web fonts* (D54): `@font-face`, fetching, `DocumentFonts`,
   `FontFace` and `document.fonts`.
3. *Text* (D55, D56): line breaking by ICU4X, the `white-space`
   longhands, `overflow-wrap`, `word-break`, `visibility`, `text-shadow`.
4. *The tree builder* (D57).
5. *Node kinds* (D58).
6. *Ranges and the rest of the DOM* (D59): `Range`, `Selection`,
   traversal, the node comparisons, `moveBefore`, `document.open`.
7. *Presentational hints* (D60).
8. *Binding chunks* (D64).
9. *Module scripts* (D61).
10. *Subresource Integrity* (D62).
11. *`Intl`* (D63).
12. *The flush* (D65).
13. *Computed values and `currentcolor`* (D66).
14. *The CSSOM* (D66).
15. *The acceptance*: WPT's directories, the whole gate, T4.3, fuzzing,
    documents, then the push.

## 4. Corrected to the code

What the work found that this note did not say, or said otherwise, by
step:

- *Fonts and web fonts* (D53, D54, closed after the batch): a
  `unicode-range` is read from its declaration's source text, which the
  parser keeps the spans of (CSS Syntax 3 section 7.1), and kept as
  `U+X-Y` identifiers; the faces of a family's best match that cover
  some code points make a chain the font carries (`Font::chain`, kept
  in the collection's registry beside the faces lent), and a run is
  split by the chain's face for each code point, a face not had yet
  asked for only when a text needs it. `local()` is a system face by
  its full or PostScript name, read from its name table. `font-family`
  is kept as the reference browser writes it, a quoted generic name a
  name, which the text crate reads back. The `font-variant` longhands
  are computed; `font` and `font-variant` are kept as their longhands in
  a `style` object and said from them. An `@font-face` rule's `style`
  holds its descriptors, checked and written back
  (`style::font_face::descriptor`). `document.fonts` holds the rules'
  faces (CSS-connected), loads them through the engine, and hears from
  it when its faces start and stop loading (a `FontFaceSetLoadEvent`
  task); a `FontFace`'s urls are fetched in turn as fonts; workers have
  `self.fonts`. A text control's caret is measured in its own font.
