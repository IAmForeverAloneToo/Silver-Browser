//! SVG (M32, `docs/design/svg.md`) on arbitrary bytes, read as an SVG
//! image is (an XML document whose root is an `<svg>`, styled by its own
//! sheets) and as a page whose markup holds inline ones: each outer
//! `<svg>`'s render tree built with its `<use>`s restyled and text in a
//! face, drawn on a small bitmap at a scale the input picks, hit at
//! points, and every element asked the scripts' questions. Nothing
//! panics; a tree holds no more elements than `MAX_SVG_ELEMENTS`; the
//! bitmap stays the size it was made, four bytes a pixel.

#![no_main]

use std::sync::OnceLock;

use canvas::{Canvas, Matrix};
use dom::{Document, NodeId};
use libfuzzer_sys::fuzz_target;
use style::{CascadeContext, Stylesheets};

/// The bundled test face, made once.
fn fonts() -> &'static text::FontCollection {
    static FONTS: OnceLock<text::FontCollection> = OnceLock::new();
    FONTS.get_or_init(|| {
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the font parses")
    })
}

/// Whether `node` is an outer `<svg>`: an SVG `svg` whose parent is no
/// SVG element.
fn is_outer(document: &Document, node: NodeId) -> bool {
    document.svg_tag(node) == Some("svg")
        && document
            .parent(node)
            .is_none_or(|parent| document.svg_tag(parent).is_none())
}

/// Builds, draws, hits and asks of each outer svg of `document`, its
/// sizes and scale from `knobs`.
fn exercise(document: &Document, knobs: &[u8]) {
    let knob = |i: usize| knobs.get(i).copied().unwrap_or(7);
    let sheets: Vec<css::Stylesheet> = document
        .subtree(document.root())
        .filter(|&n| document.svg_tag(n) == Some("style"))
        .filter_map(|n| css::parse_stylesheet(&document.text_content(n)).ok())
        .collect();
    let sheets = Stylesheets::new(sheets);
    let context = CascadeContext::default();
    let styles = sheets.cascade(document, &context);
    let recascade = sheets.recascader(document, &styles, &context);
    let context = svg::Context {
        document,
        styles: &styles,
        recascade: Some(&recascade),
        fonts: Some(fonts()),
    };
    let outers: Vec<NodeId> = document
        .subtree(document.root())
        .filter(|&n| is_outer(document, n))
        .take(3)
        .collect();
    for (i, svg) in outers.into_iter().enumerate() {
        let (w, h) = (1 + u32::from(knob(i)) % 48, 1 + u32::from(knob(i + 1)) % 48);
        let scale = [1.0, 0.5, 1.25, 2.0, 3.0][usize::from(knob(i + 2)) % 5];
        let tree = svg::build(context, svg, f64::from(w), f64::from(h));
        assert!(tree.elements <= svg::MAX_SVG_ELEMENTS, "{}", tree.elements);
        let (rw, rh) = (
            ((w as f32 * scale).ceil() as u32).max(1),
            ((h as f32 * scale).ceil() as u32).max(1),
        );
        let mut canvas = Canvas::new(rw, rh, false);
        svg::draw(
            &tree,
            &mut canvas,
            &Matrix::new(scale.into(), 0.0, 0.0, scale.into(), 0.0, 0.0),
        );
        assert_eq!((canvas.width(), canvas.height()), (rw, rh));
        if let Some(bytes) = canvas.bytes() {
            assert_eq!(bytes.len(), (rw * rh * 4) as usize);
        }
        for p in 0..4u8 {
            let x = f64::from(knob(usize::from(p) + 3)) / 4.0;
            let y = f64::from(knob(usize::from(p) + 7)) / 4.0;
            let _ = tree.hit(x, y);
        }
        let questions = [
            svg::Query::BBox,
            svg::Query::Ctm,
            svg::Query::TextLength,
            svg::Query::TotalLength,
            svg::Query::PointAtLength(f64::from(knob(9))),
            svg::Query::InFill(1.0, 1.0),
            svg::Query::InStroke(2.0, 2.0),
        ];
        // Asked as the engine asks: of one tree while nothing changes.
        let Some(asked) = svg::query_tree(context, svg) else {
            continue;
        };
        for node in document.subtree(svg).take(24) {
            for question in questions {
                let _ = svg::query_in(context, &asked, node, question);
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let (knobs, rest) = data.split_at(data.len().min(12));
    // As an SVG image or document: XML.
    if let Ok(document) = html::xml::parse_xml(rest) {
        exercise(&document, knobs);
    }
    // As a page's markup, inline svgs among it.
    if let Ok(text) = std::str::from_utf8(rest)
        && let Ok(node) = html::parse_document(text)
    {
        exercise(&Document::from_node(&node), knobs);
    }
});
