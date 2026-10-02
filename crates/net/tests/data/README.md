# web-platform-tests' URL data

Copied unchanged from the web-platform-tests repository,
https://github.com/web-platform-tests/wpt, directory `url/resources/`,
on 2 October 2026:

| File | What `tests/url.rs` does with it |
|---|---|
| `urltestdata.json` | Parses every input, with its base if any, and compares every component the file gives, or expects failure. |
| `toascii.json` | Parses `https://<input>/x` and compares the host. |
| `IdnaTestV2.json` | The same, for the UTS #46 cases generated for URLs. |
| `percent-encoding.json` | Parses a query in each document encoding the `encoding` crate has. The CJK encodings are skipped, each by name. |

`setters_tests.json` is not here: the URL API's setters come with
scripts (M25).

Licence: BSD-3-Clause, `LICENSE-WPT.md`.
