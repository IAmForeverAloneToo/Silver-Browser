//! The engine glue: document + pipeline orchestration.
//!
//! `Engine` owns the session state — the document, its stylesheets, and
//! the render backend — and drives the per-frame pipeline (`style` ->
//! `layout` -> `paint` -> `render`). Documents enter as HTML source
//! through `html::parse_document`; `<style>` element contents are parsed
//! into [`css::Stylesheet`]s and cascaded by `style::style_document` (M5)
//! into a `style::StyledDom`; `layout::build_box_tree` + `layout::layout`
//! (M6) resolve that into real, positioned `layout::Fragment`s — actual
//! box-model geometry (width/margin resolution, margin collapsing), not
//! a per-tag guess or a flat vertical list — which `paint::paint_fragments`
//! consumes each frame. As the engine grows, this crate gains navigation
//! and resource orchestration (`net`). It owns no platform state:
//! windowing is `shell`'s job.

use dom::Node;
use paint::paint_fragments;
use render::{RenderBackend as _, SoftwareBackend};
use style::StyledDom;
use text::FontError;

/// The document rendered when no other is loaded.
const DOCUMENT_SOURCE: &str = "\
<html>\
  <head>\
    <style>\
      h1 { color: #cdd6f4; }\
      p { color: #a6adc8; }\
      .highlight { color: #f9e2af; font-size: 20px; }\
      .card {\
        display: block;\
        margin: 16px 0;\
        padding: 12px;\
        border-width: 2px;\
        border-style: solid;\
        border-color: #89b4fa;\
        background-color: #313244;\
      }\
    </style>\
  </head>\
  <body>\
    <h1>Hello from Silver Browser</h1>\
    <p>A hobby browser engine in Rust — milestone 6:</p>\
    <p class=\"highlight\">this page is laid out by our own block formatting context — real width/margin resolution and margin collapsing, not a flat vertical list.</p>\
    <div class=\"card\">\
      <p>This card's background and border are painted from real fragment geometry (M6.8) — the box model actually exists now.</p>\
    </div>\
  </body>\
</html>";

/// A browsing session: one document, its stylesheets, and its rasterizer.
pub struct Engine {
    backend: SoftwareBackend,
    /// Loaded once and reused every frame — `text::FontCollection::new()`
    /// runs real system-font queries plus file reads/parses (the primary
    /// font and, since M8.1, its per-script fallbacks), not something to
    /// redo per frame. A second instance from `backend`'s own (`render`
    /// owns its font collection privately) — see `layout::layout`'s doc
    /// for why that duplication is today's accepted v1 tradeoff.
    layout_fonts: text::FontCollection,
    document: Node,
    stylesheets: Vec<css::Stylesheet>,
}

impl Engine {
    /// Creates a session rendering the built-in document.
    pub fn new() -> Result<Self, FontError> {
        let mut engine = Engine {
            backend: SoftwareBackend::new()?,
            layout_fonts: text::FontCollection::new()?,
            document: Node::document(Vec::new()),
            stylesheets: Vec::new(),
        };
        engine.load_document(DOCUMENT_SOURCE);
        Ok(engine)
    }

    /// Replaces the session document by parsing HTML `source`, including
    /// any `<style>` elements (parsed into stylesheets).
    pub fn load_document(&mut self, source: &str) {
        if let Ok(document) = html::parse_document(source) {
            let stylesheets = style_texts(&document)
                .iter()
                .filter_map(|text| css::parse_stylesheet(text).ok())
                .collect();
            self.document = document;
            self.stylesheets = stylesheets;
        }
    }

    /// The current document (for inspection and tests).
    pub fn document(&self) -> &Node {
        &self.document
    }

    /// The document's stylesheets, in document order.
    pub fn stylesheets(&self) -> &[css::Stylesheet] {
        &self.stylesheets
    }

    /// Cascades the current document and its stylesheets into a
    /// `StyledDom` (built-in UA stylesheet included automatically — see
    /// `style::style_document`).
    pub fn styled_dom(&self) -> StyledDom {
        style::style_document(&self.document, &self.stylesheets)
    }

    /// Lays the current document out against a `viewport_width`-wide
    /// initial containing block (M6.6) and returns its fragment tree.
    pub fn fragments(&self, viewport_width: f32) -> Vec<layout::Fragment> {
        let styled = self.styled_dom();
        let boxes = layout::build_box_tree(&styled);
        layout::layout(&boxes, viewport_width, &self.layout_fonts)
    }

