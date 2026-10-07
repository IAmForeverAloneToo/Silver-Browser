# web-platform-tests subset

The tests here are copied from
[web-platform-tests](https://github.com/web-platform-tests/wpt),
under its BSD licence (`LICENSE.md` beside this file), at commit

```text
dc7f6a3c0000e1da9e575ff4ef5618cff136697b (2026-10-03)
```

They are the tests of `css/css-flexbox`, `css/css-grid`, `css/css-transforms`, `css/css-will-change`, `css/mediaqueries`, `css/css-conditional`, `css/css-cascade`, `css/css-variables`, `css/css-values`, `css/css-env`, `css/css-lists`, `css/css-counter-styles`, `css/css-content`, `css/css-text-decor`, `css/css-text/text-align`, `css/css-text/text-justify`, `css/CSS2/lists` and `css/CSS2/generated-content`
that use only what this engine has, chosen by the rule in
`examples/wpt.rs` and copied with their references and stylesheets
at their own paths by

```sh
cargo run -p engine --example wpt -- pull <checkout>
```

which rewrites them, `MANIFEST` and this file. `tests/wpt.rs` runs
them; `expectations.txt` names those expected to fail, and why.

## Taken

| Kind | Tests |
| --- | ---: |
| checklayout | 41 |
| crashtest | 32 |
| reftest | 1630 |
| all | 1703 |

## Left out

The first reason the rule found for each test it left out:

| Reason | Tests |
| --- | ---: |
| script | 1154 |
| missing file fonts/ahem.css | 774 |
| declaration of font | 704 |
| declaration of display | 494 |
| CDATA section in XHTML | 378 |
| element <svg> | 326 |
| declaration of line-height | 216 |
| declaration of writing-mode | 201 |
| not a reftest, a check-layout test or a crashtest | 191 |
| script (reftest-wait) | 136 |
| element <img> | 134 |
| element <br> | 93 |
| declaration of direction | 86 |
| declaration of white-space | 77 |
| declaration of container-type | 66 |
| @font-face | 47 |
| declaration of background | 47 |
| element <canvas> | 47 |
| declaration of font-variant-numeric | 43 |
| declaration of outline | 41 |
| self-closing <div/> in XHTML | 40 |
| declaration of flex-wrap | 38 |
| declaration of aspect-ratio | 35 |
| declaration of background-image | 35 |
| @keyframes | 34 |
| @counter-style | 33 |
| element <input> | 32 |
| dir=rtl | 31 |
| declaration of visibility | 29 |
| missing file css/CSS2/reference/pass-if-number-0-30px.xht | 28 |
| element <iframe> | 27 |
| declaration of grid-template-rows | 26 |
| declaration of inline-size | 26 |
| declaration of transform | 26 |
| @scope | 23 |
| nested rule | 23 |
| @container | 22 |
| @property | 21 |
| declaration of grid-template-columns | 21 |
| selector | 21 |
| declaration of background-color | 19 |
| declaration of color | 18 |
| declaration of overflow-y | 16 |
| declaration of text-shadow | 16 |
| declaration of width | 16 |
| declaration of block-size | 13 |
| declaration of fill | 12 |
| declaration of text-decoration-inset | 12 |
| declaration of align-self | 11 |
| declaration of height | 11 |
| declaration of all | 10 |
| declaration of column-count | 10 |
| declaration of overflow-x | 10 |
| declaration of text-decoration-skip-spaces | 10 |
| declaration of text-emphasis | 10 |
| declaration of transition | 10 |
| @namespace | 9 |
| declaration of animation | 9 |
| declaration of text-decoration-skip-ink | 9 |
| unchecked attribute data-foo | 9 |
| declaration of break-after | 7 |
| declaration of contain | 7 |
| declaration of grid-template | 7 |
| element <button> | 7 |
| declaration of columns | 6 |
| declaration of flex | 6 |
| element <fieldset> | 6 |
| unchecked attribute data-val | 6 |
| declaration of --a | 5 |
| declaration of break-before | 5 |
| declaration of filter | 5 |
| declaration of overflow | 5 |
| declaration of background-clip | 4 |
| declaration of min-width | 4 |
| declaration of scrollbar-gutter | 4 |
| declaration of scrollbar-width | 4 |
| declaration of text-indent | 4 |
| self-closing <canvas/> in XHTML | 4 |
| declaration of appearance | 3 |
| declaration of border-radius | 3 |
| declaration of box-shadow | 3 |
| declaration of clip-path | 3 |
| declaration of column-width | 3 |
| declaration of container-name | 3 |
| declaration of content | 3 |
| declaration of flex-line-count | 3 |
| declaration of font-weight | 3 |
| declaration of inset | 3 |
| declaration of min-height | 3 |
| declaration of padding-inline-start | 3 |
| declaration of text-orientation | 3 |
| declaration of transform-origin | 3 |
| declaration of transition-property | 3 |
| declaration of z-index | 3 |
| element <math> | 3 |
| element <select> | 3 |
| declaration of -webkit-text-fill-color | 2 |
| declaration of animation-timing-function | 2 |
| declaration of border-top-left-radius | 2 |
| declaration of flex-basis | 2 |
| declaration of flex-grow | 2 |
| declaration of flex-shrink | 2 |
| declaration of float | 2 |
| declaration of margin-block-end | 2 |
| declaration of order | 2 |
| declaration of padding-left | 2 |
| declaration of text-decoration-line | 2 |
| declaration of text-emphasis-style | 2 |
| declaration of text-overflow | 2 |
| declaration of unicode-bidi | 2 |
| element <details> | 2 |
| element <textarea> | 2 |
| missing file css/CSS2/reference/no-red-on-blank-page-ref.xht | 2 |
| missing file css/css-transforms/about:blank | 2 |
| self-closing <fieldset/> in XHTML | 2 |
| self-closing <iframe/> in XHTML | 2 |
| self-closing <textarea/> in XHTML | 2 |
| self-closing <video/> in XHTML | 2 |
| unchecked attribute data-color | 2 |
| unchecked attribute data-marker | 2 |
| @custom-media | 1 |
| @page | 1 |
| declaration of -- | 1 |
| declaration of --prop | 1 |
| declaration of --var | 1 |
| declaration of --w | 1 |
| declaration of -webkit-appearance | 1 |
| declaration of accent-color | 1 |
| declaration of align-items | 1 |
| declaration of border-block | 1 |
| declaration of border-image | 1 |
| declaration of border-inline | 1 |
| declaration of border-inline-start-color | 1 |
| declaration of column-rule | 1 |
| declaration of contain-intrinsic-size | 1 |
| declaration of content-visibility | 1 |
| declaration of expando | 1 |
| declaration of flex-direction | 1 |
| declaration of font-feature-settings | 1 |
| declaration of font-size | 1 |
| declaration of font-variant | 1 |
| declaration of grid | 1 |
| declaration of grid-column-start | 1 |
| declaration of grid-row | 1 |
| declaration of grid-template-column | 1 |
| declaration of list-style | 1 |
| declaration of list-style-image | 1 |
| declaration of list-style-position | 1 |
| declaration of margin | 1 |
| declaration of margin-inline | 1 |
| declaration of max-width | 1 |
| declaration of mix-blend-mode | 1 |
| declaration of padding-block-end | 1 |
| declaration of perspective | 1 |
| declaration of pointer-events | 1 |
| declaration of scrollbar-color | 1 |
| declaration of stroke-dasharray | 1 |
| declaration of text-align | 1 |
| declaration of text-autospace | 1 |
| declaration of text-decoration | 1 |
| declaration of text-decoration-skip | 1 |
| declaration of var-a | 1 |
| declaration of will-change | 1 |
| declaration of xcolor | 1 |
| declaration of zoom | 1 |
| element <audio> | 1 |
| element <frameset> | 1 |
| element <hr> | 1 |
| element <marquee> | 1 |
| element <video> | 1 |
| missing file css/CSS2/reference/ref-nothing-below.xht | 1 |
| missing file css/css-cascade/data:text/css,@layer foo;@media(all){} | 1 |
| missing file css/css-cascade/data:text/css,@layer{target{background-color:green !important}} | 1 |
| missing file css/css-cascade/data:text/css,@layer{target{background-color:green}} | 1 |
| missing file css/css-flexbox/resources/flexbox.css | 1 |
| missing file quirks/reference/green-100px-square-no-red.html | 1 |
| self-closing <param/> in XHTML | 1 |
| unchecked attribute data-bar | 1 |
| unchecked attribute data-crash | 1 |
| unchecked attribute data-expected | 1 |
| unchecked attribute data-expected-left | 1 |
| unchecked attribute data-expected-padding-left | 1 |
| unchecked attribute data-expected-scroll-height | 1 |
| unchecked attribute data-name | 1 |
| unchecked attribute data-x | 1 |
