//! M10.7's, M11.7's, M12.7's and M16's acceptance: the box model,
//! floats, positioning, stacking, tables and generated boxes against a
//! reference browser.
//!
//! Every page under `tests/reference/` is a case built of boxes alone —
//! no text drawn, since text rasterizers differ by design; M21's pages
//! of list markers and text decorations lay out text drawn transparent —
//! and
//! `tests/reference/capture.sh` captures each one at 800 by 600,
//! device scale one, scrollbars hidden, from a headless browser into
//! the `.png` beside it. This test renders each page with the engine
//! at the same size and compares the pixels byte for byte. A page may
//! ask for another size and for what it is drawn under (M20), which the
//! capture and the test both read from its head: `reference-size`, the
//! viewport's width and height, `640x480` say; `reference-media`,
//! `print` for the print medium on the screen, which the reference
//! browser draws through its `mediaTypeOverride` and the engine through
//! [`Engine::set_media_type`]; and `reference-color-scheme`, `dark`, and
//! `reference-reduced-motion`, `reduce`, the user's preferences. When they
//! differ, the engine's render and a difference image (the differing
//! pixels in red over the capture, dimmed) are written under
//! `target/reference/` for a look, and the failure names the pixels.
//!
//! A page's animations are drawn at their first frame, the document's
//! timeline at zero, as the capture shows them: paused, or at a negative
//! delay, they show the moment they ask for (M22). A transition needs a
//! change after the first frame: a page may ask for changes with
//! `<meta name="reference-change">`, each an element's id, an attribute
//! and its value, separated by semicolons, and for the times they are
//! made and the page drawn with `<meta name="reference-time">`, in
//! milliseconds from the first frame. The capture's browser makes them
//! by the page's `reference-change.js`, holding every animation at each
//! time; this test makes them through [`Engine::mutate`] between frames
//! drawn at those times.
//!
//! A second test loads each page, changes its document and changes it
//! back, drawing a frame after every step, and holds what is left to
//! the same capture (M13): whatever the engine keeps from frame to
//! frame, the page must draw as it did.
//!
//! A page with images (M23) declares `<meta name="reference-load"
//! content="file">`: it is loaded by its `file:` URL, since a document
//! loaded from a string fetches nothing, its images beside it under
//! `images/`, and drawn once what it draws is decoded.
//!
//! A page that keeps what it stores (M28) declares `<meta
//! name="reference-load" content="http">`: it is served over `http:`,
//! the directory by a server of the test's and of the capture's, so
//! that it has an origin and with it `localStorage` and
//! `sessionStorage`, which a `file:` page's opaque origin has not here;
//! the capture's browser starts on a new profile, the engine on none.
//!
//! A page whose scripts run (M24) declares `<meta name="reference-scripts"
//! content="on">`: the engine runs them, as the reference browser runs
//! every page's, and the page is loaded by its `file:` URL, since only a
//! document parsed as it arrives runs scripts, its scripts beside it
//! under `scripts/`; it is drawn once loaded, its deferred scripts run.
//! The other pages have none, and the engine runs none there. The
//! engine is given the viewport's size before the load, which a page's
//! scripts read (M25). A page whose scripts go on after the load, by
//! timers, animation frames and events (M26), declares
//! `<meta name="reference-wait" content="MS">`: the capture's browser
//! runs it on its virtual time for that long before the screenshot, and
//! the engine, on a clock of its own moved by hand from zero, runs its
//! tasks and draws a frame every sixtieth of a second, its rendering
//! steps with it, for as long; before each step, what the page's
//! scripts fetch (M27) is waited for, as the browser's virtual time
//! waits for the network, and what its workers send (M31), which run in
//! real time, until they are quiet for a tenth of a second.
//!
//! A page may declare `<meta name="reference-tolerance" content="N">`:
//! a channel may then differ by up to `N`, and the test says how many
//! pixels did. M11's multi-colour page uses it for the last bit of the
//! browser's blending at anti-aliased mitres, and M19's pages for the
//! rounding of a blend over translucent boxes and of the anti-aliasing
//! of edges a scale puts between pixels. A page may declare
//! `<meta name="reference-edge-tolerance" content="M">` besides: a
//! pixel on an edge in the capture, one whose neighbours are not all
//! its own colour, may then differ by up to `M`, the rule M19's
//! equivalence suite holds the two backends to; M19's pages with
//! rotation or 3D use it for the resampling of a layer from its own
//! plane against the browser's raster through the transform, which
//! moves an edge's coverage, never the colour inside it, and M21's page
//! of list markers for the anti-aliasing of a disc's and a circle's
//! edge by another rasterizer; M30's canvas pages, with a level inside
//! for a gradient's rounding, for the edges of curves and strokes whose
//! coverage tiny-skia's four-by-four samples put apart from the
//! browser's (a slice of a pie at 18 against 0 percent where the exact
//! coverage is 12); M32's SVG pages, as M30's, a level or two inside for
//! a group's opacity and a gradient's rounding, and on the edges of
//! shapes, strokes and dashes (96 where a tiled SVG image's stroke
//! crosses a pixel's corner). Every other page is exact; M21's pages of
//! markers and decorations give the reference browser the engine's own
//! font by `@font-face`, whose metrics place them, their text
//! transparent. A page may declare
//! `<meta name="reference-ignore-scrollbar">` when it scrolls (M19,
//! a transformed box extending the scrollable overflow): the browser
//! captures without scrollbars, so the engine's scrollbar track at the
//! viewport's right edge is left out of the comparison, and the test
//! holds the page to scrolling at all.

