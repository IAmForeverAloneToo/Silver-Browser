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
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench animation
//! ```

use std::path::Path;
use std::time::{Duration, Instant};

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

fn main() {
    for gpu in [false, true] {
        println!("{} backend:", if gpu { "the GPU" } else { "the software" });
        for name in ["spinner", "colour-fade"] {
            run(gpu, name);
        }
    }
}
