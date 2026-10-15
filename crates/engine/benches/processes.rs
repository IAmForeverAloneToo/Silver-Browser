//! M34's: what the renderers cost (docs/design/processes.md, "Limits,
//! tests and measures"). The same pages shown by one engine and by a tab
//! of renderers (threads, the protocol and its pipes the processes'):
//! a renderer's start with a page's first load; a page of 500
//! paragraphs to its first frame; 100 fetches made one after another; a
//! thousand reads of `document.cookie`, each a call to the browser; a
//! click to the frame that shows what it changed; and, in the tab, a
//! navigation handed over to another site's renderer beside one that
//! stays. A renderer process's own start is the shell's
//! `--check-processes`, whose lines say it.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench processes
//! ```

#![forbid(unsafe_code)]

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use engine::service::{RemoteTab, ThreadLauncher};
use engine::{Button, Engine, FrameTime, LoadState, Url, View};
use net::test_server::{Reply, TestServer};

const RUNS: usize = 15;
const WAIT: Duration = Duration::from_secs(30);
const SIZE: (u32, u32) = (800, 600);

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

fn fonts() -> text::FontCollection {
    text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
}

fn server() -> TestServer {
    let mut long = String::from("<title>done</title>");
    for i in 0..500 {
        long.push_str(&format!(
            "<p>Paragraph {i}, of a page long enough to lay out.</p>"
        ));
    }
    TestServer::plain(move |request| {
        let host = request.header("Host").unwrap_or_default().to_owned();
        let port = host.rsplit(':').next().unwrap_or_default().to_owned();
        let body = match request.target.as_str() {
            "/small" => "<title>done</title><p>small</p>".to_owned(),
            "/long" => long.clone(),
            "/fetches" => "<title>-</title><script>(async () => { \
                 for (let i = 0; i < 100; i++) await (await fetch('/data')).text(); \
                 document.title = 'done'; })();</script>"
                .to_owned(),
            "/cookies" => "<title>-</title><script>document.cookie = 'c=1'; let n = 0; \
                 for (let i = 0; i < 1000; i++) n += document.cookie.length; \
                 document.title = 'done';</script>"
                .to_owned(),
            "/click" => "<title>done</title><body style='margin: 0'>\
                 <div id=d style='width: 800px; height: 600px; background: rgb(0, 0, 255)'></div>\
                 <script>document.getElementById('d').onclick = e => \
                 e.target.style.background = 'rgb(255, 0, 0)';</script>"
                .to_owned(),
            "/away" => format!(
                "<title>-</title><script>location.href = 'http://localhost:{port}/small';</script>"
            ),
            "/stay" => "<title>-</title><script>location.href = '/small';</script>".to_owned(),
            _ => "x".to_owned(),
        };
        Reply::ok("text/html", body.as_bytes())
    })
}

fn url(server: &TestServer, host: &str, path: &str) -> Url {
    Url::parse(&format!("http://{host}:{}{path}", server.port())).expect("a URL")
}

/// Takes what `view` does until `done` holds of it; how long that took.
fn until<V: View>(view: &mut V, mut done: impl FnMut(&mut V) -> bool) -> Duration {
    let began = Instant::now();
    while !done(view) {
        view.poll_network();
        view.run_tasks();
        assert!(began.elapsed() < WAIT, "{:?}", view.load_state());
        std::thread::yield_now();
    }
    began.elapsed()
}

fn loaded<V: View>(view: &mut V, url: &Url) -> bool {
    view.url() == Some(url) && *view.load_state() == LoadState::Loaded && view.title() == "done"
}

/// One engine, scripts on.
fn engine() -> Engine {
    let mut engine = Engine::with_fonts(fonts());
    engine.set_viewport(SIZE.0, SIZE.1);
    engine.set_scripting(true);
    engine
}

/// A tab of renderers on threads, scripts on, no spare.
fn tab() -> RemoteTab {
    let launcher = Arc::new(ThreadLauncher::with_fonts(fonts()));
    let mut tab = RemoteTab::with_cap(launcher, fonts(), 16, false);
    tab.set_viewport(SIZE.0, SIZE.1);
    tab.set_scripting(true);
    tab.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO));
    tab
}

/// `path` loaded in a new view `make` makes, `RUNS` times: the time from
/// the navigation to its title.
fn load<V: View>(server: &TestServer, path: &str, make: impl Fn() -> V) -> String {
    load_at(server, "127.0.0.1", path, make)
}

/// The same, the page at `host`.
fn load_at<V: View>(server: &TestServer, host: &str, path: &str, make: impl Fn() -> V) -> String {
    let mut times = Vec::new();
    for _ in 0..support::rounds(RUNS) {
        let mut view = make();
        let target = url(server, host, path);
        view.navigate(target.clone());
        times.push(ms(until(&mut view, |view| loaded(view, &target))));
    }
    spread(&mut times)
}

/// Whether the top left pixel of `view`'s frame at the page's size is
/// `pixel`.
fn shows<V: View>(view: &mut V, pixel: [u8; 4]) -> bool {
    view.frame_at(SIZE.0, SIZE.1, FrameTime::at(Duration::ZERO))
        .get(..4)
        == Some(&pixel[..])
}

/// A click on the page to the frame that shows its effect.
fn click<V: View>(server: &TestServer, make: impl Fn() -> V) -> String {
    let mut times = Vec::new();
    for _ in 0..support::rounds(RUNS) {
        let mut view = make();
        let target = url(server, "127.0.0.1", "/click");
        view.navigate(target.clone());
        until(&mut view, |view| {
            loaded(view, &target) && shows(view, [0, 0, 255, 255])
        });
        let began = Instant::now();
        view.pointer_move(100.0, 100.0);
        view.pointer_down(100.0, 100.0, Button::Primary, 1);
        view.pointer_up(100.0, 100.0, Button::Primary, 1);
        until(&mut view, |view| shows(view, [255, 0, 0, 255]));
        times.push(ms(began.elapsed()));
    }
    spread(&mut times)
}

/// A page of `from` that goes to the small page: of another site, handed
/// over, or of its own.
fn navigation(server: &TestServer, from: &str, host: &str) -> String {
    let mut times = Vec::new();
    for _ in 0..support::rounds(RUNS) {
        let mut view = tab();
        let target = url(server, host, "/small");
        view.navigate(url(server, "127.0.0.1", from));
        times.push(ms(until(&mut view, |view| loaded(view, &target))));
    }
    spread(&mut times)
}

fn main() {
    let server = server();
    println!("processes ({RUNS} runs each, one engine | a tab of renderers):");
    for (what, path) in [
        ("a small page, from a new view", "/small"),
        ("500 paragraphs to their title", "/long"),
        ("100 fetches one after another", "/fetches"),
        ("1000 reads of document.cookie", "/cookies"),
    ] {
        println!(
            "  {what}: {} | {}",
            load(&server, path, engine),
            load(&server, path, tab)
        );
    }
    println!(
        "  a click to the frame that shows it: {} | {}",
        click(&server, engine),
        click(&server, tab)
    );
    // `localhost` is [::1] first, where the server does not listen: the
    // connection refused there costs what this says, on Windows.
    println!(
        "  a small page at localhost, one engine: {}",
        load_at(&server, "localhost", "/small", engine)
    );
    println!(
        "  a navigation to another site (localhost), handed over: {}",
        navigation(&server, "/away", "localhost")
    );
    println!(
        "  a navigation that stays in its renderer: {}",
        navigation(&server, "/stay", "127.0.0.1")
    );
}
