#!/bin/bash
# T3.2 (M33.5): `unsafe` only where it is reviewed, each block argued
# (the shell's test `unsafe_is_written_in_the_platform_module_alone_
# each_block_argued` holds each to its argument). CI's workflow and the
# local gate both run this script, from the repository's root, so that
# the list cannot differ between them (the lead's ruling, 2026-11-25).
#
# The files that may write `unsafe`:
# - the shell's platform module;
# - the one reviewed reading of the bindings' bytecode (M35.12);
# - the question of a thread's stack's bounds (G253);
# - the scripts' sampler, a tool built with the js-sampler feature alone;
# - a WebAssembly memory's buffer read in place (one block);
# - a realm's context's base64 intrinsic, and a realm made under a held
#   lock (G90);
# - M40's AV1 decoder, through its C interface;
# - Web Crypto's calls into aws-lc's C library (G255);
# - a binding's buffer copied for a native (G255 step 7a, one block).
# M29's module, a crate of its own built to WebAssembly, marks its
# exports `no_mangle`. Strings and line comments are cut first.
ALLOWED=(
  crates/shell/src/platform.rs
  crates/js/src/quickjs/bytecode.rs
  crates/js/src/stack.rs
  crates/js/src/quickjs/sampler.rs
  crates/js/src/quickjs/wasm.rs
  crates/js/src/quickjs/context.rs
  crates/media/src/av1.rs
  crates/engine/tests/wasm/
  crates/crypto/src/native.rs
  crates/js/src/quickjs/bytes_in.rs
)
skip=()
for path in "${ALLOWED[@]}"; do
  case $path in
    */) skip+=(-e "^$path") ;;
    *) skip+=(-e "^$path:") ;;
  esac
done
found=$(grep -rn --include='*.rs' unsafe crates fuzz/fuzz_targets \
  | grep -v "${skip[@]}" \
  | sed -E 's/"([^"\\]|\\.)*"//g; s#//.*##' \
  | grep -E '(^|[^[:alnum:]_-])unsafe[[:space:]]*(\{|fn|impl|trait|extern)|#\[unsafe\(')
if [ -n "$found" ]; then
  printf '%s\n' "$found" >&2
  echo "::error::unsafe outside the reviewed files (tools/ci/unsafe-allowlist.sh)"
  exit 1
fi
