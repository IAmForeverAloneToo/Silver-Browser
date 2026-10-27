# WebGPU conformance test suite subset

The tests here are the
[WebGPU CTS](https://github.com/gpuweb/cts)'s, under its BSD licence
(`LICENSE.txt` beside this file), from its built `out-wpt/` (the
`gh-pages` branch at commit b009414b0, taken 2026-10-26). They are the specs
of the queries in `QUERIES`, with the CTS's `common/` runtime and
framework, made one classic script, `bundle.js` (the engine runs no
module scripts, G41): each module a function of its exports' object,
its static imports requires (their names bound once a module a cycle
reaches is set, as an ES module's are live), a dynamic `import()` a
promise of a require. The engine has no `URL` class (G68): the bundle
has its own, for itself alone, enough for the CTS's runtime to read its
query and resolve its paths. `cts.html` loads WPT's `testharness.js`,
the runner's report and the bundle, which runs the query in its URL as
the CTS's own `cts.https.html` does.

The queries are the specs' tests, some whole files, some single tests,
chosen to run in about a minute; the rest of those files (the largest,
and `error_scope`'s 100,000 scopes) were run while choosing.

The bundle is made by `cts-bundle.py` of the project's tooling (outside
this repository), from the CTS checkout and `QUERIES`:

```sh
python cts-bundle.py <out-wpt> crates/engine/tests/webgpu-cts QUERIES
```

`tests/webgpu_cts.rs` runs each query; `expectations.txt` names the
cases expected to fail (or that may), or a query expected not to finish.
