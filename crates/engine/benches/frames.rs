//! M33's acceptance (docs/design/hardening.md, "Tests and measures"): a
//! page of twenty `srcdoc` frames, each a document of its own with a
//! grid of boxes and a paragraph, from its load to its first frame, all
//! twenty loaded and drawn, at a display scale of 1 and of 2; the page
//! scrolled by a frame's height, to its next frame; and the window made
//! narrower, so that the frames move, to its next frame.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench frames
//! ```

#![forbid(unsafe_code)]

mod support;

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use engine::{Engine, LoadState};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
/// The rounds each measure takes the median of.
const ROUNDS: usize = 10;
/// The frames of the page.
const FRAMES: usize = 20;

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

/// The page: twenty frames of 300 by 200, four to a row at 1280, each a
/// document of twelve coloured boxes in a grid and a paragraph.
fn page() -> String {
    let mut page = String::from(
        "<!doctype html><style>body { margin: 8px } \
         iframe { width: 300px; height: 200px; margin: 0 4px 4px 0 }</style>",
    );
    for i in 0..support::frames(FRAMES as u32) as usize {
        let mut boxes = String::new();
        for j in 0..12 {
            boxes.push_str(&format!(
                "<div style='height: 24px; background: hsl({} 60% 60%)'></div>",
                (i * 37 + j * 23) % 360
            ));
        }
        page.push_str(&format!(
            "<iframe srcdoc=\"<body style='margin: 8px; background: hsl({} 30% 95%)'>\
             <div style='display: grid; grid-template-columns: repeat(4, 1fr); gap: 4px'>{boxes}</div>\
             <p>Frame {i}: a paragraph of a few words that wraps once or twice in its box.</p>\"></iframe>",
            (i * 53) % 360
        ));
    }
    page
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

/// The page loaded in a new engine at `scale`, its frames too, and its
/// first frame drawn.
fn loaded(scale: f32) -> (Engine, Receiver<()>) {
    let (mut engine, woken) = engine(scale);
    engine.load_document(&page());
    wait(&mut engine, &woken, |engine| {
        engine.load_state() == &LoadState::Loaded && engine.frames_settled()
    });
    std::hint::black_box(engine.frame(size(WIDTH, scale), size(HEIGHT, scale)));
    (engine, woken)
}

fn size(css: u32, scale: f32) -> u32 {
    (css as f32 * scale) as u32
}

/// The median of `times`, in milliseconds.
fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

/// The page from its load to its first frame, its frames loaded and
/// drawn: the median of the rounds.
fn first_frame(scale: f32) {
    let times = (0..support::rounds(ROUNDS))
        .map(|_| {
            let start = Instant::now();
            std::hint::black_box(loaded(scale));
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    println!(
        "  twenty frames at {scale}, the load to the first frame: median {:.2} ms",
        median(times)
    );
}

/// The page scrolled down and back by a frame's height, each to its
/// frame: the median of forty.
fn scroll() {
    let (mut engine, _woken) = loaded(1.0);
    let times = (0..support::rounds(ROUNDS) * 4)
        .map(|i| {
            let y = if i % 2 == 0 { 204.0 } else { 0.0 };
            let start = Instant::now();
            engine.set_scroll(y);
            std::hint::black_box(engine.frame(WIDTH, HEIGHT));
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    println!(
        "  twenty frames scrolled by a frame's height to the next frame: median {:.2} ms",
        median(times)
    );
}

/// The window made narrower and wider again, three frames to a row
/// and four, each to its frame: the median of forty.
fn resize() {
    let (mut engine, _woken) = loaded(1.0);
    let times = (0..support::rounds(ROUNDS) * 4)
        .map(|i| {
            let width = if i % 2 == 0 { 1000 } else { WIDTH };
            let start = Instant::now();
            engine.set_viewport(width, HEIGHT);
            std::hint::black_box(engine.frame(width, HEIGHT));
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    println!(
        "  twenty frames, the window's width changed to the next frame: median {:.2} ms",
        median(times)
    );
}

fn main() {
    for scale in [1.0, 2.0] {
        first_frame(scale);
    }
    scroll();
    resize();
}
