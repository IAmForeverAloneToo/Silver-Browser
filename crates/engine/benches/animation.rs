//! M22.6 acceptance: the milestone's two demos (`tests/demos/`) at 1280
//! by 800, on the software backend and, where the machine has an
//! adapter, on the GPU's.
//!
//! - **A spinner.** A ring turned by an infinite linear animation of
//!   `rotate`, on a page of text: its frames lay nothing out and
//!   composite its layer.
//! - **A colour fade.** A card whose background and text colours
//!   alternate between two themes, among cards that stay: its frames
//!   lay out the card alone.
//!
//! Each is drawn for ten seconds of frames sixty a second through
//! `Engine::frame_at`, the time a frame leaves before its deadline spent
//! drawing ahead as the shell spends it, and every frame's time is
//! recorded: sixty frames a second is every frame within
//! `FRAME_BUDGET`, and the jank budget is no frame over 20ms.
//! `tests/demos.rs` holds what makes that so whatever the machine.
//!
//! Then the milestone's costs (`docs/design/animations.md`, "Performance
//! and fuzzing"), on the software backend, two seconds of frames each:
//!
//! - **A transform, then a colour, animated on the mutation benchmark's
//!   page of a thousand paragraphs**: one paragraph's, the first
//!   composited, the second laid out again with what it reaches.
//! - **A thousand paragraphs animated at once**: an opacity each.
//! - **`transition: all` on every paragraph and a class changed**: the
//!   cascade of the frame that changes it, against the same page
//!   without transitions.
//! - **`composite.rs`'s turned card**, turned through its `style`
//!   attribute, now composited rather than laid out.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench animation
//! ```

#![forbid(unsafe_code)]

use std::path::Path;
use std::time::{Duration, Instant};

use dom::NodeId;
use engine::{Engine, FRAME_BUDGET, FrameStats, FrameTime};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
/// Ten seconds of frames, sixty a second.
const FRAMES: u32 = 600;
/// The jank budget: no frame longer.
const JANK: Duration = Duration::from_millis(20);

/// The demo page `name`.
fn demo(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/demos/{name}.html"));
    std::fs::read_to_string(path).expect("the demo is read")
}

/// An engine over the bundled font, on the software backend, or on the
/// GPU's when `gpu` and the machine has an adapter.
fn engine(gpu: bool) -> Option<Engine> {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    if !gpu {
        return Some(Engine::with_fonts(fonts));
    }
    match render::gpu::GpuBackend::with_fonts(fonts.clone()) {
        Ok(backend) => {
            println!("on the GPU backend, {}", backend.adapter());
            Some(Engine::with_backend(fonts, Box::new(backend)))
        }
        Err(e) => {
            println!("no GPU backend here ({e}): its rounds are skipped");
            None
        }
    }
}

/// A frame drawn for frame `i` of a sixty-a-second clock, then drawing
/// ahead until its deadline, as the shell does once it presents one;
/// the frame's stages.
fn draw(engine: &mut Engine, i: u32) -> FrameStats {
    let start = Instant::now();
    std::hint::black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET * i)));
    engine.draw_ahead(start + FRAME_BUDGET);
    engine.frame_stats()
}

