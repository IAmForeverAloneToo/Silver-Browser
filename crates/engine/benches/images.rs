//! M23.6 acceptance: the photo grid (`tests/demos/photo-grid.html`),
//! photos in PNG, JPEG, GIF and WebP, an animated GIF and gradients,
//! loaded by its `file:` URL at 1280 by 800 on the software backend and,
//! where the machine has an adapter, on the GPU's.
//!
//! - **Cold:** the frames from the first, which asks for every listed
//!   image's pixels, until they are decoded and drawn.
//! - **Scrolled:** then 40px a frame down the page and back, sixty frames
//!   a second through `Engine::frame_at`, the animated GIF turning, the
//!   time a frame leaves before its deadline spent drawing ahead as the
//!   shell spends it; every frame's time recorded. Sixty frames a second
//!   is every frame within `FRAME_BUDGET`; the jank budget is no frame
//!   over 20ms.
//!
//! Then the milestone's costs (`docs/design/images.md`), each the median
//! of its runs: decoding each format's photo; a 4096 by 2732 image made
//! a copy for 256 by 171 drawn, as a page of thumbnails would; and
//! rendering a gradient the size of the viewport, linear and radial.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand, its
//! numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench images
//! ```

use std::hint::black_box;
use std::path::Path;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use engine::{Engine, FRAME_BUDGET, FrameStats, FrameTime, LoadState, Url};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const STEP_PX: f32 = 40.0;
/// The jank budget: no frame longer.
const JANK: Duration = Duration::from_millis(20);

/// The demo's directory.
fn demos() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/demos")
}

/// An engine over the bundled font, on the software backend, or on the
/// GPU's when `gpu` and the machine has an adapter; and what its waker
/// sends.
fn engine(gpu: bool) -> Option<(Engine, Receiver<()>)> {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = if gpu {
        match render::gpu::GpuBackend::with_fonts(fonts.clone()) {
            Ok(backend) => {
                println!("on the GPU backend, {}", backend.adapter());
                Engine::with_backend(fonts, Box::new(backend))
            }
            Err(e) => {
                println!("no GPU backend here ({e}): its rounds are skipped");
                return None;
            }
        }
    } else {
        println!("on the software backend");
        Engine::with_fonts(fonts)
    };
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    Some((engine, woken))
}

/// Takes what arrives until `done`.
fn wait(engine: &mut Engine, woken: &Receiver<()>, done: impl Fn(&Engine) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done(engine) {
        engine.poll_network();
        if done(engine) {
            return;
        }
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("the page loads in a minute");
        let _ = woken.recv_timeout(left);
    }
}

/// A frame drawn for frame `i` of a sixty-a-second clock, then drawing
/// ahead until its deadline; the frame's stages.
fn frame(engine: &mut Engine, i: u32) -> FrameStats {
    let start = Instant::now();
    black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET * i)));
    let stats = engine.frame_stats();
    engine.draw_ahead(start + FRAME_BUDGET);
    stats
}

/// Milliseconds, sorted.
fn sorted(times: impl Iterator<Item = Duration>) -> Vec<f64> {
    let mut ms: Vec<f64> = times.map(|t| t.as_secs_f64() * 1000.0).collect();
    ms.sort_by(f64::total_cmp);
    ms
}

