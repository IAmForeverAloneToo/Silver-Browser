//! M22.6's, M23.6's and M24.7's acceptance, as far as it holds whatever
//! the machine: what the frames of the milestones' demos (`tests/demos/`)
//! do; for M23, the photo grid's images drawn and its frames, scrolled
//! and turning, laying nothing out, `benches/images.rs` timing them. The
//! spinner's frames lay nothing out and composite its layer without
//! drawing it again; the colour fade's lay out the card alone, the rest
//! of the page taken from the layout cache. How long the frames take,
//! sixty a second for ten seconds on each backend, is
//! `benches/animation.rs`'s to measure. M31.5's: the Mandelbrot demo's
//! workers draw the set the page draws on its own thread, while its
//! square turns; `benches/workers.rs` times its frames. M32.4's: the
//! icon set and the chart drawn at each display ratio, 1.25 to 3, are
//! the same pictures as the pages drawn that many times larger at 1;
//! `benches/svg.rs` times them.

use std::path::Path;
use std::time::{Duration, Instant};

use engine::{
    ColorScheme, ConsoleLevel, Engine, FRAME_BUDGET, FrameTime, LoadState, Preferences,
    ScriptState, Url,
};
use paint::GroupKey;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;

/// An engine over the bundled font.
fn engine() -> Engine {
    Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    )
}

/// The demo page `name`.
fn demo(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/demos/{name}.html"));
    std::fs::read_to_string(path).expect("the demo is read")
}

/// The frame of a sixty-a-second clock's `i`th tick.
fn draw(engine: &mut Engine, i: u32) -> Vec<u8> {
    engine
        .frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET * i))
        .to_vec()
}

/// The pixel at `(x, y)` of an RGBA frame.
fn px(frame: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * WIDTH + x) * 4) as usize;
    [frame[i], frame[i + 1], frame[i + 2]]
}

const GREEN: [u8; 3] = [0x3a, 0x7a, 0x4a];

#[test]
fn the_spinners_frames_lay_nothing_out_and_composite_its_layer() {
    let mut engine = engine();
    engine.load_document(&demo("spinner"));
    // The ring, 96px square with its borders, at (592, 145): its top
    // side the turning colour.
    let first = draw(&mut engine, 0);
    assert_eq!(px(&first, 640, 151), GREEN);
    assert_ne!(px(&first, 682, 193), GREEN);
    let runs = engine.layout_runs();
    let mut layer = None;
    for i in 1..=120 {
        let frame = draw(&mut engine, i);
        let layered = engine.layered();
        // The ring's layer, composited at every frame and drawn at none
        // after its first; the document's tiles it is in are drawn again
        // where it reaches, until a layer tree composites it over them
        // (M35.10).
        let boxes: Vec<GroupKey> = layered
            .composited
            .iter()
            .copied()
            .filter(|key| matches!(key, GroupKey::Box(_)))
            .collect();
        assert_eq!(boxes.len(), 1, "frame {i}: {layered:?}");
        assert_eq!(*layer.get_or_insert(boxes[0]), boxes[0]);
        if i > 1 {
            assert!(
                layered.rasterized.iter().all(|(key, _)| *key != boxes[0]),
                "frame {i}: {layered:?}"
            );
        }
        // A quarter of a second in, a quarter turn: the side on the
        // right.
        if i == 15 {
            assert_eq!(px(&frame, 682, 193), GREEN);
            assert_ne!(px(&frame, 640, 151), GREEN);
        }
    }
    assert_eq!(engine.layout_runs(), runs, "no frame laid anything out");
    assert!(engine.needs_frame(), "it turns for as long as it is shown");
}

#[test]
fn under_reduced_motion_the_pages_own_query_stops_the_spinner() {
    let mut engine = engine();
    engine.set_preferences(Preferences {
        color_scheme: ColorScheme::Light,
        reduced_motion: true,
    });
    engine.load_document(&demo("spinner"));
    let first = draw(&mut engine, 0);
    assert!(!engine.needs_frame());
    assert_eq!(engine.next_frame_at(), None);
    let later = engine
        .frame_at(WIDTH, HEIGHT, FrameTime::at(Duration::from_millis(250)))
        .to_vec();
    assert_eq!(first, later);
}

