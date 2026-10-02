# web-platform-tests' URL, MIME type and `data:` URL data

Copied unchanged from the web-platform-tests repository,
https://github.com/web-platform-tests/wpt, on 2 October 2026.

From `url/resources/`, for `tests/url.rs`:

| File | What the test does with it |
|---|---|
| `urltestdata.json` | Parses every input, with its base if any, and compares every component the file gives, or expects failure. |
| `toascii.json` | Parses `https://<input>/x` and compares the host. |
| `IdnaTestV2.json` | The same, for the UTS #46 cases generated for URLs. |
| `percent-encoding.json` | Parses a query in each document encoding the `encoding` crate has. The CJK encodings are skipped, each by name. |

`setters_tests.json` is not here: the URL API's setters come with
scripts (M25).

From `mimesniff/mime-types/resources/`, for `tests/mime.rs`:

| File | What the test does with it |
|---|---|
| `mime-types.json` | Parses and serializes every input. Each input a `Headers` object would take as it is is also a `Content-Type` field, and the type extracted from it must serialize the same. Where a case gives the `encoding` a document served with it has, the extracted `charset` must name that encoding. |
| `generated-mime-types.json` | The same, but for two inputs with a comma outside a quoted string: Fetch splits the field there before parsing, and the test expects what Fetch gives, by name. |

The `minimizedMIMEType` field is not checked: nothing in the engine
minimizes a MIME type yet.

From `fetch/data-urls/resources/`, for `tests/mime.rs`:

| File | What the test does with it |
|---|---|
| `data-urls.json` | Parses every input as a URL and runs Fetch's `data:` URL processor on it, comparing the type and the body, or expecting a network error. |
| `base64.json` | The same for `data:;base64,<input>`, comparing the body. |

Licence: BSD-3-Clause, `LICENSE-WPT.md`.
