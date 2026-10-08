# web-platform-tests subset

The tests here are copied from
[web-platform-tests](https://github.com/web-platform-tests/wpt),
under its BSD licence (`LICENSE.md` beside this file), at commit

```text
dc7f6a3c0000e1da9e575ff4ef5618cff136697b (2026-10-03)
```

They are the tests of `css/css-flexbox`, `css/css-grid`, `css/css-transforms`, `css/css-will-change`, `css/mediaqueries`, `css/css-conditional`, `css/css-cascade`, `css/css-variables`, `css/css-values`, `css/css-env`, `css/css-lists`, `css/css-counter-styles`, `css/css-content`, `css/css-text-decor`, `css/css-text/text-align`, `css/css-text/text-justify`, `css/CSS2/lists`, `css/CSS2/generated-content`, `css/css-animations`, `css/css-transitions`, `css/css-easing`, `css/css-images`, `css/css-backgrounds` and `html/semantics/embedded-content/the-img-element`
that use only what this engine has, chosen by the rule in
`examples/wpt.rs` and copied with their references, stylesheets
and images at their own paths by

```sh
cargo run -p engine --example wpt -- pull <checkout>
```

which rewrites them, `MANIFEST` and this file. `tests/wpt.rs` runs
them; `expectations.txt` names those expected to fail, and why.

## Taken

| Kind | Tests |
| --- | ---: |
| checklayout | 51 |
| crashtest | 51 |
| reftest | 2038 |
| all | 2140 |

## Left out

The first reason the rule found for each test it left out:

| Reason | Tests |
| --- | ---: |
| script | 1749 |
| @font-face | 836 |
| declaration of font | 705 |
| declaration of display | 495 |
| CDATA section in XHTML | 488 |
| element <svg> | 333 |
| SVG image | 304 |
| script (reftest-wait) | 255 |
| not a reftest, a check-layout test or a crashtest | 237 |
| declaration of line-height | 227 |
| declaration of writing-mode | 205 |
| element <br> | 126 |
| declaration of direction | 86 |
| declaration of white-space | 77 |
| declaration of border-radius | 74 |
| declaration of container-type | 71 |
| declaration of background-image | 55 |
| wptserve substitution (.sub) | 51 |
| element <iframe> | 50 |
| element <canvas> | 49 |
| element <object> | 46 |
| element <video> | 46 |
| declaration of background | 45 |
| element <embed> | 44 |
| declaration of font-variant-numeric | 43 |
| declaration of outline | 43 |
| self-closing <div/> in XHTML | 40 |
| declaration of background-color | 39 |
| declaration of flex-wrap | 38 |
| dir=rtl | 34 |
| element <input> | 34 |
| @counter-style | 33 |
| declaration of visibility | 29 |
| declaration of border | 27 |
| declaration of transform | 27 |
| @property | 26 |
| declaration of box-shadow | 26 |
| declaration of grid-template-rows | 26 |
| declaration of inline-size | 26 |
| declaration of background-clip | 25 |
| declaration of color | 24 |
| selector | 24 |
| @scope | 23 |
| nested rule | 23 |
| @container | 22 |
| declaration of grid-template-columns | 21 |
| declaration of width | 19 |
| declaration of overflow-y | 16 |
| declaration of text-shadow | 16 |
| declaration of contain | 15 |
| declaration of border-top-left-radius | 14 |
| declaration of block-size | 13 |
| declaration of fill | 13 |
| declaration of object-view-box | 13 |
| declaration of height | 12 |
| declaration of text-decoration-inset | 12 |
| declaration of align-self | 11 |
| declaration of columns | 11 |
| declaration of text-emphasis | 11 |
| declaration of all | 10 |
| declaration of column-count | 10 |
| declaration of overflow-x | 10 |
| declaration of text-decoration-skip-spaces | 10 |
| @namespace | 9 |
| declaration of text-decoration-skip-ink | 9 |
| unchecked attribute data-foo | 9 |
| declaration of filter | 8 |
| img srcset | 8 |
| declaration of break-after | 7 |
| declaration of grid-template | 7 |
| element <button> | 7 |
| declaration of flex | 6 |
| declaration of font-weight | 6 |
| declaration of min-width | 6 |
| declaration of zoom | 6 |
| element <fieldset> | 6 |
| element <picture> | 6 |
| unchecked attribute data-val | 6 |
| declaration of --a | 5 |
| declaration of border-top-right-radius | 5 |
| declaration of break-before | 5 |
| declaration of inset | 5 |
| declaration of overflow | 5 |
| declaration of clip-path | 4 |
| declaration of column-width | 4 |
| declaration of min-height | 4 |
| declaration of pointer-events | 4 |
| declaration of scrollbar-gutter | 4 |
| declaration of scrollbar-width | 4 |
| declaration of text-indent | 4 |
| declaration of z-index | 4 |
| self-closing <canvas/> in XHTML | 4 |
| @starting-style | 3 |
| declaration of appearance | 3 |
| declaration of background-size | 3 |
| declaration of border-bottom-right-radius | 3 |
| declaration of container-name | 3 |
| declaration of cursor | 3 |
| declaration of flex-line-count | 3 |
| declaration of mask-image | 3 |
| declaration of padding-inline-start | 3 |
| declaration of text-orientation | 3 |
| declaration of transform-origin | 3 |
| element <details> | 3 |
| element <dialog> | 3 |
| element <math> | 3 |
| element <select> | 3 |
| unchecked attribute data-expected | 3 |
| declaration of -webkit-text-fill-color | 2 |
| declaration of background-position-x | 2 |
| declaration of border-bottom-left-radius | 2 |
| declaration of border-image | 2 |
| declaration of content | 2 |
| declaration of flex-basis | 2 |
| declaration of flex-grow | 2 |
| declaration of flex-shrink | 2 |
| declaration of float | 2 |
| declaration of font-size | 2 |
| declaration of image-rendering | 2 |
| declaration of margin-block-end | 2 |
| declaration of margin-top | 2 |
| declaration of order | 2 |
| declaration of padding-left | 2 |
| declaration of scrollbar-color | 2 |
| declaration of text-decoration-line | 2 |
| declaration of text-emphasis-style | 2 |
| declaration of text-overflow | 2 |
| declaration of text-size-adjust | 2 |
| declaration of unicode-bidi | 2 |
| element <marquee> | 2 |
| element <textarea> | 2 |
| missing file css/css-transforms/about:blank | 2 |
| self-closing <fieldset/> in XHTML | 2 |
| self-closing <iframe/> in XHTML | 2 |
| self-closing <textarea/> in XHTML | 2 |
| self-closing <video/> in XHTML | 2 |
| unchecked attribute data-color | 2 |
| unchecked attribute data-marker | 2 |
| unchecked attribute data-section-title | 2 |
| @custom-media | 1 |
| @page | 1 |
| @position-try | 1 |
| declaration of -- | 1 |
| declaration of --prop | 1 |
| declaration of --var | 1 |
| declaration of --w | 1 |
| declaration of -webkit-appearance | 1 |
| declaration of -webkit-filter | 1 |
| declaration of -webkit-perspective-origin-y | 1 |
| declaration of -webkit-writing-mode | 1 |
| declaration of accent-color | 1 |
| declaration of align-items | 1 |
| declaration of anchor-name | 1 |
| declaration of animation-name | 1 |
| declaration of background-blend-mode | 1 |
| declaration of background-origin | 1 |
| declaration of border-block | 1 |
| declaration of border-color | 1 |
| declaration of border-inline | 1 |
| declaration of border-inline-start-color | 1 |
| declaration of border-shape | 1 |
| declaration of column-rule | 1 |
| declaration of contain-intrinsic-size | 1 |
| declaration of content-visibility | 1 |
| declaration of expando | 1 |
| declaration of flex-direction | 1 |
| declaration of font-feature-settings | 1 |
| declaration of font-size-adjust | 1 |
| declaration of font-style | 1 |
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
| declaration of mask | 1 |
| declaration of max-width | 1 |
| declaration of mix-blend-mode | 1 |
| declaration of object-fit | 1 |
| declaration of padding-block-end | 1 |
| declaration of perspective | 1 |
| declaration of rotate | 1 |
| declaration of scale | 1 |
| declaration of stroke-dasharray | 1 |
| declaration of text-align | 1 |
| declaration of text-autospace | 1 |
| declaration of text-decoration | 1 |
| declaration of text-decoration-skip | 1 |
| declaration of top | 1 |
| declaration of transition-delay | 1 |
| declaration of transition-duration | 1 |
| declaration of transition-properties | 1 |
| declaration of transition-timing-function | 1 |
| declaration of translate | 1 |
| declaration of var-a | 1 |
| declaration of will-change | 1 |
| declaration of xcolor | 1 |
| element <audio> | 1 |
| element <frameset> | 1 |
| element <hr> | 1 |
| element <output> | 1 |
| link to  | 1 |
| link to //[ | 1 |
| missing file css/css-cascade/data:text/css,@layer foo;@media(all){} | 1 |
| missing file css/css-cascade/data:text/css,@layer{target{background-color:green !important}} | 1 |
| missing file css/css-cascade/data:text/css,@layer{target{background-color:green}} | 1 |
| missing file css/css-flexbox/resources/flexbox.css | 1 |
| missing file css/css-images/tools/REPLACEME_REFERENCE_FILENAME | 1 |
| self-closing <param/> in XHTML | 1 |
| unchecked attribute data-bar | 1 |
| unchecked attribute data-crash | 1 |
| unchecked attribute data-expect | 1 |
| unchecked attribute data-expected-left | 1 |
| unchecked attribute data-expected-padding-left | 1 |
| unchecked attribute data-expected-scroll-height | 1 |
| unchecked attribute data-name | 1 |
| unchecked attribute data-natural-width | 1 |
| unchecked attribute data-x | 1 |