#[test]
fn the_colour_fades_frames_lay_out_the_card_alone() {
    let mut engine = engine();
    engine.load_document(&demo("colour-fade"));
    draw(&mut engine, 0);
    let (mut laid_out, mut reused) = (engine.boxes_laid_out(), engine.boxes_reused());
    for i in 1..=60 {
        draw(&mut engine, i);
        let (now_laid_out, now_reused) = (engine.boxes_laid_out(), engine.boxes_reused());
        // The card, its heading, paragraph and badge, which inherit its
        // colour, and the main, body and html around it; the header and
        // the other two cards taken whole.
        assert_eq!(now_laid_out - laid_out, 7, "frame {i}");
        assert_eq!(now_reused - reused, 3, "frame {i}");
        (laid_out, reused) = (now_laid_out, now_reused);
    }
    assert!(
        engine.needs_frame(),
        "it alternates for as long as it is shown"
    );
}

#[test]
fn the_photo_grids_images_are_drawn_and_its_frames_lay_nothing_out() {
    // M23.6's page: photos in four formats, an animated GIF and
    // gradients, loaded by its URL, drawn once its images are decoded.
    let mut engine = engine();
    let (wake, woken) = std::sync::mpsc::channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    let wait = |engine: &mut Engine, done: &dyn Fn(&Engine) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            engine.poll_network();
            if done(engine) {
                return;
            }
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("the page loads in time");
            let _ = woken.recv_timeout(left);
        }
    };
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/demos/photo-grid.html");
    engine.navigate(Url::from_file_path(&path).expect("a path"));
    wait(&mut engine, &|e| e.load_state() == &LoadState::Loaded);
    draw(&mut engine, 0);
    wait(&mut engine, &|e| !e.decoding());
    let frame = draw(&mut engine, 1);
    // Each photo in view, at its middle, is the photo, not its figure.
    let document = engine.document();
    let imgs: Vec<_> = document
        .subtree(document.root())
        .filter(|&n| document.tag(n) == Some("img"))
        .collect();
    let fragments = engine.fragments(WIDTH as f32);
    let mut seen = 0;
    for img in imgs {
        let Some(f) = fragments.iter().find(|f| f.node == img) else {
            continue;
        };
        let (x, y) = (f.x + f.w / 2.0, f.y + f.h / 2.0);
        if y >= HEIGHT as f32 {
            continue;
        }
        seen += 1;
        assert_ne!(px(&frame, x as u32, y as u32), [0xff, 0xfd, 0xf8], "{f:?}");
    }
    assert!(seen >= 8, "two rows of photos in view: {seen}");
    // The animated GIF in view wakes the next frame when it turns.
    assert!(!engine.needs_frame());
    let next = engine.next_frame_at().expect("the spinner turns");
    assert!(next <= Duration::from_millis(100), "{next:?}");
    // Scrolled to the bottom and back up, nothing is laid out again.
    let runs = engine.layout_runs();
    let range = engine.max_scroll();
    for i in 2..=60u32 {
        let k = if i <= 31 { i - 1 } else { 61 - i } as f32;
        engine.set_scroll(range * k / 30.0);
        draw(&mut engine, i);
    }
    assert_eq!(engine.layout_runs(), runs, "no frame laid anything out");
}

#[test]
fn the_scripts_demo_logs_titles_itself_and_notes_its_error() {
    // M24.7's: an inline script's console.log("hello") and
    // document.title = "x", a script that throws, one that runs after.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/demos/scripts.html");
    let url = Url::from_file_path(&path).expect("a path");
    for scripting in [true, false] {
        let mut engine = engine();
        engine.set_scripting(scripting);
        let (wake, woken) = std::sync::mpsc::channel();
        engine.set_waker(move || {
            let _ = wake.send(());
        });
        engine.navigate(url.clone());
        let deadline = Instant::now() + Duration::from_secs(30);
        while engine.load_state() != &LoadState::Loaded {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("the page loads in time");
            let _ = woken.recv_timeout(left);
            engine.poll_network();
        }
        let console: Vec<(ConsoleLevel, String)> = engine
            .take_console()
            .into_iter()
            .map(|entry| (entry.level, entry.text))
            .collect();
        if !scripting {
            // Off, as by default: the page as it was written.
            assert_eq!(engine.title(), "Before");
            assert!(console.is_empty());
            assert_eq!(engine.scripts(), ScriptState::Off);
            continue;
        }
        assert_eq!(engine.title(), "x");
        assert_eq!(
            console,
            [
                (ConsoleLevel::Log, "hello".to_owned()),
                (
                    ConsoleLevel::Error,
                    format!("Uncaught ReferenceError: missing is not defined at {url}:25:3")
                ),
                (ConsoleLevel::Info, "after the error 2".to_owned()),
            ]
        );
        assert_eq!(engine.scripts(), ScriptState::Running { errors: 1 });
        assert!(!draw(&mut engine, 0).is_empty());
    }
}

