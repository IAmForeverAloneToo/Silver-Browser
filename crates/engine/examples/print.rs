//! The capture of the print references (M20.4 and M20.5; see
//! `docs/design/media.md`, "Print: what is checked against the reference
//! browser"): the pages under `tests/print/` and `about:silver`'s,
//! `src/about-silver.html`, which `tests/print.rs` prints and compares,
//! page by page, with the reference browser's print of them.
//!
//! `cargo run -p engine --example print -- capture [page.html ...]`
//! prints each page to PDF with the reference browser, on US Letter at
//! its default margins and with no header or footer, reads the PDF with
//! `tests/print/reader.rs`, and writes under `tests/print/`, by the
//! page's name with `.txt` for `.html`, each page's box and area and
//! each word on it with the left end of its baseline. The pages are
//! captured alone when named, all of them when none is.
//!
//! The browser is `$BROWSER`, else `chrome-headless-shell` on the PATH,
//! with `$BROWSER_FLAGS` added (a container running as root needs
//! `--no-sandbox`). It is given this engine's own font, DejaVu Sans,
//! from `crates/text/fonts`, for the page's text, by a style at the top
//! of a copy of the page it prints, which the page's own rules may
//! override with the same font, as the pages under `tests/print/` do;
//! the capture stops if any text of the PDF is in another font. The
//! copy, `.capture-page.html`, and the PDF, `.capture-page.pdf`, are
//! written beside the page and removed after. The capture says, for
//! each page, how many pages, words and images its print has and how
//! many words differ from the capture before; look at what changed
//! before committing it.

#![forbid(unsafe_code)]

#[path = "../tests/print/reader.rs"]
mod reader;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use reader::Capture;

/// The font the browser is given, by its name in the PDF.
const FONT: &str = "DejaVuSans";

/// The pages, by their paths: those under `tests/print/` in name order,
/// then `about:silver`'s.
fn all_pages() -> Result<Vec<PathBuf>, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("tests/print");
    let mut pages: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "html"))
        .collect();
    pages.sort();
    pages.push(root.join("src/about-silver.html"));
    Ok(pages)
}

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
    let all = match all_pages() {
        Ok(all) => all,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let mut chosen = Vec::new();
    for name in pages {
        match all
            .iter()
            .find(|p| p.file_name().is_some_and(|n| n == name.as_str()))
        {
            Some(page) => chosen.push(page.clone()),
            None => {
                eprintln!("{name}: no such print page");
                return ExitCode::FAILURE;
            }
        }
    }
    let pages = if chosen.is_empty() { all } else { chosen };
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
    let stem = name.trim_end_matches(".html");
    let source = fs::read_to_string(page).map_err(|e| e.to_string())?;
    let copy = page.with_file_name(format!(".capture-{name}"));
    let pdf = page.with_file_name(format!(".capture-{stem}.pdf"));
    let printed = fs::write(&copy, with_font(&source)?)
        .map_err(|e| e.to_string())
        .and_then(|()| print(&copy, &pdf));
    let bytes = printed.and_then(|()| fs::read(&pdf).map_err(|e| e.to_string()));
    let _ = fs::remove_file(&copy);
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
    let out = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/print")
        .join(format!("{stem}.txt"));
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

/// `source` with a style giving the browser this engine's font for its
/// text, first in its head.
fn with_font(source: &str) -> Result<String, String> {
    let font = Path::new(env!("CARGO_MANIFEST_DIR")).join("../text/fonts/DejaVuSans.ttf");
    let font = fs::canonicalize(&font).map_err(|e| format!("{}: {e}", font.display()))?;
    let style = format!(
        "<style>@font-face {{ font-family: \"Silver Engine Sans\"; src: url(\"file://{}\"); }} \
         html {{ font-family: \"Silver Engine Sans\"; }}</style>",
        font.display()
    );
    let head = source
        .to_ascii_lowercase()
        .find("<head>")
        .ok_or("the page has no <head> to give the font in")?
        + "<head>".len();
    let mut out = source.to_string();
    out.insert_str(head, &style);
    Ok(out)
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
