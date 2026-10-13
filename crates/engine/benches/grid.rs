//! M18's benchmark (the grid note, "Performance and fuzzing"): what grid
//! layout costs a page. Four measures, at 1280 x 800:
//!
//! - a gallery of 1,000 figures, a picture and a caption each,
//!   auto-placed in `repeat(auto-fill, minmax(160px, 1fr))` columns: its
//!   first frame;
//! - the same gallery after one caption changes, twenty times: the frame
//!   that lays it out again, and how much of it the layout cache keeps;
//! - a dashboard of 200 cards of one to three columns and one or two
//!   rows, in `auto` and `fr` tracks: its first frame, in which the
//!   spanning items go through the spanning steps of §12.5;
//! - chains of 8 to 64 grids nested in one another, each in the `auto`
//!   column of the one above, beside a paragraph in its `1fr` column: an
//!   item in an `auto` track is measured for its contributions to the
//!   columns, laid out to size the rows and laid out again in its area.
//!   Without the measures and layouts the pass keeps, a chain would cost
//!   a power of its depth. Its time must grow linearly with the depth:
//!   twice as deep, twice as long, give or take.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench grid
//! ```

#![forbid(unsafe_code)]

use std::hint::black_box;
use std::time::Duration;

use engine::{Engine, FRAME_BUDGET, FrameStats, FrameTime};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const FIGURES: usize = 1000;
const CARDS: usize = 200;
const CHANGES: usize = 20;
const DEPTHS: [usize; 4] = [8, 16, 32, 64];
const RUNS: usize = 5;

/// The gallery of `n` figures.
fn gallery(n: usize) -> String {
    let mut source = String::from(
        "<style>\
         body { margin: 0; }\
         .gallery { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); \
                    gap: 12px; padding: 12px; }\
         figure { display: grid; grid-template-rows: 100px auto; margin: 0; }\
         .picture { background-color: #3b5f7c; }\
         figcaption { padding: 4px 6px; font-size: 13px; }\
         </style><div class=gallery>",
    );
    for i in 0..n {
        source.push_str(&format!(
            "<figure><div class=picture></div>\
             <figcaption>Picture {i}, a caption that wraps in its column</figcaption></figure>"
        ));
    }
    source.push_str("</div>");
    source
}

/// The dashboard of `n` cards.
fn dashboard(n: usize) -> String {
    let mut source = String::from(
        "<style>\
         body { margin: 0; }\
         .dashboard { display: grid; grid-template-columns: auto repeat(4, 1fr) auto; \
                      grid-auto-flow: dense; gap: 8px; padding: 8px; }\
         .card { padding: 8px; border: 1px solid #d0d7de; }\
         .card h2 { font-size: 15px; margin: 0 0 4px; }\
         .card p { font-size: 13px; margin: 0; }\
         .w2 { grid-column: span 2; }\
         .w3 { grid-column: span 3; }\
         .t2 { grid-row: span 2; }\
         </style><div class=dashboard>",
    );
    for i in 0..n {
        let wide = ["", " w2", " w3"][i % 3];
        let tall = ["", " t2"][(i / 3) % 2];
        source.push_str(&format!(
            "<section class=\"card{wide}{tall}\"><h2>Card {i}</h2>\
             <p>A card of the dashboard, its text measured for the tracks it spans.</p></section>"
        ));
    }
    source.push_str("</div>");
    source
}

/// A chain of `depth` grids, each in the `auto` column of the one
/// above, beside a paragraph.
fn chain(depth: usize) -> String {
    let mut source = String::from(
        "<style>\
         body { margin: 0; }\
         .g { display: grid; grid-template-columns: auto 1fr; padding: 1px; }\
         p { margin: 0; }\
         </style>",
    );
    for _ in 0..depth {
        source.push_str("<div class=g>");
    }
    for level in (0..depth).rev() {
        source.push_str(&format!("<p>Level {level}</p></div>"));
    }
    source
}