/// The demo `name` navigated to by its file URL, scripts on, loaded.
fn navigated(name: &str) -> Engine {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/demos/{name}.html"));
    let url = Url::from_file_path(&path).expect("a path");
    let mut engine = engine();
    engine.set_scripting(true);
    engine.set_viewport(WIDTH, HEIGHT);
    let (wake, woken) = std::sync::mpsc::channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    engine.navigate(url);
    let deadline = Instant::now() + Duration::from_secs(30);
    while engine.load_state() != &LoadState::Loaded {
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("the page loads in time");
        let _ = woken.recv_timeout(left);
        engine.poll_network();
    }
    engine
}

#[test]
fn the_particle_demos_frames_draw_its_canvas_and_lay_nothing_out() {
    // M30.4's: 2,000 circles moved and drawn each animation frame over a
    // cleared canvas, which its own layer shows; one callback a frame.
    let mut engine = navigated("particles");
    let mut last = draw(&mut engine, 0);
    let runs = engine.layout_runs();
    for i in 1..=60 {
        let frame = draw(&mut engine, i);
        if i <= 12 {
            assert_ne!(frame, last, "frame {i} moves the particles");
            let layered = engine.layered();
            assert!(
                layered
                    .composited
                    .iter()
                    .any(|key| matches!(key, GroupKey::Canvas(_))),
                "frame {i}: {layered:?}"
            );
        }
        last = frame;
    }
    assert_eq!(engine.layout_runs(), runs, "no frame laid anything out");
    assert!(engine.needs_frame(), "it moves for as long as it is shown");
    let console: Vec<String> = engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(console, ["frames 60"]);
}

#[test]
fn the_chart_demo_draws_its_bars_line_labels_and_legend() {
    // M30.4's: a chart drawn with the calls a charting library makes.
    let mut engine = navigated("canvas-chart");
    let frame = draw(&mut engine, 0);
    // The canvas's corner, white, below the heading at the body's margin.
    let top = (0..HEIGHT)
        .find(|&y| px(&frame, 24, y) == [255, 255, 255])
        .expect("the canvas is shown");
    let at = |x: u32, y: u32| px(&frame, 24 + x, top + y);
    // The legend's squares, at 80, 210 and 340, 14 high from 14.
    assert_eq!(at(87, 21), [0x2f, 0x4f, 0x6f]);
    assert_eq!(at(217, 21), [0xd0, 0x60, 0x3f]);
    assert_eq!(at(347, 21), [0x7f, 0x9f, 0x5f]);
    // The first bar, January's 420 of 1,200 over 310 pixels, in the
    // gradient: bluer at its top than at the bottom.
    let (high, low) = (at(86, 250), at(86, 345));
    assert!(high[2] > low[2], "{high:?} {low:?}");
    // The month labels' ink below the axis, the white around it.
    let ink = (86 - 15..86 + 15)
        .flat_map(|x| (358..372).map(move |y| (x, y)))
        .filter(|&(x, y)| at(x, y)[0] < 160)
        .count();
    assert!(ink > 10, "{ink} pixels of 'Jan'");
    assert_eq!(at(700, 380), [255, 255, 255]);
}

