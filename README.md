# Random Browser

A hobby browser engine in Rust — built from scratch, on neither Chromium nor
Gecko, following a Chromium-shaped layered architecture so the project can
grow without turning into a mess.

## Running

Once the Rust toolchain is installed:

```
cargo run -p random-browser
```

Opens a window rendering a document parsed by our own HTML tokenizer
and tree builder, with its stylesheet parsed by our own css-syntax-3
tokenizer and parser (milestone 3), with real font loading, real text
shaping (rustybuzz), and software rasterization (tiny-skia) presented
through winit + softbuffer.

## Architecture

The workspace is a strict layer DAG of crates (`shell → engine → render /
paint / layout / style / text / net / html / css → dom`).
**Read [`docs/architecture.md`](docs/architecture.md) before adding any
dependency edge** — the layering contract there is binding.

The milestone plan lives in [`docs/roadmap.md`](docs/roadmap.md): parity
tiers (committed target: T3, "replaces Chrome daily"), the granular
milestone ladder from M4 onward, cross-cutting tracks (testing, fuzzing,
security, performance), the decision register, and the risk register.
