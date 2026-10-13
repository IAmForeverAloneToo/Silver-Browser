//! M30's acceptance (docs/design/canvas.md, "Acceptance"): the particle
//! demo (`tests/demos/particles.html`), 2,000 filled circles moved and
//! drawn each animation frame over a cleared canvas of 1280 by 800, for
//! ten seconds of frames sixty a second through `Engine::frame_at`: the
//! page's script, its drawing and the frame's showing, each frame timed
//! whole, on the software backend and, where the machine has an adapter,
//! on the GPU's. For comparison, the same particles as 2,000 elements
//! of the page's DOM, placed absolutely and moved through their `style`.
//! Then the most particles each backend draws within the frame budget:
//! two seconds of frames for each count, by its `?n=`. Then the chart
//! demo (`tests/demos/canvas-chart.html`) from its load, its drawing
//! included, to its first frame.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench canvas
//! ```

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use engine::{Engine, FRAME_BUDGET, FrameTime, LoadState, Url};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
/// Ten seconds of frames, sixty a second.
const FRAMES: u32 = 600;
/// The jank budget: no frame longer.
const JANK: Duration = Duration::from_millis(20);

/// The particles of the demo as elements of the page's DOM, from the
/// same generator, each frame moving every one through its `style`.
const DOM_PARTICLES: &str = r#"<!doctype html>
<html><head><style>
  html, body { margin: 0; background-color: #10141a; }
  #field { position: relative; width: 1280px; height: 800px; overflow: hidden; }
  #field div { position: absolute; border-radius: 50%; }
</style></head>
<body><div id="field"></div>
<script>
  var field = document.getElementById('field');
  var seed = 7;
  var random = function () {
    seed = (seed * 16807) % 2147483647;
    return (seed - 1) / 2147483646;
  };
  var particles = [];
  for (var i = 0; i < 2000; i++) {
    var p = {
      x: random() * 1280, y: random() * 800,
      dx: (random() - 0.5) * 6, dy: (random() - 0.5) * 6,
      r: 2 + random() * 5, element: document.createElement('div')
    };
    p.element.style.width = p.element.style.height = 2 * p.r + 'px';
    p.element.style.backgroundColor = 'hsl(' + Math.floor(random() * 360) + ', 70%, 60%)';
    field.appendChild(p.element);
    particles.push(p);
  }
  var draw = function () {
    for (var i = 0; i < particles.length; i++) {
      var p = particles[i];
      p.x += p.dx;
      p.y += p.dy;
      if (p.x < p.r || p.x > 1280 - p.r) { p.dx = -p.dx; }
      if (p.y < p.r || p.y > 800 - p.r) { p.dy = -p.dy; }
      p.element.style.left = (p.x - p.r) + 'px';
      p.element.style.top = (p.y - p.r) + 'px';
    }
    requestAnimationFrame(draw);
  };
  requestAnimationFrame(draw);
</script></body></html>"#;

/// An engine over the bundled font, scripts on, on the software backend
/// or on the GPU's when `gpu` and the machine has an adapter, which it
/// names when `say`.
fn engine(gpu: bool, say: bool) -> Option<Engine> {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = if gpu {
        match render::gpu::GpuBackend::with_fonts(fonts.clone()) {
            Ok(backend) => {
                if say {
                    println!("on the GPU backend, {}", backend.adapter());
                }
                Engine::with_backend(fonts, Box::new(backend))
            }
            Err(e) => {
                if say {
                    println!("no GPU backend here ({e}): its rounds are skipped");
                }
                return None;
            }
        }
    } else {
        Engine::with_fonts(fonts)
    };
    engine.set_scripting(true);
    engine.set_viewport(WIDTH, HEIGHT);
    Some(engine)
}

/// The demo `name`'s file.
fn demo(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/demos/{name}.html"))
}

/// `engine` navigated to the file `path`, once it has loaded.
fn load(engine: &mut Engine, path: &Path) {
    load_url(engine, Url::from_file_path(path).expect("a path"));
}

/// `engine` navigated to `url`, once it has loaded.
fn load_url(engine: &mut Engine, url: Url) {
    let (wake, woken) = std::sync::mpsc::channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    engine.navigate(url);
    let deadline = Instant::now() + Duration::from_secs(60);
    while engine.load_state() != &LoadState::Loaded {
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("the page loads in time");
        let _ = woken.recv_timeout(left);
        engine.poll_network();
    }
}

/// Frame `i` of a sixty-a-second clock, then drawing ahead until its
/// deadline, as the shell does: how long the frame took.
fn draw(engine: &mut Engine, i: u32) -> Duration {
    let start = Instant::now();
    std::hint::black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET * i)));
    let took = start.elapsed();
    engine.draw_ahead(start + FRAME_BUDGET);
    took
}

