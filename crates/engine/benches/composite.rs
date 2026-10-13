//! M19.7 acceptance: compositing at 1280 by 800, on the software
//! backend and, where the machine has an adapter, on the GPU's.
//!
//! - **Scrolling.** The mutation benchmark's page of a thousand
//!   paragraphs, scrolled 40px a frame for 120 frames drawn sixty a
//!   second. The document is a layer of tiles: a frame composites the
//!   tiles the viewport meets, and the time a frame leaves before its
//!   deadline, once presented, draws the tiles a scroll brings in next.
//!   No tile may be rasterized in a frame once the margin is drawn.
//! - **A card that turns.** Twelve cards, one turned two degrees more
//!   each frame through `Engine::mutate` on its `style` attribute, for
//!   120 frames, with and without `will-change: transform`: a turned
//!   box is a layer either way (the compositing note's policy), so each
//!   frame must be a composite of the layer kept, nothing rasterized.
//! - **A card that moves.** The same with the card moved two pixels
//!   each frame: with `will-change: transform` a layer composited,
//!   without it drawn through, each frame a partial redraw of the
//!   card's reach, old and new.
//! - **Faded cards.** Two hundred cards, each translated and faded,
//!   drawn through: the first frame, then 120 scroll frames.
//!
//! Each reports its frames' milliseconds and what the backend did, the
//! layers it rasterized and composited, as the counters prove.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench composite
//! ```

#![forbid(unsafe_code)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use dom::NodeId;
use engine::{Engine, FRAME_BUDGET, FrameStats, FrameTime};
use render::Redrawn;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const FRAMES: u32 = 120;
const STEP_PX: f32 = 40.0;

/// The mutation benchmark's page: `n` paragraphs.
fn paragraphs(n: usize) -> String {
    let mut source =
        String::from("<style>body { margin: 16px; } p { margin: 0 0 12px 0; }</style><body>");
    for i in 0..n {
        source.push_str(&format!(
            "<p>Paragraph {i}. The quick brown fox jumps over the lazy dog; \
             pack my box with five dozen liquor jugs.</p>"
        ));
    }
    source.push_str("</body>");
    source
}

/// Twelve cards in rows of four, inline blocks, the first with the id
/// `spin`, which has `will-change: transform` when `promoted`. Inline
/// blocks rather than a grid: a style change lays its box out again,
/// and a grid lays all its items out with it (M35.8), which is not what
/// this measures.
fn cards(promoted: bool) -> String {
    let will_change = if promoted {
        "will-change: transform;"
    } else {
        ""
    };
    let mut source = format!(
        "<style>body {{ margin: 24px; }} \
         .card {{ display: inline-block; vertical-align: top; width: 260px; \
         margin: 0 24px 24px 0; padding: 16px; border: 1px solid #d0d7de; \
         background-color: #ffffff; }} \
         .card h2 {{ margin: 0 0 8px 0; font-size: 18px; }} \
         .card p {{ margin: 0; font-size: 14px; color: #57606a; }} \
         #spin {{ {will_change} background-color: #ddf4ff; }}</style>\
         <body><div>"
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

/// Two hundred cards, each translated by a few pixels and faded.
fn faded(n: usize) -> String {
    let mut source = String::from(
        "<style>body { margin: 16px; } \
         .card { display: inline-block; width: 280px; margin: 0 12px 12px 0; padding: 12px; \
         border: 1px solid #8c959f; background-color: #f6f8fa; } \
         .card p { margin: 0; font-size: 14px; }</style><body>",
    );
    for i in 0..n {
        let (dx, dy) = ((i % 7) as f32 * 1.5, (i % 5) as f32);
        let opacity = 0.55 + (i % 9) as f32 * 0.05;
        source.push_str(&format!(
            "<div class=card style=\"transform: translate({dx}px, {dy}px); opacity: {opacity}\">\
             <p>Card {i}, moved and faded, drawn through the page's raster.</p></div>"
        ));
    }
    source.push_str("</body>");
    source
}

/// The element with `id`.
fn by_id(engine: &Engine, id: &str) -> NodeId {
    let document = engine.document();
    document
        .subtree(document.root())
        .find(|&n| document.attr(n, "id") == Some(id))
        .expect("the element is there")
}

/// Prints the median and worst of each stage over `stats`, and the
/// frames over budget.
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
        "    over the {:.1} ms budget: {} of {}",
        FRAME_BUDGET.as_secs_f64() * 1000.0,
        stats.iter().filter(|s| s.overran).count(),
        stats.len()
    );
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
/// ahead until its budget is spent, as the shell does once it presents
/// one; the frame's stages.
fn draw(engine: &mut Engine, i: u32) -> FrameStats {
    let start = Instant::now();
    black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET * i)));
    engine.draw_ahead(start + FRAME_BUDGET);
    engine.frame_stats()
}