fn new_engine() -> Engine {
    Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    )
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// The median and the worst of `times`, in milliseconds.
fn spread(mut times: Vec<f64>) -> (f64, f64) {
    times.sort_by(f64::total_cmp);
    (times[times.len() / 2], times[times.len() - 1])
}

/// Prints the median and worst of each stage over `stats`.
fn report(stats: &[FrameStats]) {
    let stage = |name: &str, of: fn(&FrameStats) -> Duration| {
        let (median, worst) = spread(stats.iter().map(|s| ms(of(s))).collect());
        println!("  {name}: median {median:.2} ms, worst {worst:.2} ms");
    };
    stage("style", |s| s.style);
    stage("layout", |s| s.layout);
    stage("paint", |s| s.paint);
    stage("raster", |s| s.raster);
    stage("frame", |s| s.total);
    println!(
        "  over the {:.1} ms budget: {} of {}",
        ms(FRAME_BUDGET),
        stats.iter().filter(|s| s.overran).count(),
        stats.len()
    );
}

fn main() {
    // The gallery, drawn first, then a caption changed at a time.
    let mut engine = new_engine();
    engine.load_document(&gallery(FIGURES));
    let mut frame = 0u32;
    let mut draw = |engine: &mut Engine| {
        frame += 1;
        black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET * frame)));
        engine.frame_stats()
    };
    let laid_out = engine.boxes_laid_out();
    let first = draw(&mut engine);
    println!("the first frame of a gallery of {FIGURES} figures:");
    report(&[first]);
    println!("  {} boxes laid out", engine.boxes_laid_out() - laid_out);

    let captions: Vec<dom::NodeId> = {
        let document = engine.document();
        document
            .subtree(document.root())
            .filter(|&id| document.tag(id) == Some("figcaption"))
            .filter_map(|id| document.first_child(id))
            .collect()
    };
    println!("{CHANGES} frames, each after one caption changes:");
    let (laid_out, reused) = (engine.boxes_laid_out(), engine.boxes_reused());
    let mut stats = Vec::with_capacity(CHANGES);
    for i in 0..CHANGES {
        let caption = captions[(i * 37) % captions.len()];
        engine.mutate(|document| {
            document
                .set_text(caption, &format!("Picture, its caption changed {i}"))
                .expect("a text node");
        });
        stats.push(draw(&mut engine));
    }
    report(&stats);
    println!(
        "  per frame, {} boxes laid out and {} taken from the cache",
        (engine.boxes_laid_out() - laid_out) / CHANGES as u64,
        (engine.boxes_reused() - reused) / CHANGES as u64
    );

    // The dashboard: a fresh engine for each run.
    println!("the first frame of a dashboard of {CARDS} cards, median of {RUNS}:");
    let source = dashboard(CARDS);
    let mut stats = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let mut engine = new_engine();
        engine.load_document(&source);
        black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET)));
        stats.push(engine.frame_stats());
    }
    report(&stats);

    // The chains: a fresh engine for each run, so that the layout is
    // the pass's own and not the cache's.
    println!("chains of nested grids, layout of the first frame, median of {RUNS}:");
    let mut per_level = Vec::new();
    for depth in DEPTHS {
        let source = chain(depth);
        let mut times = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let mut engine = new_engine();
            engine.load_document(&source);
            black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET)));
            times.push(ms(engine.frame_stats().layout));
        }
        let (median, worst) = spread(times);
        per_level.push(median / depth as f64);
        println!(
            "  {depth} deep: median {median:.2} ms, worst {worst:.2} ms, {:.3} ms a level",
            median / depth as f64
        );
    }
    let (least, most) = per_level
        .iter()
        .fold((f64::MAX, 0.0f64), |(l, m), &v| (l.min(v), m.max(v)));
    println!(
        "  a level costs from {least:.3} to {most:.3} ms, {:.1} times apart",
        most / least
    );
}