#![forbid(unsafe_code)]

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use engine::{
    ColorScheme, Engine, FrameTime, LoadState, MediaType, Preferences, SCROLLBAR_WIDTH, Url,
};
use net::test_server::{Reply, TestServer};

/// How long a page loaded by its URL may take to load and decode.
const LOAD: Duration = Duration::from_secs(30);
/// How long a page's workers stay quiet before its clock moves on (M31),
/// and the longest they are waited for, however often they send.
const WORKERS_QUIET: Duration = Duration::from_millis(100);
const WORKERS_LONGEST: Duration = Duration::from_secs(1);

/// The size a page is drawn at unless it says another.
const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

/// What a page is drawn under: the viewport's size, the medium and the
/// user's preferences.
#[derive(Debug, Clone, Copy)]
struct Setting {
    width: u32,
    height: u32,
    media_type: MediaType,
    preferences: Preferences,
}

/// The content of a page's `<meta name="{name}" content="...">`.
fn meta<'s>(source: &'s str, name: &str) -> Option<&'s str> {
    let at = source.find(&format!("name=\"{name}\""))?;
    let rest = &source[at..];
    rest.find("content=\"")
        .and_then(|c| rest[c + 9..].split('"').next())
}

/// What a page asks to be drawn under, 800 by 600 on a screen with no
/// preferences unless it says otherwise.
fn setting_of(source: &str) -> Setting {
    let (width, height) = meta(source, "reference-size")
        .map(|size| {
            let (w, h) = size.split_once('x').expect("a size is WxH");
            (
                w.parse().expect("a width in pixels"),
                h.parse().expect("a height in pixels"),
            )
        })
        .unwrap_or((WIDTH, HEIGHT));
    let media_type = match meta(source, "reference-media") {
        Some("print") => MediaType::Print,
        _ => MediaType::Screen,
    };
    let color_scheme = match meta(source, "reference-color-scheme") {
        Some("dark") => ColorScheme::Dark,
        _ => ColorScheme::default(),
    };
    let reduced_motion = meta(source, "reference-reduced-motion") == Some("reduce");
    Setting {
        width,
        height,
        media_type,
        preferences: Preferences {
            color_scheme,
            reduced_motion,
        },
    }
}

/// The changes a page asks for with `<meta name="reference-change">`,
/// each an element's id, an attribute and its value, and the times
/// `<meta name="reference-time">` gives, of each change and then of the
/// drawing, in milliseconds (M22).
struct Changes {
    changes: Vec<(String, String, String)>,
    times: Vec<u64>,
}

/// The changes `source` asks for, if it asks for any.
fn changes_of(source: &str) -> Option<Changes> {
    let changes: Vec<(String, String, String)> = meta(source, "reference-change")?
        .split(';')
        .map(|change| {
            let mut words = change.split_whitespace();
            let id = words.next().expect("an element's id").to_string();
            let name = words.next().expect("an attribute").to_string();
            (id, name, words.collect::<Vec<_>>().join(" "))
        })
        .collect();
    let times: Vec<u64> = meta(source, "reference-time")
        .expect("a page that changes says when")
        .split_whitespace()
        .map(|time| time.parse().expect("a time in milliseconds"))
        .collect();
    assert_eq!(
        times.len(),
        changes.len() + 1,
        "a time for each change and one for the drawing"
    );
    Some(Changes { changes, times })
}

