//! M9.6 baseline: a document at least ten viewports tall, scrolled by
//! 40px a frame for 120 frames at 1280 x 800, reported as milliseconds
//! per frame. Layout runs once (M9.5) and is timed on its own; every
//! frame is paint plus raster.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, not a
//! `#[bench]`-harness target, so it needs no bench-runner dependency.
//! Run by hand and eyeballed against the milestone's budget, 60fps on
//! the debug build (16.7ms a frame):
//!
//! ```text
//! cargo bench -p engine --profile dev   # the debug build the budget names
//! cargo bench -p engine                 # the release build
//! ```
//!
//! Criterion-tracked, CI-enforced regression budgets are a T4
//! (performance track) commitment for later (M35+).

use std::hint::black_box;
use std::time::Instant;

use engine::Engine;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const FRAMES: u32 = 120;
const STEP_PX: f32 = 40.0;

/// A page of headings and paragraphs, `paragraphs` of them.
fn page(paragraphs: usize) -> String {
    let mut source = String::from(
        "<style>\
           body { margin: 16px; }\
           h2 { margin: 24px 0 8px 0; }\
           p { margin: 0 0 12px 0; }\
           .note { padding: 8px; border: 1px solid #89b4fa; background-color: #313244; }\
         </style><body>",
    );
    for i in 0..paragraphs {
        if i % 5 == 0 {
            source.push_str(&format!("<h2>Section {}</h2>", i / 5 + 1));
        }
        let class = if i % 7 == 3 { " class=note" } else { "" };
        source.push_str(&format!(
            "<p{class}>Paragraph {i}. The quick brown fox jumps over the lazy dog; \
             pack my box with five dozen liquor jugs. Sphinx of black quartz, judge my vow. \
             How vexingly quick daft zebras jump, and the five boxing wizards jump quickly.</p>"
        ));
    }
    source.push_str("</body>");
    source
}

fn main() {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);

    // Grow the page until it is at least ten viewports tall.
    let mut paragraphs = 64;
    loop {
        engine.load_document(&page(paragraphs));
        let height = engine.layout(WIDTH as f32).height;
        if height >= 10.0 * HEIGHT as f32 {
            break;
        }
        paragraphs *= 2;
    }
    // Start the caches over, so the layout is timed on its own.
    engine.load_document(&page(paragraphs));
    let start = Instant::now();
    let height = engine.layout(WIDTH as f32).height;
    let layout = start.elapsed();

    // The first frame: the glyph and shaped-run caches filling, paint,
    // raster.
    let start = Instant::now();
    black_box(engine.frame(WIDTH, HEIGHT));
    let first = start.elapsed();

    // Then scroll-only frames (M9.5): the layout is cached.
    let runs_before = engine.layout_runs();
    let start = Instant::now();
    for i in 1..=FRAMES {
        engine.set_scroll(i as f32 * STEP_PX);
        black_box(engine.frame(WIDTH, HEIGHT));
    }
    let elapsed = start.elapsed();
    let per_frame_ms = elapsed.as_secs_f64() * 1000.0 / FRAMES as f64;

    println!(
        "document: {paragraphs} paragraphs, {height:.0}px tall ({:.1} viewports of {HEIGHT}px)",
        height / HEIGHT as f32
    );
    println!("layout: {:.2} ms", layout.as_secs_f64() * 1000.0);
    println!(
        "first frame (caches filling + paint + raster): {:.2} ms",
        first.as_secs_f64() * 1000.0
    );
    println!(
        "{FRAMES} scroll frames at {WIDTH}x{HEIGHT}: {per_frame_ms:.2} ms/frame ({:.0} fps), \
         layouts run during them: {}",
        1000.0 / per_frame_ms,
        engine.layout_runs() - runs_before
    );
}
