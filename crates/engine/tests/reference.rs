//! M10.7's acceptance: the box model against a reference browser.
//!
//! Every page under `tests/reference/` is a box-model case built of
//! boxes alone — no text, since text rasterizers differ by design —
//! and `tests/reference/capture.sh` captures each one at 800 by 600,
//! device scale one, scrollbars hidden, from a headless browser into
//! the `.png` beside it. This test renders each page with the engine
//! at the same size and compares the pixels byte for byte. When they
//! differ, the engine's render and a difference image (the differing
//! pixels in red over the capture, dimmed) are written under
//! `target/reference/` for a look, and the failure names the pixels.

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

/// Compares the engine's render with the capture: `None` when they
/// match, else a description of the difference.
fn compare(name: &str, expected: &[u8], actual: &[u8], out_dir: &Path) -> Option<String> {
    if expected == actual {
        return None;
    }
    let mut differing = 0usize;
    let mut first = None;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0, 0);
    let mut diff = Vec::with_capacity(expected.len());
    for (i, (e, a)) in expected.chunks(3).zip(actual.chunks(3)).enumerate() {
        let (x, y) = (i as u32 % WIDTH, i as u32 / WIDTH);
        if e != a {
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
    fs::create_dir_all(out_dir).expect("the output directory is created");
    write_png(&out_dir.join(format!("{name}.engine.png")), actual);
    write_png(&out_dir.join(format!("{name}.diff.png")), &diff);
    let (x, y, e, a) = first.expect("some pixel differs");
    Some(format!(
        "{name}: {differing} pixels differ within ({min_x}, {min_y})-({max_x}, {max_y}); \
         the first at ({x}, {y}) is {e:02x?} in the capture and {a:02x?} from the engine; \
         see {}",
        out_dir.display()
    ))
}

#[test]
fn every_reference_page_renders_as_its_capture() {
    let pages = pages();
    assert_eq!(pages.len(), 10, "the ten cases of the design note");
    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/reference");
    let mut failures = Vec::new();
    for page in &pages {
        let name = page
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("a page name");
        let expected = read_capture(&page.with_extension("png"));
        let source = fs::read_to_string(page).expect("the page is read");
        let mut engine = engine();
        engine.load_document(&source);
        let frame = engine.frame(WIDTH, HEIGHT);
        let actual: Vec<u8> = frame.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        if let Some(failure) = compare(name, &expected, &actual, &out_dir) {
            failures.push(failure);
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
