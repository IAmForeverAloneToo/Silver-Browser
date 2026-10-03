//! M10.7's, M11.7's, M12.7's and M16's acceptance: the box model,
//! floats, positioning, stacking, tables and generated boxes against a
//! reference browser.
//!
//! Every page under `tests/reference/` is a case built of boxes alone —
//! no text, since text rasterizers differ by design — and
//! `tests/reference/capture.sh` captures each one at 800 by 600,
//! device scale one, scrollbars hidden, from a headless browser into
//! the `.png` beside it. This test renders each page with the engine
//! at the same size and compares the pixels byte for byte. When they
//! differ, the engine's render and a difference image (the differing
//! pixels in red over the capture, dimmed) are written under
//! `target/reference/` for a look, and the failure names the pixels.
//!
//! A second test loads each page, changes its document and changes it
//! back, drawing a frame after every step, and holds what is left to
//! the same capture (M13): whatever the engine keeps from frame to
//! frame, the page must draw as it did.
//!
//! A page may declare `<meta name="reference-tolerance" content="N">`:
//! a channel may then differ by up to `N`, and the test says how many
//! pixels did. One page uses it, for the last bit of the browser's
//! blending at anti-aliased mitres (M11); every other page is exact.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use engine::Engine;

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

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

/// The RGB pixels of a capture, row-major.
fn read_capture(path: &Path) -> Vec<u8> {
    let file = File::open(path).unwrap_or_else(|e| {
        panic!(
            "no capture at {}: run tests/reference/capture.sh ({e})",
            path.display()
        )
    });
    let mut reader = png::Decoder::new(file)
        .read_info()
        .expect("the capture is a PNG");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("the capture decodes");
    assert_eq!(
        (info.width, info.height),
        (WIDTH, HEIGHT),
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

fn write_png(path: &Path, rgb: &[u8]) {
    let file = File::create(path).expect("the output directory is writable");
    let mut encoder = png::Encoder::new(file, WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(rgb))
        .expect("the image is written");
}

/// The tolerance a page declares, `<meta name="reference-tolerance"
/// content="N">`, else zero.
fn tolerance_of(source: &str) -> u8 {
    let Some(at) = source.find("name=\"reference-tolerance\"") else {
        return 0;
    };
    let rest = &source[at..];
    rest.find("content=\"")
        .and_then(|c| rest[c + 9..].split('"').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// Compares the engine's render with the capture: `None` when they
/// match, a channel differing by no more than `tolerance` counting as
/// a match, else a description of the difference.
fn compare(
    name: &str,
    expected: &[u8],
    actual: &[u8],
    tolerance: u8,
    out_dir: &Path,
) -> Option<String> {
    if expected == actual {
        return None;
    }
    let within = |e: &[u8], a: &[u8]| e.iter().zip(a).all(|(e, a)| e.abs_diff(*a) <= tolerance);
    let mut differing = 0usize;
    let mut tolerated = 0usize;
    let mut first = None;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0, 0);
    let mut diff = Vec::with_capacity(expected.len());
    for (i, (e, a)) in expected.chunks(3).zip(actual.chunks(3)).enumerate() {
        let (x, y) = (i as u32 % WIDTH, i as u32 / WIDTH);
        if e != a && within(e, a) {
            tolerated += 1;
            diff.extend(e.iter().map(|c| c / 2 + 0x40));
        } else if e != a {
            differing += 1;
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
        println!("{name}: {tolerated} pixels within the tolerance of {tolerance}");
        return None;
    }
    fs::create_dir_all(out_dir).expect("the output directory is created");
    write_png(&out_dir.join(format!("{name}.engine.png")), actual);
    write_png(&out_dir.join(format!("{name}.diff.png")), &diff);
    let (x, y, e, a) = first.expect("some pixel differs");
    Some(format!(
        "{name}: {differing} pixels differ within ({min_x}, {min_y})-({max_x}, {max_y}), \
         {tolerated} more within the tolerance of {tolerance}; \
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
        69,
        "the ten cases of M10's design note, the eleven of M11's, the five of M12's, the three of M16's, \
         the twenty of M17's and the twenty of M18's"
    );
    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/reference");
    let mut failures = Vec::new();
    for page in &pages {
        let name = page
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("a page name");
        let expected = read_capture(&page.with_extension("png"));
        let source = fs::read_to_string(page).expect("the page is read");
        let tolerance = tolerance_of(&source);
        let mut engine = engine();
        engine.load_document(&source);
        let frame = engine.frame(WIDTH, HEIGHT);
        let actual: Vec<u8> = frame.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        if let Some(failure) = compare(name, &expected, &actual, tolerance, &out_dir) {
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
fn mutate_and_restore(engine: &mut Engine) {
    let frame = |engine: &mut Engine| {
        engine.frame(WIDTH, HEIGHT);
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
        let expected = read_capture(&page.with_extension("png"));
        let source = fs::read_to_string(page).expect("the page is read");
        let mut engine = engine();
        engine.load_document(&source);
        mutate_and_restore(&mut engine);
        let frame = engine.frame(WIDTH, HEIGHT);
        let actual: Vec<u8> = frame.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let name = format!("{name}.restored");
        if let Some(failure) = compare(&name, &expected, &actual, tolerance_of(&source), &out_dir) {
            failures.push(failure);
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
