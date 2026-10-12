# html5lib's tokenizer tests

The `.test` files in `tokenizer/` are copied unchanged from
[html5lib-tests](https://github.com/html5lib/html5lib-tests), under its
MIT licence (`LICENSE` beside this file), at commit

```text
c777c408b61078ea2eb4acefc2535f54dbc8b28a (2026-10-01)
```

all of its tokenizer tests but `xmlViolation.test`, which is for the
infoset coercion, not the tokenizer. `tests/html5lib_tokenizer.rs` runs
them (M24.8, `docs/design/scripting.md`): every test from each of its
initial states, its input given at once and a character at a time.

# html5lib's tree construction tests of foreign content

The `.dat` files in `tree-construction/` (`tests9.dat` to `tests12.dat`,
`svg.dat`, `math.dat`, `foreign-fragment.dat` and
`namespace-sensitivity.dat`) are copied unchanged from the copy of
html5lib-tests that web-platform-tests keeps in
`html/syntax/parsing/resources`, at its commit

```text
dc7f6a3c0 (2026-10-11)
```

under the same MIT licence. `tests/html5lib_tree.rs` runs them (M32,
`docs/design/svg.md`); `tree-construction/expectations.txt` names what
fails by a gap that is not foreign content's.
