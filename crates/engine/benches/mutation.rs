//! M13.6 acceptance: a page of 1,000 paragraphs at 1280 x 800 is drawn,
//! then 1,000 paragraphs are appended to it one a frame, then 1,000
//! more in a single frame. Each part reports its frames' milliseconds,
//! whole and stage by stage (`Engine::frame_stats`, M13.4), the frames
//! over budget, and what the incremental engine did: the element styles
//! restyles computed (M13.1) and the boxes laid out and taken from the
//! layout cache (M13.2). An appended paragraph must cost the boxes it
//! reaches, not the document: the claim of M13.6, which the engine's
//! tests hold too.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench mutation
//! ```

#![forbid(unsafe_code)]

use std::hint::black_box;
use std::time::Duration;

use dom::Document;
use engine::{Engine, FRAME_BUDGET, FrameStats, FrameTime};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const PARAGRAPHS: usize = 1000;

/// The text of paragraph `i`: a sentence or two, as the page's are.
fn sentence(i: usize) -> String {
    format!(
        "Paragraph {i}. The quick brown fox jumps over the lazy dog; \
         pack my box with five dozen liquor jugs."
    )
}

/// A page of `n` paragraphs.
fn page(n: usize) -> String {
    let mut source =
        String::from("<style>body { margin: 16px; } p { margin: 0 0 12px 0; }</style><body>");
    for i in 0..n {
        source.push_str(&format!("<p>{}</p>", sentence(i)));
    }
    source.push_str("</body>");
    source
}

/// Appends paragraph `i` to `body`.
fn append(document: &mut Document, body: dom::NodeId, i: usize) {
    let p = document.create_element("p").expect("an element");
    let text = document.create_text(&sentence(i)).expect("a text node");
    document.append_child(p, text).expect("appended");
    document.append_child(body, p).expect("appended");
}

/// The engine's counters of incremental work.
#[derive(Clone, Copy)]
struct Work {
    restyled: u64,
    laid_out: u64,
    reused: u64,
    layouts: u64,
    cascades: u64,
}

impl Work {
    fn of(engine: &Engine) -> Self {
        Work {
            restyled: engine.restyled_elements(),
            laid_out: engine.boxes_laid_out(),
            reused: engine.boxes_reused(),
            layouts: engine.layout_runs(),
            cascades: engine.style_runs(),
        }
    }

    /// What was done between `self` and `later`, as one line.
    fn since(self, later: Work) -> String {
        format!(
            "{} layouts, {} whole cascades, {} element styles restyled, \
             {} boxes laid out, {} taken from the cache",
            later.layouts - self.layouts,
            later.cascades - self.cascades,
            later.restyled - self.restyled,
            later.laid_out - self.laid_out,
            later.reused - self.reused
        )
    }
}

/// Prints the median and worst of each stage over `stats`, and the
/// frames over budget.
fn report(stats: &[FrameStats]) {
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
        "  over the {:.1} ms budget: {} of {}",
        FRAME_BUDGET.as_secs_f64() * 1000.0,
        stats.iter().filter(|s| s.overran).count(),
        stats.len()
    );
}

fn main() {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    engine.load_document(&page(PARAGRAPHS));
    let mut frame = 0u32;
    let mut draw = |engine: &mut Engine| {
        frame += 1;
        black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET * frame)));
        engine.frame_stats()
    };

    let before = Work::of(&engine);
    let first = draw(&mut engine);
    println!("the first frame of {PARAGRAPHS} paragraphs:");
    report(&[first]);
    println!("  {}", before.since(Work::of(&engine)));

    let body = engine
        .document()
        .subtree(engine.document().root())
        .find(|&id| engine.document().tag(id) == Some("body"))
        .expect("the page has a body");

    println!("{PARAGRAPHS} paragraphs appended, one a frame:");
    let before = Work::of(&engine);
    let mut stats = Vec::with_capacity(PARAGRAPHS);
    for i in 0..PARAGRAPHS {
        engine.mutate(|document| append(document, body, PARAGRAPHS + i));
        stats.push(draw(&mut engine));
    }
    report(&stats);
    println!("  {}", before.since(Work::of(&engine)));

    println!("{PARAGRAPHS} more appended in one frame:");
    let before = Work::of(&engine);
    engine.mutate(|document| {
        for i in 0..PARAGRAPHS {
            append(document, body, 2 * PARAGRAPHS + i);
        }
    });
    let burst = draw(&mut engine);
    report(&[burst]);
    println!("  {}", before.since(Work::of(&engine)));
}
