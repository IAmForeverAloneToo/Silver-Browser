//! M32's acceptance (docs/design/svg.md, "Acceptance"): the icon demo
//! (`tests/demos/svg-icons.html`, thirty icons inline through use of
//! symbols and as img files at three sizes) and the chart demo
//! (`tests/demos/svg-chart.html`) from their load to their first frame,
//! what they draw decoded, at a display scale of 1 and of 2; a change to
//! the chart (a bar's height) to its next frame; and an SVG image drawn
//! at a new size, from the change of its width to its frame drawn at it.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench svg
//! ```

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use dom::NodeId;
use engine::{Engine, LoadState, Url};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
/// The rounds each measure takes the median of.
const ROUNDS: usize = 10;

/// An engine over the bundled font at the display scale `scale`, and
/// what wakes it.
fn engine(scale: f32) -> (Engine, Receiver<()>) {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    engine.set_scale_factor(scale);
    let (wake, woken) = std::sync::mpsc::channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    (engine, woken)
}

/// The demo `name`'s file.
fn demo(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/demos/{name}.html"))
}

/// Waits on `woken` until `done`.
fn wait(engine: &mut Engine, woken: &Receiver<()>, done: impl Fn(&Engine) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done(engine) {
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("in time");
        let _ = woken.recv_timeout(left.min(Duration::from_millis(100)));
        engine.poll_network();
    }
}

/// A frame of `engine` at its scale, drawn again until what it shows is
/// decoded: the last.
fn settled(engine: &mut Engine, woken: &Receiver<()>, scale: f32) -> Vec<u8> {
    let (w, h) = (
        (WIDTH as f32 * scale) as u32,
        (HEIGHT as f32 * scale) as u32,
    );
    loop {
        let frame = engine.frame(w, h);
        if !engine.decoding() {
            return frame;
        }
        wait(engine, woken, |engine| !engine.decoding());
    }
}

/// The median of `times`, in milliseconds.
fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

/// The demo `name` from its navigation to its first frame, decoded, at
/// `scale`: the median of the rounds.
fn first_frame(name: &str, scale: f32) {
    let times = (0..ROUNDS)
        .map(|_| {
            let (mut engine, woken) = engine(scale);
            let start = Instant::now();
            engine.navigate(Url::from_file_path(&demo(name)).expect("a path"));
            wait(&mut engine, &woken, |engine| {
                engine.load_state() == &LoadState::Loaded
            });
            std::hint::black_box(settled(&mut engine, &woken, scale));
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    println!(
        "  {name} at {scale}, its load to its first frame: median {:.2} ms",
        median(times)
    );
}

/// The chart's first bar, the element of a change.
fn first_bar(engine: &Engine) -> NodeId {
    let document = engine.document();
    document
        .subtree(document.root())
        .find(|&n| {
            document.svg_tag(n) == Some("rect")
                && document
                    .parent(n)
                    .is_some_and(|p| document.attr(p, "class") == Some("bars"))
        })
        .expect("a bar")
}

/// A bar of the chart made taller or shorter, then its frame: the median
/// of a hundred changes.
fn chart_change() {
    let (mut engine, woken) = engine(1.0);
    engine.navigate(Url::from_file_path(&demo("svg-chart")).expect("a path"));
    wait(&mut engine, &woken, |engine| {
        engine.load_state() == &LoadState::Loaded
    });
    settled(&mut engine, &woken, 1.0);
    let bar = first_bar(&engine);
    let times = (0..100)
        .map(|i| {
            let height = if i % 2 == 0 { "120" } else { "42" };
            let start = Instant::now();
            let _ = engine.mutate(|document| document.set_attribute(bar, "height", height));
            std::hint::black_box(engine.frame(WIDTH, HEIGHT));
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    println!(
        "  svg-chart, a bar's height changed to its frame: median {:.2} ms",
        median(times)
    );
}

/// An SVG image given a new width, then drawn at it: the median of the
/// rounds, its drawing at the new size included.
fn image_resize() {
    let page = Path::new(env!("CARGO_TARGET_TMPDIR")).join("svg-image-resize.html");
    let icons = Url::from_file_path(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/demos/"))
        .expect("a path");
    std::fs::write(
        &page,
        format!(
            "<!doctype html><base href='{icons}/'><body style='margin: 0'>\
             <img id=i src='icons/heart.svg' style='width: 100px'></body>"
        ),
    )
    .expect("the page is written");
    let (mut engine, woken) = engine(1.0);
    engine.navigate(Url::from_file_path(&page).expect("a path"));
    wait(&mut engine, &woken, |engine| {
        engine.load_state() == &LoadState::Loaded
    });
    settled(&mut engine, &woken, 1.0);
    let document = engine.document();
    let image = document
        .subtree(document.root())
        .find(|&n| document.attr(n, "id") == Some("i"))
        .expect("the image");
    let times = (0..ROUNDS * 4)
        .map(|i| {
            let width = format!("width: {}px", 100 + 37 * (i + 1));
            let start = Instant::now();
            let _ = engine.mutate(|document| document.set_attribute(image, "style", &width));
            std::hint::black_box(settled(&mut engine, &woken, 1.0));
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    println!(
        "  an SVG image given a new width to its frame drawn at it: median {:.2} ms",
        median(times)
    );
}

fn main() {
    for scale in [1.0, 2.0] {
        first_frame("svg-icons", scale);
        first_frame("svg-chart", scale);
    }
    chart_change();
    image_resize();
}