fn grid(gpu: bool) {
    let Some((mut engine, woken)) = engine(gpu) else {
        return;
    };
    let url = Url::from_file_path(&demos().join("photo-grid.html")).expect("a path");
    let start = Instant::now();
    engine.navigate(url);
    wait(&mut engine, &woken, |e| {
        e.load_state() == &LoadState::Loaded
    });
    let loaded = start.elapsed();
    // Cold: frames until what they draw is decoded.
    let start = Instant::now();
    let mut i = 0;
    let mut cold = Vec::new();
    loop {
        cold.push(frame(&mut engine, i).total);
        i += 1;
        if !engine.decoding() {
            break;
        }
        wait(&mut engine, &woken, |e| !e.decoding());
    }
    let cold_time = start.elapsed();
    let height = engine.layout(WIDTH as f32).height;
    let range = engine.max_scroll();
    // Scrolled: down and back.
    let runs = engine.layout_runs();
    let steps = (range / STEP_PX).ceil() as u32;
    let mut stats = Vec::new();
    for k in 1..=2 * steps {
        let y = if k <= steps { k } else { 2 * steps - k } as f32 * STEP_PX;
        engine.set_scroll(y.min(range));
        stats.push(frame(&mut engine, i));
        i += 1;
        // A frame that asked for pixels draws them once they arrive.
        if engine.decoding() {
            wait(&mut engine, &woken, |e| !e.decoding());
        }
    }
    println!(
        "photo grid: {height:.0}px tall, loaded in {:.1} ms; first frames until decoded: {} in {:.1} ms (worst {:.2} ms)",
        loaded.as_secs_f64() * 1000.0,
        cold.len(),
        cold_time.as_secs_f64() * 1000.0,
        sorted(cold.into_iter()).last().copied().unwrap_or_default()
    );
    let stage = |name: &str, of: fn(&FrameStats) -> Duration| {
        let ms = sorted(stats.iter().map(of));
        println!(
            "  {name}: median {:.2} ms, 95th {:.2} ms, worst {:.2} ms",
            ms[ms.len() / 2],
            ms[ms.len() * 95 / 100],
            ms[ms.len() - 1]
        );
    };
    println!(
        "  {} scroll frames, layouts run: {}",
        stats.len(),
        engine.layout_runs() - runs
    );
    stage("paint", |s| s.paint);
    stage("raster", |s| s.raster);
    stage("frame", |s| s.total);
    println!(
        "  over the {:.1} ms budget: {}; over {} ms: {}",
        FRAME_BUDGET.as_secs_f64() * 1000.0,
        stats.iter().filter(|s| s.overran).count(),
        JANK.as_millis(),
        stats.iter().filter(|s| s.total > JANK).count()
    );
}

/// The median time of `runs` runs of `f`, in milliseconds.
fn median(runs: usize, mut f: impl FnMut()) -> f64 {
    let ms = sorted((0..runs).map(|_| {
        let start = Instant::now();
        f();
        start.elapsed()
    }));
    ms[ms.len() / 2]
}

fn costs() {
    println!("costs, medians:");
    for name in [
        "hills-1.jpg",
        "valley-1.png",
        "hills-4.webp",
        "bands-1.gif",
        "spinner.gif",
    ] {
        let bytes = std::fs::read(demos().join("photos").join(name)).expect("the photo");
        let image = images::decode(&bytes).expect("it decodes");
        let ms = median(20, || {
            black_box(images::decode(black_box(&bytes)).expect("it decodes"));
        });
        println!(
            "  decode {name} ({}x{}, {} frames): {ms:.2} ms",
            image.width,
            image.height,
            image.frames.len()
        );
    }
    let (w, h) = (4096u32, 2732u32);
    let big = images::Image {
        width: w,
        height: h,
        frames: vec![images::Frame {
            pixels: (0..w * h * 4)
                .map(|i| (i % 251) as u8)
                .collect::<Vec<u8>>()
                .into(),
            delay: 100,
        }],
        plays: images::Plays::Times(1),
    };
    let ms = median(5, || {
        black_box(images::downscale(black_box(&big), 512, 342));
    });
    println!("  downscale {w}x{h} to 512x342: {ms:.1} ms");
    for css in [
        "linear-gradient(160deg, #264653, #2a9d8f 45%, #e9c46a)",
        "radial-gradient(circle at 70% 40%, #f4a261, #e76f51 35%, #264653)",
        "repeating-linear-gradient(135deg, #f4efe6 0 12px, #efe8dc 12px 24px)",
    ] {
        let gradient = gradient(css);
        let ms = median(10, || {
            black_box(paint::gradient::render(
                &gradient,
                WIDTH as f32,
                HEIGHT as f32,
            ));
        });
        println!("  render {css} at {WIDTH}x{HEIGHT}: {ms:.2} ms");
    }
}

/// The gradient `css` computes to as a background.
fn gradient(css: &str) -> style::computed::Gradient {
    let html = format!("<div style=\"background-image: {css}\"></div>");
    let document = dom::Document::from_node(&html::parse_document(&html).expect("never fails"));
    let styles =
        style::Stylesheets::new(Vec::new()).cascade(&document, &style::CascadeContext::default());
    let div = document
        .subtree(document.root())
        .find(|&n| document.tag(n) == Some("div"))
        .expect("the div");
    match &styles.get(div).expect("styled").background().image[0] {
        Some(style::computed::Image::Gradient(g)) => (**g).clone(),
        other => panic!("not a gradient: {other:?}"),
    }
}

fn main() {
    grid(false);
    grid(true);
    costs();
}