/// The Mandelbrot demo with `query` (M31.5), run as the shell runs it, a
/// frame a tick on the real clock, until the set is drawn: its last
/// frame, and how many frames before it turned the page's own square.
fn mandelbrot(query: &str) -> (Vec<u8>, u32) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/demos/mandelbrot.html");
    let file = Url::from_file_path(&path).expect("a path");
    let url = Url::parse(&format!("{file}?{query}")).expect("a URL");
    let mut engine = engine();
    engine.set_scripting(true);
    engine.set_viewport(WIDTH, HEIGHT);
    let (wake, woken) = std::sync::mpsc::channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    engine.navigate(url);
    let start = Instant::now();
    let deadline = start + Duration::from_secs(120);
    let square = |frame: &[u8]| {
        (0..120)
            .flat_map(|y| (640..760).map(move |x| px(frame, x, y)))
            .collect::<Vec<_>>()
    };
    let (mut turned, mut last) = (0, None);
    loop {
        assert!(Instant::now() < deadline, "the set is drawn in time");
        let _ = woken.recv_timeout(FRAME_BUDGET);
        engine.poll_network();
        engine.run_tasks();
        let frame = engine
            .frame_at(WIDTH, HEIGHT, FrameTime::at(start.elapsed()))
            .to_vec();
        let drawn = engine
            .take_console()
            .into_iter()
            .any(|entry| entry.text.starts_with("set "));
        if drawn {
            return (frame, turned);
        }
        let now = square(&frame);
        if engine.load_state() == &LoadState::Loaded && last.as_ref().is_some_and(|l| *l != now) {
            turned += 1;
        }
        last = Some(now);
    }
}

#[test]
fn the_mandelbrot_demos_workers_draw_the_set_the_page_would_while_it_turns() {
    // M31.5's: four workers compute the set's bands and send them back
    // in transferred buffers; the page's square turns meanwhile. The
    // same set as the page computes on its own thread, a band a task.
    let (by_workers, turned) = mandelbrot("workers=4&iterations=20");
    assert!(turned >= 2, "the square turned in {turned} frames");
    // The main cardioid's middle stays bounded, black; the corner, -2.2
    // and -1, leaves at once.
    assert_eq!(px(&by_workers, 340, 200), [0, 0, 0]);
    assert_eq!(px(&by_workers, 0, 0), [9, 4, 135]);
    let (by_page, _) = mandelbrot("workers=0&iterations=20");
    let set = |frame: &[u8]| {
        (0..400)
            .flat_map(|y| (0..640).map(move |x| px(frame, x, y)))
            .collect::<Vec<_>>()
    };
    assert!(set(&by_workers) == set(&by_page), "the same set either way");
}

/// The page at `path`, loaded by its URL at the display scale `ratio`,
/// drawn `css` CSS pixels big once what it draws is decoded: its RGBA
/// device pixels and their size (M32.4).
fn drawn_at(path: &Path, ratio: f32, css: (u32, u32)) -> (Vec<u8>, (u32, u32)) {
    let url = Url::from_file_path(path).expect("a path");
    let device = (
        (css.0 as f32 * ratio).round() as u32,
        (css.1 as f32 * ratio).round() as u32,
    );
    let mut engine = engine();
    engine.set_scale_factor(ratio);
    let (wake, woken) = std::sync::mpsc::channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    engine.navigate(url);
    let deadline = Instant::now() + Duration::from_secs(30);
    let wait = |engine: &mut Engine, done: &dyn Fn(&Engine) -> bool| {
        while !done(engine) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("the page is drawn in time");
            let _ = woken.recv_timeout(left.min(Duration::from_millis(200)));
            engine.poll_network();
        }
    };
    wait(&mut engine, &|engine| {
        engine.load_state() == &LoadState::Loaded
    });
    loop {
        let frame = engine.frame(device.0, device.1);
        if !engine.decoding() {
            return (frame, device);
        }
        wait(&mut engine, &|engine| !engine.decoding());
    }
}