    /// Renders the current document into an RGBA8 framebuffer for a
    /// `width` x `height` viewport.
    ///
    /// A pure function of (document, stylesheets, size): no platform
    /// state, no side effects outside the backend's caches.
    pub fn frame(&mut self, width: u32, height: u32) -> Vec<u8> {
        let fragments = self.fragments(width as f32);
        let list = paint_fragments(&fragments, width as f32, height as f32);
        self.backend.rasterize(&list, width, height)
    }
}

/// Collects the concatenated text of every `<style>` element in the
/// document, in document order.
fn style_texts(document: &Node) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(node: &Node, out: &mut Vec<String>) {
        match node {
            Node::Document(children) => {
                for child in children {
                    walk(child, out);
                }
            }
            Node::Element(el) => {
                if el.tag == "style" {
                    let mut text = String::new();
                    for child in &el.children {
                        if let Node::Text(t) = child {
                            text.push_str(&t.data);
                        }
                    }
                    if !text.trim().is_empty() {
                        out.push(text);
                    }
                    return; // no nested style elements
                }
                for child in &el.children {
                    walk(child, out);
                }
            }
            Node::Text(_) => {}
        }
    }
    walk(document, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_produces_full_size_buffer() {
        let Ok(mut engine) = Engine::new() else {
            return; // No system font on this machine.
        };
        let buf = engine.frame(64, 32);
        assert_eq!(buf.len(), 64 * 32 * 4);
    }

    #[test]
    fn builtin_document_parses_to_expected_text() {
        let Ok(engine) = Engine::new() else {
            return;
        };
        assert!(
            engine
                .document()
                .text_content()
                .contains("Hello from Silver Browser")
        );
    }

    #[test]
    fn loading_a_document_replaces_the_tree() {
        let Ok(mut engine) = Engine::new() else {
            return;
        };
        engine.load_document("<h1>Alpha</h1>");
        assert_eq!(engine.document().text_content(), "Alpha");
    }

    #[test]
    fn different_documents_render_differently() {
        let Ok(mut engine) = Engine::new() else {
            return;
        };
        let before = engine.frame(128, 256);
        engine.load_document("<p>XX</p>");
        let after = engine.frame(128, 256);
        assert_ne!(before, after);
    }

    #[test]
    fn builtin_style_element_parses_into_stylesheet() {
        let Ok(engine) = Engine::new() else {
            return;
        };
        assert_eq!(engine.stylesheets().len(), 1);
        let rules = engine.stylesheets()[0].rules.len();
        assert!(rules >= 2, "expected at least h1 and p rules, got {rules}");
    }

    #[test]
    fn loading_a_document_replaces_stylesheets() {
        let Ok(mut engine) = Engine::new() else {
            return;
        };
        assert!(!engine.stylesheets().is_empty());
        engine.load_document("<p>no styles here</p>");
        assert!(engine.stylesheets().is_empty());
    }

    #[test]
    fn style_elements_accumulate_into_sheets() {
        let Ok(mut engine) = Engine::new() else {
            return;
        };
        engine.load_document(
            "<style>p { color: red; }</style><p>x</p><style>p { margin: 0; }</style>",
        );
        assert_eq!(engine.stylesheets().len(), 2);
    }

    // ---- M5 acceptance (roadmap M5.9) --------------------------------------

    #[test]
    fn deleting_the_style_block_still_yields_ua_styling() {
        let Ok(mut engine) = Engine::new() else {
            return;
        };
        // No `<style>` at all: zero author stylesheets, but the cascade
        // always includes the built-in UA sheet underneath.
        engine.load_document("<html><body><h1>Title</h1><p>Body</p></body></html>");
        assert!(engine.stylesheets().is_empty());
        let styled = engine.styled_dom();
        let h1 = styled
            .flat
            .element_indices()
            .find(|&i| styled.flat.nodes[i].tag == "h1")
            .expect("an h1 element exists");
        // ua.css: `h1 { display: block; font-size: 32px; ... }`.
        assert_eq!(styled.styles[h1].display, style::computed::Display::Block);
        assert_eq!(styled.styles[h1].font_size_px, 32.0);
    }

    #[test]
    fn adding_a_color_rule_visibly_changes_rendered_pixels() {
        // The literal M5.9 acceptance scenario: a screenshot-diff v0 —
        // compare frame buffers before and after a color-changing rule.
        let Ok(mut engine) = Engine::new() else {
            return;
        };
        engine.load_document("<p>Some text to render</p>");
        let before = engine.frame(200, 100);
        engine.load_document("<style>p { color: red; }</style><p>Some text to render</p>");
        let after = engine.frame(200, 100);
        assert_ne!(
            before, after,
            "changing `color` must change the rendered pixels"
        );
    }

    #[test]
    fn display_none_removes_the_element_from_the_frame() {
        let Ok(mut engine) = Engine::new() else {
            return;
        };
        engine.load_document("<p>Visible</p>");
        let with_p = engine.frame(200, 100);
        engine.load_document("<style>p { display: none; }</style><p>Visible</p>");
        let without_p = engine.frame(200, 100);
        assert_ne!(with_p, without_p);
    }

    // ---- M6 acceptance (roadmap M6.10) -------------------------------------

    #[test]
    fn fragments_resolve_real_box_model_geometry() {
        let Ok(mut engine) = Engine::new() else {
            return;
        };
        engine.load_document(
            "<style>div { display: block; width: 100px; height: 50px;
                          margin: 10px; padding: 5px;
                          border-width: 2px; border-style: solid; }</style>\
             <div></div>",
        );
        let fragments = engine.fragments(800.0);
        let div = fragments
            .iter()
            .find(|f| matches!(f.kind, layout::FragmentKind::Box))
            .expect("a box fragment for the div");
        // border-box: 100 + 2*5 + 2*2 = 114 wide, 50 + 2*5 + 2*2 = 64
        // tall; positioned after the 10px margin.
        assert_eq!((div.x, div.y, div.w, div.h), (10.0, 10.0, 114.0, 64.0));
    }

    #[test]
    fn adjacent_margins_collapse_through_the_real_pipeline() {
        let Ok(mut engine) = Engine::new() else {
            return;
        };
        engine.load_document(
            "<style>p { display: block; margin: 20px 0; }</style>\
             <p>one</p><p>two</p>",
        );
        let fragments = engine.fragments(800.0);
        let boxes: Vec<_> = fragments
            .iter()
            .filter(|f| matches!(f.kind, layout::FragmentKind::Box))
            .collect();
        assert_eq!(boxes.len(), 2);
        // The gap between the two border-boxes is max(20, 20) = 20, not
        // the sum (40) — `boxes[0].h` is its own real content height
        // (one line of text), unrelated to the collapsing itself.
        assert_eq!(boxes[1].y, boxes[0].y + boxes[0].h + 20.0);
    }

    #[test]
    fn builtin_document_renders_the_card_background_and_border() {
        let Ok(engine) = Engine::new() else {
            return;
        };
        let fragments = engine.fragments(800.0);
        let has_card_background = fragments.iter().any(|f| {
            matches!(f.kind, layout::FragmentKind::Box) && f.style.background_color.a > 0.0
        });
        assert!(
            has_card_background,
            "the built-in demo's .card background should paint (M6.8)"
        );
    }

    // ---- M7 acceptance (roadmap M7.8) --------------------------------------

    #[test]
    fn narrow_viewport_wraps_text_onto_more_lines_than_a_wide_one() {
        let Ok(mut engine) = Engine::new() else {
            return;
        };
        engine
            .load_document("<p>one two three four five six seven eight nine ten eleven twelve</p>");
        // f32 isn't `Ord` (NaN), so count distinct baselines by rounding
        // to whole pixels and deduping, rather than putting raw floats
        // in a set.
        let distinct_baselines = |viewport_width: f32| -> usize {
            let mut ys: Vec<i32> = engine
                .fragments(viewport_width)
                .iter()
                .filter_map(|f| match f.kind {
                    layout::FragmentKind::Text(_) => Some(f.y.round() as i32),
                    layout::FragmentKind::Box => None,
                })
                .collect();
            ys.sort_unstable();
            ys.dedup();
            ys.len()
        };

        let wide_lines = distinct_baselines(5000.0);
        let narrow_lines = distinct_baselines(60.0);
        assert!(
            narrow_lines > wide_lines,
            "a narrow viewport ({narrow_lines} lines) should wrap onto more lines than a wide one ({wide_lines})"
        );
    }

    #[test]
    fn text_align_center_shifts_text_right_of_a_left_aligned_equivalent() {
        let Ok(mut engine) = Engine::new() else {
            return;
        };
        engine.load_document("<p>hi</p>");
        let left_x = engine
            .fragments(800.0)
            .into_iter()
            .find(|f| matches!(f.kind, layout::FragmentKind::Text(_)))
            .map(|f| f.x)
            .expect("a text fragment");

        engine.load_document("<style>p { text-align: center; }</style><p>hi</p>");
        let center_x = engine
            .fragments(800.0)
            .into_iter()
            .find(|f| matches!(f.kind, layout::FragmentKind::Text(_)))
            .map(|f| f.x)
            .expect("a text fragment");

        assert!(
            center_x > left_x,
            "centered text should start further right than left-aligned text"
        );
    }
}
