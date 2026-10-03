# web-platform-tests subset

The tests here are copied from
[web-platform-tests](https://github.com/web-platform-tests/wpt),
under its BSD licence (`LICENSE.md` beside this file), at commit

```text
dc7f6a3c0000e1da9e575ff4ef5618cff136697b (2026-10-03)
```

They are the tests of `css/css-flexbox` and `css/css-grid`
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
| crashtest | 15 |
| reftest | 516 |
| all | 571 |

## Left out

The first reason the rule found for each test it left out:

| Reason | Tests |
| --- | ---: |
| @font-face | 624 |
| declaration of font | 574 |
| declaration of display | 475 |
| script | 328 |
| CDATA section in XHTML | 193 |
| declaration of writing-mode | 178 |
| element <img> | 112 |
| declaration of line-height | 100 |
| declaration of direction | 75 |
| element <br> | 50 |
| element <canvas> | 41 |
| self-closing <div/> in XHTML | 40 |
| declaration of flex-wrap | 38 |
| @import | 37 |
| declaration of outline | 37 |
| declaration of aspect-ratio | 29 |
| declaration of inline-size | 26 |
| declaration of grid-template-rows | 25 |
| script (reftest-wait) | 24 |
| declaration of grid-template-columns | 22 |
| declaration of background | 19 |
| @media | 14 |
| declaration of visibility | 13 |
| declaration of white-space | 13 |
| element <svg> | 13 |
| declaration of block-size | 12 |
| declaration of align-self | 11 |
| element <input> | 11 |
| declaration of overflow-x | 8 |
| declaration of flex | 7 |
| declaration of grid-template | 7 |
| declaration of transition | 7 |
| declaration of overflow-y | 6 |
| not a reftest, a check-layout test or a crashtest | 6 |
| element <button> | 5 |
| element <fieldset> | 5 |
| declaration of background-clip | 4 |
| declaration of contain | 4 |
| declaration of height | 4 |
| declaration of list-style | 4 |
| declaration of page-break-after | 4 |
| declaration of page-break-before | 4 |
| element <iframe> | 4 |
| self-closing <canvas/> in XHTML | 4 |
| declaration of flex-line-count | 3 |
| declaration of grid-auto-rows | 3 |
| declaration of inset | 3 |
| declaration of opacity | 3 |
| declaration of overflow | 3 |
| declaration of text-decoration | 3 |
| declaration of width | 3 |
| element <select> | 3 |
| selector | 3 |
| declaration of background-color | 2 |
| declaration of column-count | 2 |
| declaration of column-width | 2 |
| declaration of flex-basis | 2 |
| declaration of flex-grow | 2 |
| declaration of flex-shrink | 2 |
| declaration of float | 2 |
| declaration of gap | 2 |
| declaration of grid-gap | 2 |
| declaration of margin-block-end | 2 |
| declaration of order | 2 |
| declaration of scrollbar-width | 2 |
| nested rule | 2 |
| self-closing <fieldset/> in XHTML | 2 |
| self-closing <iframe/> in XHTML | 2 |
| self-closing <textarea/> in XHTML | 2 |
| self-closing <video/> in XHTML | 2 |
| @keyframes | 1 |
| declaration of --columns | 1 |
| declaration of --size | 1 |
| declaration of align-items | 1 |
| declaration of border-block | 1 |
| declaration of border-inline | 1 |
| declaration of border-radius | 1 |
| declaration of column-rule | 1 |
| declaration of columns | 1 |
| declaration of contain-intrinsic-size | 1 |
| declaration of flex-direction | 1 |
| declaration of grid | 1 |
| declaration of grid-row | 1 |
| declaration of grid-template-column | 1 |
| declaration of list-style-type | 1 |
| declaration of margin-left | 1 |
| declaration of margin-top | 1 |
| declaration of min-height | 1 |
| declaration of padding | 1 |
| declaration of padding-block-end | 1 |
| declaration of scrollbar-color | 1 |
| declaration of text-overflow | 1 |
| declaration of transform | 1 |
| declaration of xcolor | 1 |
| element <frameset> | 1 |
| element <hr> | 1 |
| element <textarea> | 1 |
| element <video> | 1 |
| missing file css/css-flexbox/resources/flexbox.css | 1 |
| missing file quirks/reference/green-100px-square-no-red.html | 1 |
| unchecked attribute data-expected-left | 1 |
| unchecked attribute data-expected-padding-left | 1 |
