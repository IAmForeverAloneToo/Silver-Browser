//! The Khronos WebGL conformance tests the engine runs (M41.6, D40): a
//! subset of `conformance` and `conformance2`, copied into
//! `tests/khronos/` with the suite's `js` and `resources` (README.md
//! there says from where), each page run in the engine with a harness of
//! the runner's standing in for the suite's (`webgl-test-harness.js`):
//! every subtest's result, and the end, said on the console. Their
//! failures are held to `expectations.txt`: a test's failing subtests'
//! messages, numbers made `#` (they are the machine's), or `timeout`.
//! They hold on Windows, where they were made; elsewhere (another driver)
//! every test must end, with no panic. Without an adapter nothing runs.
//!
//! `KHRONOS_ROOT=<suite's sdk/tests> KHRONOS_LIST=<file>` runs the
//! suite's own pages of the list instead, writing what they gave to
//! `target/khronos-found.txt`, with no expectations: how the subset and
//! its expectations are chosen.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use engine::{Engine, FrameTime, LoadState};
use net::test_server::{Reply, TestServer};

/// How long a test may take, its load and its subtests (KHRONOS_TIMEOUT
/// seconds when set).
fn timeout() -> Duration {
    let seconds = std::env::var("KHRONOS_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    Duration::from_secs(seconds)
}

/// What the runner puts before a page's own scripts: the harness the
/// suite's pages report to (js-test-pre.js finds it on `window.parent`,
/// the page itself at the top).
const HARNESS: &str = "<script>window.webglTestHarness = {\
reportResults(path, ok, msg, skipped) { console.log((skipped ? 'SKIP ' : ok ? 'PASS ' : 'FAIL ') + String(msg).split('\\n').join(' / ')); },\
notifyFinished(path) { console.log('KHRONOS DONE'); } };</script>";

/// A page the runner serves itself, saying whether a WebGL context is
/// made here: a pass if so.
const PROBE: &str = "<!DOCTYPE html><script>\
const gl = document.createElement('canvas').getContext('webgl');\
window.webglTestHarness.reportResults('', !!gl, 'a context', false);\
window.webglTestHarness.notifyFinished('');</script>";

/// Whether the expectations hold here. They were made on Windows with
/// its hardware adapter; elsewhere (Mesa's software Vulkan on CI's
/// Linux, Metal on its macOS) a subtest's outcome is the driver's as
/// much as the engine's, and the subset must only end, every test,
/// with no panic and no uncaught exception's error.
fn expectations_hold() -> bool {
    cfg!(windows)
}

fn root() -> PathBuf {
    match std::env::var("KHRONOS_ROOT") {
        Ok(root) => PathBuf::from(root),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/khronos"),
    }
}

/// The tests: the list beside the subset, or KHRONOS_LIST's.
fn list() -> Vec<String> {
    let path = match std::env::var("KHRONOS_LIST") {
        Ok(path) => PathBuf::from(path),
        Err(_) => root().join("LIST"),
    };
    fs::read_to_string(&path)
        .expect("the list is there")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" => "text/javascript",
        "css" => "text/css",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webm" => "video/webm",
        "mp4" | "m4v" => "video/mp4",
        "ogv" => "video/ogg",
        "svg" => "image/svg+xml",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

fn serve(root: PathBuf) -> TestServer {
    TestServer::plain(move |request| {
        let path = request.target.split(['?', '#']).next().unwrap_or("");
        let file = path.trim_start_matches('/');
        let safe = !file.split('/').any(|part| part == ".." || part.is_empty());
        let body = if file == "probe.html" {
            Some(PROBE.as_bytes().to_vec())
        } else if safe {
            fs::read(root.join(file)).ok()
        } else {
            None
        };
        let Some(mut body) = body else {
            return Reply::Send(
                b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 9\r\n\r\nnot found"
                    .to_vec(),
            );
        };
        if file.ends_with(".html") {
            // After the doctype, which keeps the page out of quirks mode.
            let text = String::from_utf8_lossy(&body).into_owned();
            let at = if text
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("<!doctype")
            {
                text.find('>').map_or(0, |i| i + 1)
            } else {
                0
            };
            body = format!("{}{HARNESS}{}", &text[..at], &text[at..]).into_bytes();
        }
        Reply::ok(content_type(file), &body)
    })
}

/// What a test gave: its passes, its failures' messages, and whether it
/// ended.
#[derive(Debug, Default)]
struct Outcome {
    passes: usize,
    failures: Vec<String>,
    done: bool,
    error: Option<String>,
    took: Duration,
}

fn run(server: &TestServer, path: &str) -> Outcome {
    let began = Instant::now();
    let mut outcome = run_page(server, path);
    outcome.took = began.elapsed();
    outcome
}

fn run_page(server: &TestServer, path: &str) -> Outcome {
    let mut outcome = Outcome::default();
    let mut engine = Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    );
    engine.set_scripting(true);
    engine.navigate(server.url(&format!("/{path}")));
    let began = Instant::now();
    let mut at = Duration::from_secs(1);
    loop {
        engine.poll_network();
        if let LoadState::Failed(error) = engine.load_state() {
            outcome.error = Some(error.to_string());
            return outcome;
        }
        engine.run_tasks();
        for entry in engine.take_console() {
            // An exception no script caught: a failure of its own.
            if entry.source == engine::ConsoleSource::Exception {
                outcome.failures.push(format!(
                    "exception {}",
                    entry.text.lines().next().unwrap_or("")
                ));
                continue;
            }
            let text = entry.text;
            if std::env::var("KHRONOS_ECHO").is_ok() && !text.starts_with("PASS ") {
                eprintln!("console: {}", text.chars().take(400).collect::<String>());
            }
            if text == "KHRONOS DONE" {
                outcome.done = true;
            } else if text.starts_with("PASS ") {
                outcome.passes += 1;
            } else if let Some(message) = text.strip_prefix("FAIL ") {
                outcome.failures.push(message.to_owned());
            }
        }
        if outcome.done {
            return outcome;
        }
        if engine.needs_frame() {
            at += Duration::from_millis(16);
            engine.frame_at(
                300,
                150,
                FrameTime {
                    now: at,
                    deadline: at + Duration::from_millis(16),
                },
            );
        }
        if began.elapsed() > timeout() {
            return outcome;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

/// A failure's message as the expectations hold it: its numbers `#`,
/// as they are the machine's.
fn normalized(message: &str) -> String {
    let mut out = String::new();
    let mut in_number = false;
    for c in message.chars() {
        if c.is_ascii_digit() || (in_number && c == '.') {
            if !in_number {
                out.push('#');
            }
            in_number = true;
        } else {
            in_number = false;
            out.push(if c.is_ascii() && !c.is_ascii_control() {
                c
            } else {
                '?'
            });
        }
    }
    out
}

/// What a test gave, as lines of the expectations: `timeout`, `error`,
/// and each failure's message.
fn lines(outcome: &Outcome) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = outcome.failures.iter().map(|m| normalized(m)).collect();
    if let Some(error) = &outcome.error {
        out.insert(format!("error {}", normalized(error)));
    } else if !outcome.done {
        out.insert("timeout".to_owned());
    }
    out
}

/// expectations.txt: `path<TAB>line` for each failing subtest (or
/// `timeout`); a line of `either` lets the test's outcome be passing too.
fn expectations() -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let text = fs::read_to_string(root().join("expectations.txt")).unwrap_or_default();
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if let Some((path, what)) = line.split_once('\t') {
            out.entry(path.to_owned())
                .or_default()
                .insert(what.to_owned());
        }
    }
    out
}

#[test]
fn the_khronos_webgl_subset_runs_as_expected() {
    // No GPU: nothing to check.
    if engine::gpu::local_client().is_none() {
        eprintln!("no GPU service: the Khronos subset is not run");
        return;
    }
    let tests = list();
    let server = serve(root());
    // No adapter (CI's Windows runners): nothing to check.
    if run(&server, "probe.html").passes == 0 {
        eprintln!("no adapter here: the Khronos subset is not run");
        return;
    }
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Outcome>>> = Mutex::new((0..tests.len()).map(|_| None).collect());
    let threads = thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8);
    thread::scope(|scope| {
        for _ in 0..threads {
            let work = || {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(test) = tests.get(i) else {
                        break;
                    };
                    if std::env::var("KHRONOS_ROOT").is_ok() {
                        eprintln!("start {test}");
                    }
                    // An engine that panics fails its test, not the run.
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run(&server, test)
                    }))
                    .unwrap_or_else(|_| Outcome {
                        error: Some("panic".to_owned()),
                        ..Outcome::default()
                    });
                    if std::env::var("KHRONOS_ROOT").is_ok() {
                        eprintln!("end {test}");
                    }
                    results.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(outcome);
                }
            };
            // The engine's stack (engine::STACK_SIZE): a page's scripts
            // and layout run on the thread.
            thread::Builder::new()
                .stack_size(engine::STACK_SIZE)
                .spawn_scoped(scope, work)
                .expect("a thread");
        }
    });
    let results: Vec<Outcome> = results
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .into_iter()
        .map(|o| o.expect("every test ran"))
        .collect();
    let mut report = String::new();
    let (mut passes, mut failures) = (0, 0);
    for (test, outcome) in tests.iter().zip(&results) {
        passes += outcome.passes;
        failures += outcome.failures.len();
        report.push_str(&format!(
            "{test}\t{} passed, {} failed{}, {} ms\n",
            outcome.passes,
            outcome.failures.len(),
            if outcome.done { "" } else { ", did not finish" },
            outcome.took.as_millis()
        ));
        for line in lines(outcome) {
            report.push_str(&format!("{test}\t{line}\n"));
        }
        // The first failures as they were said, numbers and all.
        for message in outcome.failures.iter().take(3) {
            let short: String = message
                .chars()
                .take(200)
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            report.push_str(&format!("raw {test}\t{short}\n"));
        }
    }
    report.push_str(&format!("all\t{passes} passed, {failures} failed\n"));
    let found = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/khronos-found.txt");
    let _ = fs::write(&found, &report);
    if std::env::var("KHRONOS_ROOT").is_ok() {
        return;
    }
    if !expectations_hold() {
        let unfinished: Vec<String> = tests
            .iter()
            .zip(&results)
            .filter(|(_, outcome)| !outcome.done || outcome.error.is_some())
            .map(|(test, outcome)| format!("{test}: {:?}", outcome.error))
            .collect();
        assert!(
            unfinished.is_empty(),
            "tests that did not end:\n{}",
            unfinished.join("\n")
        );
        return;
    }
    let expected = expectations();
    let mut unexpected = Vec::new();
    for (test, outcome) in tests.iter().zip(&results) {
        let mut wanted = expected.get(test).cloned().unwrap_or_default();
        let either = wanted.remove("either");
        let got = lines(outcome);
        if got != wanted && !(either && got.is_empty()) {
            unexpected.push(format!("{test}: expected {wanted:?}, got {got:?}"));
        }
    }
    for test in expected.keys() {
        assert!(
            tests.contains(test),
            "expectations.txt names {test}, not in LIST"
        );
    }
    assert!(
        unexpected.is_empty(),
        "unexpected outcomes:\n{}",
        unexpected.join("\n")
    );
}
