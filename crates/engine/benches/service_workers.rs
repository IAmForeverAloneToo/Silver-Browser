//! M39's benches (docs/design/service-workers.md, "The acceptance"): a
//! page of twenty resources (six sheets, six scripts that block the
//! parser, eight images) whose every reply the server holds back 10 ms,
//! loaded with no service worker, through one that passes every request
//! to the network, and through one that answers them from its cache,
//! from `Engine::navigate` to the page loaded, each run a new tab of the
//! context the worker was registered in; and a worker's start: a
//! one-line worker's `register()` to `ready`.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench service_workers
//! ```

#![forbid(unsafe_code)]

mod support;

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use engine::{Context, Engine, LoadState};
use net::Url;
use net::test_server::{Reply, ServerRequest, TestServer};

const RUNS: usize = 10;
/// How long the server holds back each reply.
const HELD: Duration = Duration::from_millis(10);

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

/// The twenty resources, by their names in a scope.
fn names() -> Vec<String> {
    let mut names: Vec<String> = (0..6).map(|n| format!("s{n}.css")).collect();
    names.extend((0..6).map(|n| format!("j{n}.js")));
    names.extend((0..8).map(|n| format!("i{n}.png")));
    names
}

/// The twenty-resource page, its resources beside it.
fn twenty() -> String {
    let mut page = String::from("<!DOCTYPE html><title>twenty</title>");
    page.extend((0..6).map(|n| format!("<link rel=stylesheet href=s{n}.css>")));
    page.extend((0..6).map(|n| format!("<script src=j{n}.js></script>")));
    page += "<p class=a>text</p>";
    page.extend((0..8).map(|n| format!("<img src=i{n}.png>")));
    page
}

/// The page that registers its scope's worker and says when it is ready.
const SETUP: &str = "<script>navigator.serviceWorker.register('sw.js').then(() => navigator.serviceWorker.ready).then(() => console.log('ready'));</script>";

/// A scope's worker: none, one passing every request on, one answering
/// from what it kept at install.
fn worker(scope: &str) -> String {
    match scope {
        "pass" => "self.onfetch = (e) => e.respondWith(fetch(e.request));".to_owned(),
        "cache" => {
            let files: Vec<String> = std::iter::once("twenty.html".to_owned())
                .chain(names())
                .map(|name| format!("'{name}'"))
                .collect();
            format!(
                "self.oninstall = (e) => e.waitUntil(caches.open('c').then(c => c.addAll([{}])));
                 self.onfetch = (e) => e.respondWith(caches.match(e.request).then(r => r || fetch(e.request)));",
                files.join(",")
            )
        }
        _ => "1;".to_owned(),
    }
}

/// The answer to `target`: a scope's page, setup page or worker, or a
/// sheet, script or image by its extension, never stored.
fn answer(target: &str) -> Reply {
    let mut parts = target.trim_start_matches('/').splitn(2, '/');
    let (scope, file) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let (kind, body) = match file {
        "twenty.html" => ("text/html", twenty()),
        "setup.html" => ("text/html", SETUP.to_owned()),
        "sw.js" => ("text/javascript", worker(scope)),
        _ => match file.rsplit('.').next() {
            Some("css") => ("text/css", ".a { color: red }".to_owned()),
            Some("js") => ("text/javascript", "var x = 1;".to_owned()),
            Some("png") => ("image/png", String::new()),
            _ => return Reply::with_fields(404, &[], b""),
        },
    };
    Reply::with_fields(
        200,
        &[("Content-Type", kind), ("Cache-Control", "no-store")],
        body.as_bytes(),
    )
}

fn held(request: &ServerRequest) -> Reply {
    Reply::After(HELD, Box::new(answer(&request.target)))
}

/// A tab of `context`, scripts on, and the channel its waker sends on.
fn tab(context: &Context) -> (Engine, Receiver<()>) {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    engine.join(context);
    engine.set_scripting(true);
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    (engine, woken)
}

/// Runs `engine` until `done` holds of it, or a minute passes.
fn until(engine: &mut Engine, woken: &Receiver<()>, mut done: impl FnMut(&mut Engine) -> bool) {
    let start = Instant::now();
    while !done(engine) {
        assert!(start.elapsed() < Duration::from_secs(60), "in time");
        let _ = woken.recv_timeout(Duration::from_millis(5));
        engine.poll_network();
        engine.run_tasks();
    }
}

/// `scope`'s worker registered in `context`, through its setup page.
fn register(context: &Context, server: &TestServer, scope: &str) -> Duration {
    let (mut engine, woken) = tab(context);
    let start = Instant::now();
    engine.navigate(server.url(&format!("/{scope}/setup.html")));
    let mut ready = false;
    until(&mut engine, &woken, |engine| {
        ready |= engine
            .take_console()
            .iter()
            .any(|entry| entry.text == "ready");
        ready
    });
    start.elapsed()
}

/// One run: a new tab of `context` loads `url`; the time from
/// `navigate` to the page loaded.
fn run(context: &Context, url: &Url) -> Duration {
    let (mut engine, woken) = tab(context);
    let start = Instant::now();
    engine.navigate(url.clone());
    until(&mut engine, &woken, |engine| {
        *engine.load_state() != LoadState::Loading
    });
    assert_eq!(engine.load_state(), &LoadState::Loaded);
    start.elapsed()
}

fn main() {
    let server = TestServer::plain(held);
    println!(
        "a page of twenty resources, each reply held back {} ms, {RUNS} runs, load to loaded:",
        HELD.as_millis()
    );
    for (scope, said) in [
        ("none", "no service worker"),
        ("pass", "a worker passing each request on"),
        ("cache", "a worker answering from its cache"),
    ] {
        let context = Context::new();
        if scope != "none" {
            register(&context, &server, scope);
        }
        let url = server.url(&format!("/{scope}/twenty.html"));
        let mut times: Vec<f64> = (0..support::rounds(RUNS))
            .map(|_| ms(run(&context, &url)))
            .collect();
        println!("  {said}: {}", spread(&mut times));
    }
    // A worker's start: each run a new context, a one-line worker.
    let mut times: Vec<f64> = (0..support::rounds(RUNS))
        .map(|_| ms(register(&Context::new(), &server, "start")))
        .collect();
    println!(
        "a worker's start, register() to ready, {RUNS} runs: {}",
        spread(&mut times)
    );
}
