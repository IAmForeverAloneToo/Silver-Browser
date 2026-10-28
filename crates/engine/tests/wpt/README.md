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

With them are the testharness.js tests of `fetch/api/headers`, `fetch/api/request`, `fetch/api/response`, `fetch/api/basic`, `xhr`, `url`, `FileAPI/blob`, `webstorage`, `wasm/jsapi` (the `interface.*` tests), `wasm/jsapi` (the `prototypes.*` tests), `wasm/jsapi/constructor`, `wasm/jsapi/global`, `wasm/jsapi/instance`, `wasm/jsapi/memory`, `wasm/jsapi/module`, `wasm/jsapi/table`, `html/canvas/element`, `html/canvas/offscreen`, `workers`, `webmessaging`, `html/infrastructure/safe-passing-of-structured-data`, `html/webappapis/structured-clone`, `content-security-policy`, `html/browsers/sandboxing`, `referrer-policy`, `html/semantics/embedded-content/the-iframe-element`, `webaudio`, `html/semantics/embedded-content/media-elements`, `webgl`, `encoding`, `FileAPI/reading-data-section`, `FileAPI/FileReader`, `dom/nodes` (the `MutationObserver-*` tests), `intersection-observer`, `resize-observer`, `requestidlecallback`, `css/cssom`, `html/webappapis/user-prompts`, `html/browsers/the-window-object/named-access-on-the-window-object` (M27, M29, M31, M40, M41, gap batch A)
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
| testharness | 5315 |
| all | 8395 |

## Left out

The first reason the rule found for each test it left out, of
those on the checkout pulled from (a sparse one may hold only
the tests taken, of some directories):

| Reason | Tests |
| --- | ---: |
| Python handler | 185 |
| the legacy CJK encodings (M42) | 141 |
| https: | 98 |
| a shared or service worker | 80 |
| not a testharness test | 71 |
| wptserve pipe | 55 |
| not a reftest, a check-layout test or a crashtest | 32 |
| script | 13 |
| a module worker (G41) | 11 |
| a window | 9 |
| missing file resources/WebIDLParser.js | 7 |
| missing file service-workers/service-worker/resources/test-helpers.sub.js | 5 |
| not for a window or a dedicated worker | 3 |
| SharedArrayBuffer (G83) | 2 |
| a frame of another kind than iframe | 2 |
| CDATA section in XHTML | 1 |
| element <iframe> | 1 |
| missing file media-source/mediasource-util.js | 1 |
| not well-formed XML: error on line 1 at column 1: Start tag expected, '<' not found | 1 |
