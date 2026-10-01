//! The engine glue: document + pipeline orchestration.
//!
//! `Engine` owns the session state — the document, its stylesheets, the
//! scroll position (M9) and the render backend — and drives the
//! per-frame pipeline (`style` -> `layout` -> `paint` -> `render`).
//! Documents enter as HTML source through `html::parse_document`;
//! `<style>` element contents are parsed into [`css::Stylesheet`]s and
//! cascaded by `style::style_document` (M5) into a `style::StyledDom`;
//! `layout::build_box_tree` + `layout::layout_document` (M6, M9.1)
//! resolve that into a real, positioned fragment tree in document
//! coordinates — actual box-model geometry (width/margin resolution,
//! margin collapsing), not a per-tag guess or a flat vertical list —
//! which `paint::paint_fragments` consumes each frame for the current
//! [`paint::Viewport`]. As the engine grows, this crate gains navigation
//! and resource orchestration (`net`). It owns no platform state:
//! windowing is `shell`'s job.
//!
//! The cascade is computed once per document and the layout once per
//! (document, viewport width) (M9.5): a scroll, or another frame at the
//! same width, reruns paint and raster only, and [`Engine::layout_runs`]
//! counts the layouts actually done, so a test can prove it. Scrolling
//! (M9.2) is a target the frame approaches a fixed fraction per
//! [`Engine::tick`] — the "fixed-tick lerp" of the roadmap, no clock
//! involved; compositor-smooth scrolling is M19. The scrollbar is
//! in-engine chrome (decision D9): drawn into the frame after the
//! document, its geometry exposed through [`Engine::scrollbar`] for the
//! shell to map presses and drags. Hit testing (M9.4) answers with the
//! `FlatDom` index of the element under a viewport point, the
//! groundwork for M26's events; [`Engine::link_at`] walks up from it to
//! the nearest `<a href>`.

use dom::Node;
use layout::{DocumentLayout, Fragment, FragmentKind};
use paint::{DisplayItem, Rgba8, Viewport, paint_fragments};
use render::{RenderBackend as _, SoftwareBackend};
use style::computed::BorderStyle;
use style::{CascadeContext, FontMeasure, StyledDom};
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
      .clipped { height: 40px; overflow: hidden; }\
      a { color: #89b4fa; }\
    </style>\
  </head>\
  <body>\
    <h1>Hello from Silver Browser</h1>\
    <p>A hobby browser engine in Rust — milestone 9:</p>\
    <p class=\"highlight\">this page is laid out by our own block formatting context — real width/margin resolution and margin collapsing, not a flat vertical list.</p>\
    <div class=\"card\">\
      <p>This card's background and border are painted from real fragment geometry (M6.8) — the box model actually exists now.</p>\
    </div>\
    <div class=\"card clipped\">\
      <p>This card is 40px tall with overflow: hidden, so this second sentence is clipped away at its padding edge rather than painted over whatever follows. The clip is a rectangle in the display list (M9.3).</p>\
    </div>\
    <p>Scroll with the wheel, the arrow keys, Page Up and Page Down, Home and End, the space bar, or by dragging the scrollbar at the right (M9.2). Click <a href=\"about:silver\">this link</a> and the engine finds the anchor under the pointer (M9.4; the shell only prints it — navigation is M15).</p>\
    <p>The paragraphs below only exist to make the page taller than the window.</p>\
    <p>One. The quick brown fox jumps over the lazy dog, and the layout is cached across scrolls (M9.5): only paint and raster run again.</p>\
    <p>Two. The quick brown fox jumps over the lazy dog.</p>\
    <p>Three. The quick brown fox jumps over the lazy dog.</p>\
    <p>Four. The quick brown fox jumps over the lazy dog.</p>\
    <p>Five. The quick brown fox jumps over the lazy dog.</p>\
    <p>Six. The quick brown fox jumps over the lazy dog.</p>\
    <p>Seven. The quick brown fox jumps over the lazy dog.</p>\
    <p>Eight. The quick brown fox jumps over the lazy dog.</p>\
    <p>Nine. The quick brown fox jumps over the lazy dog.</p>\
    <p>Ten. The end of the page.</p>\
  </body>\
</html>";

/// How much of the remaining distance to the scroll target one
/// [`Engine::tick`] covers (M9.2's fixed-tick lerp): a 1000px scroll
/// settles in about twenty ticks, a third of a second at sixty frames.
const SCROLL_TICK_FRACTION: f32 = 1.0 / 3.0;
/// Within this distance of the target, a tick lands on it exactly.
const SCROLL_SNAP_PX: f32 = 0.5;
/// The scrollbar track's width, in px, at the viewport's right edge.
pub const SCROLLBAR_WIDTH: f32 = 12.0;
/// The scrollbar thumb never gets shorter than this, however long the
/// document, so it stays something a pointer can grab.
const SCROLLBAR_MIN_THUMB: f32 = 24.0;
const SCROLLBAR_TRACK: Rgba8 = Rgba8::rgb(0x31, 0x32, 0x44);
const SCROLLBAR_THUMB: Rgba8 = Rgba8::rgb(0x6c, 0x70, 0x86);

/// A browsing session: one document, its stylesheets, its scroll
/// position and its rasterizer.
pub struct Engine {
    backend: SoftwareBackend,
    /// Loaded once and reused every frame — `text::FontCollection::new()`
    /// runs real system-font queries plus file reads/parses (the primary
    /// font and, since M8.1, its per-script fallbacks), not something to
    /// redo per frame. The backend holds a clone of the same collection
    /// (a `FontCollection` clones by sharing its face data).
    layout_fonts: text::FontCollection,
    document: Node,
    stylesheets: Vec<css::Stylesheet>,
    /// The cascade of the current document, computed on first use and
    /// dropped when a document loads, or when the viewport changes and
    /// the document used viewport units (M10.1).
    styled: Option<CachedStyle>,
    /// How many cascades have run: what proves a resize without
    /// viewport units does not.
    style_runs: u64,
    /// The layout of the current document for one viewport width,
    /// dropped when a document loads or the width changes (M9.5).
    layout: Option<CachedLayout>,
    /// How many layouts have run: what proves a scroll does not.
    layout_runs: u64,
    /// The last frame's size: what scrolling is clamped to.
    viewport: (u32, u32),
    /// The scroll offset shown, and the one being approached.
    scroll_y: f32,
    scroll_target: f32,
}

struct CachedLayout {
    width: f32,
    /// The viewport height the layout was given; it matters only when
    /// the layout says it depends on it.
    height: f32,
    layout: DocumentLayout,
}

struct CachedStyle {
    /// The viewport the cascade resolved its units against.
    viewport: (u32, u32),
    styled: StyledDom,
}

/// The engine's fonts, lent to the cascade for `ex` and `ch`.
struct FontMeasurer<'a>(&'a text::FontCollection);

