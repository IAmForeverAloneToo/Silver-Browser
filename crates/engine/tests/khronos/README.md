# Khronos WebGL conformance suite subset

The tests here are the
[Khronos WebGL conformance suite](https://github.com/KhronosGroup/WebGL)'s
(`sdk/tests`), under its MIT-style licence (`LICENSE.txt` beside this
file), at commit 3a65272e4 (21 October 2026). They are the 241 tests of
`LIST`, from 41 directories of `conformance` and `conformance2`, with
the suite's `js/` and `resources/` and the files beside each test it
names, at their own paths.

They were chosen by running the 968 tests of `conformance` and
`conformance2` the engine can load (no video, worker or other host) with
`tests/khronos.rs`'s discovery mode,

```sh
KHRONOS_ROOT=<checkout>/sdk/tests KHRONOS_LIST=<list> cargo test -p engine --test khronos
```

and taking, from each directory in turn, the quickest of those that ended
alike in two runs, until the run's time was about a minute and a half.
They are copied by `khronos-vendor.py` of the project's tooling (outside
this repository):

```sh
python khronos-vendor.py <checkout>/sdk/tests crates/engine/tests/khronos <list>
```

`tests/khronos.rs` runs each test with a harness of its own standing in
for the suite's (`webgl-test-harness.js`); `expectations.txt` names the
subtests each is expected to fail, and why.
