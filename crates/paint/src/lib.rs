//! The paint layer: fragment tree in, display list out.
//!
//! Owns the display-list types — the render backend's input.
//! [`paint_fragments`] (M6.8) walks a real `layout::Fragment` tree:
//! backgrounds and borders painted from actual box geometry, not a
//! guess. Two earlier stopgaps remain, retained rather than deleted
//! (each milestone's own exit criteria asked for the *active* pipeline
//! to move on, not for the previous stage's code to disappear):
//! [`paint_document`] (M1) walks a `dom::Node` directly with hardcoded
//! per-tag presentation, and [`paint_styled_document`] (M5) walks a
//! `style::StyledDom` with real cascaded colors/font-sizes but still no
//! real box model.

use dom::Node;
use layout::{Fragment, FragmentKind};
use style::computed::{BorderStyle, Color, Display};
use style::selector_matching::FlatChild;
use style::StyledDom;

/// An 8-bit RGBA color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba8 {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel.
    pub a: u8,
}

impl Rgba8 {
    /// An opaque color from 8-bit channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Rgba8 { r, g, b, a: 255 }
    }
}

/// One drawable operation in a display list.
#[derive(Debug, Clone)]
pub enum DisplayItem {
    /// Fill an axis-aligned rectangle.
    Rect {
        /// Left edge, px from viewport left.
        x: f32,
        /// Top edge, px from viewport top.
        y: f32,
        /// Width in px.
        w: f32,
        /// Height in px.
        h: f32,
        /// Fill color.
        color: Rgba8,
    },
    /// Draw a line of text. `y` is the BASELINE, not the top.
    Text {
        /// The characters to draw (shaping happens at raster time for
        /// now; shaped runs move here once line breaking lands).
        text: String,
        /// Left edge of the run, px from viewport left.
        x: f32,
        /// Baseline, px from viewport top.
        y: f32,
        /// Font size in px.
        size: f32,
        /// Text color.
        color: Rgba8,
    },
}

/// An ordered list of drawable operations.
#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    /// Items in paint order (later items composite over earlier ones).
    pub items: Vec<DisplayItem>,
}

/// Collects `(parent_tag, text)` pairs from all text nodes, in document
/// order. `style` and `script` subtrees are skipped: their content is not
/// rendered text.
fn collect_text_nodes(node: &Node, out: &mut Vec<(String, String)>) {
    match node {
        Node::Document(children) => {
            for child in children {
                collect_text_nodes(child, out);
            }
        }
        Node::Element(el) => {
            if el.tag == "style" || el.tag == "script" {
                return;
            }
            for child in &el.children {
                match child {
                    Node::Text(t) => out.push((el.tag.clone(), t.data.clone())),
                    other => collect_text_nodes(other, out),
                }
            }
        }
        Node::Text(_) => {}
    }
}

/// Heading text color.
const HEADING: Rgba8 = Rgba8::rgb(0xcd, 0xd6, 0xf4);
/// Body text color.
const BODY: Rgba8 = Rgba8::rgb(0xa6, 0xad, 0xc8);

/// Pragmatic per-tag presentation: (font size, color).
fn style_for(tag: &str) -> (f32, Rgba8) {
    match tag {
        "h1" => (28.0, HEADING),
        "h2" | "h3" | "h4" | "h5" | "h6" => (22.0, HEADING),
        _ => (16.0, BODY),
    }
}

