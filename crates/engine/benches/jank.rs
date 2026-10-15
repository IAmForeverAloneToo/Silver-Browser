//! M35.1: the jank benchmark (`docs/design/performance.md`, "What is
//! measured"). Ten seconds of a page twenty viewports long scrolled by
//! the wheel, down and back, while three animations run in a fixed
//! band at its top: a spinner turned (composited), a badge's colour
//! fading (a repaint) and a bar's width growing (a layout). Every frame
//! is drawn at its sixtieth of a second, and the frames over the 16.7ms
//! budget and over 20ms are counted, the worst kept.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench jank
//! ```

#![forbid(unsafe_code)]

mod support;

use std::hint::black_box;
use std::time::Duration;

use engine::{Engine, FRAME_BUDGET, FrameStats, FrameTime};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const FRAMES: u32 = 600;
const WHEEL_PX: f32 = 120.0;
const JANK: Duration = Duration::from_millis(20);

fn page() -> String {
    let mut source = String::from(
        "<style>\
           body { margin: 0; font: 16px/1.5 sans-serif }\
           .band { position: fixed; top: 0; left: 0; right: 0; height: 64px; background: #fff;\
             border-bottom: 1px solid #ccc; display: flex; align-items: center; gap: 24px; padding: 0 24px }\
           .spinner { width: 32px; height: 32px; border: 4px solid #4a90d9; border-right-color: transparent;\
             border-radius: 50%; animation: spin 1s linear infinite }\
           .badge { padding: 4px 12px; border-radius: 12px; animation: fade 2s ease-in-out infinite alternate }\
           .bar { height: 12px; background: #7ed321; animation: grow 3s ease-in-out infinite alternate }\
           @keyframes spin { to { transform: rotate(360deg) } }\
           @keyframes fade { from { background-color: #ffd814 } to { background-color: #d0021b } }\
           @keyframes grow { from { width: 40px } to { width: 400px } }\
           main { padding: 80px 24px 24px }\
           p { margin: 0 0 12px }\
         </style><div class=band><div class=spinner></div><span class=badge>Live</span>\
         <div class=bar></div></div><main>",
    );
    for i in 0..400 {
        if i % 10 == 0 {
            source.push_str(&format!("<h2>Section {}</h2>", i / 10 + 1));
        }
        source.push_str(&format!(
            "<p>Paragraph {i}. The quick brown fox jumps over the lazy dog; pack my box with \
             five dozen liquor jugs. Sphinx of black quartz, judge my vow.</p>"
        ));
    }
    source.push_str("</main>");
    source
}

fn main() {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    engine.load_document(&page());
    black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(Duration::ZERO)));
    let frames = support::frames(FRAMES);
    let mut stats: Vec<FrameStats> = Vec::new();
    let mut deepest: f32 = 0.0;
    for i in 1..=frames {
        // Down for the first half, back up for the second.
        let dy = if i <= frames / 2 { WHEEL_PX } else { -WHEEL_PX };
        if i % 4 == 0 {
            engine.wheel(WIDTH as f32 / 2.0, HEIGHT as f32 / 2.0, dy);
        }
        black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET * i)));
        stats.push(engine.frame_stats());
        deepest = deepest.max(engine.scroll_y());
    }
    let over = stats.iter().filter(|s| s.overran).count();
    let janked = stats.iter().filter(|s| s.total > JANK).count();
    println!(
        "{frames} frames of a page scrolled to {deepest:.0}px and back, three animations running:"
    );
    let stage = |name: &str, of: fn(&FrameStats) -> Duration| {
        let mut times: Vec<f64> = stats.iter().map(|s| of(s).as_secs_f64() * 1000.0).collect();
        times.sort_by(f64::total_cmp);
        println!(
            "  {name}: median {:.2} ms, worst {:.2} ms",
            times[times.len() / 2],
            times[times.len() - 1]
        );
    };
    stage("style", |s| s.style);
    stage("layout", |s| s.layout);
    stage("paint", |s| s.paint);
    stage("raster", |s| s.raster);
    stage("frame", |s| s.total);
    println!(
        "  over the {:.1} ms budget: {over} of {frames}; over {} ms: {janked}",
        FRAME_BUDGET.as_secs_f64() * 1000.0,
        JANK.as_millis()
    );
}
