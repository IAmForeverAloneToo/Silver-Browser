//! M27's: what fetching costs (docs/design/fetch.md, "Acceptance"). A
//! page that makes a thousand `fetch()`es of a small JSON body from
//! `net`'s test server on 127.0.0.1, from `Engine::navigate` to its
//! last promise, beside the same page making none; one asynchronous
//! `XMLHttpRequest` of the body and one synchronous; and the jar's
//! retrieval of a `Cookie` header, and a store, with 3,000 cookies kept.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench fetch
//! ```

#![forbid(unsafe_code)]

use std::hint::black_box;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, UNIX_EPOCH};

use engine::{Engine, LoadState};
use net::Url;
use net::cookies::{Api, Context, Jar, MAX_COOKIES};
use net::test_server::{Reply, TestServer};

const FETCHES: usize = 1000;
const RUNS: usize = 10;
/// The domains the jar's cookies are spread over, each within its limit.
const DOMAINS: usize = 20;
const LOOKUPS: usize = 1000;

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

/// The page whose script runs `script`, which titles the page "done"
/// once its last promise settles.
fn page(script: &str) -> String {
    format!("<!DOCTYPE html><title>Fetching</title><script>{script}</script>")
}

fn server() -> TestServer {
    let fetches = page(&format!(
        "let left = {FETCHES};\
         for (let i = 0; i < {FETCHES}; i++) fetch('/data.json').then(r => r.json()).then(j => {{\
           if (j.ok && --left === 0) document.title = 'done';\
         }});"
    ));
    let none = page("document.title = 'done';");
    let xhr = page(
        "const x = new XMLHttpRequest(); x.open('GET', '/data.json'); x.responseType = 'json';\
         x.onload = () => { if (x.response.ok) document.title = 'done'; }; x.send();",
    );
    let sync = page(
        "const x = new XMLHttpRequest(); x.open('GET', '/data.json', false); x.send();\
         if (JSON.parse(x.responseText).ok) document.title = 'done';",
    );
    TestServer::plain(move |request| match request.target.as_str() {
        "/fetches.html" => Reply::ok("text/html", fetches.as_bytes()),
        "/none.html" => Reply::ok("text/html", none.as_bytes()),
        "/xhr.html" => Reply::ok("text/html", xhr.as_bytes()),
        "/sync.html" => Reply::ok("text/html", sync.as_bytes()),
        "/data.json" => Reply::ok("application/json", b"{\"ok\":true,\"items\":[1,2,3]}"),
        _ => Reply::with_fields(404, &[], b""),
    })
}

/// A new engine over the bundled font with scripts on, and the channel
/// its waker sends on.
fn engine() -> (Engine, Receiver<()>) {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    engine.set_scripting(true);
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    (engine, woken)
}

/// One run: from `navigate` to the page's title saying "done", its
/// tasks run as the network brings their answers.
fn run(server: &TestServer, path: &str) -> Duration {
    let (mut engine, woken) = engine();
    let start = Instant::now();
    engine.navigate(server.url(path));
    while !(engine.title() == "done" && *engine.load_state() == LoadState::Loaded) {
        if engine.next_task_at().is_none() {
            woken
                .recv_timeout(Duration::from_secs(30))
                .expect("the page finishes");
        }
        engine.poll_network();
        engine.run_tasks();
    }
    start.elapsed()
}

/// A jar of 3,000 cookies over [`DOMAINS`] domains, and the time to
/// retrieve one domain's `Cookie` header, and to store one more, each
/// the median of [`LOOKUPS`].
fn jar() -> (usize, Duration, Duration) {
    let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let mut jar = Jar::new();
    let url = |d: usize| Url::parse(&format!("https://d{d}.example.com/a/b")).expect("a URL");
    for n in 0..MAX_COOKIES {
        let line = format!("c{n}=v{n}; Path=/{}", if n % 2 == 0 { "a" } else { "" });
        jar.store(
            line.as_bytes(),
            &url(n % DOMAINS),
            Api::Http,
            Context::SAME_SITE,
            now,
        );
    }
    let kept = jar.cookies().len();
    let target = url(7);
    let mut reads = Vec::new();
    for _ in 0..LOOKUPS {
        let start = Instant::now();
        black_box(jar.header(&target, Api::Http, Context::SAME_SITE, now));
        reads.push(start.elapsed());
    }
    let mut stores = Vec::new();
    for n in 0..LOOKUPS {
        let line = format!("s{n}=1");
        let start = Instant::now();
        black_box(jar.store(line.as_bytes(), &target, Api::Http, Context::SAME_SITE, now));
        stores.push(start.elapsed());
    }
    reads.sort();
    stores.sort();
    (kept, reads[LOOKUPS / 2], stores[LOOKUPS / 2])
}

fn main() {
    let server = server();
    for (path, what) in [
        ("/none.html", "the page alone"),
        ("/fetches.html", "1,000 fetch()es"),
        ("/xhr.html", "one XMLHttpRequest"),
        ("/sync.html", "one synchronous XMLHttpRequest"),
    ] {
        // One run first, to warm the server's connections.
        run(&server, path);
        let mut times: Vec<f64> = (0..RUNS).map(|_| ms(run(&server, path))).collect();
        println!(
            "{what}, navigate to the last promise: {}",
            spread(&mut times)
        );
    }
    let (kept, read, store) = jar();
    println!(
        "jar of {kept}: a Cookie header {:.2} us, a store {:.2} us (medians of {LOOKUPS})",
        read.as_secs_f64() * 1e6,
        store.as_secs_f64() * 1e6
    );
}