/// Paints a document into a display list for a `viewport_w` x `viewport_h`
/// viewport.
///
/// Milestone-2 presentation: dark background, an accent bar, and text
/// runs stacked top-to-bottom, sized and colored by their parent tag
/// (`h1` heading, other headings smaller, everything else body). Real
/// styling arrives with the `style` + `layout` milestones.
pub fn paint_document(root: &Node, viewport_w: f32, viewport_h: f32) -> DisplayList {
    let mut items = Vec::new();

    // Background.
    items.push(DisplayItem::Rect {
        x: 0.0,
        y: 0.0,
        w: viewport_w,
        h: viewport_h,
        color: Rgba8::rgb(0x1e, 0x1e, 0x2e),
    });
    // Accent bar.
    items.push(DisplayItem::Rect {
        x: 0.0,
        y: 0.0,
        w: 4.0,
        h: viewport_h,
        color: Rgba8::rgb(0x89, 0xb4, 0xfa),
    });

    let mut texts = Vec::new();
    collect_text_nodes(root, &mut texts);
    texts.retain(|(_, data)| !data.trim().is_empty());
    if texts.is_empty() {
        texts.push(("h1".to_string(), "Hello from Random Browser".to_string()));
    }

    // Stack text runs top-to-bottom, sized by their tag.
    let mut baseline = 48.0;
    for (tag, data) in &texts {
        let (size, color) = style_for(tag);
        items.push(DisplayItem::Text {
            text: data.clone(),
            x: 40.0,
            y: baseline,
            size,
            color,
        });
        baseline += size * 1.6;
    }

    DisplayList { items }
}

/// The left edge every text run starts at — matches [`paint_document`]'s
/// fixed indent (there's no real inline layout yet to compute one).
const TEXT_X: f32 = 40.0;
/// Where the first line's baseline sits.
const TOP_INSET: f32 = 32.0;

/// Paints a [`StyledDom`] (M5's cascade output) into a display list for
/// a `viewport_w` x `viewport_h` viewport: the same dark background +
/// accent bar chrome as [`paint_document`], then every text run in
/// document order, sized and colored by *its own* cascaded style rather
/// than a per-tag lookup table — real user CSS visibly changes the
/// screen here (roadmap M5's exit criterion). `display: none` skips an
/// element and everything under it, per spec.
pub fn paint_styled_document(styled: &StyledDom, viewport_w: f32, viewport_h: f32) -> DisplayList {
    let mut items = Vec::new();
    items.push(DisplayItem::Rect {
        x: 0.0,
        y: 0.0,
        w: viewport_w,
        h: viewport_h,
        color: Rgba8::rgb(0x1e, 0x1e, 0x2e),
    });
    items.push(DisplayItem::Rect {
        x: 0.0,
        y: 0.0,
        w: 4.0,
        h: viewport_h,
        color: Rgba8::rgb(0x89, 0xb4, 0xfa),
    });

    let mut cursor_y = TOP_INSET;
    paint_flat_children(styled, 0, &mut items, &mut cursor_y);

    if items.len() <= 2 {
        // Nothing at all rendered (e.g. a document with no visible
        // text): same fallback as `paint_document`, so an empty/failed
        // load is never a silently blank window.
        items.push(DisplayItem::Text {
            text: "Hello from Random Browser".to_string(),
            x: TEXT_X,
            y: TOP_INSET,
            size: 16.0,
            color: Rgba8::rgb(0xa6, 0xad, 0xc8),
        });
    }

    DisplayList { items }
}

/// Walks `idx`'s children in document order, emitting a `Text` item per
/// non-blank text run (colored/sized by `idx`'s own computed style — the
/// nearest element ancestor's is what a text node visually takes) and
/// recursing into element children, applying `margin` top/bottom as
/// simple additive vertical spacing (no collapsing, no box model — a
/// placeholder for the real thing layout builds in M6).
fn paint_flat_children(styled: &StyledDom, idx: usize, items: &mut Vec<DisplayItem>, cursor_y: &mut f32) {
    for child in &styled.flat.nodes[idx].children {
        match child {
            FlatChild::Element(child_idx) => {
                let style = &styled.styles[*child_idx];
                if style.display == Display::None {
                    continue; // the element and its whole subtree: skipped
                }
                // `margin: auto` has no meaning in this simplified,
                // horizontal-layout-free flow (real auto-margin
                // centering needs `layout`'s box-width math, M6) — treat
                // it as 0 here, same as an unset margin.
                *cursor_y += style.margin[style::computed::TOP].resolve(0.0, 0.0);
                paint_flat_children(styled, *child_idx, items, cursor_y);
                *cursor_y += style.margin[style::computed::BOTTOM].resolve(0.0, 0.0);
            }
            FlatChild::Text(text) => {
                if text.trim().is_empty() {
                    continue;
                }
                let style = &styled.styles[idx];
                items.push(DisplayItem::Text {
                    text: text.clone(),
                    x: TEXT_X,
                    y: *cursor_y,
                    size: style.font_size_px,
                    color: to_rgba8(style.color),
                });
                *cursor_y += style.font_size_px * 1.6; // matches paint_document's line spacing
            }
        }
    }
}

