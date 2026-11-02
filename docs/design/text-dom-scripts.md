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
`ttf-parser` (WOFF 1 by zlib; WOFF2 by `brotli-decompressor`, its
`glyf`, `loca` and `hmtx` transforms undone, after the batch). A document's faces lie
over the system collection in a `DocumentFonts` the cascade's families
resolve against first. Until a face arrives the fallback is drawn
(`font-display: swap`, the behaviour `auto` gets), and its arrival lays
the page out again.

`FontFace` (from a url or an `ArrayBuffer`), `FontFaceSet`
(`document.fonts`: `add`, `delete`, `load`, `check`, `ready`,
`status`, the loading events) and `self.fonts` in workers as an empty
set. *(corrected: a worker has `FontFace` and its own `self.fonts`,
whose faces its canvases draw in; a canvas's text is drawn in the
document's faces, and a face only a canvas asks for is fetched after
its script runs.)*

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
ICU4X's formatters with compiled data; options as ECMA-402 resolves
them, `formatToParts` and `resolvedOptions` included. The reference is
Chrome's (V8 over ICU 78, CLDR 48): its 60 languages (others resolve to
the default, `en-US`), and where ICU4X's data or behavior differ from
ICU's, tables read from Chrome's own `icudtl.dat` or its outputs:
`intl_dates.txt` (the date patterns, available formats, interval
formats, eras, day periods and zone formats of each language, the
Buddhist and Persian calendars of `th` and `fa`, the names ICU4X lacks
for `pa-Arab`, regions' hour cycles, zones' regions, metazones and
their reference zones, and the zone and region names where ICU4X's
differ), `intl_units.txt` (unit patterns ICU4X lacks or writes
otherwise) and `intl_zones.txt` (each zone's offsets from 1970 on, its
zoneinfo64's changes and final rule). A date's pattern is the one ICU's
DateTimePatternGenerator finds for V8's skeleton of the options (a
style's for `dateStyle` and `timeStyle`), formatted by ICU4X's names
with the fields ICU writes otherwise quoted in; a range's is the one
ICU's DateIntervalFormat picks for the skeleton of that pattern (the
greatest different field's interval format, else the fallback), and
zones' names follow ICU's TimeZoneFormat over ICU4X's metazone names
(standard names for a zone keeping no daylight time near the date, the
location format by region or by the city in the zone's name, a partial
location where the zone keeps another time than the metazone's
reference zone). The time zone is the system's (on Windows its zone's
key name and the user's region, which the shell reads as it starts, a
call the renderers' sandbox allows, mapped by CLDR's windowsZones), UTC,
an offset, or any IANA zone, names canonical as ICU keeps them
(`Asia/Calcutta`). Currencies (symbol, narrow symbol, code, plural
name by the formatted number's own digits, accounting) and compact
notation by `icu_experimental` 0.6 and `icu_decimal`'s compact
formatter; units by `icu_experimental`'s data provider for every
sanctioned unit in every language. `formatRange` and
`formatRangeToParts` of both formatters, `PluralRules.selectRange`
(ICU's plural ranges, `other` where a locale has none) and
`Intl.Locale`'s week (first day and weekend, no minimal days, as
Chrome 156 has it), text, hour cycle, calendar,
collation (the root order for the languages ICU keeps no collation
of), numbering system and time zone information. Calendars other
than the gregorian one, the Buddhist and the Persian (those a `-u-ca-`
asks for) keep ICU4X's own patterns. `Temporal` is not here.
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
`color-mix()` holding it stays resolved where it computes (section 2;
`background-color`'s since kept, section 4).

## 2. Not in this batch, said rather than hidden

This section was written before the code; the batch then closed most of
what it named (section 4). What is left:

- Colour fonts, and with them `font-variant-emoji`'s choice of an emoji
  face and the variation selectors; variable fonts' axes past weight,
  width and slant (`font-variation-settings`).
- Vertical writing modes and `direction` as a property (the `dir`
  attribute alone sets it), and with them right-to-left scroll offsets;
  bidi reordering across inline boxes.
- `text-spacing-trim` and `text-fit` laid out (both are parsed,
  computed and serialized); Chrome 156 ships no `hanging-punctuation`,
  `hyphenate-limit-lines`, `word-space-transform` or `text-group-align`,
  which are parsed alone.
- The CSS Typed OM, `Temporal`, speculation rules, and `url()`'s request
  modifiers.
- A second document in a realm (`createHTMLDocument`, `DOMParser`, a
  template's contents' own owner document: G47, G51), and a script
  reaching another frame's document (G90).
- A selection's painting and the user's selections (G59).
- `preservesPitch`: `playbackRate` speeds an element's sound by
  resampling it, with no time-stretch; `setSinkId`, `MediaDevices` and
  `srcObject` playback.
- In `Intl`: the Japanese calendar's Meiji years 1 to 5, the patterns'
  numbering overrides (`d=hanidec`), and the ranges of the Chinese,
  Hebrew and Indian calendars.

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
step. Midway the maintainer set a rule for the batch: a gap found while
closing gaps is closed in it, not listed (the last bullet says how). The
steps below say where each part ended, not where its first commit left
it.

- **Fonts (step 1, D53).** The family moved into `FontProps` with the
  other font properties and `line-height`, behind one pointer. Generic
  `sans-serif` and `system-ui` are the primary font's families (Segoe UI
  first), not the reference browser's Arial, so that pages in the
  initial font draw as before; Times, Courier and Helvetica are Times
  New Roman, Courier New and Arial, as Blink has them. Synthesis as
  Blink and Skia do it: bold when 600 or more is asked of a face under
  600, a slant of a quarter for italic. A face's `font-style` is an
  oblique range (`text::FaceSlant`), matched in the reference browser's
  order. `font-kerning` is a property, and it, `font-feature-settings`
  and a canvas's `lang` reach shaping: a font's shaping options are
  kept in the registry with its face chain (`Font::chain`), so that a
  fragment stays 136 bytes. `font-family` is kept as the reference
  browser writes it, a quoted generic name a name; the `font-variant`
  longhands are computed. A font asked for again is the thread's last
  choice, compared rather than looked up (`faces::Choices`): the first
  T4.3 found layout twice as slow from boxed keys hashed per word.
  Renderers get the system's faces as a manifest in the `Fonts`
  message (IPC version 2).
- **Web fonts (step 2, D54).** A document's faces are
  `text::DocumentFaces` laid over the collection, their ids lent by the
  registry. `unicode-range` is read from the declaration's source text,
  which the CSS parser keeps the spans of, and the faces of a family that
  cover some code points make a chain; a run is split by the face for
  each code point, a face asked for only when a text needs it. `local()`
  is a system face by its full or PostScript name; WOFF2 is read
  (`text::woff2`, brotli-decompressor, every size bounded before it is
  allocated). `document.fonts` holds the rules' faces (CSS-connected
  `FontFace`s kept by their descriptors), loads them through the engine,
  hears when loading starts and stops (`FontFaceSetLoadEvent`), and reads
  a `<style>` a script just put in (`Page::sheets_now`); `ready` waits
  while a style or layout is to come, the bindings laying the document
  out first so that its faces are asked for. Workers have `self.fonts`,
  their canvases drawing in their own faces. A `<link rel=preload
  as=font>` is fetched and fires `load` or `error`; a box's web font is
  asked for when the box is laid out, text in it or not. The font-
  relative units (`ex`, `ch`, `cap`, `ic`, `lh`, `rlh`, `rcap`, `rch`,
  `rex`, `ric`) measure the element's own first available font
  (`FontMeasure::units`), and styles that used them are cascaded again
  when a face comes (`Styles::uses_font_units`, G12).
