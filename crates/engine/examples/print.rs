//! The capture of the print references (M20.4; see
//! `docs/design/media.md`, "Print: what is checked against the reference
//! browser"): pages under `tests/print/`, which `tests/print.rs` prints
//! and compares, page by page, with the reference browser's print of
//! them.
//!
//! `cargo run -p engine --example print -- capture [page.html ...]`
//! prints each page to PDF with the reference browser, on US Letter at
//! its default margins and with no header or footer, reads the PDF with
//! `tests/print/reader.rs`, and writes beside the page, under its name
//! with `.txt` for `.html`, each page's box and area and each word on it
//! with the left end of its baseline. The pages are captured alone when
//! named, all of them when none is.
//!
//! The browser is `$BROWSER`, else `chrome-headless-shell` on the PATH,
//! with `$BROWSER_FLAGS` added (a container running as root needs
//! `--no-sandbox`). A page's `@font-face` gives the browser this
//! engine's own font, DejaVu Sans, from `crates/text/fonts`: the capture
//! stops if any text of the PDF is in another font. The PDF is written
//! beside the page as `.capture-page.pdf` and removed after. The capture
//! says, for each page, how many pages, words and images its print has
//! and how many words differ from the capture before; look at what
//! changed before committing it.

#[path = "../tests/print/reader.rs"]
mod reader;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use reader::Capture;

/// The font the pages give the browser, by its name in the PDF.
const FONT: &str = "DejaVuSans";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some((command, pages)) = args.split_first() else {
        eprintln!("usage: print capture [page.html ...]");
        return ExitCode::FAILURE;
    };
    if command != "capture" {
        eprintln!("usage: print capture [page.html ...]");
        return ExitCode::FAILURE;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/print");
    let pages: Vec<PathBuf> = if pages.is_empty() {
        let mut all: Vec<PathBuf> = match fs::read_dir(&dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "html"))
                .collect(),
            Err(error) => {
                eprintln!("{}: {error}", dir.display());
                return ExitCode::FAILURE;
            }
        };
        all.sort();
        all
    } else {
        pages.iter().map(|p| dir.join(p)).collect()
    };
    let mut failed = false;
    for page in &pages {
        match capture(page) {
            Ok(summary) => println!("{}: {summary}", page.display()),
            Err(error) => {
                eprintln!("{}: {error}", page.display());
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Captures `page`, saying what its print has.
fn capture(page: &Path) -> Result<String, String> {
    let name = page
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("the page's name is not text")?;
    let pdf = page.with_file_name(format!(".capture-{}.pdf", name.trim_end_matches(".html")));
    let printed = print(page, &pdf);
    let bytes = printed.and_then(|()| fs::read(&pdf).map_err(|e| e.to_string()));
    let _ = fs::remove_file(&pdf);
    let pages = reader::read(&bytes?)?;
    for (index, p) in pages.iter().enumerate() {
        if let Some(font) = p.fonts.iter().find(|f| *f != FONT) {
            return Err(format!(
                "page {}'s text is in {font}, not {FONT}: the browser did not load the page's font",
                index + 1
            ));
        }
    }
    let capture = Capture::of(&pages)?;
    let header = format!(
        "{name}, printed to PDF by the reference browser on US Letter at its\n\
         default margins and read by examples/print.rs capture: each page's\n\
         box and its area, in CSS pixels, then each word on it, the left end\n\
         of its baseline in the area, and its text."
    );
    let out = page.with_extension("txt");
    let text = capture.to_text(&header);
    let changed = match fs::read_to_string(&out) {
        Ok(before) => {
            // Compared as the file has them, to its two decimals.
            let (before, after) = (Capture::parse(&before)?, Capture::parse(&text)?);
            let differences = reader::differences(&before, &after, 0.0, 0.0);
            format!("{} differences from the capture before", differences.len())
        }
        Err(_) => "no capture before".to_string(),
    };
    fs::write(&out, text).map_err(|e| e.to_string())?;
    let words: usize = capture.pages.iter().map(|p| p.words.len()).sum();
    let images: usize = pages.iter().map(|p| p.images.len()).sum();
    Ok(format!(
        "{} pages, {words} words, {images} images; {changed}",
        capture.pages.len()
    ))
}

/// Prints `page` to `pdf` with the browser.
fn print(page: &Path, pdf: &Path) -> Result<(), String> {
    let browser = env::var("BROWSER").unwrap_or_else(|_| "chrome-headless-shell".into());
    let flags = env::var("BROWSER_FLAGS").unwrap_or_default();
    let page = fs::canonicalize(page).map_err(|e| e.to_string())?;
    let output = Command::new(&browser)
        .args([
            "--disable-gpu",
            "--force-color-profile=srgb",
            "--no-pdf-header-footer",
            "--virtual-time-budget=5000",
        ])
        .args(flags.split_whitespace())
        .arg(format!("--print-to-pdf={}", pdf.display()))
        .arg(format!("file://{}", page.display()))
        .output()
        .map_err(|e| format!("{browser}: {e}"))?;
    if !output.status.success() {
        return Err(format!("{browser} exited with {}", output.status));
    }
    Ok(())
}