/// Converts `style`'s 0.0–1.0-channel color to `paint`'s 8-bit form —
/// `style` owns its own color type (the layering contract: types belong
/// to their producer), so this is the one place that boundary is crossed.
fn to_rgba8(c: Color) -> Rgba8 {
    let scale = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba8 { r: scale(c.r), g: scale(c.g), b: scale(c.b), a: scale(c.a) }
}

/// Paints a real `layout::Fragment` tree (M6.7) into a display list for
/// a `viewport_w` x `viewport_h` viewport — "paint v2" (roadmap M6.8):
/// backgrounds and borders painted from actual fragment geometry (box
/// position/size resolved by real width/margin/collapsing math), not a
/// per-tag guess or a flat vertical list. A transparent background
/// paints nothing (an `Rgba8` with `a: 0` would be a no-op in `render`
/// anyway, but skipping it here avoids pushing dead display items for
/// the overwhelming majority of boxes, which have no background at all).
pub fn paint_fragments(fragments: &[Fragment], viewport_w: f32, viewport_h: f32) -> DisplayList {
    let mut items = Vec::new();
    items.push(DisplayItem::Rect {
        x: 0.0,
        y: 0.0,
        w: viewport_w,
        h: viewport_h,
        color: Rgba8::rgb(0x1e, 0x1e, 0x2e),
    });

    for f in fragments {
        match &f.kind {
            FragmentKind::Box => {
                if f.style.background_color.a > 0.0 {
                    items.push(DisplayItem::Rect {
                        x: f.x,
                        y: f.y,
                        w: f.w,
                        h: f.h,
                        color: to_rgba8(f.style.background_color),
                    });
                }
                if f.style.border_style != BorderStyle::None {
                    push_border(&mut items, f);
                }
            }
            FragmentKind::Text(text) => {
                if !text.trim().is_empty() {
                    items.push(DisplayItem::Text {
                        text: text.clone(),
                        x: f.x,
                        y: f.y, // a text fragment's `y` is already its baseline
                        size: f.style.font_size_px,
                        color: to_rgba8(f.style.color),
                    });
                }
            }
        }
    }

    if items.len() <= 1 {
        items.push(DisplayItem::Text {
            text: "Hello from Random Browser".to_string(),
            x: 40.0,
            y: 32.0,
            size: 16.0,
            color: Rgba8::rgb(0xa6, 0xad, 0xc8),
        });
    }

    DisplayList { items }
}

/// Paints a box fragment's border as four filled strips (there's no
/// stroke/outline display-item primitive — four rects is simplest and
/// exactly matches a solid border's actual shape, mitered corners and
/// all, as long as adjacent strips' corners overlap, which they do here).
fn push_border(items: &mut Vec<DisplayItem>, f: &Fragment) {
    let [top, right, bottom, left] = f.style.border_width;
    let color = to_rgba8(f.style.border_color);
    if top > 0.0 {
        items.push(DisplayItem::Rect { x: f.x, y: f.y, w: f.w, h: top, color });
    }
    if bottom > 0.0 {
        items.push(DisplayItem::Rect { x: f.x, y: f.y + f.h - bottom, w: f.w, h: bottom, color });
    }
    if left > 0.0 {
        items.push(DisplayItem::Rect { x: f.x, y: f.y, w: left, h: f.h, color });
    }
    if right > 0.0 {
        items.push(DisplayItem::Rect { x: f.x + f.w - right, y: f.y, w: right, h: f.h, color });
    }
}

