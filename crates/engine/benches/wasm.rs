//! M29's: what WebAssembly costs (docs/design/wasm.md, "Acceptance").
//! The acceptance module's Collatz steps for every number to 20,000,
//! against the same function in a page's script; 10,000 calls across
//! the bridge, then 100 with the module's memory seen by the script, its
//! mirror copied at each; and a module of 1,000 functions compiled. Each
//! a page, from `Engine::navigate` to its title, beside the same page
//! doing nothing.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench wasm
//! ```

#![forbid(unsafe_code)]

mod support;

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use engine::{Engine, LoadState};
use net::Url;
use net::test_server::{Reply, TestServer};

/// The acceptance module (`tests/wasm/numbers`).
const NUMBERS: &[u8] = include_bytes!("../tests/wasm/numbers/numbers.wasm");
const RUNS: usize = 10;
const FUNCTIONS: u32 = 1000;

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// The median and range of `times`, in milliseconds.
fn spread(times: &mut [f64]) -> String {
    times.sort_by(f64::total_cmp);
    format!(
        "median {:.2} ms, {:.2}-{:.2} ms",
        times[times.len() / 2],
        times[0],
        times[times.len() - 1]
    )
}

/// A WebAssembly integer in the variable-length form.
fn leb(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn section(out: &mut Vec<u8>, id: u8, body: &[u8]) {
    out.push(id);
    leb(out, body.len() as u32);
    out.extend_from_slice(body);
}

/// A module of `n` functions, each `(param i32) (result i32)` adding its
/// index to its argument, each exported by its index.
fn large_module(n: u32) -> Vec<u8> {
    let mut out = b"\0asm\x01\0\0\0".to_vec();
    section(&mut out, 1, &[1, 0x60, 1, 0x7f, 1, 0x7f]);
    let mut functions = Vec::new();
    leb(&mut functions, n);
    functions.extend(std::iter::repeat_n(0, n as usize));
    section(&mut out, 3, &functions);
    let mut exports = Vec::new();
    leb(&mut exports, n);
    for i in 0..n {
        let name = format!("f{i}");
        leb(&mut exports, name.len() as u32);
        exports.extend_from_slice(name.as_bytes());
        exports.push(0);
        leb(&mut exports, i);
    }
    section(&mut out, 7, &exports);
    let mut code = Vec::new();
    leb(&mut code, n);
    for i in 0..n {
        let mut body = vec![0, 0x20, 0, 0x41];
        // The index as a signed LEB: positive and under 2^13, so its
        // unsigned form with a clear sign bit.
        let mut constant = Vec::new();
        leb(&mut constant, i);
        if constant.last().is_some_and(|byte| byte & 0x40 != 0) {
            let last = constant.len() - 1;
            constant[last] |= 0x80;
            constant.push(0);
        }
        body.extend_from_slice(&constant);
        body.extend_from_slice(&[0x6a, 0x0b]);
        leb(&mut code, body.len() as u32);
        code.extend_from_slice(&body);
    }
    section(&mut out, 10, &code);
    out
}

fn page(script: &str) -> String {
    format!("<!DOCTYPE html><title>WebAssembly</title><script>{script}</script>")
}

/// The pages: each sets its title to `done` when its work is.
fn server() -> TestServer {
    let numbers = |work: &str| {
        page(&format!(
            "WebAssembly.instantiateStreaming(fetch('/numbers.wasm')).then((made) => {{ \
             const e = made.instance.exports; {work} document.title = 'done'; }});"
        ))
    };
    let pages = [
        ("/none.html", page("document.title = 'done';")),
        ("/module.html", numbers("")),
        (
            "/collatz-wasm.html",
            numbers("let s = 0; for (let n = 1; n <= 20000; n++) s += e.collatz(BigInt(n));"),
        ),
        (
            "/collatz-script.html",
            numbers(
                "const collatz = (n) => { let k = 0; while (n > 1) { n = n % 2 === 0 ? n / 2 : 3 * n + 1; k++; } return k; }; \
                 let s = 0; for (let n = 1; n <= 20000; n++) s += collatz(n);",
            ),
        ),
        (
            "/calls.html",
            numbers("let s = 0; for (let i = 0; i < 10000; i++) s = e.add(s, 1);"),
        ),
        (
            "/calls-memory.html",
            numbers(
                "const seen = new Uint8Array(e.memory.buffer); let s = 0; \
                 for (let i = 0; i < 100; i++) s = e.add(s, seen[0] + 1);",
            ),
        ),
        (
            "/compile.html",
            page(
                "fetch('/large.wasm').then((r) => r.arrayBuffer()).then((b) => { \
                 const m = new WebAssembly.Module(b); \
                 if (WebAssembly.Module.exports(m).length > 0) document.title = 'done'; });",
            ),
        ),
        (
            "/fetch-only.html",
            page(
                "fetch('/large.wasm').then((r) => r.arrayBuffer()).then((b) => { \
                 if (b.byteLength > 0) document.title = 'done'; });",
            ),
        ),
    ];
    let large = large_module(FUNCTIONS);
    TestServer::plain(move |request| {
        let target = request.target.as_str();
        if let Some((_, body)) = pages.iter().find(|(path, _)| *path == target) {
            return Reply::ok("text/html", body.as_bytes());
        }
        match target {
            "/numbers.wasm" => Reply::ok("application/wasm", NUMBERS),
            "/large.wasm" => Reply::ok("application/wasm", &large),
            _ => Reply::with_fields(404, &[], b""),
        }
    })
}

/// A new engine over the bundled font with scripts on, and the channel
/// its waker sends on.
fn engine() -> (Engine, Receiver<()>) {
    let mut engine = Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the font"),
    );
    engine.set_scripting(true);
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    (engine, woken)
}

/// The time from `navigate` to the page's title being `done`.
fn load(url: &Url) -> Duration {
    let (mut engine, woken) = engine();
    let started = Instant::now();
    engine.navigate(url.clone());
    loop {
        engine.poll_network();
        engine.run_tasks();
        if *engine.load_state() != LoadState::Loading && engine.title() == "done" {
            return started.elapsed();
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "{url} never finished: {:?}",
            engine.take_console()
        );
        let _ = woken.recv_timeout(Duration::from_millis(1));
    }
}

fn main() {
    let server = server();
    println!("WebAssembly (M29), median of {RUNS}:");
    for (path, what) in [
        ("/none.html", "a page doing nothing"),
        (
            "/module.html",
            "the acceptance module fetched and instantiated",
        ),
        ("/collatz-wasm.html", "Collatz to 20,000 in the module"),
        ("/collatz-script.html", "the same in the page's script"),
        ("/calls.html", "10,000 calls across the bridge"),
        ("/calls-memory.html", "100 calls with the memory seen"),
        ("/fetch-only.html", "a module of 1,000 functions fetched"),
        ("/compile.html", "the same compiled"),
    ] {
        let url = server.url(path);
        let mut times: Vec<f64> = (0..support::rounds(RUNS)).map(|_| ms(load(&url))).collect();
        println!("  {what}: {}", spread(&mut times));
    }
}