impl FontMeasure for FontMeasurer<'_> {
    fn x_height(&self, font_size_px: f32) -> f32 {
        self.0.x_height(font_size_px)
    }

    fn zero_advance(&self, font_size_px: f32) -> f32 {
        self.0.zero_advance(font_size_px)
    }
}

/// The scrollbar's geometry for the last frame (M9.2), in viewport
/// pixels: the track at the right edge and the thumb within it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scrollbar {
    /// The track: `x` is its left edge, `w` its width, `h` the viewport's
    /// height (it starts at the top).
    pub x: f32,
    pub w: f32,
    pub h: f32,
    /// The thumb's top and height within the track.
    pub thumb_y: f32,
    pub thumb_h: f32,
}

impl Scrollbar {
    /// Whether a viewport point is on the track (thumb included).
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= 0.0 && y < self.h
    }

    /// Whether a viewport point is on the thumb.
    pub fn thumb_contains(&self, x: f32, y: f32) -> bool {
        self.contains(x, y) && y >= self.thumb_y && y < self.thumb_y + self.thumb_h
    }
}

impl Engine {
    /// Creates a session rendering the built-in document, with the
    /// system's fonts.
    pub fn new() -> Result<Self, FontError> {
        Ok(Self::with_fonts(text::FontCollection::new()?))
    }

    /// Creates a session rendering the built-in document, with fonts
    /// already loaded (a test's bundled font, say). Layout and the
    /// rasterizer share the collection by cloning its handles.
    pub fn with_fonts(fonts: text::FontCollection) -> Self {
        let mut engine = Engine {
            backend: SoftwareBackend::with_fonts(fonts.clone()),
            layout_fonts: fonts,
            document: Node::document(Vec::new()),
            stylesheets: Vec::new(),
            styled: None,
            style_runs: 0,
            layout: None,
            layout_runs: 0,
            viewport: (0, 0),
            scroll_y: 0.0,
            scroll_target: 0.0,
        };
        engine.load_document(DOCUMENT_SOURCE);
        engine
    }

