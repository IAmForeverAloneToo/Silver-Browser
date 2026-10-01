//! The box tree: a `style::StyledDom` in, a tree of [`LayoutBox`]es out
//! (roadmap M6.1) — one box per element that generates one (`display:
//! none` generates none, and skips its whole subtree, per spec), plus an
//! anonymous box per non-blank text run.

use style::computed::Display;
use style::selector_matching::FlatChild;
use style::{ComputedStyle, StyledDom};

/// One node in the box tree.
#[derive(Debug, Clone)]
pub struct LayoutBox {
    /// The generating element's computed style — owned, not `&'a
    /// ComputedStyle` as the roadmap's own sketch first put it: a
    /// borrowed field would thread a lifetime parameter through every
    /// consumer (`layout`, `paint`, tests) for what's currently a small,
    /// cheap-to-clone struct. Revisit with arenas if profiling ever
    /// shows it matters (that's what M35's "shared data arenas" line is
    /// for).
    pub style: ComputedStyle,
    /// What kind of box this is, and its own children (a [`BoxKind::Text`]
    /// never has children).
    pub kind: BoxKind,
    pub children: Vec<LayoutBox>,
}

/// A box's kind, per `display` (M6 v1: block/inline only — flex, grid,
/// inline-block, and so on generate real boxes only from M17 on).
#[derive(Debug, Clone, PartialEq)]
pub enum BoxKind {
    Block,
    Inline,
    /// An anonymous box wrapping one text run. Its `style` (on the
    /// containing [`LayoutBox`]) is a copy of the *nearest element
    /// ancestor's* — exactly how CSS defines an anonymous box's style:
    /// text has no element of its own to be styled, so it visually
    /// takes whatever's around it.
    Text(String),
}

/// Builds the box tree for all of `styled`'s top-level content (the
/// document's direct children — there's no single root box for them,
/// they're laid out as if they were children of the viewport itself,
/// see `block::layout`).
pub fn build_box_tree(styled: &StyledDom) -> Vec<LayoutBox> {
    build_children(styled, 0)
}

/// Builds boxes for `idx`'s children, in document order.
fn build_children(styled: &StyledDom, idx: usize) -> Vec<LayoutBox> {
    let containing_style = &styled.styles[idx];
    let mut out = Vec::new();
    for child in &styled.flat.nodes[idx].children {
        match child {
            FlatChild::Element(child_idx) => {
                let style = &styled.styles[*child_idx];
                let kind = match style.display {
                    Display::Block => BoxKind::Block,
                    Display::Inline => BoxKind::Inline,
                    Display::None => continue, // the element AND its subtree: no box at all
                };
                out.push(LayoutBox {
                    style: style.clone(),
                    kind,
                    children: build_children(styled, *child_idx),
                });
            }
            FlatChild::Text(text) => {
                // Runs of nothing but collapsible white space (css-text-3
                // §4.1: space, tab, newline, carriage return, form feed —
                // a no-break space is content) contribute no box.
                if text.chars().all(crate::inline::is_css_space) {
                    continue;
                }
                out.push(LayoutBox {
                    style: containing_style.clone(),
                    kind: BoxKind::Text(text.clone()),
                    children: Vec::new(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dom::Element;

    fn styled(root: Element, css_src: &str) -> StyledDom {
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(css_src).expect("test css is valid");
        style::style_document(&doc, &[sheet])
    }

    #[test]
    fn block_and_inline_elements_get_matching_box_kinds() {
        let root = Element::new("div").push(Element::new("span").text("x"));
        let tree = build_box_tree(&styled(
            root,
            "div { display: block; } span { display: inline; }",
        ));
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].kind, BoxKind::Block);
        assert_eq!(tree[0].children[0].kind, BoxKind::Inline);
    }

    #[test]
    fn display_none_produces_no_box_at_all() {
        let root = Element::new("div")
            .push(Element::new("p").text("hidden").attr("class", "gone"))
            .push(Element::new("span").text("shown"));
        let tree = build_box_tree(&styled(root, "p { display: none; }"));
        assert_eq!(tree[0].children.len(), 1); // the p is gone entirely, not just empty
        assert_eq!(tree[0].children[0].kind, BoxKind::Inline);
    }

    #[test]
    fn text_runs_become_anonymous_boxes_carrying_the_parent_style() {
        let root = Element::new("p").text("hello");
        let tree = build_box_tree(&styled(root, "p { color: #ff0000; }"));
        match &tree[0].children[0].kind {
            BoxKind::Text(t) => assert_eq!(t, "hello"),
            other => panic!("expected a text box, got {other:?}"),
        }
        assert_eq!(
            tree[0].children[0].style.color,
            style::computed::Color::rgb8(255, 0, 0)
        );
    }

    #[test]
    fn whitespace_only_text_produces_no_box() {
        let root = Element::new("div").text("   \n  ");
        let tree = build_box_tree(&styled(root, ""));
        assert!(tree[0].children.is_empty());
        // A no-break space is content, not white space.
        let root = Element::new("div").text("\u{a0}");
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(tree[0].children.len(), 1);
    }
}