- **Text (step 3, D55, D56).** `white-space` is a pair of longhands.
  `icu_segmenter` is pinned `~2.3.0`: 2.4 moved idna's ICU crates and
  failed IdnaTestV2. Between printable ASCII the reference browser does
  not follow UAX #14: its own table, measured from Chrome 156
  (`ascii_pair_breaks`), is used both in a fast path that skips the
  segmenter for ASCII (a test holds the two equal over every text of up
  to five of sixteen characters) and over it. `line-break`'s strictness
  is the segmenter's, tailored by the content language, which the
  cascade keeps from `lang`; breaks go across inline boxes where UAX #14
  has one. A word stays one item with its opportunities, cut only where
  a line breaks. `hyphens` (the `hypher` crate's patterns), `tab-size`
  with stops from the line's start, `text-indent` with `hanging` and
  `each-line`, `text-transform`'s full-width, kana and language
  tailoring, `text-wrap: balance` (beside floats too) and `pretty`,
  `text-autospace`, `letter-spacing` by grapheme cluster and not inside
  a cursive join, hanging other space separators, justification of kept
  spaces and of ideographs, control characters drawn. A run with no
  right-to-left letter is shaped left to right, white space at its ends
  apart. A text shadow is blurred over the whole ink, whatever the clip,
  and decorations cast it. `visibility: collapse` collapses columns.
  ASCII left to right skips bidi reordering.
- **The tree builder (step 4, D57).** html5lib's tree tests went from
  1,285 to 1,959 of 1,959: the scripted ones run in the engine
  (`loader::html5lib_tests`), and `selectedcontent` takes a clone of the
  selected option, at parse and as the selection changes. The implied
  `tbody`, `colgroup` and foster parenting changed 41 of layout's table
  tests. XHTML is the XML parser's (step 15).
- **Node kinds (step 5, D58).** The doctype's fields are `Box<str>`, so
  `NodeData` stays 64 bytes. `insert_before` does the whole of "ensure
  pre-insertion validity" and moves a fragment's children. Template
  contents are a fragment `allocate` makes with each HTML `template`.
  `to_node` leaves comments, doctypes and processing instructions out, so
  the differential comparison counts only what a snapshot keeps.
  `:empty` passes over comments, as the reference browser does.
- **Ranges and the rest (step 6, D59).** Live ranges and node iterators
  are kept in the document and moved directly by its operations, not by
  the journal; character data's mutators go through `replace_data`, by
  UTF-16 offsets. A range set in an `Attr` is at (attr, 0), its own
  root. `moveBefore` journals `Moved` after its removal and insertion,
  so that frames, animations and media keep their state. `document.open`
  gives the host a parser of its own; it aborts a load's parser and
  fetch, erases the listeners, and leaves the document loading until
  `close`; `DOMContentLoaded` is a task of its own. A selection's string
  is its rendered text. Template contents, live state and the move flag
  share one boxed field, for clippy's `large_enum_variant`.
- **Presentational hints (step 7, D60).** The legacy alignments are kept
  beside `text-align` with the parent's (`legacy_align`, `placed_by`).
  The font-size keywords were missing altogether and came with it. A
  `body`'s margins from `marginwidth`/`leftmargin` and
  `marginheight`/`topmargin`, an iframe's own in their place (sent to a
  frame of another renderer with its hole); a table's `frame`, `rules`
  and `bordercolor`; `pre`'s `wrap`. `legend` and `fieldset` are laid
  out as the reference browser does (`layout::fieldset`): the legend in
  the top border, the border's gap behind it.
