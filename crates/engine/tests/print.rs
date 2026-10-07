//! The print references (M20.4; see `docs/design/media.md`, "Print: what
//! is checked against the reference browser"): the pages under
//! `tests/print/`, each printed by the engine and compared, page by page,
//! with the reference browser's print of it, which `examples/print.rs
//! capture` read from its PDF into the text file beside the page. The
//! print must have as many pages, each of the same size with its area in
//! the same place, and each word on the same page as the reference
//! browser's within [`SLACK`] pixels of the same place.
//!
//! A word is the text between spaces. The engine's are read from the
//! display list of each page, where a text item is a word, at the left
//! end of its baseline through the groups around it; the reference
//! browser's from its PDF, which places each glyph, at the place of the
//! word's first glyph. Text a clip hides is read as if it showed, on
//! both sides, and text under `white-space: pre`, an item of several
//! words, is not read as words: the pages have none.
//!
//! The engine's own PDF of each page is read back by the same reader:
//! a page for each page printed, at its box's size, with its area's
//! raster over its area and nothing else.

#[path = "print/reader.rs"]
mod reader;

use std::fs;
use std::path::{Path, PathBuf};

use engine::{Engine, PrintOptions, PrintedPage};
use paint::{DisplayItem, DisplayList};
use reader::{Capture, CapturedPage, Rect, Word};

/// How far a word may be from where the reference browser puts it, in
/// CSS pixels, each way.
const SLACK: f32 = 2.0;

/// How far a page box's size, or its area's place or size, may be from
/// the reference browser's, in CSS pixels: its PDF gives a page's size
/// in device pixels at 300 dots per inch, a third of a CSS pixel.
const BOX_SLACK: f32 = 0.5;

/// The print pages, in order.
fn pages() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/print");
    let mut pages: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("the print pages are there")
        .map(|e| e.expect("the directory reads").path())
        .filter(|p| p.extension().is_some_and(|e| e == "html"))
        .collect();
    pages.sort();
    assert!(!pages.is_empty(), "no print pages in {}", dir.display());
    pages
}

/// An engine with the bundled font showing `page`.
fn engine(page: &Path) -> Engine {
    let source = fs::read_to_string(page).expect("the page reads");
    let mut engine = Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    );
    engine.load_document(&source);
    engine
}

/// What a capture keeps of the engine's print of its document.
fn capture(pages: &[PrintedPage]) -> Capture {
    let pages = pages
        .iter()
        .map(|page| {
            let page_box = page.page_box;
            let (x, y) = page_box.origin();
            let (width, height) = page_box.area();
            let area = Rect {
                x,
                y,
                width,
                height,
            };
            CapturedPage::new(
                (page_box.width, page_box.height),
                Some(area),
                words(&page.list),
            )
        })
        .collect();
    Capture { pages }
}

/// The words `list` shows, each text item one at the left end of its
/// baseline, mapped by the groups around it into the list's own
/// coordinates.
fn words(list: &DisplayList) -> Vec<Word> {
    let mut groups = Vec::new();
    let mut out = Vec::new();
    for item in &list.items {
        match item {
            DisplayItem::PushGroup { matrix, .. } => groups.push(*matrix),
            DisplayItem::PopGroup => {
                groups.pop();
            }
            DisplayItem::Text { text, x, y, .. } => {
                let (mut x, mut y) = (*x, *y);
                for matrix in groups.iter().rev() {
                    let [px, py, _, w] = matrix.apply(x, y, 0.0);
                    (x, y) = (px / w, py / w);
                }
                out.push(Word {
                    text: text.clone(),
                    x,
                    y,
                });
            }
            _ => {}
        }
    }
    out
}

#[test]
fn each_page_prints_as_the_reference_browser_prints_it() {
    let mut failures = Vec::new();
    for page in pages() {
        let captured = page.with_extension("txt");
        let expected =
            fs::read_to_string(&captured).unwrap_or_else(|e| panic!("{}: {e}", captured.display()));
        let expected =
            Capture::parse(&expected).unwrap_or_else(|e| panic!("{}: {e}", captured.display()));
        let printed = engine(&page)
            .printed_pages(&PrintOptions::default())
            .expect("the page prints");
        let differences = reader::differences(&expected, &capture(&printed), SLACK, BOX_SLACK);
        let name = page.file_name().expect("a name").to_string_lossy();
        failures.extend(differences.into_iter().map(|d| format!("{name}: {d}")));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_engines_pdf_reads_back_as_its_pages() {
    // A coarse raster: the PDF's structure is what is read.
    let options = PrintOptions {
        dpi: 24.0,
        ..PrintOptions::default()
    };
    // A raster pixel, in CSS pixels.
    let dot = 96.0 / options.dpi;
    for page in pages() {
        let engine = engine(&page);
        let printed = engine.printed_pages(&options).expect("the page prints");
        let pdf = engine.print(&options).expect("the page prints");
        let read = reader::read(&pdf).expect("the engine's PDF reads");
        let name = page.display();
        assert_eq!(read.len(), printed.len(), "{name}");
        for (index, (r, p)) in read.iter().zip(&printed).enumerate() {
            let at = format!("{name}, page {}", index + 1);
            let page_box = p.page_box;
            let near = |a: f32, b: f32, slack: f32| (a - b).abs() <= slack;
            assert!(
                near(r.size.0, page_box.width, 0.01) && near(r.size.1, page_box.height, 0.01),
                "{at}: {:?}",
                r.size
            );
            let [image] = r.images[..] else {
                panic!("{at}: {} images", r.images.len());
            };
            let (x, y) = page_box.origin();
            let (width, height) = page_box.area();
            assert!(
                near(image.x, x, 0.01) && near(image.y, y, 0.01),
                "{at}: {image:?}"
            );
            // The raster covers the area, rounded up to its pixels.
            assert!(
                (width..width + dot).contains(&image.width)
                    && (height..height + dot).contains(&image.height),
                "{at}: {image:?}"
            );
            assert!(
                r.words.is_empty() && r.fonts.is_empty() && r.clip.is_none(),
                "{at}"
            );
        }
    }
}
