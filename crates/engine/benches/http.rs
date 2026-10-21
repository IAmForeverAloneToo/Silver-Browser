//! M38's benches (docs/design/http.md, "The acceptance"): a page of
//! twenty resources (six sheets, six scripts that block the parser,
//! eight images) whose every reply the server holds back 40 ms, over
//! HTTP/1.1 and over HTTP/2, both over TLS, from `Engine::navigate` to
//! the page loaded; and a page whose first script, held back, blocks
//! the parser before ten more scripts and images, with the preload
//! scanner and without. Each run starts a new engine and a new client,
//! so its connections are new too, and nothing is cached.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench http
//! ```

#![forbid(unsafe_code)]

mod support;

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use engine::{Engine, LoadState};
use net::Url;
use net::test_server::{Reply, ServerRequest, TestAuthority, TestServer, VALID};
use net::transport::Config;

const RUNS: usize = 10;
/// How long the server holds back each reply.
const HELD: Duration = Duration::from_millis(40);

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

/// The twenty-resource page.
fn twenty() -> String {
    let mut page = String::from("<!DOCTYPE html><title>twenty</title>");
    page.extend((0..6).map(|n| format!("<link rel=stylesheet href=/s{n}.css>")));
    page.extend((0..6).map(|n| format!("<script src=/j{n}.js></script>")));
    page += "<p class=a>text</p>";
    page.extend((0..8).map(|n| format!("<img src=/i{n}.png>")));
    page
}

/// The page a held-back script blocks: ten more after it.
fn blocked() -> String {
    let mut page =
        String::from("<!DOCTYPE html><title>blocked</title><script src=/first.js></script>");
    page.extend((0..5).map(|n| format!("<script src=/j{n}.js></script>")));
    page += "<p>text</p>";
    page.extend((0..5).map(|n| format!("<img src=/i{n}.png>")));
    page
}

/// The answer to `target`: one of the pages, or a sheet, script or
/// image by its extension, never stored.
fn answer(target: &str) -> Reply {
    let (kind, body) = match target {
        "/twenty.html" => ("text/html", twenty()),
        "/blocked.html" => ("text/html", blocked()),
        _ => match target.rsplit('.').next() {
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

/// One run: a new engine with a new client of `config` loads `url`, the
/// scanner on or off. The time from `navigate` to the page loaded.
fn run(config: &Config, url: &Url, scanner: bool) -> Duration {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    engine.set_scripting(true);
    engine.set_preload_scanner(scanner);
    engine.set_network(net::Client::new(config.clone()));
    let (wake, woken): (_, Receiver<()>) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    let start = Instant::now();
    engine.navigate(url.clone());
    while *engine.load_state() == LoadState::Loading {
        let _ = woken.recv_timeout(Duration::from_millis(50));
        engine.poll_network();
        engine.run_tasks();
    }
    assert_eq!(engine.load_state(), &LoadState::Loaded);
    start.elapsed()
}

fn times(config: &Config, url: &Url, scanner: bool) -> String {
    let mut times: Vec<f64> = (0..support::rounds(RUNS))
        .map(|_| ms(run(config, url, scanner)))
        .collect();
    spread(&mut times)
}

fn main() {
    let authority = TestAuthority::new();
    let h1 = TestServer::tls(authority.server(&["a.localhost"], VALID), held);
    let h2 = TestServer::tls(authority.server_h2(&["a.localhost"], VALID), held);
    let plain = TestServer::plain(held);
    // Its name resolved here: the system's resolver takes a quarter of a
    // second for a `.localhost` name on Windows, which each new client
    // would pay.
    let config = Config::default()
        .trust(authority.root.clone())
        .resolve("a.localhost", std::net::IpAddr::from([127, 0, 0, 1]));
    let at = |server: &TestServer, path: &str| {
        Url::parse(&format!("https://a.localhost:{}{path}", server.port())).unwrap()
    };
    println!(
        "a page of twenty resources, each reply held back {} ms, {RUNS} runs, load to loaded:",
        HELD.as_millis()
    );
    println!(
        "  over HTTP/1.1: {}",
        times(&config, &at(&h1, "/twenty.html"), true)
    );
    println!(
        "  over HTTP/2: {}",
        times(&config, &at(&h2, "/twenty.html"), true)
    );
    println!("a page a held-back script blocks before ten more, over HTTP/1.1:");
    let url = plain.url("/blocked.html");
    println!("  with the preload scanner: {}", times(&config, &url, true));
    println!("  without: {}", times(&config, &url, false));
}