/// Makes `changes` as `reference-change.js` makes them for the capture:
/// the first frame drawn at time zero; each change made between two
/// frames drawn at its time, the first showing the animations there, the
/// second starting the transitions the change asks for; then a frame at
/// the drawing's time.
fn make_changes(engine: &mut Engine, changes: &Changes, setting: Setting) {
    let at = |engine: &mut Engine, ms: u64| {
        engine.frame_at(
            setting.width,
            setting.height,
            FrameTime::at(Duration::from_millis(ms)),
        );
    };
    at(engine, 0);
    for ((id, name, value), &time) in changes.changes.iter().zip(&changes.times) {
        at(engine, time);
        let element = {
            let document = engine.document();
            document
                .subtree(document.root())
                .find(|&n| document.attr(n, "id") == Some(id.as_str()))
                .unwrap_or_else(|| panic!("no element has the id {id}"))
        };
        engine
            .mutate(|document| document.set_attribute(element, name, value))
            .expect("an element");
        at(engine, time);
    }
    at(engine, *changes.times.last().expect("a time to draw at"));
}

/// Takes what arrives until `done` says the engine is there.
fn wait(engine: &mut Engine, woken: &Receiver<()>, page: &Path, done: impl Fn(&Engine) -> bool) {
    let deadline = Instant::now() + LOAD;
    loop {
        engine.poll_network();
        if done(engine) {
            return;
        }
        let left = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_else(|| panic!("{} took more than {LOAD:?}", page.display()));
        let _ = woken.recv_timeout(left);
    }
}

/// An engine with the bundled font showing the page at `page`, whose
/// source is `source`, as it asks, and its frame, RGB: a page with
/// images loaded by its URL, and drawn once its images are decoded.
fn render(
    page: &Path,
    source: &str,
    setting: Setting,
    change: impl FnOnce(&mut Engine),
) -> (Engine, Vec<u8>) {
    let mut engine = engine();
    engine.set_media_type(setting.media_type);
    engine.set_preferences(setting.preferences);
    // The page's scripts read the window's size as it loads (M25).
    engine.set_viewport(setting.width, setting.height);
    let scripts = meta(source, "reference-scripts") == Some("on");
    engine.set_scripting(scripts);
    // A page that runs on after its load runs on a clock moved by hand.
    let waits: Option<u64> = meta(source, "reference-wait").and_then(|ms| ms.parse().ok());
    let clock = Arc::new(AtomicU64::new(0));
    if waits.is_some() {
        let hand = Arc::clone(&clock);
        engine.set_clock(Arc::new(move || {
            Duration::from_micros(hand.load(Ordering::Relaxed))
        }));
    }
    let load = meta(source, "reference-load");
    let by_url = scripts || load == Some("file") || load == Some("http");
    // Served over http, the page has an origin (M28).
    let server = (load == Some("http")).then(|| serve_directory(page));
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    if by_url {
        let url = match &server {
            Some(server) => {
                let name = page.file_name().and_then(|name| name.to_str());
                server.url(&format!("/{}", name.expect("the page has a name")))
            }
            None => Url::from_file_path(page).expect("the page has a path"),
        };
        engine.navigate(url);
        wait(&mut engine, &woken, page, |engine| {
            engine.load_state() == &LoadState::Loaded
        });
    } else {
        engine.load_document(source);
    }
    if let Some(ms) = waits {
        let mut at = 0;
        while at <= ms * 1000 {
            clock.store(at, Ordering::Relaxed);
            engine.poll_network();
            wait(&mut engine, &woken, page, |engine| !engine.fetching());
            // A page's workers (M31) run in real time: what they send is
            // taken until they are quiet for a while.
            let longest = Instant::now() + WORKERS_LONGEST;
            while engine.has_workers()
                && Instant::now() < longest
                && woken.recv_timeout(WORKERS_QUIET).is_ok()
            {
                engine.poll_network();
            }
            engine.run_tasks();
            engine.frame_at(
                setting.width,
                setting.height,
                FrameTime::at(Duration::from_micros(at)),
            );
            at += 16_667;
        }
    }
    change(&mut engine);
    // Frames load as the network is polled (M33.2), at first and again
    // when one is taken out and put back.
    let done = |engine: &Engine| engine.frames_settled() && (!by_url || !engine.decoding());
    let frame = loop {
        let frame = engine.frame(setting.width, setting.height);
        if done(&engine) {
            break frame;
        }
        wait(&mut engine, &woken, page, done);
    };
    let rgb = frame.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    (engine, rgb)
}

