# web-platform-tests subset

The tests here are copied from
[web-platform-tests](https://github.com/web-platform-tests/wpt),
under its BSD licence (`LICENSE.md` beside this file), at commit

```text
dc7f6a3c0000e1da9e575ff4ef5618cff136697b (2026-10-03)
```

They are the tests of `css/css-flexbox` that use only what this engine
has, chosen by the rule in `examples/wpt.rs` and copied with their
references and stylesheets at their own paths by

```sh
cargo run -p engine --example wpt -- pull <checkout>
```

which rewrites them, `MANIFEST` and this file. `tests/wpt.rs` runs
them; `expectations.txt` names those expected to fail, and why.

## Taken

| Kind | Tests |
| --- | ---: |
| checklayout | 32 |
| crashtest | 8 |
| reftest | 310 |
| all | 350 |

## Left out

The first reason the rule found for each test it left out:

| Reason | Tests |
| --- | ---: |
| script | 160 |
| CDATA section in XHTML | 127 |
| declaration of writing-mode | 113 |
| element <img> | 82 |
| declaration of line-height | 65 |
| @font-face | 56 |
| declaration of direction | 54 |
| element <br> | 43 |
| self-closing <div/> in XHTML | 40 |
| declaration of flex-wrap | 38 |
| declaration of outline | 26 |
| declaration of display | 23 |
| declaration of font | 21 |
| declaration of aspect-ratio | 19 |
| element <canvas> | 17 |
| declaration of background | 15 |
| @media | 13 |
| declaration of white-space | 13 |
| declaration of visibility | 11 |
| declaration of inline-size | 10 |
| declaration of block-size | 9 |
| element <input> | 9 |
| element <svg> | 9 |
| declaration of flex | 7 |
| declaration of transition | 7 |
| declaration of overflow-x | 6 |
| declaration of overflow-y | 6 |
| not a reftest, a check-layout test or a crashtest | 6 |
| script (reftest-wait) | 5 |
| declaration of height | 4 |
| declaration of page-break-after | 4 |
| declaration of page-break-before | 4 |
| self-closing <canvas/> in XHTML | 4 |
| @import | 3 |
| declaration of flex-line-count | 3 |
| declaration of list-style | 3 |
| declaration of opacity | 3 |
| declaration of width | 3 |
| element <fieldset> | 3 |
| element <select> | 3 |
| selector | 3 |
| declaration of background-color | 2 |
| declaration of flex-basis | 2 |
| declaration of flex-grow | 2 |
| declaration of flex-shrink | 2 |
| declaration of float | 2 |
| declaration of inset | 2 |
| declaration of margin-block-end | 2 |
| declaration of order | 2 |
| declaration of overflow | 2 |
| element <button> | 2 |
| nested rule | 2 |
| self-closing <fieldset/> in XHTML | 2 |
| self-closing <iframe/> in XHTML | 2 |
| self-closing <textarea/> in XHTML | 2 |
| self-closing <video/> in XHTML | 2 |
| @keyframes | 1 |
| declaration of align-items | 1 |
| declaration of align-self | 1 |
| declaration of border-radius | 1 |
| declaration of column-count | 1 |
| declaration of column-rule | 1 |
| declaration of columns | 1 |
| declaration of contain | 1 |
| declaration of flex-direction | 1 |
| declaration of justify-self | 1 |
| declaration of list-style-type | 1 |
| declaration of scrollbar-color | 1 |
| declaration of text-overflow | 1 |
| declaration of transform | 1 |
| declaration of xcolor | 1 |
| element <frameset> | 1 |
| element <iframe> | 1 |
| element <textarea> | 1 |
| element <video> | 1 |
| missing file css/css-flexbox/resources/flexbox.css | 1 |
