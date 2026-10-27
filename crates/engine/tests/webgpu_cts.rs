//! The WebGPU conformance test suite's queries the engine runs (M41.6,
//! D39): a subset of the CTS's `api`, `shader` and `web_platform` specs,
//! their modules made one classic script (`tests/webgpu-cts/bundle.js`;
//! the engine runs no module scripts, G41; README.md there says from
//! where and how), each query of `QUERIES` run on `cts.html` with WPT's
//! `testharness.js` and a report of the runner's on the console. Failing
//! cases are held to `expectations.txt`: `query<TAB>case`,
//! `query<TAB>case<TAB>flaky` (either), or `query<TAB>timeout`. They hold
//! on Windows, where they were made; elsewhere every query must end.
//! `target/webgpu-cts-found.txt` says what each query gave, and how long
//! it took.

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

/// How long a query may take.
const TIMEOUT: Duration = Duration::from_secs(120);

/// testharnessreport.js's stand-in: each test's result, and the end.
/// The harness's own timeout is off: the runner's [`TIMEOUT`] is the
/// query's (queries run side by side, each slower than alone).
const REPORT: &str = "setup({explicit_timeout: true});\n\
add_result_callback(function (t) {\n\
  console.log((t.status === 0 ? 'PASS ' : 'FAIL ') + t.name + (t.status === 0 ? '' : ' :: ' + String(t.message).split('\\n').filter((l) => l.trim() !== '').join(' / ').slice(0, 400)));\n\
});\n\
add_completion_callback(function (tests, status) {\n\
  if (status.status !== 0) console.log('FAIL harness :: ' + status.message);\n\
  console.log('CTS DONE');\n\
});\n";

/// A page the runner serves itself, saying whether WebGPU has an adapter
/// here: a pass if so.
const PROBE: &str = "<script>(navigator.gpu ? navigator.gpu.requestAdapter() : Promise.resolve(null))\
  .then((a) => console.log(a ? 'PASS adapter' : 'FAIL adapter'), () => console.log('FAIL adapter'))\
  .then(() => console.log('CTS DONE'));</script>";

/// Whether the expectations hold here. They were made on Windows with
/// its hardware adapter; elsewhere (Mesa's software Vulkan on CI's
/// Linux, Metal on its macOS) a case's outcome is the driver's as much
/// as the engine's, and every query must only end.
fn expectations_hold() -> bool {
    cfg!(windows)
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/webgpu-cts")
}

fn queries() -> Vec<String> {
    fs::read_to_string(root().join("QUERIES"))
        .expect("the queries are there")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

fn serve() -> TestServer {
    let root = root();
    let harness =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wpt/resources/testharness.js");
    TestServer::plain(move |request| {
        let path = request.target.split(['?', '#']).next().unwrap_or("");
        match path {
            "/resources/testharness.js" => match fs::read(&harness) {
                Ok(body) => Reply::ok("text/javascript", &body),
                Err(_) => {
                    Reply::Send(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec())
                }
            },
            "/resources/testharnessreport.js" => Reply::ok("text/javascript", REPORT.as_bytes()),
            "/probe.html" => Reply::ok("text/html", PROBE.as_bytes()),
            "/cts.html" | "/bundle.js" => {
                let file = path.trim_start_matches('/');
                let kind = if file.ends_with(".js") {
                    "text/javascript"
                } else {
                    "text/html; charset=utf-8"
                };
                match fs::read(root.join(file)) {
                    Ok(body) => Reply::ok(kind, &body),
                    Err(_) => {
                        Reply::Send(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec())
                    }
                }
            }
            _ => Reply::Send(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()),
        }
    })
}

/// A query's cases that failed (`case :: message`), and whether it ended.
#[derive(Debug, Default)]
struct Outcome {
    passes: usize,
    failures: Vec<(String, String)>,
    done: bool,
    took: Duration,
}

fn run(server: &TestServer, query: &str) -> Outcome {
    let began = Instant::now();
    let mut outcome = run_query(server, query);
    outcome.took = began.elapsed();
    outcome
}

fn run_query(server: &TestServer, query: &str) -> Outcome {
    let mut outcome = Outcome::default();
    let mut engine = Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    );
    engine.set_scripting(true);
    let path = if query == "probe" {
        "/probe.html".to_owned()
    } else {
        format!("/cts.html?q={query}")
    };
    engine.navigate(server.url(&path));
    let began = Instant::now();
    let mut at = Duration::from_secs(1);
    loop {
        engine.poll_network();
        if let LoadState::Failed(error) = engine.load_state() {
            outcome
                .failures
                .push(("load".to_owned(), error.to_string()));
            return outcome;
        }
        engine.run_tasks();
        for entry in engine.take_console() {
            let text = entry.text;
            if std::env::var("CTS_ECHO").is_ok() && !text.starts_with("PASS ") {
                eprintln!("console: {}", text.chars().take(400).collect::<String>());
            }
            if text == "CTS DONE" {
                outcome.done = true;
            } else if text.starts_with("PASS ") {
                outcome.passes += 1;
            } else if let Some(rest) = text.strip_prefix("FAIL ") {
                let (case, message) = rest.split_once(" :: ").unwrap_or((rest, ""));
                outcome.failures.push((case.to_owned(), message.to_owned()));
            }
        }
        if outcome.done || began.elapsed() > TIMEOUT {
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
        thread::sleep(Duration::from_millis(1));
    }
}

/// A case as the expectations name it: ASCII, its query's prefix gone.
fn case_name(query: &str, case: &str) -> String {
    let short = case
        .strip_prefix(query.trim_end_matches('*'))
        .unwrap_or(case);
    short
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() {
                c
            } else {
                '?'
            }
        })
        .collect()
}

