//! The whole pipeline: parse, cascade, box tree, layout at several
//! widths, paint at several scroll offsets, raster. The bundled test
//! font, so every machine shapes the same text; a small raster, so the
//! time goes into the engine rather than into filling pixels.
//!
//! The rasterizer draws each frame from the last one, this input's or
//! the one before's, redrawing only what changed (M13.3): every frame
//! must be byte for byte what a raster of the whole list gives.

#![no_main]

use libfuzzer_sys::fuzz_target;
use render::RenderBackend as _;
use std::cell::RefCell;
use std::sync::OnceLock;

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
    for width in [0.0f32, 1.0, 97.0, 800.0] {
        let context = style::CascadeContext {
            viewport_width: width,
            viewport_height: 64.0,
            fonts: None,
        };
        let styles = style::cascade_document(&doc, &sheets, &context);
        let boxes = layout::build_box_tree(&doc, &styles);
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
                assert!(
                    pixels == whole.rasterize(&list, width as u32, 64),
                    "{:?} redrawn from the last frame differs from a whole raster",
                    backend.redrawn()
                );
            });
        }
    }
});
