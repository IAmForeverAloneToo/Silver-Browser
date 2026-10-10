//! The acceptance of M29 (docs/design/wasm.md): plain Rust compiled to
//! WebAssembly (`tests/wasm/numbers`, the module beside its source),
//! fetched by a page's script and instantiated from the response, given
//! numbers and giving them back; a panic of the module's a trap the page
//! catches; a call that never returns stopped as a script that loops is,
//! the page's scripts running on after it.

use std::time::{Duration, Instant};

use net::test_server::Reply;

use crate::LoadState;
use crate::testing::*;

/// The module: `lib.rs` beside it, built for `wasm32-unknown-unknown`.
const NUMBERS: &[u8] = include_bytes!("../../tests/wasm/numbers/numbers.wasm");

/// The page: the module fetched, instantiated as it streams in, and its
/// functions called, each answer logged.
const PAGE: &str = r#"<!DOCTYPE html><title>numbers</title><script>
WebAssembly.instantiateStreaming(fetch("numbers.wasm")).then((made) => {
  const e = made.instance.exports;
  window.numbers = e;
  console.log("add " + e.add(40, 2) + " " + e.add(2147483647, 1) + " " + e.add(2.9, "8"));
  console.log("hypot " + e.hypot(3, 4) + " " + e.hypot(0, 0));
  console.log("fib " + e.fib(10) + " " + e.fib(90) + " " + typeof e.fib(1));
  console.log("collatz " + e.collatz(27n) + " " + e.collatz(1n));
  try { e.divide(1, 0); } catch (error) { console.log("trap " + error.constructor.name + ": " + error.message); }
  console.log("divide " + e.divide(84, 2) + " " + e.divide(-7, 2));
  console.log("memory " + (e.memory instanceof WebAssembly.Memory));
}, (error) => console.log("failed " + error));
</script>"#;

#[test]
fn a_page_gives_a_compiled_rust_function_numbers_and_gets_numbers_back() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(PAGE)),
        "/numbers.wasm" => Some(Reply::ok("application/wasm", NUMBERS)),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    let url = server.url("/page.html");
    tab.engine.navigate(url.clone());
    tab.wait_until(|engine| {
        incoming(engine).is_none()
            && engine.url() == Some(&url)
            && engine.load_state() == &LoadState::Loaded
            && engine.network.in_flight() == 0
    });
    let lines = |tab: &mut Tab| -> Vec<String> {
        tab.engine
            .take_console()
            .into_iter()
            .map(|entry| entry.text)
            .collect()
    };
    // The module's answer waits as a task: run until the page has said
    // all it says.
    let mut said = Vec::new();
    let deadline = Instant::now() + WAIT;
    while said.len() < 7 && Instant::now() < deadline {
        tab.engine.poll_network();
        tab.engine.run_tasks();
        said.extend(lines(&mut tab));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        said,
        [
            "add 42 -2147483648 10",
            "hypot 5 0",
            "fib 55 2880067194370816120 bigint",
            "collatz 111 0",
            "trap RuntimeError: unreachable",
            "divide 42 -3",
            "memory true",
        ]
    );
    // A call that never returns: stopped as a loop of the page's own
    // script is, and the page's next script runs.
    tab.engine.run_classic("numbers.spin(1)");
    let stopped = lines(&mut tab);
    assert_eq!(stopped.len(), 1, "{stopped:?}");
    assert!(
        stopped[0].starts_with("Script ran too long and was stopped"),
        "{stopped:?}"
    );
    tab.engine
        .run_classic("console.log('after ' + numbers.add(1, 2) + ' ' + numbers.fib(12))");
    assert_eq!(lines(&mut tab), ["after 3 144"]);
}
