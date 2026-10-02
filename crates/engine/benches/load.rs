//! M14 exit: the time from `Engine::navigate` (M14's `load`) to the
//! first frame for a page of 1,000 paragraphs and three stylesheets,
//! served by `net`'s test server on 127.0.0.1 (docs/design/net.md,
//! "Order of work", step 9).
//! The page links its sheets from its `<head>`, so they block it: it is
//! shown once all three have arrived, styled, and the frame after that
//! is the first. Each run starts a new engine, so its connections are
//! new too. Apart, for what they cost on their own: fetching the
//! document with a client of its own, and parsing its bytes.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench load
//! ```

use std::hint::black_box;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use engine::{Engine, FrameStats, FrameTime, LoadState};
use net::test_server::{Reply, TestServer};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const PARAGRAPHS: usize = 1000;
const RUNS: usize = 10;

/// The page: three sheets linked from the head, then the paragraphs,
/// every third of them in a class the sheets style.
fn page() -> String {
    let mut source = String::from(
        "<!DOCTYPE html><html><head><title>A long page</title>\
         <link rel=stylesheet href=/base.css><link rel=stylesheet href=/text.css>\
         <link rel=stylesheet href=/notes.css></head><body><h1>A long page</h1>",
    );
    for i in 0..PARAGRAPHS {
        let class = if i % 3 == 0 { " class=note" } else { "" };
        source.push_str(&format!(
            "<p{class}>Paragraph {i}. The quick brown fox jumps over the lazy dog; \
             pack my box with five dozen liquor jugs.</p>"
        ));
    }
    source.push_str("</body></html>");
    source
}

const BASE: &str = "html { background-color: #fdfdfd } \
                    body { margin: 16px; color: #222 } h1 { font-size: 28px }";
const TEXT: &str = "p { margin: 0 0 12px 0; line-height: 1.4 } p:first-child { margin-top: 0 }";
const NOTES: &str = ".note { padding: 4px 8px; border-left: 3px solid #48a; \
                     background-color: #eef }";

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// The median and range of `times`, in milliseconds.
fn spread(times: &mut [f64]) -> String {
    times.sort_by(f64::total_cmp);
    format!(
        "median {:.1} ms, {:.1}-{:.1} ms",
        times[times.len() / 2],
        times[0],
        times[times.len() - 1]
    )
}

/// One run: a new engine loads the page, and draws its first frame.
/// Returns the time until the page was shown, the first frame's stats,
/// and the time from `load` to the frame's end.
fn run(server: &TestServer) -> (Duration, FrameStats, Duration) {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    let start = Instant::now();
    engine.navigate(server.url("/page.html"));
    while engine.url().is_none() {
        woken
            .recv_timeout(Duration::from_secs(30))
            .expect("the page loads");
        engine.poll_network();
        assert!(
            !matches!(engine.load_state(), LoadState::Failed(_)),
            "the load failed: {:?}",
            engine.load_state()
        );
    }
    let shown = start.elapsed();
    assert_eq!(engine.load_state(), &LoadState::Loaded);
    assert_eq!(engine.stylesheets().len(), 3, "the three sheets apply");
    black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(Duration::ZERO)));
    (shown, engine.frame_stats(), start.elapsed())
}

fn main() {
    let html = page();
    let server = {
        let html = html.clone();
        TestServer::plain(move |request| match request.target.as_str() {
            "/page.html" => Reply::ok("text/html; charset=utf-8", html.as_bytes()),
            "/base.css" => Reply::ok("text/css", BASE.as_bytes()),
            "/text.css" => Reply::ok("text/css", TEXT.as_bytes()),
            "/notes.css" => Reply::ok("text/css", NOTES.as_bytes()),
            _ => Reply::with_fields(404, &[], b""),
        })
    };
    println!(
        "a page of {PARAGRAPHS} paragraphs, {} bytes, and three sheets, {RUNS} runs:",
        html.len()
    );

    // Apart: the document fetched alone, and parsed alone.
    let mut fetched = Vec::new();
    let mut parsed = Vec::new();
    for _ in 0..RUNS {
        let client = net::Client::default();
        let start = Instant::now();
        let response = client
            .fetch(&net::Request::get(
                server.url("/page.html"),
                net::Destination::Document,
            ))
            .expect("the page is fetched");
        fetched.push(ms(start.elapsed()));
        let start = Instant::now();
        let decoded = html::parse_document_from_bytes(&response.body, None);
        black_box(dom::Document::from_node(&decoded.document));
        parsed.push(ms(start.elapsed()));
    }
    println!("  the document fetched alone: {}", spread(&mut fetched));
    println!("  parsed alone: {}", spread(&mut parsed));

    let mut shown = Vec::new();
    let mut total = Vec::new();
    let mut stages: [Vec<f64>; 5] = Default::default();
    for _ in 0..RUNS {
        let (until_shown, stats, until_frame) = run(&server);
        shown.push(ms(until_shown));
        total.push(ms(until_frame));
        for (times, stage) in stages.iter_mut().zip([
            stats.style,
            stats.layout,
            stats.paint,
            stats.raster,
            stats.total,
        ]) {
            times.push(ms(stage));
        }
    }
    println!(
        "  load to the page shown, its sheets in: {}",
        spread(&mut shown)
    );
    for (name, times) in [
        "its style",
        "its layout",
        "its paint",
        "its raster",
        "whole",
    ]
    .iter()
    .zip(stages.iter_mut())
    {
        println!("  the first frame, {name}: {}", spread(times));
    }
    println!("  load to the first frame drawn: {}", spread(&mut total));
}