fn expectations() -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let text = fs::read_to_string(root().join("expectations.txt")).unwrap_or_default();
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if let Some((query, case)) = line.split_once('\t') {
            out.entry(query.to_owned())
                .or_default()
                .insert(case.to_owned());
        }
    }
    out
}

#[test]
fn the_webgpu_cts_subset_runs_as_expected() {
    if engine::gpu::local_client().is_none() {
        eprintln!("no GPU service: the WebGPU CTS subset is not run");
        return;
    }
    let queries = queries();
    let server = serve();
    // No adapter (CI's Windows runners): nothing to check.
    if run(&server, "probe").passes == 0 {
        eprintln!("no adapter here: the WebGPU CTS subset is not run");
        return;
    }
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Outcome>>> =
        Mutex::new((0..queries.len()).map(|_| None).collect());
    let threads = thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8);
    thread::scope(|scope| {
        for _ in 0..threads {
            let work = || {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(query) = queries.get(i) else {
                        break;
                    };
                    let outcome = run(&server, query);
                    results.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(outcome);
                }
            };
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
        .map(|o| o.expect("every query ran"))
        .collect();
    let expected = expectations();
    let hold = expectations_hold();
    let mut report = String::new();
    let mut unexpected = Vec::new();
    for (query, outcome) in queries.iter().zip(&results) {
        report.push_str(&format!(
            "{query}\t{} passed, {} failed{}, {} ms\n",
            outcome.passes,
            outcome.failures.len(),
            if outcome.done { "" } else { ", did not finish" },
            outcome.took.as_millis()
        ));
        let mut got: BTreeSet<String> = BTreeSet::new();
        for (case, message) in &outcome.failures {
            let name = case_name(query, case);
            report.push_str(&format!("{query}\t{name}\t{message}\n"));
            got.insert(name);
        }
        if !outcome.done {
            got.insert("timeout".to_owned());
        }
        let mut wanted = expected.get(query).cloned().unwrap_or_default();
        // A case `<TAB>flaky` may pass or fail (its timing is the load's).
        let flaky: Vec<String> = wanted
            .iter()
            .filter_map(|case| case.strip_suffix("\tflaky").map(str::to_owned))
            .collect();
        wanted.retain(|case| !case.ends_with("\tflaky"));
        for case in &flaky {
            got.remove(case);
        }
        if !hold {
            if !outcome.done {
                unexpected.push(format!("{query}: did not end"));
            }
        } else if got != wanted {
            unexpected.push(format!(
                "{query}: newly failing {:?}, newly passing {:?}",
                got.difference(&wanted).collect::<Vec<_>>(),
                wanted.difference(&got).collect::<Vec<_>>()
            ));
        }
    }
    let found = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/webgpu-cts-found.txt");
    let _ = fs::write(&found, &report);
    for query in expected.keys() {
        assert!(
            queries.contains(query),
            "expectations.txt names {query}, not in QUERIES"
        );
    }
    assert!(
        unexpected.is_empty(),
        "unexpected outcomes:\n{}",
        unexpected.join("\n")
    );
}