    /// Replaces the session document by parsing HTML `source`, including
    /// any `<style>` elements (parsed into stylesheets). The cascade and
    /// layout caches and the glyph cache start over, and the new
    /// document starts at the top.
    pub fn load_document(&mut self, source: &str) {
        if let Ok(document) = html::parse_document(source) {
            let stylesheets = style_texts(&document)
                .iter()
                .filter_map(|text| css::parse_stylesheet(text).ok())
                .collect();
            self.document = document;
            self.stylesheets = stylesheets;
            self.styled = None;
            self.layout = None;
            self.scroll_y = 0.0;
            self.scroll_target = 0.0;
            self.backend.clear_glyph_cache();
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
    /// `style::style_document_in`), against the last frame's viewport
    /// and the engine's fonts. A fresh computation for inspection; the
    /// frame pipeline uses the cached one.
    pub fn styled_dom(&self) -> StyledDom {
        self.cascade(self.viewport)
    }

    fn cascade(&self, viewport: (u32, u32)) -> StyledDom {
        let fonts = FontMeasurer(&self.layout_fonts);
        let context = CascadeContext {
            viewport_width: viewport.0 as f32,
            viewport_height: viewport.1 as f32,
            fonts: Some(&fonts),
        };
        style::style_document_in(&self.document, &self.stylesheets, &context)
    }

    /// How many cascades this session has run.
    pub fn style_runs(&self) -> u64 {
        self.style_runs
    }

    /// Lays the current document out against a `viewport_width`-wide
    /// initial containing block (M6.6) and returns its fragments in
    /// paint order, flat — a fresh computation for inspection; the
    /// frame pipeline uses [`Engine::layout`].
    pub fn fragments(&self, viewport_width: f32) -> Vec<Fragment> {
        let styled = self.cascade((viewport_width as u32, self.viewport.1));
        let boxes = layout::build_box_tree(&styled);
        layout::flatten(
            layout::layout_document_in(
                &boxes,
                viewport_width,
                Some(self.viewport.1 as f32),
                &self.layout_fonts,
            )
            .roots,
        )
    }

    /// The cached cascade for the current viewport: recomputed when none
    /// is cached, or when the viewport changed and the document's styles
    /// depend on it (M10.1). A recomputation drops the cached layout.
    fn styled(&mut self) -> &StyledDom {
        let stale = self.styled.as_ref().is_none_or(|cached| {
            cached.styled.uses_viewport_units && cached.viewport != self.viewport
        });
        if stale {
            let styled = self.cascade(self.viewport);
            self.style_runs += 1;
            self.layout = None;
            self.styled = Some(CachedStyle {
                viewport: self.viewport,
                styled,
            });
        }
        &self.styled.as_ref().expect("just computed").styled
    }

    /// The document's layout for `viewport_width`, computed once per
    /// document and width (M9.5) and reused by every frame, scroll and
    /// hit test at that width.
    pub fn layout(&mut self, viewport_width: f32) -> &DocumentLayout {
        // A viewport change can invalidate the cascade, and with it the
        // layout — so the cascade is checked first.
        self.styled();
        let viewport_height = self.viewport.1 as f32;
        let stale = self.layout.as_ref().is_none_or(|cached| {
            cached.width != viewport_width
                || (cached.layout.depends_on_viewport_height && cached.height != viewport_height)
        });
        if stale {
            let styled = &self.styled.as_ref().expect("just computed").styled;
            let boxes = layout::build_box_tree(styled);
            let layout = layout::layout_document_in(
                &boxes,
                viewport_width,
                Some(viewport_height),
                &self.layout_fonts,
            );
            self.layout_runs += 1;
            self.layout = Some(CachedLayout {
                width: viewport_width,
                height: viewport_height,
                layout,
            });
        }
        &self.layout.as_ref().expect("just computed").layout
    }

    /// How many layouts this session has run.
    pub fn layout_runs(&self) -> u64 {
        self.layout_runs
    }

    /// Renders the current document into an RGBA8 framebuffer for a
    /// `width` x `height` viewport at the current scroll offset, with
    /// the scrollbar when the document is taller than the viewport. A
    /// viewport over `render::MAX_FRAME_PIXELS` gets an empty buffer
    /// rather than an allocation the machine cannot make.
    ///
    /// A pure function of (document, stylesheets, size, scroll): no
    /// platform state, no side effects outside the caches.
    pub fn frame(&mut self, width: u32, height: u32) -> Vec<u8> {
        if width as u64 * height as u64 > render::MAX_FRAME_PIXELS {
            return Vec::new();
        }
        self.viewport = (width, height);
        // The document may be shorter at this width than where the
        // offset was.
        let max = self.max_scroll();
        self.scroll_target = self.scroll_target.clamp(0.0, max);
        self.scroll_y = self.scroll_y.clamp(0.0, max);
        let viewport = Viewport {
            width: width as f32,
            height: height as f32,
            scroll_y: self.scroll_y,
        };
        let scrollbar = self.scrollbar();
        let layout = self.layout(width as f32);
        let mut list = paint_fragments(&layout.roots, viewport);
        if let Some(bar) = scrollbar {
            list.items.push(DisplayItem::Rect {
                x: bar.x,
                y: 0.0,
                w: bar.w,
                h: bar.h,
                color: SCROLLBAR_TRACK,
            });
            list.items.push(DisplayItem::Rect {
                x: bar.x + 2.0,
                y: bar.thumb_y,
                w: bar.w - 4.0,
                h: bar.thumb_h,
                color: SCROLLBAR_THUMB,
            });
        }
        self.backend.rasterize(&list, width, height)
    }

    // ---- Scrolling (M9.2) ---------------------------------------------------

    /// The scroll offset the current frame shows, in px.
    pub fn scroll_y(&self) -> f32 {
        self.scroll_y
    }

    /// The scroll offset being approached (equal to [`Engine::scroll_y`]
    /// once the animation has settled).
    pub fn scroll_target(&self) -> f32 {
        self.scroll_target
    }

    /// The furthest the last frame's viewport can scroll: the document's
    /// height past the viewport's, never negative. Zero before the first
    /// frame, since there is no viewport to measure against yet; only
    /// vertical scrolling exists (the document is laid out to the
    /// viewport's width, and a wider word overflows to the right).
    pub fn max_scroll(&mut self) -> f32 {
        let (width, height) = self.viewport;
        if width == 0 {
            return 0.0;
        }
        (self.layout(width as f32).height - height as f32).max(0.0)
    }

    /// Scrolls by `dy` px (down when positive), animated over the next
    /// ticks; the target is clamped to the document.
    pub fn scroll_by(&mut self, dy: f32) {
        let target = self.scroll_target + dy;
        self.scroll_to(target);
    }

    /// Scrolls to offset `y`, animated over the next ticks; clamped to
    /// the document (so an infinite `y` means the bottom or the top),
    /// and a NaN is ignored.
    pub fn scroll_to(&mut self, y: f32) {
        if y.is_nan() {
            return;
        }
        let max = self.max_scroll();
        self.scroll_target = y.clamp(0.0, max);
    }

    /// Jumps to offset `y` at once, no animation (a thumb drag follows
    /// the pointer); clamped to the document.
    pub fn set_scroll(&mut self, y: f32) {
        self.scroll_to(y);
        self.scroll_y = self.scroll_target;
    }

    /// Jumps to the fraction `f` of the scroll range (0 is the top, 1
    /// the bottom), as a thumb drag asks.
    pub fn scroll_to_fraction(&mut self, f: f32) {
        let max = self.max_scroll();
        self.set_scroll(f.clamp(0.0, 1.0) * max);
    }

    /// Advances the scroll animation one frame: the shown offset moves
    /// [`SCROLL_TICK_FRACTION`] of the way to the target, landing on it
    /// within [`SCROLL_SNAP_PX`]. Returns whether another frame is
    /// wanted (the offset has not reached the target).
    pub fn tick(&mut self) -> bool {
        let remaining = self.scroll_target - self.scroll_y;
        if remaining.abs() <= SCROLL_SNAP_PX {
            self.scroll_y = self.scroll_target;
            return false;
        }
        self.scroll_y += remaining * SCROLL_TICK_FRACTION;
        true
    }

    /// The scrollbar for the last frame's viewport, or `None` when the
    /// document fits in it (or no frame has been drawn).
    pub fn scrollbar(&mut self) -> Option<Scrollbar> {
        let (width, height) = self.viewport;
        let max = self.max_scroll();
        if max <= 0.0 {
            return None;
        }
        let (width, height) = (width as f32, height as f32);
        let document_height = max + height;
        let thumb_h = (height * height / document_height)
            .max(SCROLLBAR_MIN_THUMB)
            .min(height);
        let thumb_y = (height - thumb_h) * (self.scroll_y / max);
        Some(Scrollbar {
            x: width - SCROLLBAR_WIDTH,
            w: SCROLLBAR_WIDTH,
            h: height,
            thumb_y,
            thumb_h,
        })
    }

    // ---- Hit testing (M9.4) -------------------------------------------------

    /// The element under the viewport point `(x, y)` at the current
    /// scroll offset, as its `FlatDom` index: the last fragment in paint
    /// order whose box contains the point (a text fragment's box is its
    /// line, and answers with its containing element). Content clipped
    /// away by an ancestor's `overflow` is not hit. `None` over nothing
    /// — the viewport background, or the scrollbar, which the shell
    /// checks first.
    pub fn hit_test(&mut self, x: f32, y: f32) -> Option<usize> {
        let (width, _) = self.viewport;
        if width == 0 || !x.is_finite() || !y.is_finite() {
            return None;
        }
        let py = y + self.scroll_y;
        let layout = self.layout(width as f32);
        let mut hit = None;
        hit_in(&layout.roots, x, py, &mut hit);
        hit
    }

    /// The `href` of the nearest `<a>` at or above the element under
    /// the viewport point, if any (M9.4's "link clicks"; following it
    /// is M15's navigation).
    pub fn link_at(&mut self, x: f32, y: f32) -> Option<String> {
        let mut node = self.hit_test(x, y)?;
        let flat = &self.styled().flat;
        loop {
            let n = &flat.nodes[node];
            if n.tag == "a"
                && let Some((_, href)) = n.attrs.iter().find(|(name, _)| name == "href")
            {
                return Some(href.clone());
            }
            node = flat.parent_element(node)?;
        }
    }
}

/// Finds the last fragment in paint order containing the document
/// point, descending into every box whose content could be at the
/// point: all of them, unless the box clips, in which case only when
/// the point is inside its padding box.
fn hit_in(fragments: &[Fragment], x: f32, y: f32, hit: &mut Option<usize>) {
    for f in fragments {
        match f.kind {
            FragmentKind::Box => {
                if f.style.overflow.clips() && !padding_box_contains(f, x, y) {
                    continue;
                }
                if f.contains(x, y) {
                    *hit = Some(f.node);
                }
                hit_in(&f.children, x, y, hit);
            }
            FragmentKind::Text(_) => {
                if f.contains(x, y) {
                    *hit = Some(f.node);
                }
            }
        }
    }
}

/// Whether the point is within the box's padding box — what `overflow`
/// clips to (CSS2 §11.1.1), the border counting as zero-width when
/// `border-style` is `none`.
fn padding_box_contains(f: &Fragment, x: f32, y: f32) -> bool {
    let [top, right, bottom, left] = if f.style.border_style == BorderStyle::None {
        [0.0; 4]
    } else {
        f.style.border_width
    };
    x >= f.x + left && x < f.x + f.w - right && y >= f.y + top && y < f.y + f.h - bottom
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

    /// An engine over the bundled test font: the same text metrics on
    /// every machine, and a machine without fonts still runs the tests
    /// rather than passing them on nothing.
    fn test_engine() -> Engine {
        Engine::with_fonts(
            text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
                .expect("the bundled font parses"),
        )
    }

    #[test]
    fn frame_produces_full_size_buffer() {
        let mut engine = test_engine();
        let buf = engine.frame(64, 32);
        assert_eq!(buf.len(), 64 * 32 * 4);
    }

    #[test]
    fn builtin_document_parses_to_expected_text() {
        let engine = test_engine();
        assert!(
            engine
                .document()
                .text_content()
                .contains("Hello from Silver Browser")
        );
    }

    #[test]
    fn loading_a_document_replaces_the_tree() {
        let mut engine = test_engine();
        engine.load_document("<h1>Alpha</h1>");
        assert_eq!(engine.document().text_content(), "Alpha");
    }

    #[test]
    fn different_documents_render_differently() {
        let mut engine = test_engine();
        let before = engine.frame(128, 256);
        engine.load_document("<p>XX</p>");
        let after = engine.frame(128, 256);
        assert_ne!(before, after);
    }

    #[test]
    fn builtin_style_element_parses_into_stylesheet() {
        let engine = test_engine();
        assert_eq!(engine.stylesheets().len(), 1);
        let rules = engine.stylesheets()[0].rules.len();
        assert!(rules >= 2, "expected at least h1 and p rules, got {rules}");
    }

    #[test]
    fn loading_a_document_replaces_stylesheets() {
        let mut engine = test_engine();
        assert!(!engine.stylesheets().is_empty());
        engine.load_document("<p>no styles here</p>");
        assert!(engine.stylesheets().is_empty());
    }

    #[test]
    fn style_elements_accumulate_into_sheets() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>p { color: red; }</style><p>x</p><style>p { margin: 0; }</style>",
        );
        assert_eq!(engine.stylesheets().len(), 2);
    }

