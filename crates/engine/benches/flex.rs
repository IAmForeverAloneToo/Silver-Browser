//! M17's benchmark (the flexbox note, "Performance and fuzzing"): what
//! flex layout costs a page. Three measures, at 1280 x 800:
//!
//! - a grid of 1,000 cards in a wrapping row, each card a column of a
//!   title, a paragraph and a row of actions: its first frame;
//! - the same grid after one card's text changes, twenty times: the
//!   frame that lays it out again, and how much of it the layout cache
//!   keeps;
//! - chains of 8 to 64 flex containers nested in one another, rows and
//!   columns by turns, each beside a paragraph and grown into what that
//!   leaves: an item is laid out more than once, to measure it and at
//!   its flexed size, and its content widths are measured for its
//!   automatic minimum size. Without the item layouts the pass keeps a
//!   chain would cost two to the power of its depth, and without the
//!   content widths it keeps the square of it. Its time must grow
//!   linearly with the depth: twice as deep, twice as long, give or take.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench flex
//! ```

#![forbid(unsafe_code)]

use std::hint::black_box;
use std::time::Duration;

use engine::{Engine, FRAME_BUDGET, FrameStats, FrameTime};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const CARDS: usize = 1000;
const CHANGES: usize = 20;
const DEPTHS: [usize; 4] = [8, 16, 32, 64];
const RUNS: usize = 5;

/// The grid of `n` cards.
fn grid(n: usize) -> String {
    let mut source = String::from(
        "<style>\
         body { margin: 0; }\
         .grid { display: flex; flex-wrap: wrap; gap: 12px; padding: 12px; }\
         .card { display: flex; flex-direction: column; flex: 1 1 200px; \
                 padding: 8px; border: 1px solid #d0d7de; }\
         .card h2 { font-size: 16px; margin: 0 0 4px; }\
         .card p { font-size: 13px; margin: 0 0 8px; }\
         .actions { display: flex; justify-content: space-between; margin-top: auto; }\
         </style><div class=grid>",
    );
    for i in 0..n {
        source.push_str(&format!(
            "<div class=card><h2>Card {i}</h2>\
             <p>A card of the grid, its text wrapping in the room its line leaves it.</p>\
             <div class=actions><span>{i} min</span><span>Open</span></div></div>"
        ));
    }
    source.push_str("</div>");
    source
}

/// A chain of `depth` flex containers, rows and columns by turns, each
/// beside a paragraph.
fn chain(depth: usize) -> String {
    let mut source = String::from(
        "<style>\
         body { margin: 0; }\
         .row { display: flex; }\
         .column { display: flex; flex-direction: column; }\
         .row, .column { flex: 1; padding: 1px; }\
         p { margin: 0; }\
         </style>",
    );
    for level in 0..depth {
        let kind = if level % 2 == 0 { "row" } else { "column" };
        source.push_str(&format!("<div class={kind}><p>Level {level}</p>"));
    }
    for _ in 0..depth {
        source.push_str("</div>");
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
    // The grid, drawn first, then a card changed at a time.
    let mut engine = new_engine();
    engine.load_document(&grid(CARDS));
    let mut frame = 0u32;
    let mut draw = |engine: &mut Engine| {
        frame += 1;
        black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET * frame)));
        engine.frame_stats()
    };
    let laid_out = engine.boxes_laid_out();
    let first = draw(&mut engine);
    println!("the first frame of a grid of {CARDS} cards:");
    report(&[first]);
    println!("  {} boxes laid out", engine.boxes_laid_out() - laid_out);

    let titles: Vec<dom::NodeId> = {
        let document = engine.document();
        document
            .subtree(document.root())
            .filter(|&id| document.tag(id) == Some("h2"))
            .filter_map(|id| document.first_child(id))
            .collect()
    };
    println!("{CHANGES} frames, each after one card's title changes:");
    let (laid_out, reused) = (engine.boxes_laid_out(), engine.boxes_reused());
    let mut stats = Vec::with_capacity(CHANGES);
    for i in 0..CHANGES {
        let title = titles[(i * 37) % titles.len()];
        engine.mutate(|document| {
            document
                .set_text(title, &format!("Card, changed {i}"))
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

    // The chains: a fresh engine for each run, so that the layout is
    // the pass's own and not the cache's.
    println!("chains of nested flex containers, layout of the first frame, median of {RUNS}:");
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