#[cfg(test)]
mod fragment_tests {
    use super::*;
    use dom::Element;

    const VIEWPORT: f32 = 800.0;

    /// The full `style` -> `box_tree` -> `block` -> `paint` pipeline, for
    /// one element and some CSS — exactly as `engine` drives it. `None`
    /// on machines with no usable system font (M7's text measurement
    /// needs one) — callers skip the test entirely in that case.
    fn display_list_of(root: Element, css_src: &str) -> Option<DisplayList> {
        let fonts = text::FontCollection::new().ok()?;
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(css_src).expect("test css is valid");
        let styled = style::style_document(&doc, &[sheet]);
        let boxes = layout::build_box_tree(&styled);
        let fragments = layout::layout(&boxes, VIEWPORT, &fonts);
        Some(paint_fragments(&fragments, VIEWPORT, 600.0))
    }

    fn rects(list: &DisplayList) -> Vec<&DisplayItem> {
        list.items.iter().filter(|i| matches!(i, DisplayItem::Rect { .. })).collect()
    }

    #[test]
    fn background_color_paints_a_rect_at_the_fragment_geometry() {
        let Some(list) = display_list_of(
            Element::new("div"),
            "div { display: block; width: 100px; height: 50px; background-color: #ff0000; }",
        ) else {
            return;
        };
        let backgrounds: Vec<&DisplayItem> = rects(&list)
            .into_iter()
            .filter(|i| matches!(i, DisplayItem::Rect { color, .. } if *color == Rgba8::rgb(255, 0, 0)))
            .collect();
        assert_eq!(backgrounds.len(), 1);
        match backgrounds[0] {
            DisplayItem::Rect { x, y, w, h, .. } => assert_eq!((*x, *y, *w, *h), (0.0, 0.0, 100.0, 50.0)),
            _ => unreachable!(),
        }
    }

    #[test]
    fn transparent_background_paints_nothing() {
        let Some(list) = display_list_of(
            Element::new("div"),
            "div { display: block; width: 100px; height: 50px; }", // background-color: initial = transparent
        ) else {
            return;
        };
        // Just the viewport background rect — no second rect for the div.
        assert_eq!(rects(&list).len(), 1);
    }

    #[test]
    fn border_paints_four_strips_at_the_box_edges() {
        let Some(list) = display_list_of(
            Element::new("div"),
            "div { display: block; width: 100px; height: 50px; margin: 0;
                   border-width: 4px; border-style: solid; border-color: #00ff00; }",
        ) else {
            return;
        };
        let green: Vec<&DisplayItem> = rects(&list)
            .into_iter()
            .filter(|i| matches!(i, DisplayItem::Rect { color, .. } if *color == Rgba8::rgb(0, 255, 0)))
            .collect();
        assert_eq!(green.len(), 4); // top, bottom, left, right
        // Border-box is 100+8=108 wide, 50+8=58 tall (border adds to content).
        let has = |want: (f32, f32, f32, f32)| {
            green.iter().any(|i| matches!(i, DisplayItem::Rect { x, y, w, h, .. } if (*x, *y, *w, *h) == want))
        };
        assert!(has((0.0, 0.0, 108.0, 4.0)), "top strip");
        assert!(has((0.0, 54.0, 108.0, 4.0)), "bottom strip");
        assert!(has((0.0, 0.0, 4.0, 58.0)), "left strip");
        assert!(has((104.0, 0.0, 4.0, 58.0)), "right strip");
    }

    #[test]
    fn border_style_none_paints_no_border_even_with_width_set() {
        let Some(list) = display_list_of(
            Element::new("div"),
            "div { display: block; width: 100px; height: 50px; border-width: 4px; }", // no border-style
        ) else {
            return;
        };
        assert_eq!(rects(&list).len(), 1); // just the viewport background
    }