/// M32.4's acceptance: the demo `name` drawn at each display scale is
/// the same picture as the demo with its sizes that many times as large
/// (what `larger` makes of its source) drawn at 1: the same pixels, edges
/// as sharp, within `levels` of a channel (two rasterizations of the same
/// geometry, the one under a scale, the other at its size).
fn same_at_every_ratio(
    name: &str,
    css: (u32, u32),
    levels: u8,
    larger: impl Fn(&str, f32) -> String,
) {
    let demos = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/demos");
    let page = demos.join(format!("{name}.html"));
    let source = demo(name);
    let base = Url::from_file_path(&demos).expect("a path");
    let dir = std::env::temp_dir().join(format!("svg-ratios-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a directory");
    for ratio in [1.25f32, 1.5, 2.0, 3.0] {
        let (scaled, device) = drawn_at(&page, ratio, css);
        // The larger page, its images and files found where the demo's are.
        let larger_page = dir.join(format!("{name}-{ratio}.html"));
        let text = larger(&source, ratio).replacen(
            "<head>",
            &format!("<head>\n<base href=\"{base}/\">"),
            1,
        );
        std::fs::write(&larger_page, text).expect("the larger page is written");
        let (at_one, size) = drawn_at(&larger_page, 1.0, device);
        assert_eq!(size, device);
        let mut differing = 0;
        let mut most = 0u8;
        for (a, b) in scaled.chunks(4).zip(at_one.chunks(4)) {
            let by = (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap_or(0);
            most = most.max(by);
            differing += usize::from(by > levels);
        }
        assert!(
            differing == 0,
            "{name} at {ratio}: {differing} pixels differ by more than {levels}, by {most} at most"
        );
        println!("{name} at {ratio}: within {most} of the page drawn larger at 1");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_icons_are_the_same_picture_at_every_display_ratio() {
    // Thirty icons, a stroked family and a filled one, through use of
    // symbols and as img files, at 16, 24 and 48 pixels.
    same_at_every_ratio("svg-icons", (1280, 400), 2, |source, ratio| {
        [16, 24, 48].iter().fold(source.to_string(), |text, size| {
            let at = (*size as f32 * ratio).round();
            text.replace(
                &format!(".i{size} {{ width: {size}px; height: {size}px; }}"),
                &format!(".i{size} {{ width: {at}px; height: {at}px; }}"),
            )
        })
    });
}

#[test]
fn the_chart_is_the_same_picture_at_every_display_ratio() {
    // Axes, ticks and labels, gridlines, bars, a line with points, an area
    // under a gradient and a legend, as a charting library writes them.
    same_at_every_ratio("svg-chart", (640, 360), 2, |source, ratio| {
        source.replacen(
            "width='640' height='360'",
            &format!("width='{}' height='{}'", 640.0 * ratio, 360.0 * ratio),
            1,
        )
    });
}

#[test]
fn the_icons_and_the_chart_draw_their_shapes_and_text() {
    // At 1: an icon of each family in its colour, inline and as an image;
    // the chart's bars, its line's points, its labels' ink.
    let demos = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/demos");
    let (icons, (w, _)) = drawn_at(&demos.join("svg-icons.html"), 1.0, (1280, 400));
    let at = |frame: &[u8], w: u32, x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [frame[i], frame[i + 1], frame[i + 2]]
    };
    // The 24-pixel rows start at 32 (two 16-pixel rows): the house's roof
    // (stroked, #2f4f6f) and the heart (filled, #8f3f2f) at its middle.
    let ink = |rgb: [u8; 3], want: [u8; 3]| rgb.iter().zip(want).all(|(a, b)| a.abs_diff(b) < 40);
    let row = |y0: u32| {
        let roof = (0..24).any(|x| ink(at(&icons, w, x, y0 + 4), [0x2f, 0x4f, 0x6f]));
        let heart = ink(at(&icons, w, 15 * 24 + 12, y0 + 12), [0x8f, 0x3f, 0x2f]);
        (roof, heart)
    };
    assert_eq!(row(32), (true, true), "inline, through use");
    assert_eq!(row(56), (true, true), "as img files");
    let (chart, (w, _)) = drawn_at(&demos.join("svg-chart.html"), 1.0, (640, 360));
    // The first bar (returning, 180 of 1200) and the first point's white
    // middle on the line, the y axis's labels' ink left of it.
    assert!(ink(at(&chart, w, 50 + 26, 40 + 270), [0x9f, 0xb4, 0xc8]));
    assert_eq!(at(&chart, w, 74, 222), [255, 255, 255]);
    let labels = (20..44)
        .flat_map(|x| (40..320).map(move |y| (x, y)))
        .filter(|&(x, y)| at(&chart, w, x, y)[0] < 128)
        .count();
    assert!(labels > 50, "{labels}");
}