fn scrolling(gpu: bool) {
    let Some(mut engine) = engine(gpu) else {
        return;
    };
    engine.load_document(&paragraphs(1000));
    let start = Instant::now();
    draw(&mut engine, 0);
    let first = start.elapsed();
    let mut stats = Vec::new();
    let (mut drawn_in_frame, mut prepainted) = (0u32, 0u32);
    let mut composited = 0u32;
    for i in 1..=FRAMES {
        engine.set_scroll(i as f32 * STEP_PX);
        stats.push(draw(&mut engine, i));
        let layered = engine.layered();
        if !layered.rasterized.is_empty() {
            drawn_in_frame += 1;
        }
        if !layered.composited.is_empty() {
            composited += 1;
        }
        prepainted += layered.prepainted;
    }
    println!(
        "  scrolling 1,000 paragraphs, {FRAMES} frames of {STEP_PX}px: the first frame {:.1} ms",
        first.as_secs_f64() * 1000.0
    );
    report(&stats);
    println!(
        "    frames composited from tiles: {composited} of {FRAMES}; frames that rasterized a \
         tile: {drawn_in_frame}; tiles drawn ahead after their frames: {prepainted}; \
         {} layers, {:.1} MiB",
        engine.layers().len(),
        engine.layer_bytes() as f64 / (1 << 20) as f64
    );
}

fn moving_card(gpu: bool, promoted: bool, turning: bool) {
    let Some(mut engine) = engine(gpu) else {
        return;
    };
    engine.load_document(&cards(promoted));
    let spin = by_id(&engine, "spin");
    draw(&mut engine, 0);
    let mut stats = Vec::new();
    let (mut rasterized, mut composited, mut partial, mut whole) = (0u32, 0u32, 0u32, 0u32);
    let (mut redrawn_area, mut tiles) = (0u64, 0u32);
    for i in 1..=FRAMES {
        let style = if turning {
            format!("transform: rotate({}deg)", 2 * i)
        } else {
            format!("transform: translate({}px, 0)", 2 * i)
        };
        engine
            .mutate(|document| document.set_attribute(spin, "style", &style))
            .expect("an element");
        stats.push(draw(&mut engine, i));
        let layered = engine.layered();
        // The card's own layer, and the document's tiles it is in.
        for (key, n) in &layered.rasterized {
            match key {
                paint::GroupKey::Document(_) => tiles += n,
                _ => rasterized += 1,
            }
        }
        if layered
            .composited
            .iter()
            .any(|k| !matches!(k, paint::GroupKey::Document(_)))
        {
            composited += 1;
        }
        match engine.redrawn() {
            Redrawn::Whole => whole += 1,
            Redrawn::Rects(rects) => {
                partial += 1;
                redrawn_area += rects.iter().map(|r| r.area()).sum::<u64>();
            }
        }
    }
    println!(
        "  a card of twelve {} two {} a frame{}, {FRAMES} frames:",
        if turning { "turned" } else { "moved" },
        if turning { "degrees" } else { "pixels" },
        if promoted {
            ", with will-change: transform"
        } else {
            ""
        }
    );
    report(&stats);
    println!(
        "    frames that rasterized the card's layer: {rasterized}; that composited it: \
         {composited}; the document's tiles drawn again where it reaches: {tiles}; \
         frames drawn whole: {whole}; drawn in part: {partial}, {:.0} pixels a frame",
        redrawn_area as f64 / partial.max(1) as f64
    );
}

fn faded_cards(gpu: bool) {
    let Some(mut engine) = engine(gpu) else {
        return;
    };
    engine.load_document(&faded(200));
    let start = Instant::now();
    draw(&mut engine, 0);
    let first = start.elapsed();
    let mut stats = Vec::new();
    for i in 1..=FRAMES {
        engine.set_scroll(i as f32 * STEP_PX);
        stats.push(draw(&mut engine, i));
    }
    println!(
        "  200 translated and faded cards: the first frame {:.1} ms; {FRAMES} scroll frames:",
        first.as_secs_f64() * 1000.0
    );
    report(&stats);
    println!(
        "    {} layers, {:.1} MiB",
        engine.layers().len(),
        engine.layer_bytes() as f64 / (1 << 20) as f64
    );
}

fn main() {
    for gpu in [false, true] {
        println!("{} backend:", if gpu { "the GPU" } else { "the software" });
        scrolling(gpu);
        for turning in [true, false] {
            for promoted in [false, true] {
                moving_card(gpu, promoted, turning);
            }
        }
        faded_cards(gpu);
    }
}