    // ---- M5 acceptance (roadmap M5.9) --------------------------------------

    #[test]
    fn deleting_the_style_block_still_yields_ua_styling() {
        let mut engine = test_engine();
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
        let mut engine = test_engine();
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
        let mut engine = test_engine();
        engine.load_document("<p>Visible</p>");
        let with_p = engine.frame(200, 100);
        engine.load_document("<style>p { display: none; }</style><p>Visible</p>");
        let without_p = engine.frame(200, 100);
        assert_ne!(with_p, without_p);
    }

    // ---- M6 acceptance (roadmap M6.10) -------------------------------------

    #[test]
    fn fragments_resolve_real_box_model_geometry() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>div { display: block; width: 100px; height: 50px;
                          margin: 10px; padding: 5px;
                          border-width: 2px; border-style: solid; }</style>\
             <div></div>",
        );
        let boxes = boxes_of(&engine, "div");
        let div = &boxes[0];
        // border-box: 100 + 2*5 + 2*2 = 114 wide, 50 + 2*5 + 2*2 = 64
        // tall. The implied body's 8px margin (the UA sheet) puts it at
        // x = 18; its top margin and the div's adjoin, so y is their
        // collapsed 10, not 18.
        assert_eq!((div.x, div.y, div.w, div.h), (18.0, 10.0, 114.0, 64.0));
    }

    /// The box fragments of every `tag` element, in paint order.
    fn boxes_of(engine: &Engine, tag: &str) -> Vec<layout::Fragment> {
        let styled = engine.styled_dom();
        engine
            .fragments(800.0)
            .into_iter()
            .filter(|f| {
                matches!(f.kind, layout::FragmentKind::Box) && styled.flat.nodes[f.node].tag == tag
            })
            .collect()
    }

    #[test]
    fn adjacent_margins_collapse_through_the_real_pipeline() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>p { display: block; margin: 20px 0; }</style>\
             <p>one</p><p>two</p>",
        );
        let boxes = boxes_of(&engine, "p");
        assert_eq!(boxes.len(), 2);
        // The gap between the two border-boxes is max(20, 20) = 20, not
        // the sum (40) — `boxes[0].h` is its own real content height
        // (one line of text), unrelated to the collapsing itself.
        assert_eq!(boxes[1].y, boxes[0].y + boxes[0].h + 20.0);
    }

    #[test]
    fn builtin_document_renders_the_card_background_and_border() {
        let engine = test_engine();
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
        let mut engine = test_engine();
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
        let mut engine = test_engine();
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

    #[test]
    fn a_frame_past_the_pixel_cap_is_empty_rather_than_gigabytes() {
        let mut engine = test_engine();
        assert!(engine.frame(100_000, 100_000).is_empty());
        assert_eq!(engine.frame(64, 32).len(), 64 * 32 * 4);
    }

    #[test]
    fn a_hostile_font_size_renders_without_taking_the_process_down() {
        // 1e9px used to overflow the mask arithmetic (debug) or ask for
        // gigabytes (release); 1e39px is infinity in f32.
        let mut engine = test_engine();
        for size in ["1e9px", "1e39px", "50000px", "-16px"] {
            engine.load_document(&format!("<p style=\"font-size:{size};margin:0\">M</p>"));
            assert_eq!(engine.frame(200, 100).len(), 200 * 100 * 4, "{size}");
        }
    }

    #[test]
    fn non_finite_lengths_do_not_put_text_at_the_corner() {
        // `1e39px` margins used to collapse to NaN and every fragment
        // landed at y = NaN, which the rasterizer drew at the top-left.
        let mut engine = test_engine();
        engine.load_document(
            "<div style=\"margin-top:1e39px;margin-bottom:-1e39px\"></div><p style=\"margin:0\">hi</p>",
        );
        for f in engine.fragments(800.0) {
            assert!(f.x.is_finite() && f.y.is_finite(), "{f:?}");
        }
    }

    // ---- M9 ---------------------------------------------------------------

    /// Enough paragraphs to be far taller than a 100px viewport.
    const TALL: &str = "<style>p { margin: 0; font-size: 20px; }</style>\
        <p>one</p><p>two</p><p>three</p><p>four</p><p>five</p><p>six</p>\
        <p>seven</p><p>eight</p><p>nine</p><p>ten</p><p>eleven</p><p>twelve</p>";

    /// The pixel at `(x, y)` of a `width`-wide RGBA8 buffer.
    fn px(buf: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * width + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn scrolling_is_clamped_to_the_document_and_animates_to_its_target() {
        let mut engine = test_engine();
        engine.load_document(TALL);
        // Before a frame there is no viewport to scroll in.
        engine.scroll_by(100.0);
        assert_eq!(engine.scroll_target(), 0.0);
        engine.frame(200, 100);
        let max = engine.max_scroll();
        assert!(max > 100.0, "twelve 20px lines overflow a 100px viewport");
        engine.scroll_by(1e9);
        assert_eq!(engine.scroll_target(), max);
        assert_eq!(engine.scroll_y(), 0.0, "the shown offset moves per tick");
        let mut ticks = 0;
        while engine.tick() {
            ticks += 1;
            assert!(ticks < 100, "the animation settles");
        }
        assert_eq!(engine.scroll_y(), max);
        assert!(ticks >= 3, "it took more than one frame ({ticks})");
        engine.scroll_by(-1e9);
        assert_eq!(engine.scroll_target(), 0.0);
        engine.scroll_to(f32::INFINITY);
        assert_eq!(engine.scroll_target(), max);
        engine.scroll_to(f32::NAN);
        assert_eq!(engine.scroll_target(), max, "a NaN is ignored");
        engine.scroll_to(f32::NEG_INFINITY);
        assert_eq!(engine.scroll_target(), 0.0);
        engine.set_scroll(50.0);
        assert_eq!((engine.scroll_y(), engine.scroll_target()), (50.0, 50.0));
        assert!(!engine.tick());
        engine.scroll_to_fraction(1.0);
        assert_eq!(engine.scroll_y(), max);
        engine.scroll_to_fraction(0.0);
        assert_eq!(engine.scroll_y(), 0.0);
        // A new document starts at the top.
        engine.set_scroll(50.0);
        engine.load_document(TALL);
        assert_eq!(engine.scroll_y(), 0.0);
    }

    #[test]
    fn a_scroll_reruns_paint_and_raster_but_not_layout() {
        let mut engine = test_engine();
        engine.load_document(TALL);
        assert_eq!(engine.layout_runs(), 0);
        engine.frame(200, 100);
        assert_eq!(engine.layout_runs(), 1);
        engine.scroll_by(40.0);
        while engine.tick() {}
        engine.frame(200, 100);
        engine.frame(200, 100);
        engine.hit_test(10.0, 10.0);
        assert_eq!(
            engine.layout_runs(),
            1,
            "scrolling and hit testing reuse the layout"
        );
        engine.frame(200, 150);
        assert_eq!(
            engine.layout_runs(),
            1,
            "a taller viewport changes nothing about layout"
        );
        engine.frame(300, 150);
        assert_eq!(engine.layout_runs(), 2, "a wider viewport does");
        engine.load_document("<p>another</p>");
        engine.frame(300, 150);
        assert_eq!(engine.layout_runs(), 3, "a new document does");
    }

    #[test]
    fn a_scrolled_frame_shows_the_content_further_down() {
        // A 100px red box at the top of the document, text, then a tall
        // spacer so there is room to scroll. Column 70 is right of the
        // text and left of the scrollbar.
        let mut engine = test_engine();
        engine.load_document(
            "<div style=\"height: 100px; background-color: #ff0000; margin: 0\"></div>\
             <p style=\"margin:0\">below</p>\
             <div style=\"height: 1000px; margin: 0\"></div>",
        );
        let top = engine.frame(100, 100);
        assert_eq!(px(&top, 100, 70, 50), [0xff, 0, 0, 0xff]);
        engine.set_scroll(60.0);
        assert_eq!(engine.scroll_y(), 60.0);
        let scrolled = engine.frame(100, 100);
        assert_eq!(
            px(&scrolled, 100, 70, 30),
            [0xff, 0, 0, 0xff],
            "the box moved up by 60"
        );
        assert_ne!(
            px(&scrolled, 100, 70, 50),
            [0xff, 0, 0, 0xff],
            "its bottom edge is now at 40"
        );
        assert_eq!(
            px(&scrolled, 100, 70, 0),
            [0xff, 0, 0, 0xff],
            "the top row is still inside it"
        );
    }

    #[test]
    fn hit_test_finds_the_innermost_box_or_text_under_the_point() {
        let mut engine = test_engine();
        // outer: 0..200 x 0..100; inner: at (20, 10), 100 x 50 with text.
        // The body's UA margin is zeroed so the numbers stay these.
        engine.load_document(
            "<style>body { margin: 0 }</style>\
             <div class=outer style=\"width: 200px; height: 100px; margin: 0; padding: 10px 0 0 20px\">\
               <div class=inner style=\"width: 100px; height: 50px; margin: 0\">hi</div>\
             </div>",
        );
        engine.frame(400, 300);
        let styled = engine.styled_dom();
        let tag = |node: Option<usize>| node.map(|n| styled.flat.nodes[n].tag.clone());
        let class = |node: Option<usize>| {
            node.and_then(|n| {
                styled.flat.nodes[n]
                    .attrs
                    .iter()
                    .find(|(k, _)| k == "class")
                    .map(|(_, v)| v.clone())
            })
        };
        // On the inner box's text line.
        assert_eq!(
            class(engine.hit_test(25.0, 15.0)),
            Some("inner".to_string())
        );
        // On the inner box past the text, still the inner box.
        assert_eq!(
            class(engine.hit_test(110.0, 55.0)),
            Some("inner".to_string())
        );
        // In the outer box's padding.
        assert_eq!(class(engine.hit_test(5.0, 5.0)), Some("outer".to_string()));
        // Right of both boxes: the implied body, as wide as the viewport.
        assert_eq!(tag(engine.hit_test(300.0, 50.0)).as_deref(), Some("body"));
        // Below everything, and off the viewport: nothing.
        assert_eq!(engine.hit_test(50.0, 250.0), None);
        assert_eq!(engine.hit_test(-1.0, 5.0), None);
        assert_eq!(engine.hit_test(f32::NAN, 5.0), None);
    }

    #[test]
    fn hit_test_follows_the_scroll_offset_and_respects_clipping() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }</style>\
             <div class=spacer style=\"height: 100px; margin: 0\"></div>\
             <div class=clip style=\"height: 10px; overflow: hidden; margin: 0\">\
               <p style=\"margin: 0; font-size: 40px; width: 50px\">tall</p>\
             </div>\
             <div class=tail style=\"height: 1000px; margin: 0\"></div>",
        );
        engine.frame(200, 100);
        let styled = engine.styled_dom();
        let name = |node: Option<usize>| {
            node.map(|n| {
                let n = &styled.flat.nodes[n];
                n.attrs
                    .iter()
                    .find(|(k, _)| k == "class")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| n.tag.clone())
            })
        };
        // The clipping box is at document y 100..110, off the unscrolled
        // viewport; after scrolling by 100 it is at viewport y 0..10,
        // and the visible top of its text is what is under the pointer.
        assert_eq!(name(engine.hit_test(5.0, 5.0)), Some("spacer".to_string()));
        engine.set_scroll(100.0);
        assert_eq!(engine.scroll_y(), 100.0);
        assert_eq!(name(engine.hit_test(5.0, 5.0)), Some("p".to_string()));
        // Right of the 50px-wide p, still inside the clipping box.
        assert_eq!(name(engine.hit_test(150.0, 5.0)), Some("clip".to_string()));
        // The 40px text line extends past the 10px box, but is clipped
        // there: viewport y 20 (document 120) is the tail, not the text.
        assert_eq!(name(engine.hit_test(5.0, 20.0)), Some("tail".to_string()));
    }

    #[test]
    fn link_at_walks_up_to_the_nearest_anchor() {
        let mut engine = test_engine();
        engine.load_document(
            "<p style=\"margin: 0; font-size: 20px\">\
               <a href=\"https://example.test/x\"><b>link</b></a> plain</p>",
        );
        engine.frame(400, 100);
        let frags = engine.fragments(400.0);
        let word = |w: &str| {
            frags
                .iter()
                .find(|f| matches!(&f.kind, layout::FragmentKind::Text(t) if t == w))
                .expect("the word is laid out")
                .clone()
        };
        let link = word("link");
        let plain = word("plain");
        assert_eq!(
            engine.link_at(link.x + 1.0, link.top() + 1.0).as_deref(),
            Some("https://example.test/x")
        );
        assert_eq!(engine.link_at(plain.x + 1.0, plain.top() + 1.0), None);
        assert_eq!(engine.link_at(1.0, 90.0), None);
    }

    #[test]
    fn viewport_units_recascade_only_when_the_document_uses_them() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }</style><div style=\"height: 50vh; width: 25vw\"></div>",
        );
        // The div's size in the engine's own cached layout at the
        // frame's width (`boxes_of` would lay out afresh at 800px).
        fn div_size(engine: &mut Engine) -> (f32, f32) {
            let styled = engine.styled_dom();
            let div = styled
                .flat
                .element_indices()
                .find(|&i| styled.flat.nodes[i].tag == "div")
                .expect("the div");
            let f = engine
                .layout(200.0)
                .iter()
                .find(|f| f.node == div)
                .expect("the div's fragment");
            (f.w, f.h)
        }
        engine.frame(200, 100);
        assert_eq!(div_size(&mut engine), (50.0, 50.0));
        assert_eq!(engine.style_runs(), 1);
        engine.frame(200, 200);
        assert_eq!(engine.style_runs(), 2, "the height changed and vh was used");
        assert_eq!(engine.layout_runs(), 2, "and the layout with it");
        assert_eq!(div_size(&mut engine), (50.0, 100.0));
        engine.frame(200, 200);
        assert_eq!(
            engine.style_runs(),
            2,
            "the same viewport: nothing recomputed"
        );

        engine.load_document("<style>body { margin: 0 }</style><div style=\"height: 50px\"></div>");
        engine.frame(200, 100);
        engine.frame(200, 300);
        assert_eq!(
            engine.style_runs(),
            3,
            "no viewport unit: a taller viewport recascades nothing"
        );
    }

    #[test]
    fn a_root_percentage_height_follows_the_viewport_and_only_then_relays_out() {
        let mut engine = test_engine();
        engine.load_document("<style>html, body { height: 100%; margin: 0 }</style><p>x</p>");
        engine.frame(200, 100);
        assert_eq!(boxes_of(&engine, "html")[0].h, 100.0);
        assert_eq!(engine.layout_runs(), 1);
        engine.frame(200, 300);
        assert_eq!(
            engine.layout_runs(),
            2,
            "the height changed and the root depends on it"
        );
        assert_eq!(boxes_of(&engine, "body")[0].h, 300.0);
        engine.load_document("<style>body { margin: 0 }</style><p>x</p>");
        engine.frame(200, 100);
        engine.frame(200, 300);
        assert_eq!(
            engine.layout_runs(),
            3,
            "no percentage height: a taller viewport relays out nothing"
        );
    }

    #[test]
    fn rem_ex_and_ch_resolve_through_the_engine_fonts() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>html { font-size: 20px } body { margin: 0 } \
                    p { margin: 0; padding: 1rem 2ch 1ex 0; font-size: 16px }</style><p>x</p>",
        );
        engine.frame(400, 100);
        let styled = engine.styled_dom();
        let p = styled
            .flat
            .element_indices()
            .find(|&i| styled.flat.nodes[i].tag == "p")
            .expect("the p");
        let padding = styled.styles[p].padding;
        let fonts = &engine.layout_fonts;
        use style::computed::LengthPercentage::Px;
        assert_eq!(padding[0], Px(20.0), "1rem of the root's 20px");
        assert_eq!(padding[1], Px(2.0 * fonts.zero_advance(16.0)));
        assert_eq!(padding[2], Px(fonts.x_height(16.0)));
    }

    #[test]
    fn the_scrollbar_appears_only_when_the_document_is_taller_than_the_viewport() {
        let mut engine = test_engine();
        engine.load_document("<p>short</p>");
        engine.frame(200, 100);
        assert_eq!(engine.scrollbar(), None);
        engine.load_document(TALL);
        engine.frame(200, 100);
        let bar = engine.scrollbar().expect("a scrollbar");
        assert_eq!(
            (bar.x, bar.w, bar.h),
            (200.0 - SCROLLBAR_WIDTH, SCROLLBAR_WIDTH, 100.0)
        );
        assert_eq!(bar.thumb_y, 0.0);
        assert!(bar.thumb_h >= SCROLLBAR_MIN_THUMB && bar.thumb_h < 100.0);
        assert!(bar.contains(195.0, 50.0) && !bar.contains(100.0, 50.0));
        assert!(bar.thumb_contains(195.0, 1.0) && !bar.thumb_contains(195.0, 99.0));
        engine.scroll_to_fraction(1.0);
        let bar = engine.scrollbar().expect("a scrollbar");
        assert_eq!(
            bar.thumb_y + bar.thumb_h,
            100.0,
            "the thumb reaches the bottom"
        );
        // The frame draws it: the track column differs from the background.
        let frame = engine.frame(200, 100);
        assert_eq!(px(&frame, 200, 194, 50), [0x31, 0x32, 0x44, 0xff]);
    }
}
