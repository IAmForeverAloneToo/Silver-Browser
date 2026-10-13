//! M31's acceptance (docs/design/workers.md, "Acceptance"): the
//! Mandelbrot demo (`tests/demos/mandelbrot.html`) computed by four
//! workers while the page animates a square of its own, its frames drawn
//! as the shell draws them, each timed whole; and the same computed on
//! the page's own thread, a band a task, for comparison. Then what a
//! worker costs: its start to its first message, a message's round trip,
//! and a 4 MB buffer transferred there and back.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench workers
//! ```

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use engine::{Engine, FRAME_BUDGET, FrameTime, LoadState, Url};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;
/// The jank budget: no longer from one frame to the next, a tick of the
/// display missed past it.
const JANK: Duration = Duration::from_millis(20);
/// How long before a tick the frame loop stops waiting on the timer.
const SPIN: Duration = Duration::from_millis(16);

/// An engine over the bundled font, scripts on, on the software backend,
/// and the channel its waker sends on.
fn engine() -> (Engine, Receiver<()>) {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    engine.set_scripting(true);
    engine.set_viewport(WIDTH, HEIGHT);
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    (engine, woken)
}

/// The demo `name`'s file.
fn demo(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/demos/{name}.html"))
}

/// `engine` navigated to `url`, once it has loaded.
fn load(engine: &mut Engine, woken: &Receiver<()>, url: Url) {
    engine.navigate(url);
    let deadline = Instant::now() + Duration::from_secs(60);
    while engine.load_state() != &LoadState::Loaded {
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("the page loads in time");
        let _ = woken.recv_timeout(left);
        engine.poll_network();
    }
}

/// What a run of frames measured: each frame's own time, and the time
/// from each frame to the next, in milliseconds.
struct Frames {
    times: Vec<f64>,
    gaps: Vec<f64>,
}

/// Runs `engine` as the shell does, its frames at the display's ticks,
/// sixty a second on the real clock (a tick missed is skipped, not
/// caught up), what arrives and the tasks due taken between them, until
/// a console line starts with `last`: that line, and the frames.
fn frames_until(engine: &mut Engine, woken: &Receiver<()>, last: &str) -> (String, Frames) {
    let start = Instant::now();
    let deadline = start + Duration::from_secs(120);
    let mut frames = Frames {
        times: Vec::new(),
        gaps: Vec::new(),
    };
    let mut next = start;
    let mut previous: Option<Instant> = None;
    loop {
        engine.poll_network();
        engine.run_tasks();
        for entry in engine.take_console() {
            if entry.text.starts_with(last) {
                return (entry.text, frames);
            }
        }
        let now = Instant::now();
        assert!(now < deadline, "the page gets there in time");
        if now >= next {
            if let Some(previous) = previous {
                frames.gaps.push((now - previous).as_secs_f64() * 1000.0);
            }
            previous = Some(now);
            let at = FrameTime::at(now - start);
            std::hint::black_box(engine.frame_at(WIDTH, HEIGHT, at));
            frames.times.push(now.elapsed().as_secs_f64() * 1000.0);
            while next <= Instant::now() {
                next += FRAME_BUDGET;
            }
        } else if next - now > SPIN {
            // The platform's timer is coarse (15.6 ms on Windows): it
            // is waited on until shortly before the tick, the rest spun.
            let _ = woken.recv_timeout(next - now - SPIN);
        } else {
            std::thread::yield_now();
        }
    }
}

/// The median, the 95th percentile and the worst of `values`.
fn spread(values: &mut [f64]) -> (f64, f64, f64) {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    (values[n / 2], values[n * 95 / 100], values[n - 1])
}

/// The demo with `workers` workers (0: on the page's thread): how long
/// the set took, and its frames meanwhile.
fn mandelbrot(workers: u32) {
    let (mut engine, woken) = engine();
    let file = Url::from_file_path(&demo("mandelbrot")).expect("a path");
    let url = Url::parse(&format!("{file}?workers={workers}")).expect("a URL");
    load(&mut engine, &woken, url);
    let (line, mut frames) = frames_until(&mut engine, &woken, "set ");
    let missed = frames
        .gaps
        .iter()
        .filter(|&&gap| gap > JANK.as_secs_f64() * 1000.0)
        .count();
    let count = frames.times.len();
    let (time, time95, time_worst) = spread(&mut frames.times);
    let (gap, gap95, gap_worst) = spread(&mut frames.gaps);
    let what = match workers {
        0 => "on the page's thread".to_owned(),
        n => format!("by {n} workers"),
    };
    println!(
        "  the set {what}: {line}; {count} frames, each {time:.2} ms (95th {time95:.2}, worst \
         {time_worst:.2}); from one to the next {gap:.1} ms (95th {gap95:.1}, worst \
         {gap_worst:.1}), {missed} over {} ms",
        JANK.as_millis(),
    );
}

/// A page whose script `body` logs `done <ms>` when it has measured.
fn page(body: &str) -> String {
    format!(
        "<!doctype html><script>\n\
         var echo = 'data:text/javascript,' + encodeURIComponent(\
         'onmessage = function (e) {{ postMessage(e.data, e.data instanceof ArrayBuffer ? [e.data] : []); }};');\n\
         {body}\n</script>"
    )
}

/// The page `source` run until it logs `done`: the milliseconds it says.
fn measured(source: &str) -> f64 {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("workers-bench.html");
    std::fs::write(&path, source).expect("the page is written");
    let (mut engine, woken) = engine();
    load(
        &mut engine,
        &woken,
        Url::from_file_path(&path).expect("a path"),
    );
    let (line, _) = frames_until(&mut engine, &woken, "done ");
    line["done ".len()..].parse().expect("a number")
}

fn main() {
    println!("the Mandelbrot demo, 640 by 400, 200 iterations:");
    mandelbrot(4);
    mandelbrot(0);
    // A worker's start: its first message, ten workers one after another.
    let start = measured(&page(
        "var n = 0, total = 0;\n\
         var one = function () { var t = performance.now(); var w = new Worker(echo);\n\
           w.onmessage = function () { total += performance.now() - t; w.terminate();\n\
             if (++n < 10) { one(); } else { console.log('done ' + total / 10); } };\n\
           w.postMessage(1); };\n\
         one();",
    ));
    println!("  a worker's start to its first message: {start:.2} ms");
    let trip = measured(&page(
        "var w = new Worker(echo), n = 0, t;\n\
         w.onmessage = function () { if (n === 0) { t = performance.now(); }\n\
           if (++n <= 1000) { w.postMessage({ n: n, text: 'hello' }); }\n\
           else { console.log('done ' + (performance.now() - t) / 1000); } };\n\
         w.postMessage(0);",
    ));
    println!("  a message's round trip: {trip:.3} ms");
    let transfer = measured(&page(
        "var w = new Worker(echo), n = 0, t;\n\
         var send = function () { var b = new ArrayBuffer(4 << 20); w.postMessage(b, [b]); };\n\
         w.onmessage = function () { if (n === 0) { t = performance.now(); }\n\
           if (++n <= 20) { send(); } else { console.log('done ' + (performance.now() - t) / 20); } };\n\
         send();",
    ));
    println!("  4 MB transferred there and back: {transfer:.2} ms");
}
