# Wycheproof's vectors

Copied from [C2SP/wycheproof](https://github.com/C2SP/wycheproof), under
its Apache License 2.0 (`LICENSE` beside this file), at commit

```text
65ca494727a52944c204441c860b039e6786b99b (2026-11-11)
```

| File | From |
| --- | --- |
| `aes_gcm_test.json` | `testvectors_v1/aes_gcm_test.json` |
| `LICENSE` | `LICENSE` |

Their SHA-256s are in `MANIFEST`, which a test holds every file here to
(but `MANIFEST` and this README): the vector files hold test keys, which
gitleaks would take for secrets, and `/.gitleaks.toml` allows this
directory's `*_test.json` files alone, upstream's bytes and nothing else.

`tests/wycheproof.rs` runs every vector through Web Crypto's own calls
(G255): an import, then each operation as a page would make it.
