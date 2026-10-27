# web-platform-tests subset

The tests here are copied from
[web-platform-tests](https://github.com/web-platform-tests/wpt),
under its BSD licence (`LICENSE.md` beside this file), at commit

```text
dc7f6a3c0000e1da9e575ff4ef5618cff136697b (2026-10-03)
```

They are the tests of `css/css-flexbox`, `css/css-grid`, `css/css-transforms`, `css/css-will-change`, `css/mediaqueries`, `css/css-conditional`, `css/css-cascade`, `css/css-variables`, `css/css-values`, `css/css-lists`, `css/css-counter-styles`, `css/css-content`, `css/css-text-decor`, `css/css-text/text-align`, `css/css-text/text-justify`, `css/CSS2/lists`, `css/CSS2/generated-content`, `css/css-animations`, `css/css-transitions`, `css/css-easing`, `css/css-images`, `css/css-backgrounds`, `html/semantics/embedded-content/the-img-element`, `css/selectors/invalidation`, `dom/nodes`, `html/dom`, `css/selectors`, `html/semantics/forms`, `svg/painting/reftests`, `svg/shapes/reftests`, `svg/struct/reftests`, `svg/pservers/reftests`, `svg/coordinate-systems`, `svg/styling`, `svg/text/reftests`, `svg/render/reftests`, `svg/as-image` and `svg/crashtests`
that use only what this engine has, chosen by the rule in
`examples/wpt.rs` and copied with their references, stylesheets
and images at their own paths by

```sh
cargo run -p engine --example wpt -- pull <checkout>
```

which rewrites them, `MANIFEST` and this file. `tests/wpt.rs` runs
them; `expectations.txt` names those expected to fail, and why.

With them are the testharness.js tests of `fetch/api/headers`, `fetch/api/request`, `fetch/api/response`, `fetch/api/basic`, `xhr`, `url` (the `urlsearchparams-*` tests), `FileAPI/blob`, `webstorage`, `wasm/jsapi` (the `interface.*` tests), `wasm/jsapi` (the `prototypes.*` tests), `wasm/jsapi/constructor`, `wasm/jsapi/global`, `wasm/jsapi/instance`, `wasm/jsapi/memory`, `wasm/jsapi/module`, `wasm/jsapi/table`, `html/canvas/element`, `html/canvas/offscreen`, `workers`, `webmessaging`, `html/infrastructure/safe-passing-of-structured-data`, `html/webappapis/structured-clone`, `content-security-policy`, `html/browsers/sandboxing`, `referrer-policy`, `html/semantics/embedded-content/the-iframe-element`, `webaudio`, `html/semantics/embedded-content/media-elements`, `webgl` (M27, M29, M31, M40, M41)
that the runner's one server can serve: no Python handler, no `.sub`
template, no other host, nothing over `https:`, no shared or
service worker and no module worker. `resources/testharness.js` is
the real one; the runner serves its own `testharnessreport.js`, and
the pages wptserve makes of an `.any.js` test (for a window and,
as its `global` says, a dedicated worker), a `.window.js` test or a
`.worker.js` test, and an `.any.js` test's worker script. Each
subtest they fail is named in `expectations.txt`, and why.

## Taken

| Kind | Tests |
| --- | ---: |
| checklayout | 70 |
| crashtest | 101 |
| reftest | 2909 |
| testharness | 4673 |
| all | 7753 |

## Left out

The first reason the rule found for each test it left out:

| Reason | Tests |
| --- | ---: |
| script | 3153 |
| Python handler | 1985 |
| @font-face | 902 |
| declaration of font | 713 |
| CDATA section in XHTML | 550 |
| declaration of display | 505 |
| the manual reftests (by the reftest rule) | 407 |
| layers (beginLayer, not in the standard) | 374 |
| https: | 337 |
| not a reftest, a check-layout test or a crashtest | 292 |
| not a testharness test | 275 |
| selector | 241 |
| declaration of line-height | 230 |
| declaration of writing-mode | 207 |
| element <br> | 179 |
| canvas filters (G81) | 150 |
| a shared or service worker | 126 |
| dir=rtl | 113 |
| element <iframe> | 101 |
| declaration of direction | 91 |
| SVG <pattern> (G87) | 87 |
| declaration of border-radius | 85 |
| wptserve pipe | 82 |
| declaration of white-space | 81 |
| declaration of container-type | 78 |
| wptserve substitution (.sub) | 69 |
| declaration of background-image | 65 |
| element <object> | 56 |
| declaration of font-weight | 54 |
| element <canvas> | 52 |
| a window | 48 |
| self-closing <div/> in XHTML | 48 |
| element <video> | 47 |
| element <embed> | 46 |
| declaration of background | 44 |
| declaration of font-variant-numeric | 44 |
| declaration of flex-wrap | 38 |
| SVG <textpath> (G87) | 37 |
| declaration of transform | 36 |
| nested rule | 36 |
| element <hr> | 35 |
| @counter-style | 34 |
| @scope | 32 |
| declaration of visibility | 32 |
| declaration of box-shadow | 30 |
| declaration of appearance | 29 |
| declaration of background-clip | 28 |
| @property | 27 |
| colour spaces past sRGB (G81) | 27 |
| declaration of inline-size | 27 |
| declaration of color | 26 |
| declaration of grid-template-rows | 26 |
| declaration of overflow-y | 25 |
| @container | 23 |
| declaration of width | 22 |
| declaration of grid-template-columns | 21 |
| declaration of text-shadow | 21 |
| element <fieldset> | 21 |
| SVG <marker> (G87) | 20 |
| declaration of background-color | 20 |
| declaration of contain | 20 |
| input type=range | 18 |
| a frame of another kind than iframe | 17 |
| external SVG reference (G87) | 17 |
| declaration of filter | 16 |
| @namespace | 15 |
| declaration of border-top-left-radius | 14 |
| declaration of link-parameters | 14 |
| declaration of object-view-box | 14 |
| declaration of zoom | 14 |
| BroadcastChannel (G84) | 13 |
| SVG <image> (G87) | 13 |
| declaration of block-size | 13 |
| declaration of columns | 13 |
| declaration of text-decoration-inset | 13 |
| SVG <foreignobject> (G87) | 12 |
| colour types past 8 bits (G81) | 12 |
| declaration of all | 12 |
| declaration of height | 12 |
| declaration of overflow-x | 12 |
| img srcset | 12 |
| unchecked attribute data-foo | 12 |
| a module worker (G41) | 11 |
| declaration of align-self | 11 |
| declaration of column-count | 11 |
| declaration of text-emphasis | 11 |
| declaration of text-decoration-skip-spaces | 10 |
| declaration of text-decoration-skip-ink | 9 |
| unchecked attribute data-expected | 9 |
| a blob: URL (G68) | 8 |
| element <meter> | 8 |
| declaration of break-after | 7 |
| declaration of grid-template | 7 |
| declaration of inset | 7 |
| element <math> | 7 |
| declaration of flex | 6 |
| declaration of min-width | 6 |
| element <dialog> | 6 |
| element <output> | 6 |
| element <picture> | 6 |
| input type=date | 6 |
| unchecked attribute data-val | 6 |
| declaration of --a | 5 |
| declaration of border-top-right-radius | 5 |
| declaration of break-before | 5 |
| declaration of clip-path | 5 |
| declaration of pointer-events | 5 |
| declaration of -webkit-locale | 4 |
| declaration of column-width | 4 |
| declaration of container-name | 4 |
| declaration of mask-image | 4 |
| declaration of min-height | 4 |
| declaration of mix-blend-mode | 4 |
| declaration of resize | 4 |
| declaration of scrollbar-gutter | 4 |
| declaration of scrollbar-width | 4 |
| declaration of text-indent | 4 |
| declaration of z-index | 4 |
| element <legend> | 4 |
| element <progress> | 4 |
| input type=image | 4 |
| input type=month | 4 |
| input type=time | 4 |
| self-closing <canvas/> in XHTML | 4 |
| @starting-style | 3 |
| HDR (G81) | 3 |
| SVG <filter> (G87) | 3 |
| declaration of -webkit-text-fill-color | 3 |
| declaration of accent-color | 3 |
| declaration of background-position-x | 3 |
| declaration of background-size | 3 |
| declaration of border-bottom-right-radius | 3 |
| declaration of caret-color | 3 |
| declaration of color-scheme | 3 |
| declaration of flex-line-count | 3 |
| declaration of padding-inline-start | 3 |
| declaration of text-orientation | 3 |
| declaration of transform-origin | 3 |
| element <audio> | 3 |
| element <details> | 3 |
| element <marquee> | 3 |
| input type=datetime-local | 3 |
| input type=file | 3 |
| missing file resources/WebIDLParser.js | 3 |
| missing file svg/styling/about:blank | 3 |
| not for a window or a dedicated worker | 3 |
| SVG <mask> (G87) | 2 |
| SVG <set> (G87) | 2 |
| SharedArrayBuffer (G83) | 2 |
| declaration of border | 2 |
| declaration of border-bottom-left-radius | 2 |
| declaration of border-image | 2 |
| declaration of content | 2 |
| declaration of flex-basis | 2 |
| declaration of flex-grow | 2 |
| declaration of flex-shrink | 2 |
| declaration of float | 2 |
| declaration of font-size | 2 |
| declaration of font-style | 2 |
| declaration of image-rendering | 2 |
| declaration of margin-block-end | 2 |
| declaration of margin-top | 2 |
| declaration of mask | 2 |
| declaration of order | 2 |
| declaration of outline | 2 |
| declaration of padding-left | 2 |
| declaration of rotate | 2 |
| declaration of scrollbar-color | 2 |
| declaration of text-decoration-line | 2 |
| declaration of text-emphasis-style | 2 |
| declaration of text-overflow | 2 |
| declaration of text-size-adjust | 2 |
| declaration of translate | 2 |
| declaration of unicode-bidi | 2 |
| missing file content-security-policy/script-src/data:text/javascript;charset=utf-8;base64,ZGF0YVNjcmlwdFJhbiA9IHRydWU7 | 2 |
| missing file css/css-transforms/about:blank | 2 |
| namespaces in XHTML | 2 |
| self-closing <fieldset/> in XHTML | 2 |
| self-closing <iframe/> in XHTML | 2 |
| self-closing <param/> in XHTML | 2 |
| self-closing <textarea/> in XHTML | 2 |
| self-closing <video/> in XHTML | 2 |
| unchecked attribute data-affiliation | 2 |
| unchecked attribute data-color | 2 |
| unchecked attribute data-description | 2 |
| unchecked attribute data-document | 2 |
| unchecked attribute data-marker | 2 |
| unchecked attribute data-name | 2 |
| unchecked attribute data-section-title | 2 |
| unchecked attribute data-target | 2 |
| @custom-media | 1 |
| @page | 1 |
| @position-try | 1 |
| SVG <animatetransform> (G87) | 1 |
| declaration of -- | 1 |
| declaration of --prop | 1 |
| declaration of --var | 1 |
| declaration of --w | 1 |
| declaration of -moz-appearance | 1 |
| declaration of -webkit-appearance | 1 |
| declaration of -webkit-filter | 1 |
| declaration of -webkit-mask-image | 1 |
| declaration of -webkit-perspective-origin-y | 1 |
| declaration of -webkit-text-stroke-color | 1 |
| declaration of -webkit-writing-mode | 1 |
| declaration of align-items | 1 |
| declaration of anchor-name | 1 |
| declaration of animation-composition | 1 |
| declaration of animation-name | 1 |
| declaration of background-blend-mode | 1 |
| declaration of background-origin | 1 |
| declaration of baseline-source | 1 |
| declaration of border-block | 1 |
| declaration of border-inline | 1 |
| declaration of border-inline-start-color | 1 |
| declaration of border-shape | 1 |
| declaration of column-rule | 1 |
| declaration of column-rule-color | 1 |
| declaration of contain-intrinsic-size | 1 |
| declaration of content-visibility | 1 |
| declaration of cx | 1 |
| declaration of expando | 1 |
| declaration of field-sizing | 1 |
| declaration of flex-direction | 1 |
| declaration of flood-color | 1 |
| declaration of font-feature-settings | 1 |
| declaration of font-size-adjust | 1 |
| declaration of font-variant | 1 |
| declaration of font-variation-settings | 1 |
| declaration of foo | 1 |
| declaration of grid | 1 |
| declaration of grid-column-start | 1 |
| declaration of grid-row | 1 |
| declaration of grid-template-column | 1 |
| declaration of list-style | 1 |
| declaration of list-style-position | 1 |
| declaration of margin | 1 |
| declaration of margin-inline | 1 |
| declaration of marker | 1 |
| declaration of max-width | 1 |
| declaration of object-fit | 1 |
| declaration of offset-position | 1 |
| declaration of overflow-wrap | 1 |
| declaration of padding-block-end | 1 |
| declaration of path-length | 1 |
| declaration of perspective | 1 |
| declaration of scale | 1 |
| declaration of shape-outside | 1 |
| declaration of stroke-dasharray | 1 |
| declaration of text-align | 1 |
| declaration of text-autospace | 1 |
| declaration of text-decoration | 1 |
| declaration of text-decoration-skip | 1 |
| declaration of text-emphasis-color | 1 |
| declaration of top | 1 |
| declaration of transform-bxox | 1 |
| declaration of transition-delay | 1 |
| declaration of transition-duration | 1 |
| declaration of transition-properties | 1 |
| declaration of transition-timing-function | 1 |
| declaration of var-a | 1 |
| declaration of will-change | 1 |
| declaration of xcolor | 1 |
| element <frameset> | 1 |
| input type=color | 1 |
| input type=week | 1 |
| its frame's page asks a Python handler (delay.py): on the runner's server the request ends at once, before the navigation or after it by chance | 1 |
| its lazy image's second source, set in its load event, races the screenshot: it fails as M23 named it, and passed once in M33's gates | 1 |
| link to  | 1 |
| link to //[ | 1 |
| link to {{location[scheme]}}://{{location[host]}}/content-security-policy/support/checkReport.sub.js | 1 |
| missing file css/css-cascade/data:text/css,@layer foo;@media(all){} | 1 |
| missing file css/css-cascade/data:text/css,@layer{target{background-color:green !important}} | 1 |
| missing file css/css-cascade/data:text/css,@layer{target{background-color:green}} | 1 |
| missing file css/css-flexbox/resources/flexbox.css | 1 |
| missing file css/css-images/tools/REPLACEME_REFERENCE_FILENAME | 1 |
| missing file css/selectors/data:,a{ | 1 |
| missing file css/selectors/data:text/css,.a{--x:%22%EF%89%BB%22;}.b{} | 1 |
| missing file css/selectors/data:text/css,a{xyz | 1 |
| missing file dom/nodes/insertion-removing-steps/data:text/css,%23div{display:none} | 1 |
| missing file media-source/mediasource-util.js | 1 |
| missing file svg/painting/reftests/about:blank | 1 |
| missing file svg/styling/render/transform-origin-not-explicit-style-ref.svg | 1 |
| not well-formed XML: error on line 1 at column 1: Start tag expected, '<' not found | 1 |
| not well-formed XML: error on line 1 at column 1: StartTag: invalid element name | 1 |
| self-closing <rect/> in XHTML | 1 |
| self-closing <span/> in XHTML | 1 |
| two frames race for its one result, and a same-origin frame's location is unreadable here (G90): it passes or fails as their responses come | 1 |
| unchecked attribute data-bar | 1 |
| unchecked attribute data-crash | 1 |
| unchecked attribute data-e2 | 1 |
| unchecked attribute data-expect | 1 |
| unchecked attribute data-expect-match | 1 |
| unchecked attribute data-expected-left | 1 |
| unchecked attribute data-expected-padding-left | 1 |
| unchecked attribute data-expected-scroll-height | 1 |
| unchecked attribute data-expected-scroll-width | 1 |
| unchecked attribute data-in | 1 |
| unchecked attribute data-main | 1 |
| unchecked attribute data-natural-width | 1 |
| unchecked attribute data-sizes | 1 |
| unchecked attribute data-weapons | 1 |
| unchecked attribute data-width | 1 |
| unchecked attribute data-x | 1 |
| unchecked attribute data-yes | 1 |
| video (with media) | 1 |
