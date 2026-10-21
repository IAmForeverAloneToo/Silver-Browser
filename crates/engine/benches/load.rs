//! M14 exit: the time from `Engine::navigate` (M14's `load`) to the
//! first frame for a page of 1,000 paragraphs and three stylesheets,
//! served by `net`'s test server on 127.0.0.1 (docs/design/net.md,
//! "Order of work", step 9). And M15's: the time from going back to
//! that page to its first frame, when the back-forward cache kept it
//! and when it was pushed out and is fetched again
//! (docs/design/navigation.md, "The back-forward cache").
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

#![forbid(unsafe_code)]

mod support;

use std::hint::black_box;
use std::sync::mpsc::{Receiver, channel};
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

/// A new engine over the bundled font, and the channel its waker sends
/// on.
fn engine() -> (Engine, Receiver<()>) {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    (engine, woken)
}

/// Takes results as the waker says they come until the page under way
/// is shown, all there.
fn wait(engine: &mut Engine, woken: &Receiver<()>) {
    while *engine.load_state() == LoadState::Loading {
        woken
            .recv_timeout(Duration::from_secs(30))
            .expect("the page loads");
        engine.poll_network();
    }
    assert_eq!(engine.load_state(), &LoadState::Loaded);
}

/// Takes results until none has come for a while: the page's icon, a
/// fetch of its own that would keep it from the back-forward cache if
/// the page were left with it on its way.
fn settle(engine: &mut Engine, woken: &Receiver<()>) {
    while woken.recv_timeout(Duration::from_millis(200)).is_ok() {
        engine.poll_network();
    }
}

fn first_frame(engine: &mut Engine) {
    black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(Duration::ZERO)));
}

/// One run: a new engine loads the page, and draws its first frame.
/// Returns the time until the page was shown, the first frame's stats,
/// and the time from `navigate` to the frame's end.
fn run(server: &TestServer) -> (Duration, FrameStats, Duration) {
    let (mut engine, woken) = engine();
    let start = Instant::now();
    engine.navigate(server.url("/page.html"));
    wait(&mut engine, &woken);
    let shown = start.elapsed();
    assert_eq!(engine.stylesheets().len(), 3, "the three sheets apply");
    first_frame(&mut engine);
    (shown, engine.frame_stats(), start.elapsed())
}

/// One run back: a new engine shows the page, leaves it for a small
/// one, or for seven of them so that the back-forward cache, which
/// keeps six, lets it go, and goes back to it. Returns the time from
/// the traversal to the end of the first frame drawn after it.
fn back(server: &TestServer, kept: bool) -> Duration {
    let (mut engine, woken) = engine();
    engine.navigate(server.url("/page.html"));
    wait(&mut engine, &woken);
    first_frame(&mut engine);
    settle(&mut engine, &woken);
    let away = if kept { 1 } else { 7 };
    for n in 0..away {
        engine.navigate(server.url(&format!("/small{n}.html")));
        wait(&mut engine, &woken);
        first_frame(&mut engine);
        settle(&mut engine, &woken);
    }
    let fetched = server.requests().len();
    let start = Instant::now();
    engine.traverse(-(away as isize));
    wait(&mut engine, &woken);
    first_frame(&mut engine);
    let took = start.elapsed();
    assert_eq!(engine.url(), Some(&server.url("/page.html")));
    let asked: Vec<String> = server.requests()[fetched..]
        .iter()
        .map(|request| request.target.clone())
        .collect();
    // Not kept, the document is parsed again from the HTTP cache, whose
    // stored response a traversal takes (M38).
    assert!(
        !asked.contains(&"/page.html".to_owned()),
        "from the cache: {asked:?}"
    );
    took
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
            small if small.starts_with("/small") => {
                Reply::ok("text/html", b"<title>A small page</title><p>small")
            }
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
    for _ in 0..support::rounds(RUNS) {
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
        black_box(decoded.document);
        parsed.push(ms(start.elapsed()));
    }
    println!("  the document fetched alone: {}", spread(&mut fetched));
    println!("  parsed alone: {}", spread(&mut parsed));

    let mut shown = Vec::new();
    let mut total = Vec::new();
    let mut stages: [Vec<f64>; 5] = Default::default();
    for _ in 0..support::rounds(RUNS) {
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

    for (kept, name) in [(true, "from the cache"), (false, "fetched again")] {
        let mut times: Vec<f64> = (0..support::rounds(RUNS))
            .map(|_| ms(back(&server, kept)))
            .collect();
        println!(
            "  back to the page {name}, to its first frame drawn: {}",
            spread(&mut times)
        );
    }
}
