//! The site sweep's capture (`--sweep-site`, the scoreboard) against
//! three pages of a local server, with no network: one that loads, one
//! whose renderers crash as they start, and one whose script waits on a
//! synchronous request the server holds past the sweep's patience.

#![forbid(unsafe_code)]
// The one-process build has no sweep: its renderers are what it captures.
#![cfg(not(feature = "single-process"))]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use devtools::Json;
use net::test_server::{Reply, TestServer};

const BROWSER: &str = env!("CARGO_BIN_EXE_silver-browser");

fn server() -> TestServer {
    TestServer::plain(|request| match request.target.as_str() {
        "/ok.html" => Reply::ok(
            "text/html",
            b"<!DOCTYPE html><title>ok</title><style>body { background: #0a8; height: 5000px }\
              </style><p>A page that loads.</p><script>\
              onscroll = () => { onscroll = null; setTimeout(() => notOnScroll()); };\
              notInThisEngine();</script>",
        ),
        "/hang.html" => Reply::ok(
            "text/html",
            b"<!DOCTYPE html><p>Waiting</p><script>\
              var request = new XMLHttpRequest(); request.open('GET', '/slow', false);\
              request.send();</script>",
        ),
        "/slow" => {
            // Longer than the sweep waits for an answer from the renderer.
            std::thread::sleep(Duration::from_secs(25));
            Reply::ok("text/plain", b"late")
        }
        _ => Reply::with_fields(404, &[("Content-Type", "text/plain")], b"no"),
    })
}

/// The sweep's record of `path`, captured into a folder of the test's.
fn sweep(server: &TestServer, path: &str, crash: bool) -> (Json, PathBuf) {
    let out = std::env::temp_dir().join(format!(
        "silver-sweep-{}-{}{}",
        std::process::id(),
        path.trim_start_matches('/').replace('.', "-"),
        if crash { "-crash" } else { "" }
    ));
    let _ = std::fs::remove_dir_all(&out);
    let mut command = Command::new(BROWSER);
    command
        .arg("--sweep-site")
        .arg(server.url(path).as_str())
        .arg("--sweep-out")
        .arg(&out);
    if crash {
        command.arg("--sweep-crash-renderers");
    }
    let ran = command.output().expect("the sweep ran");
    let text = std::fs::read_to_string(out.join("silver.json"))
        .unwrap_or_else(|e| panic!("no record ({e}): {}", String::from_utf8_lossy(&ran.stderr)));
    (Json::parse(&text).expect("a record"), out)
}

fn field<'a>(record: &'a Json, name: &str) -> &'a Json {
    record.get(name).unwrap_or(&Json::Null)
}

fn png(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|bytes| bytes.starts_with(b"\x89PNG"))
}

#[test]
fn a_page_that_loads_is_captured_with_its_timings_counts_and_screenshots() {
    let server = server();
    let (record, out) = sweep(&server, "/ok.html", false);
    assert_eq!(field(&record, "outcome").as_str(), Some("ok"), "{record:?}");
    let timings = field(&record, "timings");
    for name in ["firstPaint", "load", "settled"] {
        assert!(
            field(timings, name).as_f64().is_some(),
            "{name}: {record:?}"
        );
    }
    // Settled once loaded, however early its layouts stopped.
    assert!(
        field(timings, "settled").as_f64() >= field(timings, "load").as_f64(),
        "{record:?}"
    );
    assert_eq!(
        field(&record, "finalUrl").as_str(),
        Some(server.url("/ok.html").as_str())
    );
    let missing = field(field(&record, "counters"), "missing");
    assert_eq!(
        field(missing, "notInThisEngine").as_u64(),
        Some(1),
        "{record:?}"
    );
    // Counted after the scroll too: the record has the page's last counts.
    assert_eq!(
        field(missing, "notOnScroll").as_u64(),
        Some(1),
        "{record:?}"
    );
    // Then left alone: a page with nothing running draws no frame and
    // runs no task in that time.
    let idle = field(&record, "idleCounters");
    let counters = field(&record, "counters");
    let delta = |name: &str| {
        field(idle, name).as_f64().unwrap_or(f64::NAN)
            - field(counters, name).as_f64().unwrap_or(f64::NAN)
    };
    assert_eq!(delta("frames"), 0.0, "{record:?}");
    let tasks = |of: &Json| {
        field(field(of, "tasks"), "timers")
            .as_f64()
            .unwrap_or(f64::NAN)
            + field(field(of, "tasks"), "animationFrames")
                .as_f64()
                .unwrap_or(f64::NAN)
    };
    assert_eq!(tasks(idle) - tasks(counters), 0.0, "{record:?}");
    let console = field(&record, "console").as_array().expect("entries");
    assert!(
        console
            .iter()
            .any(|e| field(e, "source").as_str() == Some("exception")),
        "{record:?}"
    );
    assert!(field(field(&record, "network"), "requests").as_u64() >= Some(1));
    let memory = field(&record, "memory");
    assert!(
        field(memory, "browserPeakWorkingSet").as_f64() > Some(0.0),
        "{record:?}"
    );
    assert!(png(&out.join("silver-top.png")));
    assert!(png(&out.join("silver-scrolled.png")));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_renderer_that_crashes_is_said_so() {
    let server = server();
    let (record, out) = sweep(&server, "/ok.html", true);
    assert_eq!(
        field(&record, "outcome").as_str(),
        Some("renderer crash"),
        "{record:?}"
    );
    assert!(!out.join("silver-top.png").exists());
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_renderer_that_answers_nothing_has_hung() {
    let server = server();
    let (record, out) = sweep(&server, "/hang.html", false);
    assert_eq!(
        field(&record, "outcome").as_str(),
        Some("hang"),
        "{record:?}"
    );
    let _ = std::fs::remove_dir_all(&out);
}