/// A server of the files beside `page`, by their names, for a page
/// loaded over `http:` (M28).
fn serve_directory(page: &Path) -> TestServer {
    let dir = page
        .parent()
        .expect("the page is in a directory")
        .to_path_buf();
    TestServer::plain(move |request| {
        let name = request.target.trim_start_matches('/');
        let kind = match name.rsplit_once('.').map(|(_, extension)| extension) {
            Some("html") => "text/html",
            Some("js") => "text/javascript",
            Some("css") => "text/css",
            Some("png") => "image/png",
            _ => "application/octet-stream",
        };
        match fs::read(dir.join(name)) {
            Ok(bytes) if !name.contains("..") => Reply::ok(kind, &bytes),
            _ => Reply::with_fields(404, &[("Content-Type", "text/plain")], b"not here"),
        }
    })
}

/// An engine with the bundled font: the pages have no text, but the
/// engine needs a font to exist.
fn engine() -> Engine {
    Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    )
}

/// The pages, in name order.
fn pages() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference");
    let mut pages: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("the reference directory exists")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
        .collect();
    pages.sort();
    pages
}

/// The RGB pixels of a capture, row-major, which must be `setting`'s
/// size.
fn read_capture(path: &Path, setting: Setting) -> Vec<u8> {
    let file = File::open(path).unwrap_or_else(|e| {
        panic!(
            "no capture at {}: run tests/reference/capture.sh ({e})",
            path.display()
        )
    });
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .expect("the capture is a PNG");
    let mut buf = vec![0; reader.output_buffer_size().expect("its size fits")];
    let info = reader.next_frame(&mut buf).expect("the capture decodes");
    assert_eq!(
        (info.width, info.height),
        (setting.width, setting.height),
        "{}: captured at the wrong size",
        path.display()
    );
    assert_eq!(info.bit_depth, png::BitDepth::Eight, "{}", path.display());
    let data = &buf[..info.buffer_size()];
    match info.color_type {
        png::ColorType::Rgb => data.to_vec(),
        png::ColorType::Rgba => data.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        other => panic!("{}: unexpected colour type {other:?}", path.display()),
    }
}

fn write_png(path: &Path, rgb: &[u8], setting: Setting) {
    let file = File::create(path).expect("the output directory is writable");
    let mut encoder = png::Encoder::new(file, setting.width, setting.height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(rgb))
        .expect("the image is written");
}

/// Whether a page declares `<meta name="reference-ignore-scrollbar">`:
/// it scrolls, and the engine's scrollbar is left out of the
/// comparison.
fn ignores_scrollbar(source: &str) -> bool {
    source.contains("name=\"reference-ignore-scrollbar\"")
}

/// `frame`'s pixels with the scrollbar track at the right edge
/// overwritten by `other`'s, so that the two compare equal there.
fn without_scrollbar(frame: &[u8], other: &[u8], setting: Setting) -> Vec<u8> {
    let (width, height) = (setting.width as usize, setting.height as usize);
    let mut out = frame.to_vec();
    let from = (width - SCROLLBAR_WIDTH as usize) * 3;
    for y in 0..height {
        let row = y * width * 3;
        out[row + from..row + width * 3].copy_from_slice(&other[row + from..row + width * 3]);
    }
    out
}

/// How far a page's pixels may differ from its capture, a channel at a
/// time: `inside` anywhere, `edge` at a pixel on an edge in the
/// capture.
#[derive(Debug, Clone, Copy)]
struct Tolerance {
    inside: u8,
    edge: u8,
}