- **Binding chunks (step 8, D64).** Six chunks, each a script of its own
  that `native.chunk(name)` reads: `gpu`, `intl`, `idb`, `audio`, `wasm`
  and `canvas`. A chunk's name is an accessor until the chunk is made; a
  window's realm is made in 6.2 ms, from 7.5.
- **Module scripts (step 9, D61).** The graph's imports are found by
  declaring each module in a throwaway context, and modules are handed
  over dependencies first. An `import()` in code a string compiled
  resolves against, and fetches with the options of, the script that
  made it; a muted script's against `about:blank`. JSON, CSS and text
  modules by their `type`, the module map keyed by URL and type, a CSS
  module's sheet a constructed `CSSStyleSheet`. Import maps after the
  first are merged; module service workers run; the preload scanner sees
  module scripts. `import('bootstrap')` had given any page the
  bootstrap's module; it is refused now.
- **Subresource Integrity (step 10, D62).** Checked on a body taken
  whole, `NetworkError::Integrity` (wire tag 30); CSP lets through a
  script whose `integrity` hashes the policy lists.
- **`Intl` (step 11, D63).** ICU4X 2.3, with `icu_experimental` for
  currencies, units and compact notation. Its data falls short of the
  reference browser's in many places, so date patterns, intervals, zone
  names and corrections are read from Chrome's own ICU data into
  `intl_dates.txt`, `intl_units.txt`, `intl_zones.txt`, `intl_names.txt`
  and `intl_islamic.txt` by generators kept in
  `silver-work/tooling/intl-gen`, and compared with Chrome 156
  (`--lang=en-US`) over more than 200,000 outputs. Windows' zone is read
  in the sandbox (`--check-processes sandbox` holds it). `Segmenter`,
  `DurationFormat` and `DisplayNames` came too.
