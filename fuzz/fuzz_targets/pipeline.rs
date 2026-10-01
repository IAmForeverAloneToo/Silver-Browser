//! The whole pipeline: parse, cascade, box tree, layout at several
//! widths, paint, raster. The bundled test font, so every machine shapes
//! the same text; a small raster, so the time goes into the engine
//! rather than into filling pixels.

#![no_main]

use libfuzzer_sys::fuzz_target;
use render::RenderBackend as _;
use std::cell::RefCell;
use std::sync::OnceLock;

static FONTS: OnceLock<text::FontCollection> = OnceLock::new();

thread_local! {
    static BACKEND: RefCell<render::SoftwareBackend> =
        RefCell::new(render::SoftwareBackend::with_fonts(fonts().clone()));
}

fn fonts() -> &'static text::FontCollection {
    FONTS.get_or_init(|| {
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses")
    })
}

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    if s.len() > 4096 {
        return;
    }
    let Ok(doc) = html::parse_document(s) else {
        return;
    };
    let mut sheets = Vec::new();
    if let Ok(sheet) = css::parse_stylesheet(s) {
        sheets.push(sheet);
    }
    let styled = style::style_document(&doc, &sheets);
    let boxes = layout::build_box_tree(&styled);
    for width in [0.0f32, 1.0, 97.0, 800.0] {
        let fragments = layout::layout(&boxes, width, fonts());
        let list = paint::paint_fragments(&fragments, width, 64.0);
        BACKEND.with(|b| {
            let _ = b.borrow_mut().rasterize(&list, width as u32, 64);
        });
    }
});
