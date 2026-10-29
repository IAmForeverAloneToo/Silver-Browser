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

# html5lib's tree construction tests

The `.dat` files in `tree-construction/` are all of those in
web-platform-tests' `html/syntax/parsing/resources/`, its copy of
html5lib-tests' tree construction tests, copied unchanged at its commit

```text
dc7f6a3c0 (2026-10-11)
```

under the same MIT licence. `tests/html5lib_tree.rs` runs every one of
them (M32, `docs/design/svg.md`; D57, `docs/design/text-dom-scripts.md`);
`tree-construction/expectations.txt` names what fails, each with its
reason.
