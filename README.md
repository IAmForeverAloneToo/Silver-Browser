# Silver Browser

A hobby browser engine in Rust — built from scratch, on neither Chromium nor
Gecko, following a Chromium-shaped layered architecture so the project can
grow without turning into a mess.

## Running

Once the Rust toolchain is installed:

```
cargo run -p silver-browser
```

Opens a window on the browser's own page, `about:silver`, with the
address field ready to type into. Given an address, a URL or a file,
it opens that instead:

```
cargo run -p silver-browser -- example.com
```

Ctrl+L focuses the address field (Cmd+L on macOS), Enter goes there,
Alt+Left and Alt+Right go back and forward (Cmd+[ and Cmd+]), and F5
reloads. Everything on screen, the toolbar included, is parsed, laid
out and drawn by the engine's own crates, and presented through winit
and softbuffer.

## Architecture

The workspace is a strict layer DAG of crates (`shell → ui / engine`,
`engine → render / paint / layout / style / text / net / html / css →
dom / encoding`, and `ui → render / paint / text`).
**Read [`docs/architecture.md`](docs/architecture.md) before adding any
dependency edge** — the layering contract there is binding.

The milestone plan lives in [`docs/roadmap.md`](docs/roadmap.md): parity
tiers (committed target: T3, "replaces Chrome daily"), the granular
milestone ladder from M4 onward, cross-cutting tracks (testing, fuzzing,
security, performance), the decision register, and the risk register.