    #[test]
    fn text_fragment_paints_at_its_baseline() {
        let Ok(fonts) = text::FontCollection::new() else {
            return;
        };
        let Some(list) = display_list_of(
            Element::new("p").text("hi"),
            "p { display: block; margin: 0; color: #0000ff; font-size: 12px; }",
        ) else {
            return;
        };
        let texts: Vec<&DisplayItem> =
            list.items.iter().filter(|i| matches!(i, DisplayItem::Text { .. })).collect();
        assert_eq!(texts.len(), 1);
        match texts[0] {
            DisplayItem::Text { text, x, y, size, color } => {
                assert_eq!(text, "hi");
                // The baseline is the font's own ascent at this size
                // (M7.1/M7.4) — not simply the font-size.
                assert_eq!((*x, *y, *size), (0.0, fonts.primary().metrics(12.0).ascent, 12.0));
                assert_eq!(*color, Rgba8::rgb(0, 0, 255));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn empty_document_falls_back_to_hint_text() {
        let Ok(fonts) = text::FontCollection::new() else {
            return;
        };
        let doc = dom::Node::document(vec![]);
        let styled = style::style_document(&doc, &[]);
        let boxes = layout::build_box_tree(&styled);
        let fragments = layout::layout(&boxes, VIEWPORT, &fonts);
        let list = paint_fragments(&fragments, VIEWPORT, 600.0);
        assert!(list.items.iter().any(|i| matches!(i, DisplayItem::Text { .. })));
    }
}

#[cfg(test)]
mod styled_tests {
    use super::*;
    use dom::Element;

    /// Builds a `StyledDom` from an already-built element and CSS source.
    fn styled_from(root: Element, css_src: &str) -> StyledDom {
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(css_src).expect("test css is valid");
        style::style_document(&doc, &[sheet])
    }

    fn text_items(list: &DisplayList) -> Vec<&DisplayItem> {
        list.items.iter().filter(|i| matches!(i, DisplayItem::Text { .. })).collect()
    }

    #[test]
    fn cascaded_color_reaches_the_display_list() {
        let root = Element::new("p").text("hello");
        let styled = styled_from(root, "p { color: #ff0000; }");
        let list = paint_styled_document(&styled, 800.0, 600.0);
        let texts = text_items(&list);
        assert_eq!(texts.len(), 1);
        match texts[0] {
            DisplayItem::Text { color, text, .. } => {
                assert_eq!(*color, Rgba8::rgb(255, 0, 0));
                assert_eq!(text, "hello");
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn cascaded_font_size_reaches_the_display_list() {
        let root = Element::new("p").text("hi");
        let styled = styled_from(root, "p { font-size: 40px; }");
        let list = paint_styled_document(&styled, 800.0, 600.0);
        match text_items(&list)[0] {
            DisplayItem::Text { size, .. } => assert_eq!(*size, 40.0),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn display_none_hides_the_element_and_its_subtree() {
        let root = Element::new("div")
            .push(Element::new("p").text("hidden"))
            .push(Element::new("span").text("visible"));
        let styled = styled_from(root, "p { display: none; }");
        let list = paint_styled_document(&styled, 800.0, 600.0);
        let texts: Vec<&str> = text_items(&list)
            .into_iter()
            .filter_map(|i| match i {
                DisplayItem::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["visible"]);
    }

    #[test]
    fn margin_advances_vertical_position_between_siblings() {
        let root = Element::new("div")
            .push(Element::new("p").text("first"))
            .push(Element::new("p").text("second"));
        let styled = styled_from(root, "p { margin: 50px 0; font-size: 10px; }");
        let list = paint_styled_document(&styled, 800.0, 600.0);
        let texts = text_items(&list);
        let ys: Vec<f32> = texts
            .iter()
            .map(|i| match i {
                DisplayItem::Text { y, .. } => *y,
                _ => unreachable!(),
            })
            .collect();
        // second's margin-top (50) plus first's own line advance (10*1.6)
        // separate them by well over the plain per-line spacing alone.
        assert!(ys[1] - ys[0] > 50.0);
    }

    #[test]
    fn empty_document_falls_back_to_hint_text() {
        let doc = dom::Node::document(vec![]);
        let styled = style::style_document(&doc, &[]);
        let list = paint_styled_document(&styled, 100.0, 100.0);
        assert!(text_items(&list).iter().any(|i| matches!(i, DisplayItem::Text { .. })));
    }

    #[test]
    fn background_and_accent_chrome_still_present() {
        let root = Element::new("p").text("x");
        let styled = styled_from(root, "");
        let list = paint_styled_document(&styled, 800.0, 600.0);
        assert!(matches!(list.items[0], DisplayItem::Rect { w: 800.0, h: 600.0, .. }));
        assert!(matches!(list.items[1], DisplayItem::Rect { w: 4.0, .. }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dom::Element;

    #[test]
    fn paints_background_accent_and_texts() {
        let body = Element::new("body")
            .push(Element::new("h1").text("Heading"))
            .push(Element::new("p").text("Body line"));
        let doc = Node::document(vec![Element::new("html").push(body).into()]);

        let list = paint_document(&doc, 800.0, 600.0);
        assert!(list.items.len() >= 4, "expected bg + accent + 2 texts");
        match &list.items[0] {
            DisplayItem::Rect { w, h, color, .. } => {
                assert_eq!((*w, *h), (800.0, 600.0));
                assert_eq!(*color, Rgba8::rgb(0x1e, 0x1e, 0x2e));
            }
            other => panic!("first item must be the background rect, got {other:?}"),
        }
        let text_items: Vec<&DisplayItem> = list
            .items
            .iter()
            .filter(|i| matches!(i, DisplayItem::Text { .. }))
            .collect();
        assert_eq!(text_items.len(), 2);
    }

    #[test]
    fn empty_document_gets_fallback_text() {
        let list = paint_document(&Node::document(vec![]), 100.0, 100.0);
        assert!(
            list.items.iter().any(|i| matches!(i, DisplayItem::Text { .. })),
            "expected a fallback text item"
        );
    }

    #[test]
    fn text_sized_by_tag_stacks_top_to_bottom() {
        let body = Element::new("body")
            .push(Element::new("h1").text("Heading"))
            .push(Element::new("p").text("Body line"));
        let doc = Node::document(vec![Element::new("html").push(body).into()]);
        let list = paint_document(&doc, 800.0, 600.0);
        let texts: Vec<&DisplayItem> = list
            .items
            .iter()
            .filter(|i| matches!(i, DisplayItem::Text { .. }))
            .collect();
        match (texts[0], texts[1]) {
            (
                DisplayItem::Text { size: s1, y: y1, .. },
                DisplayItem::Text { size: s2, y: y2, .. },
            ) => {
                assert_eq!((*s1, *s2), (28.0, 16.0));
                assert!(*y2 > *y1, "body must render below the heading");
            }
            _ => panic!("expected two text items"),
        }
    }

    #[test]
    fn whitespace_only_texts_are_skipped() {
        let doc = Node::document(vec![Element::new("p").text("   \n  ").into()]);
        let list = paint_document(&doc, 100.0, 100.0);
        // No visible text anywhere: the fallback heading fires.
        match list.items.get(2) {
            Some(DisplayItem::Text { text, .. }) => {
                assert_eq!(text, "Hello from Random Browser");
            }
            other => panic!("expected fallback text, got {other:?}"),
        }
    }

    #[test]
    fn style_and_script_text_is_not_rendered() {
        let doc = Node::document(vec![
            Element::new("style").text("h1 { color: red; }").into(),
            Element::new("script").text("alert(1)").into(),
            Element::new("p").text("Visible").into(),
        ]);
        let list = paint_document(&doc, 100.0, 100.0);
        let texts: Vec<&str> = list
            .items
            .iter()
            .filter_map(|i| match i {
                DisplayItem::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["Visible"]);
    }
}
