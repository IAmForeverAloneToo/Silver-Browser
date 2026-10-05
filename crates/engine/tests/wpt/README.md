# web-platform-tests subset

The tests here are copied from
[web-platform-tests](https://github.com/web-platform-tests/wpt),
under its BSD licence (`LICENSE.md` beside this file), at commit

```text
dc7f6a3c0000e1da9e575ff4ef5618cff136697b (2026-10-03)
```

They are the tests of `css/css-flexbox`, `css/css-grid`, `css/css-transforms` and `css/css-will-change`
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
| crashtest | 20 |
| reftest | 836 |
| all | 896 |

## Left out

The first reason the rule found for each test it left out:

| Reason | Tests |
| --- | ---: |
| missing file fonts/ahem.css | 627 |
| declaration of font | 574 |
| declaration of display | 478 |
| script | 450 |
| element <svg> | 321 |
| CDATA section in XHTML | 213 |
| declaration of writing-mode | 181 |
| element <img> | 117 |
| declaration of line-height | 100 |
| script (reftest-wait) | 86 |
| declaration of direction | 75 |
| element <br> | 51 |
| declaration of background | 43 |
| declaration of outline | 41 |
| element <canvas> | 41 |
| self-closing <div/> in XHTML | 40 |
| declaration of flex-wrap | 38 |
| @import | 37 |
| declaration of aspect-ratio | 29 |
| element <input> | 29 |
| declaration of inline-size | 26 |
| declaration of transform | 26 |
| declaration of grid-template-rows | 25 |
| declaration of grid-template-columns | 22 |
| @media | 14 |
| declaration of white-space | 14 |
| declaration of visibility | 13 |
| not a reftest, a check-layout test or a crashtest | 13 |
| declaration of block-size | 12 |
| declaration of align-self | 11 |
| declaration of fill | 11 |
| declaration of overflow-x | 9 |
| declaration of overflow-y | 9 |
| declaration of flex | 7 |
| declaration of grid-template | 7 |
| declaration of transition | 7 |
| declaration of column-count | 6 |
| element <button> | 6 |
| element <iframe> | 6 |
| @keyframes | 5 |
| declaration of background-color | 5 |
| declaration of filter | 5 |
| declaration of overflow | 5 |
| element <fieldset> | 5 |
| declaration of background-clip | 4 |
| declaration of contain | 4 |
| declaration of height | 4 |
| declaration of list-style | 4 |
| declaration of page-break-after | 4 |
| declaration of page-break-before | 4 |
| self-closing <canvas/> in XHTML | 4 |
| declaration of clip-path | 3 |
| declaration of column-width | 3 |
| declaration of columns | 3 |
| declaration of flex-line-count | 3 |
| declaration of grid-auto-rows | 3 |
| declaration of inset | 3 |
| declaration of text-decoration | 3 |
| declaration of transform-origin | 3 |
| declaration of width | 3 |
| element <select> | 3 |
| selector | 3 |
| declaration of flex-basis | 2 |
| declaration of flex-grow | 2 |
| declaration of flex-shrink | 2 |
| declaration of float | 2 |
| declaration of gap | 2 |
| declaration of grid-gap | 2 |
| declaration of margin-block-end | 2 |
| declaration of order | 2 |
| declaration of scrollbar-width | 2 |
| missing file css/css-transforms/about:blank | 2 |
| nested rule | 2 |
| self-closing <fieldset/> in XHTML | 2 |
| self-closing <iframe/> in XHTML | 2 |
| self-closing <textarea/> in XHTML | 2 |
| self-closing <video/> in XHTML | 2 |
| @supports | 1 |
| declaration of --columns | 1 |
| declaration of --size | 1 |
| declaration of align-items | 1 |
| declaration of background-image | 1 |
| declaration of border-block | 1 |
| declaration of border-inline | 1 |
| declaration of border-radius | 1 |
| declaration of column-rule | 1 |
| declaration of contain-intrinsic-size | 1 |
| declaration of flex-direction | 1 |
| declaration of font-weight | 1 |
| declaration of grid | 1 |
| declaration of grid-row | 1 |
| declaration of grid-template-column | 1 |
| declaration of list-style-type | 1 |
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
| element <frameset> | 1 |
| element <hr> | 1 |
| element <marquee> | 1 |
| element <textarea> | 1 |
| element <video> | 1 |
| missing file css/css-flexbox/resources/flexbox.css | 1 |
| missing file quirks/reference/green-100px-square-no-red.html | 1 |
| unchecked attribute data-expected-left | 1 |
| unchecked attribute data-expected-padding-left | 1 |
