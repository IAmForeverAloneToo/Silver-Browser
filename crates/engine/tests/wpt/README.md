# web-platform-tests subset

The tests here are copied from
[web-platform-tests](https://github.com/web-platform-tests/wpt),
under its BSD licence (`LICENSE.md` beside this file), at commit

```text
dc7f6a3c0000e1da9e575ff4ef5618cff136697b (2026-10-03)
```

They are the tests of `css/css-flexbox`, `css/css-grid`, `css/css-transforms`, `css/css-will-change`, `css/mediaqueries`, `css/css-conditional`, `css/css-cascade`, `css/css-variables`, `css/css-values`, `css/css-lists`, `css/css-counter-styles`, `css/css-content`, `css/css-text-decor`, `css/css-text`, `css/css-fonts`, `css/CSS2/lists`, `css/CSS2/generated-content`, `css/css-animations`, `css/css-transitions`, `css/css-easing`, `css/css-images`, `css/css-backgrounds`, `css/css-color`, `css/css-color-adjust`, `css/css-display`, `css/css-sizing/aspect-ratio`, `css/filter-effects`, `html/semantics/embedded-content/the-img-element`, `css/selectors/invalidation`, `dom/nodes`, `html/dom`, `css/selectors`, `html/semantics/forms`, `svg/painting/reftests`, `svg/shapes/reftests`, `svg/struct/reftests`, `svg/pservers/reftests`, `svg/coordinate-systems`, `svg/styling`, `svg/text/reftests`, `svg/render/reftests`, `svg/as-image` and `svg/crashtests`
that use only what this engine has, chosen by the rule in
`examples/wpt.rs` and copied with their references, stylesheets
and images at their own paths by

```sh
cargo run -p engine --example wpt -- pull <checkout>
```

which rewrites them, `MANIFEST` and this file. `tests/wpt.rs` runs
them; `expectations.txt` names those expected to fail, and why.

With them are the testharness.js tests of `fetch/api/headers`, `fetch/api/request`, `fetch/api/response`, `fetch/api/basic`, `xhr`, `url`, `FileAPI/blob`, `webstorage`, `wasm/jsapi` (the `interface.*` tests), `wasm/jsapi` (the `prototypes.*` tests), `wasm/jsapi/constructor`, `wasm/jsapi/global`, `wasm/jsapi/instance`, `wasm/jsapi/memory`, `wasm/jsapi/module`, `wasm/jsapi/table`, `html/canvas/element`, `html/canvas/offscreen`, `workers`, `webmessaging`, `html/infrastructure/safe-passing-of-structured-data`, `html/webappapis/structured-clone`, `content-security-policy`, `html/browsers/sandboxing`, `referrer-policy`, `html/semantics/embedded-content/the-iframe-element`, `webaudio`, `html/semantics/embedded-content/media-elements`, `webgl`, `encoding`, `FileAPI/reading-data-section`, `FileAPI/FileReader`, `dom/nodes` (the `MutationObserver-*` tests), `intersection-observer`, `resize-observer`, `requestidlecallback`, `css/cssom`, `html/webappapis/user-prompts`, `html/browsers/the-window-object/named-access-on-the-window-object`, `css/css-color`, `css/css-color-adjust`, `css/css-backgrounds`, `css/filter-effects`, `css/css-values`, `css/css-images`, `css/css-display`, `css/css-sizing/aspect-ratio`, `html/syntax/parsing`, `dom/ranges`, `dom/traversal`, `selection`, `html/semantics/scripting-1`, `subresource-integrity`, `css/css-fonts`, `css/css-text`, `css/cssom-view` (M27, M29, M31, M40, M41, gap batches A and E)
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
| checklayout | 75 |
| crashtest | 121 |
| reftest | 3689 |
| testharness | 7310 |
| all | 11195 |

## Left out

The first reason the rule found for each test it left out, of
those on the checkout pulled from (a sparse one may hold only
the tests taken, of some directories):

| Reason | Tests |
| --- | ---: |
| not a testharness test | 4453 |
| @font-face | 1451 |
| script | 1131 |
| CDATA section in XHTML | 470 |
| Python handler | 334 |
| not a reftest, a check-layout test or a crashtest | 151 |
| wptserve pipe | 150 |
| the legacy CJK encodings (M42) | 141 |
| declaration of display | 134 |
| SVG <filter> (G87) | 113 |
| https: | 109 |
| a frame of another kind than iframe | 106 |
| declaration of backdrop-filter | 85 |
| a shared or service worker | 82 |
| wptserve substitution (.sub) | 51 |
| element <object> | 49 |
| declaration of hyphens | 47 |
| dir=rtl | 47 |
| element <video> | 47 |
| element <embed> | 46 |
| declaration of filter | 45 |
| declaration of background-image | 39 |
| element <iframe> | 38 |
| declaration of background-clip | 37 |
| declaration of columns | 35 |
| declaration of writing-mode | 32 |
| declaration of word-space-transform | 28 |
| declaration of text-autospace | 19 |
| declaration of background-color | 18 |
| declaration of tab-size | 18 |
| declaration of overflow-y | 17 |
| declaration of text-indent | 17 |
| @property | 16 |
| declaration of inline-size | 15 |
| declaration of line-break | 15 |
| declaration of width | 15 |
| declaration of contain | 14 |
| declaration of font-kerning | 14 |
| declaration of object-view-box | 14 |
| element <canvas> | 14 |
| declaration of word-break | 13 |
| declaration of direction | 12 |
| missing file resources/WebIDLParser.js | 12 |
| a module worker (G41) | 11 |
| declaration of background | 11 |
| a window | 10 |
| declaration of height | 10 |
| declaration of text-transform | 10 |
| not for a window or a dedicated worker | 10 |
| declaration of font-synthesis | 9 |
| declaration of text-wrap-style | 8 |
| unchecked attribute data-foo | 8 |
| declaration of zoom | 7 |
| declaration of color | 6 |
| declaration of min-width | 6 |
| declaration of font-size | 5 |
| declaration of font-size-adjust | 5 |
| declaration of font-variation-settings | 5 |
| declaration of hanging-punctuation | 5 |
| declaration of mask-image | 5 |
| missing file service-workers/service-worker/resources/test-helpers.sub.js | 5 |
| selector | 5 |
| declaration of border-image | 4 |
| declaration of container-type | 4 |
| declaration of min-height | 4 |
| declaration of overflow-x | 4 |
| declaration of scrollbar-gutter | 4 |
| element <hr> | 4 |
| @font-feature-values | 3 |
| SVG <foreignobject> (G87) | 3 |
| SVG <image> (G87) | 3 |
| declaration of background-size | 3 |
| declaration of border-radius | 3 |
| declaration of box-shadow | 3 |
| declaration of clip-path | 3 |
| declaration of content | 3 |
| declaration of mask | 3 |
| declaration of scrollbar-width | 3 |
| declaration of text-fit | 3 |
| declaration of text-orientation | 3 |
| element <fieldset> | 3 |
| missing file css/css-conditional/container-queries/support/cq-testcommon.js | 3 |
| unchecked attribute data-color | 3 |
| @font-palette-values | 2 |
| SVG <mask> (G87) | 2 |
| SharedArrayBuffer (G83) | 2 |
| declaration of all | 2 |
| declaration of image-rendering | 2 |
| declaration of letter-spacing | 2 |
| declaration of line-height | 2 |
| declaration of text-overflow | 2 |
| declaration of unicode-bidi | 2 |
| declaration of word-spacing | 2 |
| declaration of z-index | 2 |
| img srcset | 2 |
| link to http://{{domains[www2]}}:{{ports[http][0]}}/html/semantics/scripting-1/the-script-element/module/crossorigin-scripterror.js | 2 |
| missing file css/mediaqueries/resources/matchmedia-utils.js | 2 |
| unchecked attribute data-expected | 2 |
| @color-profile | 1 |
| @container | 1 |
| @counter-style | 1 |
| @namespace | 1 |
| @page | 1 |
| SVG <pattern> (G87) | 1 |
| declaration of --prop | 1 |
| declaration of --w | 1 |
| declaration of -moz-tab-size | 1 |
| declaration of -webkit-mask-image | 1 |
| declaration of -webkit-perspective-origin-y | 1 |
| declaration of -webkit-text-stroke-width | 1 |
| declaration of background-blend-mode | 1 |
| declaration of background-origin | 1 |
| declaration of block-size | 1 |
| declaration of border-shape | 1 |
| declaration of box-decoration-break | 1 |
| declaration of column-rule-color | 1 |
| declaration of contain-intrinsic-size | 1 |
| declaration of fill | 1 |
| declaration of float | 1 |
| declaration of font-family | 1 |
| declaration of font-style | 1 |
| declaration of font-variant | 1 |
| declaration of font-variant-caps | 1 |
| declaration of font-variant-emoji | 1 |
| declaration of font-variant-ligatures | 1 |
| declaration of font-weight | 1 |
| declaration of grid-column-start | 1 |
| declaration of hyphenate-character | 1 |
| declaration of inset | 1 |
| declaration of isolation | 1 |
| declaration of line-clamp | 1 |
| declaration of list-style-image | 1 |
| declaration of margin | 1 |
| declaration of margin-inline | 1 |
| declaration of margin-inline-start | 1 |
| declaration of max-inline-size | 1 |
| declaration of max-width | 1 |
| declaration of mix-blend-mode | 1 |
| declaration of object-fit | 1 |
| declaration of offset-position | 1 |
| declaration of overflow | 1 |
| declaration of padding-left | 1 |
| declaration of position-area | 1 |
| declaration of rotate | 1 |
| declaration of scale | 1 |
| declaration of stroke-dasharray | 1 |
| declaration of text-align | 1 |
| declaration of text-emphasis | 1 |
| declaration of text-size-adjust | 1 |
| declaration of top | 1 |
| declaration of transform | 1 |
| declaration of transition-delay | 1 |
| declaration of translate | 1 |
| declaration of view-transition-name | 1 |
| declaration of white-space | 1 |
| declaration of word-space-transfor | 1 |
| element <audio> | 1 |
| element <details> | 1 |
| element <dialog> | 1 |
| element <marquee> | 1 |
| element <picture> | 1 |
| external SVG reference (G87) | 1 |
| link to  | 1 |
| missing file css/css-images/tools/REPLACEME_REFERENCE_FILENAME | 1 |
| missing file css/css-text/interpolation/resources/interpolation-test.js | 1 |
| missing file css/css-transitions/support/helper.js | 1 |
| missing file html/dom/original-harness.js | 1 |
| missing file html/semantics/scripting-1/the-script-element/data:,throw 1 | 1 |
| missing file html/semantics/scripting-1/the-script-element/module/no-such-file.js | 1 |
| missing file html/syntax/parsing/data:text/javascript,order.push('defer');t.step_timeout(()=>{order.push('timeout');check()},0) | 1 |
| missing file media-source/mediasource-util.js | 1 |
| missing file wai-aria/scripts/aria-utils.js | 1 |
| missing file web-animations/testcommon.js | 1 |
| namespaces in XHTML | 1 |
| nested rule | 1 |
| not well-formed XML: error on line 1 at column 1: Start tag expected, '<' not found | 1 |
| unchecked attribute data-bar | 1 |
| unchecked attribute data-crash | 1 |
| unchecked attribute data-name | 1 |
| unchecked attribute data-x | 1 |
