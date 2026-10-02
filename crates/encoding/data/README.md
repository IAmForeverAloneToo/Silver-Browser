# The Encoding Standard's data

Copied unchanged from the WHATWG Encoding Standard,
https://encoding.spec.whatwg.org/, on 2 October 2026 (the standard's
revision of 21 May 2026):

- `encodings.json`: every encoding, by group, with its labels.
- `index-*.txt`: the indexes of the 28 legacy single-byte encodings.
  ISO-8859-8-I shares ISO-8859-8's, so there are 27 files. Each is
  dated 2024-09-18 and carries its SHA-256 identifier in its header.

`src/tables.rs` is generated from these files. The test
`tables_match_the_standard` in `tests/standard.rs` generates it again
and fails if the two differ. To regenerate it after replacing the
files, run:

```text
ENCODING_REGENERATE=1 cargo test -p encoding --test standard tables_match_the_standard
```

The indexes of the legacy CJK encodings are not here; they come with
their decoders (see `docs/design/net.md`).

Licence: the standard's portions carried into source code are under
BSD-3-Clause; see `../LICENSE-WHATWG`.
