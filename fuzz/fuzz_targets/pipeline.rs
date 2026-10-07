//! The whole pipeline: parse, cascade, box tree, layout at several
//! widths, paint at several scroll offsets, raster. The bundled test
//! font, so every machine shapes the same text; a small raster, so the
//! time goes into the engine rather than into filling pixels.
//!
//! The rasterizer draws each frame from the last one, this input's or
//! the one before's, redrawing only what changed (M13.3), and keeps the
//! groups that are layers, the document's as tiles, which it draws
//! ahead of a scroll after each frame (M19): every frame must be byte
//! for byte what a raster of the whole list gives. Drawing ahead is
//! given all the time it wants, so that a run does the same work
//! however fast the machine is.
//!
//! Then the document is printed (M20): cascaded for paper, laid out at
//! the width its `@page` rules give the first page, kept to a few hundred
//! pixels, cut into pages as tall as theirs give each, each page painted
//! and drawn small, and the PDF written. The pages must be one under
//! another from the top, at least one and [`layout::MAX_PAGES`] at most.

#![no_main]

use libfuzzer_sys::fuzz_target;
use render::RenderBackend as _;
use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

static FONTS: OnceLock<text::FontCollection> = OnceLock::new();

thread_local! {
    /// The backend under test, which keeps its last frame, and one that
    /// is made to rasterize every list whole.
    static BACKENDS: RefCell<(render::SoftwareBackend, render::SoftwareBackend)> =
        RefCell::new((
            render::SoftwareBackend::with_fonts(fonts().clone()),
            render::SoftwareBackend::with_fonts(fonts().clone()),
        ));
}

fn fonts() -> &'static text::FontCollection {
    FONTS.get_or_init(|| {
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses")
    })
}

/// Prints `doc` with `sheets` as the engine prints a page, small.
fn print(doc: &dom::Document, sheets: &style::Stylesheets) {
    let paper = (120.0, 100.0);
    let media = style::Media {
        media_type: style::MediaType::Print,
        ..style::Media::screen(paper.0, paper.1)
    };
    let page_box = |position: style::PagePosition| {
        sheets
            .page_style(&media, 16.0, position)
            .page_box(paper, 10.0)
    };
    let first = style::PagePosition {
        first: true,
        ..style::PagePosition::default()
    };
    let (width, height) = page_box(first).layout_area();
    let width = width.min(400.0);
    let context = style::CascadeContext {
        viewport_width: width,
        viewport_height: height,
        fonts: None,
        language: None,
        media,
    };
    let styles = sheets.cascade(doc, &context);
    let recascade = sheets.recascader(doc, &styles, &context);
    let boxes = layout::build_box_tree(doc, &styles, &recascade);
    let laid = layout::layout_document_in(&boxes, width, Some(height), fonts());
    let paginated = layout::paginate(laid, &|position| page_box(position).layout_area().1);
    assert!(!paginated.pages.is_empty() && paginated.pages.len() <= layout::MAX_PAGES);
    let mut top = 0.0f32;
    for page in &paginated.pages {
        assert_eq!(page.top, top, "the pages are one under another");
        top = page.top + page.height;
    }
    let mut pages = Vec::new();
    for page in paginated.pages.iter().take(4) {
        let viewport = paint::Viewport {
            width,
            height: page.height,
            scroll_y: page.top,
        };
        let list = paint::paint_fragments_in(&paginated.roots, viewport, &());
        let (w, h) = (97, (page.height as u32).clamp(1, 64));
        let rgba = BACKENDS.with(|backends| {
            let (_, whole) = &mut *backends.borrow_mut();
            whole.forget_frame();
            whole.rasterize(&list, w, h).to_vec()
        });
        let rgb: Vec<u8> = rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        pages.push(render::pdf::PdfPage::new(
            paper,
            (10.0, 10.0),
            96.0,
            (w, h),
            &rgb,
        ));
    }
    let pdf = render::pdf::write_pdf(&pages, "fuzz");
    assert!(pdf.starts_with(b"%PDF-"));
}

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    if s.len() > 4096 {
        return;
    }
    let Ok(parsed) = html::parse_document(s) else {
        return;
    };
    let doc = dom::Document::from_node(&parsed);
    let mut sheets = Vec::new();
    if let Ok(sheet) = css::parse_stylesheet(s) {
        sheets.push(sheet);
    }
    let sheets = style::Stylesheets::new(sheets);
    for width in [0.0f32, 1.0, 97.0, 800.0] {
        let context = style::CascadeContext {
            viewport_width: width,
            viewport_height: 64.0,
            fonts: None,
            language: None,
            media: style::Media::screen(width, 64.0),
        };
        let styles = sheets.cascade(&doc, &context);
        let recascade = sheets.recascader(&doc, &styles, &context);
        let boxes = layout::build_box_tree(&doc, &styles, &recascade);
        let document = layout::layout_document(&boxes, width, fonts());
        for scroll_y in [0.0f32, 30.0, document.height] {
            let viewport = paint::Viewport {
                width,
                height: 64.0,
                scroll_y,
            };
            let list = paint::paint_fragments(&document.roots, viewport);
            BACKENDS.with(|backends| {
                let (backend, whole) = &mut *backends.borrow_mut();
                let pixels = backend.rasterize(&list, width as u32, 64);
                whole.forget_frame();
                let same = pixels == whole.rasterize(&list, width as u32, 64);
                assert!(
                    same,
                    "{:?} redrawn from the last frame differs from a whole raster",
                    backend.redrawn()
                );
                backend.draw_ahead(Instant::now() + Duration::from_secs(3600));
            });
        }
    }
    print(&doc, &sheets);
});
