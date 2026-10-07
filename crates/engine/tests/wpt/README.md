# web-platform-tests subset

The tests here are copied from
[web-platform-tests](https://github.com/web-platform-tests/wpt),
under its BSD licence (`LICENSE.md` beside this file), at commit

```text
dc7f6a3c0000e1da9e575ff4ef5618cff136697b (2026-10-03)
```

They are the tests of `css/css-flexbox`, `css/css-grid`, `css/css-transforms`, `css/css-will-change`, `css/mediaqueries`, `css/css-conditional` and `css/css-cascade`
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
| checklayout | 40 |
| crashtest | 23 |
| reftest | 903 |
| all | 966 |

## Left out

The first reason the rule found for each test it left out:

| Reason | Tests |
| --- | ---: |
| script | 753 |
| missing file fonts/ahem.css | 639 |
| declaration of font | 608 |
| declaration of display | 482 |
| element <svg> | 321 |
| CDATA section in XHTML | 304 |
| declaration of writing-mode | 182 |
| element <img> | 119 |
| declaration of line-height | 101 |
| script (reftest-wait) | 94 |
| declaration of direction | 76 |
| declaration of container-type | 62 |
| element <br> | 52 |
| element <canvas> | 47 |
| declaration of background | 44 |
| declaration of outline | 41 |
| self-closing <div/> in XHTML | 40 |
| declaration of flex-wrap | 38 |
| declaration of aspect-ratio | 30 |
| element <input> | 30 |
| declaration of grid-template-rows | 26 |
| declaration of inline-size | 26 |
| declaration of transform | 26 |
| @scope | 23 |
| declaration of grid-template-columns | 22 |
| @container | 21 |
| declaration of background-color | 16 |
| declaration of visibility | 14 |
| declaration of white-space | 14 |
| declaration of widows | 13 |
| not a reftest, a check-layout test or a crashtest | 13 |
| declaration of block-size | 12 |
| element <iframe> | 12 |
| declaration of align-self | 11 |
| declaration of fill | 11 |
| nested rule | 11 |
| declaration of all | 10 |
| declaration of --x | 9 |
| declaration of overflow-x | 9 |
| declaration of overflow-y | 9 |
| declaration of transition | 9 |
| declaration of column-count | 8 |
| @keyframes | 7 |
| @namespace | 7 |
| declaration of flex | 7 |
| declaration of grid-template | 7 |
| declaration of width | 6 |
| element <button> | 6 |
| declaration of background-image | 5 |
| declaration of contain | 5 |
| declaration of filter | 5 |
| declaration of overflow | 5 |
| element <fieldset> | 5 |
| selector | 5 |
| declaration of background-clip | 4 |
| declaration of color | 4 |
| declaration of height | 4 |
| declaration of list-style | 4 |
| declaration of page-break-after | 4 |
| declaration of page-break-before | 4 |
| self-closing <canvas/> in XHTML | 4 |
| declaration of animation | 3 |
| declaration of clip-path | 3 |
| declaration of column-width | 3 |
| declaration of columns | 3 |
| declaration of container-name | 3 |
| declaration of flex-line-count | 3 |
| declaration of grid-auto-rows | 3 |
| declaration of inset | 3 |
| declaration of text-decoration | 3 |
| declaration of transform-origin | 3 |
| element <select> | 3 |
| declaration of animation-timing-function | 2 |
| declaration of appearance | 2 |
| declaration of flex-basis | 2 |
| declaration of flex-grow | 2 |
| declaration of flex-shrink | 2 |
| declaration of float | 2 |
| declaration of gap | 2 |
| declaration of grid-gap | 2 |
| declaration of margin-block-end | 2 |
| declaration of order | 2 |
| declaration of scrollbar-width | 2 |
| element <math> | 2 |
| missing file css/css-transforms/about:blank | 2 |
| self-closing <fieldset/> in XHTML | 2 |
| self-closing <iframe/> in XHTML | 2 |
| self-closing <textarea/> in XHTML | 2 |
| self-closing <video/> in XHTML | 2 |
| @counter-style | 1 |
| @custom-media | 1 |
| @property | 1 |
| declaration of --a | 1 |
| declaration of --badcolor | 1 |
| declaration of --columns | 1 |
| declaration of --size | 1 |
| declaration of accent-color | 1 |
| declaration of align-items | 1 |
| declaration of border-block | 1 |
| declaration of border-inline | 1 |
| declaration of border-radius | 1 |
| declaration of column-rule | 1 |
| declaration of contain-intrinsic-size | 1 |
| declaration of content-visibility | 1 |
| declaration of flex-direction | 1 |
| declaration of font-weight | 1 |
| declaration of grid | 1 |
| declaration of grid-row | 1 |
| declaration of grid-template-column | 1 |
| declaration of list-style-type | 1 |
| declaration of margin | 1 |
| declaration of margin-left | 1 |
| declaration of margin-top | 1 |
| declaration of min-height | 1 |
| declaration of mix-blend-mode | 1 |
| declaration of padding | 1 |
| declaration of padding-block-end | 1 |
| declaration of perspective | 1 |
| declaration of pointer-events | 1 |
| declaration of scrollbar-color | 1 |
| declaration of text-overflow | 1 |
| declaration of top | 1 |
| declaration of translate | 1 |
| declaration of will-change | 1 |
| declaration of xcolor | 1 |
| element <audio> | 1 |
| element <frameset> | 1 |
| element <hr> | 1 |
| element <li> | 1 |
| element <marquee> | 1 |
| element <textarea> | 1 |
| element <video> | 1 |
| missing file css/css-cascade/data:text/css,@layer foo;@media(all){} | 1 |
| missing file css/css-cascade/data:text/css,@layer{target{background-color:green !important}} | 1 |
| missing file css/css-cascade/data:text/css,@layer{target{background-color:green}} | 1 |
| missing file css/css-flexbox/resources/flexbox.css | 1 |
| missing file quirks/reference/green-100px-square-no-red.html | 1 |
| unchecked attribute data-expected-left | 1 |
| unchecked attribute data-expected-padding-left | 1 |
| unchecked attribute data-expected-scroll-height | 1 |