/// Prints the median and worst of each stage over `stats`, and the
/// frames over the frame budget and over the jank budget.
fn report(stats: &[FrameStats]) {
    let stage = |name: &str, of: fn(&FrameStats) -> Duration| {
        let mut times: Vec<f64> = stats.iter().map(|s| of(s).as_secs_f64() * 1000.0).collect();
        times.sort_by(f64::total_cmp);
        println!(
            "    {name}: median {:.2} ms, worst {:.2} ms",
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
        "    over the {:.1} ms budget: {} of {}; over the {} ms jank budget: {}",
        FRAME_BUDGET.as_secs_f64() * 1000.0,
        stats.iter().filter(|s| s.total > FRAME_BUDGET).count(),
        stats.len(),
        JANK.as_millis(),
        stats.iter().filter(|s| s.total > JANK).count()
    );
}

/// Ten seconds of the demo `name`'s frames.
fn run(gpu: bool, name: &str) {
    let Some(mut engine) = engine(gpu) else {
        return;
    };
    engine.load_document(&demo(name));
    let start = Instant::now();
    draw(&mut engine, 0);
    let first = start.elapsed();
    let (layouts, laid_out) = (engine.layout_runs(), engine.boxes_laid_out());
    let stats: Vec<FrameStats> = (1..=FRAMES).map(|i| draw(&mut engine, i)).collect();
    println!(
        "  {name}, {FRAMES} frames sixty a second: the first {:.1} ms; layouts run {}, \
         boxes laid out {}",
        first.as_secs_f64() * 1000.0,
        engine.layout_runs() - layouts,
        engine.boxes_laid_out() - laid_out
    );
    report(&stats);
}

/// Two seconds of frames.
const COST_FRAMES: u32 = 120;

/// The text of paragraph `i`, as the mutation benchmark's are.
fn sentence(i: usize) -> String {
    format!(
        "Paragraph {i}. The quick brown fox jumps over the lazy dog; \
         pack my box with five dozen liquor jugs."
    )
}

/// The mutation benchmark's page of 1,000 paragraphs, with `css` and
/// the five hundredth paragraph's id `middle`.
fn paragraphs(css: &str) -> String {
    let mut source =
        format!("<style>body {{ margin: 16px; }} p {{ margin: 0 0 12px 0; }} {css}</style><body>");
    for i in 0..1000 {
        let id = if i == 500 { " id=middle" } else { "" };
        source.push_str(&format!("<p{id}>{}</p>", sentence(i)));
    }
    source.push_str("</body>");
    source
}

/// The engine's counters of the work its frames did.
#[derive(Clone, Copy)]
struct Work {
    restyled: u64,
    laid_out: u64,
    layouts: u64,
    kept: u64,
}

impl Work {
    fn of(engine: &Engine) -> Self {
        Work {
            restyled: engine.restyled_elements(),
            laid_out: engine.boxes_laid_out(),
            layouts: engine.layout_runs(),
            kept: engine.layouts_kept(),
        }
    }

    /// What was done between `self` and `later`, as one line.
    fn since(self, later: Work) -> String {
        format!(
            "{} element styles restyled, {} layouts, {} kept, {} boxes laid out",
            later.restyled - self.restyled,
            later.layouts - self.layouts,
            later.kept - self.kept,
            later.laid_out - self.laid_out
        )
    }
}

/// Two seconds of frames of the page `source`, animated, after its
/// first: their stages and the work they did.
fn animated(label: &str, source: &str) {
    let Some(mut engine) = engine(false) else {
        return;
    };
    engine.load_document(source);
    let start = Instant::now();
    draw(&mut engine, 0);
    let first = start.elapsed();
    let work = Work::of(&engine);
    let stats: Vec<FrameStats> = (1..=COST_FRAMES).map(|i| draw(&mut engine, i)).collect();
    println!(
        "  {label}: the first frame {:.1} ms; {COST_FRAMES} frames: {}",
        first.as_secs_f64() * 1000.0,
        work.since(Work::of(&engine))
    );
    report(&stats);
}

/// The body of `engine`'s document.
fn body(engine: &Engine) -> NodeId {
    let document = engine.document();
    document
        .subtree(document.root())
        .find(|&n| document.tag(n) == Some("body"))
        .expect("the parser implies a body")
}

/// The thousand paragraphs, `p { transition: all ... }` when
/// `transitions`, and the body's class changed twenty times, two
/// seconds apart: the cascade of each frame that changes it.
fn class_changed(transitions: bool) {
    let Some(mut engine) = engine(false) else {
        return;
    };
    let transition = if transitions {
        "p { transition: all 1s linear; }"
    } else {
        ""
    };
    engine.load_document(&paragraphs(&format!(
        "{transition} .on p {{ color: #c80000; padding-left: 4px; }}"
    )));
    draw(&mut engine, 0);
    let body = body(&engine);
    let mut stats = Vec::new();
    for k in 1..=20u32 {
        let class = if k % 2 == 1 { "on" } else { "" };
        engine
            .mutate(|document| document.set_attribute(body, "class", class))
            .expect("an element");
        // The frame that changes it, then one after its transitions.
        stats.push(draw(&mut engine, k * 120));
        draw(&mut engine, k * 120 + 90);
    }
    println!(
        "  the body's class changed twenty times{}:",
        if transitions {
            ", every paragraph with transition: all"
        } else {
            ""
        }
    );
    report(&stats);
}

/// `composite.rs`'s twelve cards, inline blocks, the first with the id
/// `spin`.
fn cards() -> String {
    let mut source = String::from(
        "<style>body { margin: 24px; } \
         .card { display: inline-block; vertical-align: top; width: 260px; \
         margin: 0 24px 24px 0; padding: 16px; border: 1px solid #d0d7de; \
         background-color: #ffffff; } \
         .card h2 { margin: 0 0 8px 0; font-size: 18px; } \
         .card p { margin: 0; font-size: 14px; color: #57606a; } \
         #spin { background-color: #ddf4ff; transform: rotate(0deg); }</style><body><div>",
    );
    for i in 0..12 {
        let id = if i == 0 { " id=spin" } else { "" };
        source.push_str(&format!(
            "<div class=card{id}><h2>Card {i}</h2><p>A card of a dozen, with a title and \
             two lines of text under it, as a dashboard's are.</p></div>"
        ));
    }
    source.push_str("</div></body>");
    source
}

/// The turned card: two degrees more a frame through its `style`
/// attribute, composited (M22.4) where M19 laid it out again.
fn turned_card() {
    let Some(mut engine) = engine(false) else {
        return;
    };
    engine.load_document(&cards());
    draw(&mut engine, 0);
    let spin = {
        let document = engine.document();
        document
            .subtree(document.root())
            .find(|&n| document.attr(n, "id") == Some("spin"))
            .expect("the card is there")
    };
    let work = Work::of(&engine);
    let mut stats = Vec::new();
    for i in 1..=COST_FRAMES {
        let style = format!("transform: rotate({}deg)", 2 * i);
        engine
            .mutate(|document| document.set_attribute(spin, "style", &style))
            .expect("an element");
        stats.push(draw(&mut engine, i));
    }
    println!(
        "  a card of twelve turned two degrees a frame, {COST_FRAMES} frames: {}",
        work.since(Work::of(&engine))
    );
    report(&stats);
}

fn main() {
    for gpu in [false, true] {
        println!("{} backend:", if gpu { "the GPU" } else { "the software" });
        for name in ["spinner", "colour-fade"] {
            run(gpu, name);
        }
    }
    println!("the costs, on the software backend:");
    animated(
        "a transform on one of 1,000 paragraphs",
        &paragraphs(
            "#middle { animation: slide 1s linear infinite alternate; } \
             @keyframes slide { to { translate: 200px 0; } }",
        ),
    );
    animated(
        "a colour on one of 1,000 paragraphs",
        &paragraphs(
            "#middle { animation: tint 1s linear infinite alternate; } \
             @keyframes tint { to { color: #c80000; } }",
        ),
    );
    animated(
        "an opacity on each of 1,000 paragraphs",
        &paragraphs(
            "p { animation: fade 2s linear infinite alternate; } \
             @keyframes fade { to { opacity: 0.5; } }",
        ),
    );
    class_changed(false);
    class_changed(true);
    turned_card();
}
