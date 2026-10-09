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
