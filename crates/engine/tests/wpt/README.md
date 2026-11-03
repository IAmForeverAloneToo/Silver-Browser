# web-platform-tests subset

The tests here are copied from
[web-platform-tests](https://github.com/web-platform-tests/wpt),
under its BSD licence (`LICENSE.md` beside this file), at commit

```text
dc7f6a3c0000e1da9e575ff4ef5618cff136697b (2026-10-03)
```

They are the tests of `css/css-flexbox`, `css/css-grid`, `css/css-transforms`, `css/css-will-change`, `css/mediaqueries`, `css/css-conditional`, `css/css-cascade`, `css/css-variables`, `css/css-values`, `css/css-lists`, `css/css-counter-styles`, `css/css-content`, `css/css-text-decor`, `css/css-text`, `css/css-fonts`, `css/CSS2/lists`, `css/CSS2/generated-content`, `css/css-animations`, `css/css-transitions`, `css/css-easing`, `css/css-images`, `css/css-backgrounds`, `css/css-color`, `css/css-color-adjust`, `css/css-display`, `css/css-sizing/aspect-ratio`, `css/filter-effects`, `html/semantics/embedded-content/the-img-element`, `css/selectors/invalidation`, `dom/nodes`, `html/dom`, `css/selectors`, `html/semantics/forms`, `svg/painting/reftests`, `svg/shapes/reftests`, `svg/struct/reftests`, `svg/pservers/reftests`, `svg/coordinate-systems`, `svg/styling`, `svg/text/reftests`, `svg/render/reftests`, `svg/as-image`, `svg/crashtests`, `shadow-dom`, `custom-elements`, `css/css-nesting`, `css/css-logical`, `css/css-properties-values-api`, `css/css-env`, `web-animations`, `scroll-animations`, `html/semantics/interactive-elements`, `html/semantics/popovers`, `html/semantics/the-button-element`, `css/css-highlight-api`, `css/css-contain`, `css/css-scroll-snap`, `css/css-position`, `html/rendering` and `css/css-pseudo`
that use only what this engine has, chosen by the rule in
`examples/wpt.rs` and copied with their references, stylesheets
and images at their own paths by

```sh
cargo run -p engine --example wpt -- pull <checkout>
```

which rewrites them, `MANIFEST` and this file. `tests/wpt.rs` runs
them; `expectations.txt` names those expected to fail, and why.

With them are the testharness.js tests of `fetch/api/headers`, `fetch/api/request`, `fetch/api/response`, `fetch/api/basic`, `xhr`, `url`, `FileAPI/blob`, `webstorage`, `wasm/jsapi` (the `interface.*` tests), `wasm/jsapi` (the `prototypes.*` tests), `wasm/jsapi/constructor`, `wasm/jsapi/global`, `wasm/jsapi/instance`, `wasm/jsapi/memory`, `wasm/jsapi/module`, `wasm/jsapi/table`, `html/canvas/element`, `html/canvas/offscreen`, `workers`, `webmessaging`, `html/infrastructure/safe-passing-of-structured-data`, `html/webappapis/structured-clone`, `content-security-policy`, `html/browsers/sandboxing`, `referrer-policy`, `html/semantics/embedded-content/the-iframe-element`, `webaudio`, `html/semantics/embedded-content/media-elements`, `webgl`, `encoding`, `FileAPI/reading-data-section`, `FileAPI/FileReader`, `dom/nodes` (the `MutationObserver-*` tests), `intersection-observer`, `resize-observer`, `requestidlecallback`, `css/cssom`, `html/webappapis/user-prompts`, `html/browsers/the-window-object/named-access-on-the-window-object`, `css/css-color`, `css/css-color-adjust`, `css/css-backgrounds`, `css/filter-effects`, `css/css-values`, `css/css-images`, `css/css-display`, `css/css-sizing/aspect-ratio`, `html/syntax/parsing`, `dom/ranges`, `dom/traversal`, `selection`, `html/semantics/scripting-1`, `subresource-integrity`, `css/css-fonts`, `css/css-text`, `css/cssom-view`, `shadow-dom`, `custom-elements`, `domparsing`, `css/css-nesting`, `css/css-conditional`, `css/css-cascade`, `css/css-logical`, `css/css-properties-values-api`, `css/css-env`, `css/selectors/invalidation`, `css/css-animations`, `css/css-transitions`, `web-animations`, `scroll-animations`, `html/semantics/interactive-elements`, `html/semantics/popovers`, `html/semantics/the-button-element`, `close-watcher`, `css/css-highlight-api`, `css/css-contain`, `html/interaction/focus`, `css/css-scroll-snap`, `css/css-position`, `html/rendering`, `css/css-pseudo` (M27, M29, M31, M40, M41, gap batches A and E)
that the runner's one server can serve: no Python handler, no `.sub`
template, no other host, nothing over HTTP/2, no shared or
service worker and no module worker. `resources/testharness.js` is
the real one; the runner serves its own `testharnessreport.js`, and
the pages wptserve makes of an `.any.js` test (for a window and,
as its `global` says, a dedicated worker), a `.window.js` test or a
`.worker.js` test, and an `.any.js` test's worker script. Each
subtest they fail is named in `expectations.txt`, and why.

## Taken

| Kind | Tests |
| --- | ---: |
| checklayout | 78 |
| crashtest | 162 |
| reftest | 4572 |
| testharness | 10543 |
| all | 15355 |

## Left out

The first reason the rule found for each test it left out, of
those on the checkout pulled from (a sparse one may hold only
the tests taken, of some directories):

| Reason | Tests |
| --- | ---: |
| not a testharness test | 4473 |
| script | 3892 |
| @font-face | 1603 |
| Python handler | 370 |
| declaration of contain | 307 |
| selector | 231 |
| declaration of display | 208 |
| not a reftest, a check-layout test or a crashtest | 184 |
| declaration of content-visibility | 161 |
| wptserve pipe | 157 |
| a frame of another kind than iframe | 142 |
| the legacy CJK encodings (M42) | 141 |
| element <iframe> | 127 |
| SVG <filter> (G87) | 113 |
| declaration of writing-mode | 95 |
| a shared or service worker | 91 |
| declaration of scroll-snap-type | 91 |
| declaration of backdrop-filter | 86 |
| element <fieldset> | 77 |
| element <dialog> | 70 |
| dir=rtl | 64 |
| wptserve substitution (.sub) | 61 |
| declaration of columns | 59 |
| element <object> | 58 |
| @container | 56 |
| element <video> | 56 |
| nested rule | 54 |
| element <embed> | 49 |
| declaration of filter | 47 |
| @property | 42 |
| element <details> | 39 |
| declaration of background-clip | 37 |
| declaration of background-image | 31 |
| declaration of scrollbar-width | 28 |
| a window | 26 |
| declaration of appearance | 26 |
| element <hr> | 26 |
| element <canvas> | 25 |
| missing file resources/WebIDLParser.js | 23 |
| @scope | 22 |
| SharedArrayBuffer (G83) | 14 |
| declaration of object-view-box | 14 |
| declaration of width | 12 |
| a module worker (G41) | 11 |
| declaration of background | 11 |
| declaration of color | 11 |
| declaration of column-count | 11 |
| declaration of zoom | 11 |
| missing file wai-aria/scripts/aria-utils.js | 11 |
| declaration of height | 10 |
| declaration of text-overflow | 10 |
| not for a window or a dedicated worker | 10 |
| declaration of font-synthesis | 9 |
| declaration of line-clamp | 9 |
| declaration of scrollbar-gutter | 9 |
| declaration of unicode-bidi | 9 |
| missing file trusted-types/support/helper.sub.js | 9 |
| declaration of field-sizing | 8 |
| input type=date | 8 |
| unchecked attribute data-foo | 8 |
| declaration of font-size-adjust | 7 |
| declaration of scroll-initial-target | 7 |
| element <audio> | 7 |
| input type=image | 7 |
| declaration of contain-intrinsic-size | 6 |
| declaration of mask-image | 6 |
| declaration of min-width | 6 |
| element <legend> | 6 |
| element <marquee> | 6 |
| input type=file | 6 |
| declaration of -webkit-appearance | 5 |
| declaration of clip-path | 5 |
| declaration of column-rule | 5 |
| declaration of column-width | 5 |
| declaration of font-variation-settings | 5 |
| element <frameset> | 5 |
| element <summary> | 5 |
| missing file permissions-policy/resources/permissions-policy.js | 5 |
| missing file service-workers/service-worker/resources/test-helpers.sub.js | 5 |
| SVG <foreignobject> (G87) | 4 |
| declaration of -webkit-locale | 4 |
| declaration of border-image | 4 |
| declaration of min-height | 4 |
| declaration of scroll-snap-align | 4 |
| declaration of text-decoration-skip-ink | 4 |
| declaration of text-orientation | 4 |
| missing file scroll-animations/view-timelines/support/testcommon.js | 4 |
| unchecked attribute data-color | 4 |
| unchecked attribute data-expected | 4 |
| @font-feature-values | 3 |
| @namespace | 3 |
| @starting-style | 3 |
| SVG <image> (G87) | 3 |
| declaration of accent-color | 3 |
| declaration of border-radius | 3 |
| declaration of content | 3 |
| declaration of mask | 3 |
| declaration of mix-blend-mode | 3 |
| declaration of overlay | 3 |
| declaration of position-area | 3 |
| declaration of top | 3 |
| declaration of z-index | 3 |
| element <math> | 3 |
| element <meter> | 3 |
| element <progress> | 3 |
| missing file scroll-animations/css/testcommon.js | 3 |
| @counter-style | 2 |
| @font-palette-values | 2 |
| SVG <mask> (G87) | 2 |
| declaration of -webkit-text-stroke-width | 2 |
| declaration of background-color | 2 |
| declaration of box-shadow | 2 |
| declaration of caret-animation | 2 |
| declaration of caret-color | 2 |
| declaration of column-rule-color | 2 |
| declaration of font-family | 2 |
| declaration of foo | 2 |
| declaration of grid-column-end | 2 |
| declaration of image-rendering | 2 |
| declaration of scroll-start | 2 |
| declaration of text-size-adjust | 2 |
| declaration of view-transition-name | 2 |
| element <output> | 2 |
| element <picture> | 2 |
| img srcset | 2 |
| input type=color | 2 |
| link to http://{{domains[www2]}}:{{ports[http][0]}}/html/semantics/scripting-1/the-script-element/module/crossorigin-scripterror.js | 2 |
| missing file css/css-typed-om/resources/testhelper.js | 2 |
| missing file css/mediaqueries/resources/matchmedia-utils.js | 2 |
| not well-formed XML: error on line 1 at column 1: Start tag expected, '<' not found | 2 |
| not well-formed XML: error on line 17 at column 1: Premature end of data in tag body | 2 |
| unchecked attribute data-affiliation | 2 |
| unchecked attribute data-name | 2 |
| unchecked attribute data-section-title | 2 |
| @color-profile | 1 |
| @page | 1 |
| @position-try | 1 |
| SVG <marker> (G87) | 1 |
| SVG <pattern> (G87) | 1 |
| SVG <textpath> (G87) | 1 |
| declaration of --prop | 1 |
| declaration of --w | 1 |
| declaration of -moz-tab-size | 1 |
| declaration of -webkit-filter | 1 |
| declaration of -webkit-mask-image | 1 |
| declaration of -webkit-perspective-origin-y | 1 |
| declaration of -webkit-text-emphasis | 1 |
| declaration of -webkit-writing-mode | 1 |
| declaration of anchor-name | 1 |
| declaration of animation-name | 1 |
| declaration of animation-range-start | 1 |
| declaration of background-blend-mode | 1 |
| declaration of background-origin | 1 |
| declaration of background-size | 1 |
| declaration of border-bottom-left-radius | 1 |
| declaration of border-bottom-right-radius | 1 |
| declaration of border-shape | 1 |
| declaration of border-top-left-radius | 1 |
| declaration of border-top-right-radius | 1 |
| declaration of box-decoration-break | 1 |
| declaration of clip | 1 |
| declaration of column-fill | 1 |
| declaration of fill | 1 |
| declaration of flood-color | 1 |
| declaration of font-style | 1 |
| declaration of font-weight | 1 |
| declaration of grid | 1 |
| declaration of grid-column-start | 1 |
| declaration of grid-template | 1 |
| declaration of grid-template-rows | 1 |
| declaration of initial-letter | 1 |
| declaration of isolation | 1 |
| declaration of line-height | 1 |
| declaration of list-style-image | 1 |
| declaration of margin | 1 |
| declaration of marker | 1 |
| declaration of max-width | 1 |
| declaration of object-fit | 1 |
| declaration of offset-position | 1 |
| declaration of overflow-wrap | 1 |
| declaration of pointer-events | 1 |
| declaration of rotate | 1 |
| declaration of scale | 1 |
| declaration of scroll-marker-group | 1 |
| declaration of scroll-snap-stop | 1 |
| declaration of scrollbar-color | 1 |
| declaration of shape-outside | 1 |
| declaration of stroke | 1 |
| declaration of stroke-dasharray | 1 |
| declaration of stroke-width | 1 |
| declaration of tab-size | 1 |
| declaration of text-align | 1 |
| declaration of text-box | 1 |
| declaration of text-emphasis | 1 |
| declaration of transform | 1 |
| declaration of transition-delay | 1 |
| declaration of transition-duration | 1 |
| declaration of transition-properties | 1 |
| declaration of translate | 1 |
| declaration of white-space | 1 |
| declaration of white-space-collapse | 1 |
| declaration of white-space-trim | 1 |
| declaration of word-space-transfor | 1 |
| external SVG reference (G87) | 1 |
| input type=time | 1 |
| link out of the checkout: ../../../../../images/blue.png | 1 |
| link to  | 1 |
| missing file css/CSS2/reference/ref-filled-green-100px-square.xht | 1 |
| missing file css/css-cascade/data:text/css,@layer foo;@media(all){} | 1 |
| missing file css/css-cascade/data:text/css,@layer{target{background-color:green !important}} | 1 |
| missing file css/css-cascade/data:text/css,@layer{target{background-color:green}} | 1 |
| missing file css/css-contain/content-visibility/support/helper.js | 1 |
| missing file css/css-easing/testcommon.js | 1 |
| missing file css/css-images/tools/REPLACEME_REFERENCE_FILENAME | 1 |
| missing file css/css-scroll-snap/snap-events/resources/programmatic-scroll-common.js | 1 |
| missing file css/css-text/interpolation/resources/interpolation-test.js | 1 |
| missing file html/browsers/browsing-the-web/resources/helpers.js | 1 |
| missing file html/dom/original-harness.js | 1 |
| missing file html/semantics/forms/form-submission-0/resources/targetted-form.js | 1 |
| missing file html/semantics/interestfor/resources/invoker-utils.js | 1 |
| missing file html/semantics/scripting-1/the-script-element/data:,throw 1 | 1 |
| missing file html/semantics/scripting-1/the-script-element/module/no-such-file.js | 1 |
| missing file html/syntax/parsing/data:text/javascript,order.push('defer');t.step_timeout(()=>{order.push('timeout');check()},0) | 1 |
| missing file media-source/mediasource-util.js | 1 |
| missing file resources/SVGAnimationTestCase-testharness.js | 1 |
| missing file resources/accesskey.js | 1 |
| missing file scroll-animations/scroll-timelines/support/testcommon.js | 1 |
| missing file scroll-to-text-fragment/stash.js | 1 |
| missing file shadow-dom/focus/resources/shadow-dom.js | 1 |
| missing file web-animations/animation-model/keyframe-effects/support/testcommon.js | 1 |
| missing file web-animations/interfaces/css-animations/support/testcommon.js | 1 |
| unchecked attribute data-bar | 1 |
| unchecked attribute data-crash | 1 |
| unchecked attribute data-expect | 1 |
| unchecked attribute data-expected-scroll-height | 1 |
| unchecked attribute data-expected-scroll-width | 1 |
| unchecked attribute data-x | 1 |