- **The flush (step 12, D65).** Not the borrow this note describes: the
  host lays out a copy from its own cascade, at most once a document
  version, but is lent the shown page's images, fonts, scroll offsets
  and animations (`scripts::Lent`), so `getComputedStyle` and the layout
  queries sample the running animations and transitions (G46).
  `getComputedStyle`'s insets are used values. Scrolling goes across as
  well as down, by script, wheel, arrows and a scrollbar the shell asks
  for (IPC version 3); `scroll-behavior`, `scroll-margin` and
  `scroll-padding`; a range's rectangles from its text fragments.
- **Computed values (step 13, D66).** `currentcolor` is a spare bit of
  `Color::missing`, resolved against the element's final `color`;
  `color-mix()` with it in `background-color` is kept for inheritance.
  `getComputedStyle` lists the longhands and serializes their
  shorthands; `overflow-x`/`-y` and `background-position-x`/`-y` are
  longhands of their own; `all` is every longhand but `direction` and
  `unicode-bidi`. Gradients take 4,096 stops.
- **The CSSOM (step 14, D66).** The rules a script sees are a view of
  the parsed sheet the cascade reads, edited in place; a run's `<style>`
  sheets are made again as the document changes (`Page::rescan_inline`),
  and a sheet change restyles what the changed rules match. Specified
  values, selectors and rule blocks are serialized canonically, the box
  shorthands kept as their longhands; a longhand is read from a
  shorthand kept as written (`scripts::cssom::longhands`). Constructed
  and adopted sheets, `disabled` and media lists in the cascade, the
  namespace, layer, page, container, counter-style and font-feature-
  values rules.
- **The acceptance (step 15).** WPT's batch directories came in as
  planned, and the runner gained `WPT_ONLY`, a results file, variants, a
  second port, https with a root of its own, the Python handlers the
  tests need written natively, `test_driver.bless`, and a monospace face
  (DejaVu Sans Mono) for the tests. What it found: a `br` had never
  broken a line; the batch's support files (the modules the scripting
  tests import) had never been copied; layout was twice as slow (step
  1's note); the dom_api fuzz target's comparison counted nodes a
  snapshot leaves out and missed the CSSOM's edits; a run's host took
  the window rather than copying it, so the second of two deferred
  scripts had none. The last T4.3 found three slowdowns, each mended: a
  line's words asked the font collection again for their face, metrics
  and gap (the load page's first-frame layout 23 ms against the base's
  15.4; now 15.6, `inline::LineMemo`); a `font-variant` longhand copied
  whole to compare one field of it, and a transition looked for among
  the running and completed ones for each of 198 longhands (twenty class
  changes under `transition: all`, 36.7 ms of style against 29.7; now
  26.8 against 27.5). Six fuzz targets no longer built against the
  batch's interfaces. Eight targets (dom_api, html_parse, xml_parse,
  pipeline, style_document, mutation, css_stylesheet, values) then
  replayed their corpora and ran fifteen minutes each, dom_api, at two
  inputs a second, twenty minutes more from 200 of its seeds: nothing
  found. The
  gate's whole run then found a playback rate that overwrote a seek
  still to be done, so that the element waited on it for ever (WPT's
  preserves-pitch, by the pipeline thread's timing), and
  `decodeAudioData` decoding 44.8 million samples to refuse them by a
  length the container had said (`media::decode_all_within`); 3,651
  tests pass workspace-wide, three ignored.
- **Closing what the batch found.** Under the maintainer's rule every
  gap the steps set aside, and the 1,533 WPT entries the first pull
  marked undiagnosed, were worked through by agents in worktrees, each
  merged by patch with its own tests and sabotage and the whole WPT run
  at the merge. The WPT subset grew from 11,037 tests to 12,069 (XHTML,
  variants, `.https`, `br`), and passes 9,284 of them and 83,166 of
  112,336 subtests, every other result named with its reason. What is
  left is in section 2.