/// Ten seconds of the page at `path`'s frames: the median, the 95th
/// percentile and the worst, and those over the budgets.
fn run(gpu: bool, what: &str, path: &Path) {
    let Some(mut engine) = engine(gpu, true) else {
        return;
    };
    load(&mut engine, path);
    draw(&mut engine, 0);
    let layouts = engine.layout_runs();
    let mut times: Vec<f64> = (1..=FRAMES)
        .map(|i| draw(&mut engine, i).as_secs_f64() * 1000.0)
        .collect();
    let over = times
        .iter()
        .filter(|&&t| t > FRAME_BUDGET.as_secs_f64() * 1000.0)
        .count();
    let jank = times
        .iter()
        .filter(|&&t| t > JANK.as_secs_f64() * 1000.0)
        .count();
    times.sort_by(f64::total_cmp);
    println!(
        "  {what}, {FRAMES} frames sixty a second: median {:.2} ms, 95th {:.2} ms, worst {:.2} ms; \
         over the {:.1} ms budget {over}, over the {} ms jank budget {jank}; layouts run {}",
        times[times.len() / 2],
        times[times.len() * 95 / 100],
        times[times.len() - 1],
        FRAME_BUDGET.as_secs_f64() * 1000.0,
        JANK.as_millis(),
        engine.layout_runs() - layouts,
    );
}

/// The median frame of two seconds of the particle demo with `count`
/// particles.
fn particles(gpu: bool, count: u32) -> Option<f64> {
    let mut engine = engine(gpu, false)?;
    let file = Url::from_file_path(&demo("particles")).expect("a path");
    load_url(
        &mut engine,
        Url::parse(&format!("{file}?n={count}")).expect("a URL"),
    );
    draw(&mut engine, 0);
    let mut times: Vec<f64> = (1..=120)
        .map(|i| draw(&mut engine, i).as_secs_f64() * 1000.0)
        .collect();
    times.sort_by(f64::total_cmp);
    Some(times[times.len() / 2])
}

/// The most particles, by hundreds, whose median frame is within the
/// frame budget.
fn most(gpu: bool) {
    let budget = FRAME_BUDGET.as_secs_f64() * 1000.0;
    let mut best = None;
    for count in (100..=2000).step_by(100) {
        match particles(gpu, count) {
            Some(median) if median <= budget => best = Some((count, median)),
            Some(_) => break,
            None => return,
        }
    }
    match best {
        Some((count, median)) => println!(
            "  the most particles within the {budget:.1} ms budget: {count}, median {median:.2} ms"
        ),
        None => println!("  not even 100 particles within the {budget:.1} ms budget"),
    }
}

/// The chart demo from its navigation to its first frame, ten times:
/// the median.
fn chart() {
    let mut times: Vec<f64> = (0..10)
        .map(|_| {
            let mut engine = engine(false, false).expect("the software backend");
            let start = Instant::now();
            load(&mut engine, &demo("canvas-chart"));
            draw(&mut engine, 0);
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    println!(
        "  the chart demo, its load to its first frame: median {:.2} ms",
        times[times.len() / 2]
    );
}

fn main() {
    let dom = Path::new(env!("CARGO_TARGET_TMPDIR")).join("particles-dom.html");
    std::fs::write(&dom, DOM_PARTICLES).expect("the page is written");
    for gpu in [false, true] {
        println!("{} backend:", if gpu { "the GPU" } else { "the software" });
        run(gpu, "2,000 particles on a canvas", &demo("particles"));
        run(gpu, "2,000 particles as elements", &dom);
        most(gpu);
    }
    chart();
}
