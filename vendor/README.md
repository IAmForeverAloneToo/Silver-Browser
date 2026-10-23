# Vendored crates

Crates the build takes from here rather than from crates.io, each with
why and until when. `Cargo.toml`'s `[patch.crates-io]` (and the fuzz
crate's) points at them; they are not workspace members, and the
workspace's lints and formatting do not apply to them.

## rav1d 1.1.0

The AV1 decoder (M40, D36, `docs/design/audio-video.md`): crates.io's
`rav1d` 1.1.0, the newest release, as published, with one change, the
upstream commit 6c6f46378 of 14 September 2026 ("frame_hdr may be None
after rav1d_decode_frame_exit", `src/decode.rs`), which no release has
yet. Without it, a corrupted keyframe makes the decoder's error cleanup
unwrap a frame header it no longer has: a panic that is not sound to
unwind through (M40's fuzzing).

And three changes of ours, each marked "Silver:". Two for leaks CI's
LeakSanitizer found (48 bytes a time, an empty ITU-T T.35 list): in
`include/dav1d/picture.rs`, a picture handed to C carries no reference
to an empty list (`dav1d_picture_unref` wrote back an emptied picture
holding one, as did a `dav1d_get_picture` that had no picture, and
nothing released them); in `src/picture.rs`, the default picture
allocator reads only the picture's parameters (it converted the whole
picture, making such a list, and forgot it). One for an overflow
the fuzzer found: in `src/recon.rs`, `read_golomb` subtracts with
wrapping, as dav1d's unsigned C does (a code of 32 zeros left 0, and a
build that checks overflow panicked).

Left out of the published crate: its assembly (`src/**/*.asm`, `*.S`,
built only with the `asm` feature, which this build does not turn on),
its lock file and toolchain file, its scripts, and its `THANKS.md`,
`NEWS.dav1d` and `CONTRIBUTING.md`; in `COPYING`, the copyright sign is
written "(c)" (the repository is ASCII). Added to its `Cargo.toml`:
`[lints.rust] warnings = "allow"`, the cap a registry crate's warnings
get. Dropped once a release of `rav1d` carries these fixes.
