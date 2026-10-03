//! M16.5: what pseudo-elements cost a page. A page of 1,000 paragraphs
//! of a few lines, each with a link and an emphasis on its first line,
//! is drawn at 1280 x 800 under each of a few sheets: none; `::before`
//! and `::after` on every paragraph; `::first-letter`; `::first-line`
//! setting inherited properties, which has the box tree cascade each
//! paragraph's inline elements again at every build; `::first-line`
//! setting a background alone, which has it cascade nothing again; and
//! all of them. For each, the first frame, and the frames that each
//! append a paragraph: the box tree is built again at each, its
//! pseudo-elements with it, while the cascade and the layout are
//! incremental.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench pseudo
//! ```

use std::hint::black_box;
use std::time::Duration;

use dom::Document;
use engine::{Engine, FRAME_BUDGET, FrameStats, FrameTime};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const PARAGRAPHS: usize = 1000;
const APPENDED: usize = 100;

/// The sheets compared, by name.
const SHEETS: [(&str, &str); 6] = [
    ("no pseudo-elements", ""),
    (
        "::before and ::after",
        "p::before { content: '> ' } p::after { content: ' <' }",
    ),
    (
        "::first-letter",
        "p::first-letter { font-size: 2em; color: #a00 }",
    ),
    (
        "::first-line, inherited properties",
        "p::first-line { color: #00a; font-size: 18px }",
    ),
    (
        "::first-line, a background",
        "p::first-line { background-color: #ffc }",
    ),
    (
        "all of them",
        "p::before { content: '> ' } p::after { content: ' <' }
         p::first-letter { font-size: 2em; color: #a00 }
         p::first-line { color: #00a; font-size: 18px; background-color: #ffc }",
    ),
];

/// A page of `n` paragraphs under `sheet`.
fn page(n: usize, sheet: &str) -> String {
    let mut source = format!(
        "<style>body {{ margin: 16px; }} p {{ margin: 0 0 12px 0; }} {sheet}</style><body>"
    );
    for i in 0..n {
        source.push_str(&format!(
            "<p>Paragraph {i}. The quick brown <a href=#p{i}>fox jumps</a> over the \
             <em>lazy dog</em>; pack my box with five dozen liquor jugs. How vexingly \
             quick daft zebras jump, and sphinx of black quartz, judge my vow. The five \
             boxing wizards jump quickly; jackdaws love my big sphinx of quartz, while \
             a wizard's job is to vex chumps quickly in fog.</p>"
        ));
    }
    source.push_str("</body>");
    source
}

/// Appends a paragraph to `body`.
fn append(document: &mut Document, body: dom::NodeId, i: usize) {
    let p = document.create_element("p").expect("an element");
    let text = document
        .create_text(&format!("Paragraph {i}, appended."))
        .expect("a text node");
    document.append_child(p, text).expect("appended");
    document.append_child(body, p).expect("appended");
}

/// The median of `of` over `stats`, in milliseconds.
fn median(stats: &[FrameStats], of: fn(&FrameStats) -> Duration) -> f64 {
    let mut times: Vec<f64> = stats.iter().map(|s| of(s).as_secs_f64() * 1000.0).collect();
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

fn main() {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    for (name, sheet) in SHEETS {
        let mut engine = Engine::with_fonts(fonts.clone());
        engine.load_document(&page(PARAGRAPHS, sheet));
        let mut frame = 0u32;
        let mut draw = |engine: &mut Engine| {
            frame += 1;
            black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET * frame)));
            engine.frame_stats()
        };
        let first = draw(&mut engine);
        let body = engine
            .document()
            .subtree(engine.document().root())
            .find(|&id| engine.document().tag(id) == Some("body"))
            .expect("the page has a body");
        let mut stats = Vec::with_capacity(APPENDED);
        for i in 0..APPENDED {
            engine.mutate(|document| append(document, body, PARAGRAPHS + i));
            stats.push(draw(&mut engine));
        }
        println!("{name}:");
        println!(
            "  the first frame: {:.1} ms, its style {:.1} ms and layout {:.1} ms",
            first.total.as_secs_f64() * 1000.0,
            first.style.as_secs_f64() * 1000.0,
            first.layout.as_secs_f64() * 1000.0
        );
        println!(
            "  a paragraph appended, median of {APPENDED}: {:.2} ms, its layout {:.2} ms",
            median(&stats, |s| s.total),
            median(&stats, |s| s.layout)
        );
    }
}
