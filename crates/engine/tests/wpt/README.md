# web-platform-tests subset

The tests here are copied from
[web-platform-tests](https://github.com/web-platform-tests/wpt),
under its BSD licence (`LICENSE.md` beside this file), at commit

```text
dc7f6a3c0000e1da9e575ff4ef5618cff136697b (2026-10-03)
```

They are the tests of `css/css-flexbox`, `css/css-grid`, `css/css-transforms`, `css/css-will-change`, `css/mediaqueries`, `css/css-conditional`, `css/css-cascade`, `css/css-variables`, `css/css-values`, `css/css-lists`, `css/css-counter-styles`, `css/css-content`, `css/css-text-decor`, `css/css-text/text-align`, `css/css-text/text-justify`, `css/CSS2/lists`, `css/CSS2/generated-content`, `css/css-animations`, `css/css-transitions`, `css/css-easing`, `css/css-images`, `css/css-backgrounds`, `css/css-color`, `css/css-color-adjust`, `css/css-display`, `css/css-sizing/aspect-ratio`, `css/filter-effects`, `html/semantics/embedded-content/the-img-element`, `css/selectors/invalidation`, `dom/nodes`, `html/dom`, `css/selectors`, `html/semantics/forms`, `svg/painting/reftests`, `svg/shapes/reftests`, `svg/struct/reftests`, `svg/pservers/reftests`, `svg/coordinate-systems`, `svg/styling`, `svg/text/reftests`, `svg/render/reftests`, `svg/as-image` and `svg/crashtests`
that use only what this engine has, chosen by the rule in
`examples/wpt.rs` and copied with their references, stylesheets
and images at their own paths by

```sh
cargo run -p engine --example wpt -- pull <checkout>
```

which rewrites them, `MANIFEST` and this file. `tests/wpt.rs` runs
them; `expectations.txt` names those expected to fail, and why.

With them are the testharness.js tests of `fetch/api/headers`, `fetch/api/request`, `fetch/api/response`, `fetch/api/basic`, `xhr`, `url`, `FileAPI/blob`, `webstorage`, `wasm/jsapi` (the `interface.*` tests), `wasm/jsapi` (the `prototypes.*` tests), `wasm/jsapi/constructor`, `wasm/jsapi/global`, `wasm/jsapi/instance`, `wasm/jsapi/memory`, `wasm/jsapi/module`, `wasm/jsapi/table`, `html/canvas/element`, `html/canvas/offscreen`, `workers`, `webmessaging`, `html/infrastructure/safe-passing-of-structured-data`, `html/webappapis/structured-clone`, `content-security-policy`, `html/browsers/sandboxing`, `referrer-policy`, `html/semantics/embedded-content/the-iframe-element`, `webaudio`, `html/semantics/embedded-content/media-elements`, `webgl`, `encoding`, `FileAPI/reading-data-section`, `FileAPI/FileReader`, `dom/nodes` (the `MutationObserver-*` tests), `intersection-observer`, `resize-observer`, `requestidlecallback`, `css/cssom`, `html/webappapis/user-prompts`, `html/browsers/the-window-object/named-access-on-the-window-object`, `css/css-color`, `css/css-color-adjust`, `css/css-backgrounds`, `css/filter-effects`, `css/css-values`, `css/css-images`, `css/css-display`, `css/css-sizing/aspect-ratio` (M27, M29, M31, M40, M41, gap batches A and E)
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
| checklayout | 73 |
| crashtest | 111 |
| reftest | 3321 |
| testharness | 5926 |
| all | 9431 |

## Left out

The first reason the rule found for each test it left out, of
those on the checkout pulled from (a sparse one may hold only
the tests taken, of some directories):

| Reason | Tests |
| --- | ---: |
| not a testharness test | 2435 |
| script | 661 |
| CDATA section in XHTML | 449 |
| Python handler | 190 |
| the legacy CJK encodings (M42) | 141 |
| declaration of display | 125 |
| not a reftest, a check-layout test or a crashtest | 118 |
| SVG <filter> (G87) | 111 |
| a frame of another kind than iframe | 104 |
| https: | 98 |
| element <br> | 94 |
| a shared or service worker | 80 |
| wptserve pipe | 78 |
| declaration of backdrop-filter | 77 |
| missing file fonts/ahem.css | 65 |
| wptserve substitution (.sub) | 51 |
| element <object> | 49 |
| element <video> | 47 |
| element <embed> | 46 |
| declaration of filter | 44 |
| element <iframe> | 36 |
| declaration of background-image | 35 |
| declaration of columns | 34 |
| declaration of line-height | 30 |
| declaration of background-clip | 29 |
| declaration of background-color | 25 |
| declaration of writing-mode | 24 |
| @font-face | 23 |
| declaration of overflow-y | 17 |
| @property | 16 |
| declaration of contain | 14 |
| declaration of object-view-box | 14 |
| element <canvas> | 14 |
| declaration of background | 12 |
| declaration of width | 12 |
| a module worker (G41) | 11 |
| declaration of font | 10 |
| declaration of height | 10 |
| a window | 9 |
| missing file resources/WebIDLParser.js | 9 |
| declaration of font-kerning | 8 |
| unchecked attribute data-foo | 8 |
| declaration of color | 6 |
| declaration of font-weight | 6 |
| declaration of min-width | 6 |
| declaration of zoom | 5 |
| missing file service-workers/service-worker/resources/test-helpers.sub.js | 5 |
| selector | 5 |
| declaration of border-image | 4 |
| declaration of container-type | 4 |
| declaration of inline-size | 4 |
| declaration of min-height | 4 |
| declaration of overflow-x | 4 |
| declaration of scrollbar-gutter | 4 |
| declaration of z-index | 4 |
| SVG <image> (G87) | 3 |
| declaration of background-size | 3 |
| declaration of border-radius | 3 |
| declaration of box-shadow | 3 |
| declaration of clip-path | 3 |
| declaration of content | 3 |
| declaration of direction | 3 |
| declaration of mask | 3 |
| declaration of mask-image | 3 |
| declaration of scrollbar-width | 3 |
| declaration of text-orientation | 3 |
| dir=rtl | 3 |
| element <fieldset> | 3 |
| missing file css/css-conditional/container-queries/support/cq-testcommon.js | 3 |
| not for a window or a dedicated worker | 3 |
| SVG <foreignobject> (G87) | 2 |
| SVG <mask> (G87) | 2 |
| SharedArrayBuffer (G83) | 2 |
| declaration of all | 2 |
| declaration of font-variant | 2 |
| declaration of image-rendering | 2 |
| declaration of text-indent | 2 |
| declaration of visibility | 2 |
| img srcset | 2 |
| missing file css/mediaqueries/resources/matchmedia-utils.js | 2 |
| unchecked attribute data-color | 2 |
| unchecked attribute data-expected | 2 |
| @color-profile | 1 |
| @container | 1 |
| @counter-style | 1 |
| @namespace | 1 |
| @page | 1 |
| SVG <pattern> (G87) | 1 |
| declaration of --prop | 1 |
| declaration of --w | 1 |
| declaration of -webkit-mask-image | 1 |
| declaration of -webkit-perspective-origin-y | 1 |
| declaration of background-blend-mode | 1 |
| declaration of background-origin | 1 |
| declaration of border-shape | 1 |
| declaration of contain-intrinsic-size | 1 |
| declaration of fill | 1 |
| declaration of font-feature-settings | 1 |
| declaration of font-style | 1 |
| declaration of font-variation-settings | 1 |
| declaration of grid-column-start | 1 |
| declaration of inset | 1 |
| declaration of list-style-image | 1 |
| declaration of margin-inline | 1 |
| declaration of max-inline-size | 1 |
| declaration of max-width | 1 |
| declaration of mix-blend-mode | 1 |
| declaration of object-fit | 1 |
| declaration of offset-position | 1 |
| declaration of position-area | 1 |
| declaration of rotate | 1 |
| declaration of scale | 1 |
| declaration of stroke-dasharray | 1 |
| declaration of text-emphasis | 1 |
| declaration of text-overflow | 1 |
| declaration of text-shadow | 1 |
| declaration of text-size-adjust | 1 |
| declaration of top | 1 |
| declaration of transform | 1 |
| declaration of transition-delay | 1 |
| declaration of translate | 1 |
| declaration of view-transition-name | 1 |
| declaration of white-space | 1 |
| element <audio> | 1 |
| element <details> | 1 |
| element <dialog> | 1 |
| element <hr> | 1 |
| element <marquee> | 1 |
| element <picture> | 1 |
| external SVG reference (G87) | 1 |
| link to  | 1 |
| missing file css/css-images/tools/REPLACEME_REFERENCE_FILENAME | 1 |
| missing file css/css-transitions/support/helper.js | 1 |
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