/// The number a page's `<meta name="{name}" content="N">` gives, else
/// zero.
fn meta_number(source: &str, name: &str) -> u8 {
    meta(source, name).and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// The tolerance a page declares: `reference-tolerance` anywhere,
/// `reference-edge-tolerance` on an edge besides, zero for what it
/// does not declare.
fn tolerance_of(source: &str) -> Tolerance {
    let inside = meta_number(source, "reference-tolerance");
    Tolerance {
        inside,
        edge: inside.max(meta_number(source, "reference-edge-tolerance")),
    }
}

/// Whether the pixel at `(x, y)` of an RGB capture `width` by `height`
/// is on an edge: one of its eight neighbours is not its own colour.
fn on_edge(capture: &[u8], x: u32, y: u32, (width, height): (u32, u32)) -> bool {
    let at = |x: u32, y: u32| {
        let i = (y * width + x) as usize * 3;
        &capture[i..i + 3]
    };
    let own = at(x, y);
    (y.saturating_sub(1)..(y + 2).min(height))
        .flat_map(|ny| (x.saturating_sub(1)..(x + 2).min(width)).map(move |nx| (nx, ny)))
        .any(|(nx, ny)| at(nx, ny) != own)
}

/// Compares the engine's render with the capture: `None` when they
/// match, a channel differing by no more than `tolerance` counting as
/// a match, else a description of the difference.
fn compare(
    name: &str,
    expected: &[u8],
    actual: &[u8],
    tolerance: Tolerance,
    setting: Setting,
    out_dir: &Path,
) -> Option<String> {
    if expected == actual {
        return None;
    }
    let within = |e: &[u8], a: &[u8], by: u8| e.iter().zip(a).all(|(e, a)| e.abs_diff(*a) <= by);
    let mut differing = 0usize;
    let mut tolerated = 0usize;
    let mut largest = 0u8;
    let mut first = None;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0, 0);
    let mut diff = Vec::with_capacity(expected.len());
    for (i, (e, a)) in expected.chunks(3).zip(actual.chunks(3)).enumerate() {
        let (x, y) = (i as u32 % setting.width, i as u32 / setting.width);
        let size = (setting.width, setting.height);
        let by = if tolerance.edge > tolerance.inside && on_edge(expected, x, y, size) {
            tolerance.edge
        } else {
            tolerance.inside
        };
        if e != a && within(e, a, by) {
            tolerated += 1;
            diff.extend(e.iter().map(|c| c / 2 + 0x40));
        } else if e != a {
            differing += 1;
            largest = largest.max(
                e.iter()
                    .zip(a)
                    .map(|(e, a)| e.abs_diff(*a))
                    .max()
                    .unwrap_or(0),
            );
            first.get_or_insert((x, y, [e[0], e[1], e[2]], [a[0], a[1], a[2]]));
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            diff.extend_from_slice(&[0xff, 0x00, 0x00]);
        } else {
            diff.extend(e.iter().map(|c| c / 2 + 0x40));
        }
    }
    if differing == 0 {
        println!("{name}: {tolerated} pixels within the tolerance of {tolerance:?}");
        return None;
    }
    fs::create_dir_all(out_dir).expect("the output directory is created");
    write_png(&out_dir.join(format!("{name}.engine.png")), actual, setting);
    write_png(&out_dir.join(format!("{name}.diff.png")), &diff, setting);
    let (x, y, e, a) = first.expect("some pixel differs");
    Some(format!(
        "{name}: {differing} pixels differ within ({min_x}, {min_y})-({max_x}, {max_y}), \
         by {largest} at most, {tolerated} more within the tolerance of {tolerance:?}; \
         the first at ({x}, {y}) is {e:02x?} in the capture and {a:02x?} from the engine; \
         see {}",
        out_dir.display()
    ))
}

