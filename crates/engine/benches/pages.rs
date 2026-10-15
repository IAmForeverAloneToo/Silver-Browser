//! M35.1: the page-load benchmark (`docs/design/performance.md`, "What
//! is measured"). Nineteen pages, each of a class the web is made of,
//! written for it into `benches/pages/` with their sheets and images,
//! and the browser's own `about:silver`: each loaded by a new engine
//! from a local server to its first frame at 1280 by 800, scripts on,
//! the bundled font. It prints the time to the page shown, every sheet,
//! image, script and frame of it in, and the first frame by stage. A
//! maintainer's own saved pages are measured the same way, after the
//! twenty, from the folder `SILVER_PAGES` names; the pages here are
//! written, not saved, since a saved page is its owner's work.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench pages
//! ```

#![forbid(unsafe_code)]

mod support;

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use engine::{Engine, FrameStats, FrameTime, LoadState, Url};
use net::test_server::{Reply, TestServer};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const RUNS: usize = 5;

/// What a file of the pages' folder is served as.
fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css",
        Some("js") => "text/javascript",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

/// A server of the files under `root`, a request's query left out; no
/// path may climb out of it.
fn serve(root: PathBuf) -> TestServer {
    TestServer::plain(move |request| {
        let path = request.target.split(['?', '#']).next().unwrap_or("");
        let path = path.trim_start_matches('/');
        let file = root.join(path);
        if path.split('/').any(|part| part == "..") || !file.is_file() {
            return Reply::with_fields(404, &[("Content-Type", "text/plain")], b"not here");
        }
        match std::fs::read(&file) {
            Ok(bytes) => Reply::ok(content_type(&file), &bytes),
            Err(_) => Reply::with_fields(500, &[("Content-Type", "text/plain")], b"unread"),
        }
    })
}

/// The `.html` files of `root`, by name.
fn pages_in(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".html"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// A new engine over the bundled font, scripts on, and the channel its
/// waker sends on.
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

/// One run: a new engine loads `url` and draws its first frame. The
/// time until the page was shown, all of it in; the first frame's
/// stats; the time from the navigation to the frame's end.
fn run(url: &Url) -> (Duration, FrameStats, Duration) {
    let (mut engine, woken) = engine();
    let start = Instant::now();
    engine.navigate(url.clone());
    while *engine.load_state() == LoadState::Loading {
        woken
            .recv_timeout(Duration::from_secs(30))
            .unwrap_or_else(|_| panic!("{url} loads"));
        engine.poll_network();
        engine.run_tasks();
    }
    assert_eq!(engine.load_state(), &LoadState::Loaded, "{url}");
    let shown = start.elapsed();
    black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(Duration::ZERO)));
    (shown, engine.frame_stats(), start.elapsed())
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

/// Measures `url`, `bytes` long, and prints a line of it.
fn measure(name: &str, url: &Url, bytes: usize) -> f64 {
    let runs: Vec<_> = (0..support::rounds(RUNS)).map(|_| run(url)).collect();
    let shown = median(runs.iter().map(|(shown, ..)| ms(*shown)).collect());
    let stage = |of: fn(&FrameStats) -> Duration| {
        median(runs.iter().map(|(_, stats, _)| ms(of(stats))).collect())
    };
    let whole = median(runs.iter().map(|(.., whole)| ms(*whole)).collect());
    println!(
        "  {name}, {} KB: shown {shown:.1} ms; first frame {:.1} ms (style {:.1}, layout {:.1}, \
         paint {:.1}, raster {:.1}); to the frame {whole:.1} ms",
        bytes / 1024,
        stage(|s| s.total),
        stage(|s| s.style),
        stage(|s| s.layout),
        stage(|s| s.paint),
        stage(|s| s.raster),
    );
    whole
}

fn measure_folder(root: &Path, label: &str) -> Vec<f64> {
    let server = serve(root.to_path_buf());
    let names = pages_in(root);
    println!(
        "{label}: {} pages, medians of {}:",
        names.len(),
        support::rounds(RUNS)
    );
    names
        .iter()
        .map(|name| {
            let bytes = std::fs::metadata(root.join(name)).map_or(0, |data| data.len() as usize);
            measure(name, &server.url(&format!("/{name}")), bytes)
        })
        .collect()
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/pages");
    let mut wholes = measure_folder(&root, "the pages written for the benchmark");
    let about = Url::parse("about:silver").expect("a URL");
    wholes.push(measure("about:silver", &about, 0));
    let mut sorted = wholes.clone();
    sorted.sort_by(f64::total_cmp);
    println!(
        "the twenty, from the navigation to the first frame: median {:.1} ms, worst {:.1} ms",
        sorted[sorted.len() / 2],
        sorted[sorted.len() - 1]
    );
    if let Some(own) = std::env::var_os("SILVER_PAGES") {
        measure_folder(Path::new(&own), "the pages of SILVER_PAGES");
    }
}
