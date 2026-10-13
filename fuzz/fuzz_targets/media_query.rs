//! Media queries and feature queries (M20) on arbitrary text: the text
//! parsed as a media query list, evaluated against media of every kind —
//! a screen and paper, of no size, of a usual one and of a vast one, at
//! several resolutions, with each of the user's preferences — and as an
//! `@supports` condition; then as a style sheet, its `@media`,
//! `@supports`, `@layer` and `@import` rules prepared, a small fixed
//! document cascaded under each medium, and its `@page` rules resolved
//! for each page position. Nothing may panic, a list must answer the same
//! each time it is asked, and styles cascaded under a medium must not
//! count as changed under the same one.

#![no_main]

use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;
use style::{ColorScheme, Media, MediaType, PagePosition, Preferences};

const PAGE: &str = "<div id=a class=b><p class=c><span>x</span><em>y</em></p>\
                    <ul><li>1<li>2</ul></div>";

static DOC: OnceLock<dom::Document> = OnceLock::new();

/// The media the text is evaluated against.
fn media() -> Vec<Media> {
    let mut out = Vec::new();
    for media_type in [MediaType::Screen, MediaType::Print] {
        for (width, height) in [(0.0, 0.0), (800.0, 600.0), (600.0, 800.0), (1e7, 3.0)] {
            for resolution in [1.0, 2.5] {
                for (color_scheme, reduced_motion) in
                    [(ColorScheme::Light, false), (ColorScheme::Dark, true)]
                {
                    out.push(Media {
                        media_type,
                        width,
                        height,
                        device_width: width,
                        device_height: height,
                        resolution,
                        scripting: reduced_motion,
                        preferences: Preferences {
                            color_scheme,
                            reduced_motion,
                        },
                    });
                }
            }
        }
    }
    out
}

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    if s.len() > 4096 {
        return;
    }
    let tokens = css::tokenize(s);
    let list = style::parse_media_query_list(&tokens);
    let _ = style::supports_condition(&tokens);
    let media = media();
    for m in &media {
        let answer = list.evaluate(m);
        assert_eq!(
            answer,
            list.evaluate(m),
            "a list answers the same each time"
        );
    }
    let Ok(sheet) = css::parse_stylesheet(s) else {
        return;
    };
    let sheets = style::Stylesheets::new(vec![sheet]);
    let doc = DOC.get_or_init(|| {
        let page = html::parse_document(PAGE).expect("the fixed page parses");
        dom::Document::from_node(&page)
    });
    for m in media.iter().step_by(3) {
        let context = style::CascadeContext {
            viewport_width: m.width,
            viewport_height: m.height,
            fonts: None,
            language: None,
            media: *m,
            animations: None,
        };
        let styles = sheets.cascade(doc, &context);
        assert!(
            !sheets.media_changed(&styles, m),
            "styles cascaded under a medium are its own"
        );
        for first in [false, true] {
            for left in [false, true] {
                let position = PagePosition {
                    first,
                    left,
                    blank: first && left,
                };
                let page = sheets.page_style(m, 16.0, position);
                let page_box = page.page_box((816.0, 1056.0), 37.0);
                let (w, h) = page_box.area();
                assert!(w >= 1.0 && h >= 1.0, "a page area is a pixel at least");
            }
        }
    }
});