#[test]
fn every_reference_page_renders_as_its_capture() {
    let pages = pages();
    assert_eq!(
        pages.len(),
        143,
        "the ten cases of M10's design note, the eleven of M11's, the five of M12's, the three of M16's, \
         the twenty of M17's, the twenty of M18's, the ten of M19's, the eleven of M20's, the nine \
         of M21's, the ten of M22's, the six of M23's, the three of M24's, the four of M25's, \
         the three of M26's, the two of M27's, the one of M28's, the one of M29's \
         the two of M30's, the one of M31's, the six of M32's, the four of M33's and          gap batch E's"
    );
    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/reference");
    let mut failures = Vec::new();
    for page in &pages {
        let name = page
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("a page name");
        let source = fs::read_to_string(page).expect("the page is read");
        let setting = setting_of(&source);
        let expected = read_capture(&page.with_extension("png"), setting);
        let tolerance = tolerance_of(&source);
        let changes = changes_of(&source);
        let (mut engine, mut actual) = render(page, &source, setting, |engine| {
            if let Some(changes) = &changes {
                make_changes(engine, changes, setting);
            }
        });
        if ignores_scrollbar(&source) {
            assert!(
                engine.max_scroll() > 0.0,
                "{name}: the page does not scroll"
            );
            actual = without_scrollbar(&actual, &expected, setting);
        }
        if let Some(failure) = compare(name, &expected, &actual, tolerance, setting, &out_dir) {
            failures.push(failure);
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Changes `engine`'s document four ways and changes it back, drawing
/// a frame after every step (M13): the body's first element hidden and
/// shown again, its last element in tree order taken out and put back
/// where it was, a text node appended and removed, and a tall block
/// inserted first and removed. What is left is the page as loaded.
fn mutate_and_restore(engine: &mut Engine, setting: Setting) {
    let frame = |engine: &mut Engine| {
        engine.frame(setting.width, setting.height);
    };
    let (body, first, last) = {
        let document = engine.document();
        let html = document
            .document_element()
            .expect("the parser implies html");
        let body = document
            .element_children(html)
            .find(|&e| document.tag(e) == Some("body"))
            .expect("the parser implies a body");
        let first = document.element_children(body).next();
        let last = document
            .subtree(body)
            .filter(|&n| n != body && document.is_element(n))
            .last();
        (body, first, last)
    };
    frame(engine);
    if let Some(first) = first {
        let old = engine.document().attr(first, "style").map(str::to_string);
        engine
            .mutate(|document| document.set_attribute(first, "style", "display: none"))
            .expect("an element");
        frame(engine);
        engine
            .mutate(|document| match &old {
                Some(value) => document.set_attribute(first, "style", value),
                None => document.remove_attribute(first, "style"),
            })
            .expect("an element");
        frame(engine);
    }
    if let Some(last) = last {
        let (parent, next) = {
            let document = engine.document();
            (
                document.parent(last).expect("attached"),
                document.next_sibling(last),
            )
        };
        engine
            .mutate(|document| document.remove_child(parent, last))
            .expect("a child");
        frame(engine);
        engine
            .mutate(|document| document.insert_before(parent, last, next))
            .expect("back where it was");
        frame(engine);
    }
    let text = engine.mutate(|document| {
        let text = document.create_text("x").expect("a text node");
        document.append_child(body, text).expect("appended");
        text
    });
    frame(engine);
    engine.mutate(|document| {
        document.remove_child(body, text).expect("a child");
        document.free(text).expect("detached");
    });
    frame(engine);
    let block = engine.mutate(|document| {
        let block = document.create_element("div").expect("an element");
        document
            .set_attribute(block, "style", "height: 300px; background-color: #ff0000")
            .expect("an element");
        let first_child = document.first_child(body);
        document
            .insert_before(body, block, first_child)
            .expect("inserted");
        block
    });
    frame(engine);
    engine.mutate(|document| {
        document.remove_child(body, block).expect("a child");
        document.free(block).expect("detached");
    });
}

#[test]
fn every_reference_page_changed_and_changed_back_renders_as_its_capture() {
    // The engine keeps what it computed between frames; whatever it
    // keeps, a page mutated and restored must draw as it did (M13).
    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/reference");
    let mut failures = Vec::new();
    for page in &pages() {
        let name = page
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("a page name");
        let source = fs::read_to_string(page).expect("the page is read");
        let setting = setting_of(&source);
        let expected = read_capture(&page.with_extension("png"), setting);
        // A page's own changes after the steps, which would cancel the
        // transitions they start: an element hidden has its
        // transitions cancelled, and shown again starts none.
        let changes = changes_of(&source);
        let (_, mut actual) = render(page, &source, setting, |engine| {
            mutate_and_restore(engine, setting);
            if let Some(changes) = &changes {
                make_changes(engine, changes, setting);
            }
        });
        if ignores_scrollbar(&source) {
            actual = without_scrollbar(&actual, &expected, setting);
        }
        let name = format!("{name}.restored");
        let tolerance = tolerance_of(&source);
        if let Some(failure) = compare(&name, &expected, &actual, tolerance, setting, &out_dir) {
            failures.push(failure);
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
