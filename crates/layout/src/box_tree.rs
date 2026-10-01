//! The box tree: a `style::StyledDom` in, a tree of [`LayoutBox`]es out
//! (roadmap M6.1) — one box per element that generates one (`display:
//! none` generates none, and skips its whole subtree, per spec), plus an
//! anonymous box per non-blank text run, plus (M10.4) the anonymous
//! block boxes CSS2 §9.2.1.1 requires so that every block container's
//! children are block-level: a run of inline-level children is wrapped
//! in one, and a block-level box found inside an inline element is
//! hoisted up to the containing block with its inline ancestors split
//! around it. A float or an absolutely positioned box (M11) keeps its
//! place in the tree, where its element is: inline-level content when
//! it occurs in a run or an inline element, which it does not split, a
//! block-level child when it occurs between blocks.

use style::computed::{Display, Float};
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
    /// The element this box belongs to, as its `FlatDom` index (M9.4):
    /// the generating element for an element's box, the containing
    /// element for an anonymous text or block box. Hit testing answers
    /// with this, and the fragments the box produces carry it.
    pub node: usize,
}

/// A box's kind, per `display` (block, inline and inline-block — flex,
/// grid, tables and so on generate real boxes only from their own
/// milestones on).
///
/// The builder's contract (CSS2 §9.2.1.1), which `block` relies on: a
/// block container's ([`BoxKind::Block`]'s, [`BoxKind::InlineBlock`]'s,
/// [`BoxKind::Float`]'s or [`BoxKind::Absolute`]'s) children are all
/// block-level ([`BoxKind::Block`] or [`BoxKind::AnonymousBlock`]) or
/// out of flow ([`BoxKind::Float`], [`BoxKind::Absolute`]); an
/// [`BoxKind::AnonymousBlock`]'s and an [`BoxKind::Inline`]'s children
/// are all inline-level ([`BoxKind::Inline`], [`BoxKind::InlineBlock`]
/// or [`BoxKind::Text`]) or out of flow.
#[derive(Debug, Clone, PartialEq)]
pub enum BoxKind {
    Block,
    Inline,
    /// An atomic inline-level box that is a block container inside
    /// (CSS2 §9.2.4, M10.4): an item on its parent's line, as wide as
    /// its margin box, its content laid out like a block's.
    InlineBlock,
    /// A floated box (CSS2 §9.5, M11): out of flow, a block container
    /// inside and a block formatting context root, sized like an
    /// inline-block and placed by the float context of the formatting
    /// context it is in. Its `style.float` says which side. It sits
    /// where its element is: in a run or an inline element when met
    /// there, placed when its line is laid out; among a block
    /// container's children when met between blocks, placed at the
    /// flow position it is met at.
    Float,
    /// An absolutely positioned box (CSS2 §9.6, M11: `position:
    /// absolute` or `fixed`, which `style.position` says): out of
    /// flow, a block container inside and a block formatting context
    /// root, laid out against its containing block — the nearest
    /// positioned ancestor's padding box, or the initial containing
    /// block; the viewport for `fixed` — once that is known, from the
    /// static position it is met at.
    Absolute,
    /// An anonymous block box (CSS2 §9.2.1.1, M10.4) wrapping one run of
    /// inline-level boxes inside a block container, so that the
    /// container's children are all block-level. Its `style` inherits
    /// from the containing block and takes the initial value for every
    /// other property (so no margin, padding, border, background or
    /// size of its own); it produces no fragment of its own — its
    /// content's fragments go into its parent's.
    AnonymousBlock,
    /// An anonymous box wrapping one text run. Its `style` (on the
    /// containing [`LayoutBox`]) is a copy of the *nearest element
    /// ancestor's* — exactly how CSS defines an anonymous box's style:
    /// text has no element of its own to be styled, so it visually
    /// takes whatever's around it. A run of nothing but collapsible
    /// white space is kept where it separates inline-level content
    /// (M10.4: it is the gap between two words, or two inline-blocks)
    /// and dropped at the start and end of a block container's inline
    /// run, where a line would discard it anyway (css-text-3 §4.1.3).
    Text(String),
}

impl BoxKind {
    /// Whether a box of this kind is block-level: a child of a block
    /// container, stacked vertically by `block::layout_children`. A
    /// float is neither block-level nor inline-level: it is out of flow.
    pub fn is_block_level(&self) -> bool {
        matches!(self, BoxKind::Block | BoxKind::AnonymousBlock)
    }
}

/// Builds the box tree for all of `styled`'s top-level content (the
/// document's direct children — there's no single root box for them,
/// they're laid out as if they were children of the viewport itself,
/// see `block::layout`). The document node is the block container of
/// the top-level content: an inline run at the top goes into an
/// anonymous block inheriting from the document's (initial-valued)
/// style.
pub fn build_box_tree(styled: &StyledDom) -> Vec<LayoutBox> {
    build_block_container(styled, 0)
}

/// The block-level children of the block container `idx` generates
/// (CSS2 §9.2.1.1): its element children in document order, each
/// inline-level run wrapped in an anonymous block, each block found
/// inside an inline child hoisted out to this level.
fn build_block_container(styled: &StyledDom, idx: usize) -> Vec<LayoutBox> {
    let mut container = BlockContainer {
        styled,
        node: idx,
        out: Vec::new(),
        run: Vec::new(),
        open: Vec::new(),
    };
    container.add_children_of(idx);
    container.finish()
}

/// The children of one block container while they are built: the
/// block-level boxes so far, the inline-level run being collected for
/// the next anonymous block, and the chain of inline elements currently
/// open around the position being built — which is what a block found
/// inside them is hoisted out of, and what is reopened after it.
struct BlockContainer<'a> {
    styled: &'a StyledDom,
    /// The container's element: what its anonymous boxes name.
    node: usize,
    out: Vec<LayoutBox>,
    run: Vec<LayoutBox>,
    open: Vec<LayoutBox>,
}

impl BlockContainer<'_> {
    /// Adds the boxes of `idx`'s children, in document order, to
    /// whatever is innermost: the open inline chain, else the run.
    fn add_children_of(&mut self, idx: usize) {
        let styled = self.styled;
        for child in &styled.flat.nodes[idx].children {
            match child {
                FlatChild::Element(child_idx) => {
                    let style = &styled.styles[*child_idx];
                    match style.display {
                        Display::None => {} // the element AND its subtree: no box at all
                        // A float or an absolutely positioned box computes
                        // to `display: block` (CSS2 §9.7, the cascade's
                        // rule) and is a block container inside; outside
                        // it is out of flow, and goes where it is met:
                        // into the run or inline open around it, else
                        // among the container's blocks.
                        Display::Block
                            if style.float != Float::None || style.position.is_out_of_flow() =>
                        {
                            let kind = if style.position.is_out_of_flow() {
                                BoxKind::Absolute
                            } else {
                                BoxKind::Float
                            };
                            let out_of_flow = LayoutBox {
                                style: style.clone(),
                                kind,
                                children: build_block_container(styled, *child_idx),
                                node: *child_idx,
                            };
                            if self.open.is_empty() && self.run.is_empty() {
                                self.out.push(out_of_flow);
                            } else {
                                self.add_inline_level(out_of_flow);
                            }
                        }
                        Display::Block => {
                            let block = LayoutBox {
                                style: style.clone(),
                                kind: BoxKind::Block,
                                children: build_block_container(styled, *child_idx),
                                node: *child_idx,
                            };
                            self.add_block(block);
                        }
                        Display::Inline => {
                            self.open.push(LayoutBox {
                                style: style.clone(),
                                kind: BoxKind::Inline,
                                children: Vec::new(),
                                node: *child_idx,
                            });
                            self.add_children_of(*child_idx);
                            let inline = self.open.pop().expect("the inline just opened");
                            self.add_inline_level(inline);
                        }
                        Display::InlineBlock => {
                            let block = LayoutBox {
                                style: style.clone(),
                                kind: BoxKind::InlineBlock,
                                children: build_block_container(styled, *child_idx),
                                node: *child_idx,
                            };
                            self.add_inline_level(block);
                        }
                    }
                }
                FlatChild::Text(text) => {
                    // White space at the start of the run (css-text-3
                    // §4.1: space, tab, newline, carriage return, form
                    // feed — a no-break space is content) is dropped:
                    // a line discards it. Elsewhere it is kept, as the
                    // gap it is; `flush_run` drops it at the end.
                    if self.open.is_empty()
                        && self.run.is_empty()
                        && text.chars().all(crate::inline::is_css_space)
                    {
                        continue;
                    }
                    self.add_inline_level(LayoutBox {
                        style: styled.styles[idx].clone(),
                        kind: BoxKind::Text(text.clone()),
                        children: Vec::new(),
                        node: idx,
                    });
                }
            }
        }
    }

    /// An inline-level box goes to the innermost open inline, or to the
    /// run when none is open.
    fn add_inline_level(&mut self, b: LayoutBox) {
        match self.open.last_mut() {
            Some(inline) => inline.children.push(b),
            None => self.run.push(b),
        }
    }

    /// A block-level box becomes a child of the container itself,
    /// wherever it was found (CSS2 §9.2.1.1): the inline chain open
    /// around it is closed, the run so far wrapped into an anonymous
    /// block before it, and the chain reopened after it — the halves
    /// keep the inline elements' style and node, and either may end up
    /// empty.
    fn add_block(&mut self, block: LayoutBox) {
        let chain: Vec<(ComputedStyle, usize)> = self
            .open
            .iter()
            .map(|b| (b.style.clone(), b.node))
            .collect();
        while let Some(inline) = self.open.pop() {
            self.add_inline_level(inline);
        }
        self.flush_run();
        self.out.push(block);
        for (style, node) in chain {
            self.open.push(LayoutBox {
                style,
                kind: BoxKind::Inline,
                children: Vec::new(),
                node,
            });
        }
    }

    /// Wraps the run so far, if any, in an anonymous block. White space
    /// at its end is dropped first: a line discards it, and a run of
    /// nothing else is no content at all.
    fn flush_run(&mut self) {
        while self.run.last().is_some_and(
            |b| matches!(&b.kind, BoxKind::Text(t) if t.chars().all(crate::inline::is_css_space)),
        ) {
            self.run.pop();
        }
        if self.run.is_empty() {
            return;
        }
        let children = std::mem::take(&mut self.run);
        self.out.push(LayoutBox {
            style: ComputedStyle::inherited_from(&self.styled.styles[self.node]),
            kind: BoxKind::AnonymousBlock,
            children,
            node: self.node,
        });
    }

    fn finish(mut self) -> Vec<LayoutBox> {
        debug_assert!(
            self.open.is_empty(),
            "every inline is closed by its own element"
        );
        self.flush_run();
        self.out
    }
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

    /// `b`'s subtree as `(kind, node)` pairs in preorder, a text box's
    /// kind spelled as its text, for assertions on structure.
    fn shape(b: &LayoutBox, out: &mut Vec<(String, usize)>) {
        let kind = match &b.kind {
            BoxKind::Block => "block".to_string(),
            BoxKind::Inline => "inline".to_string(),
            BoxKind::InlineBlock => "inline-block".to_string(),
            BoxKind::Float => "float".to_string(),
            BoxKind::Absolute => "absolute".to_string(),
            BoxKind::AnonymousBlock => "anon".to_string(),
            BoxKind::Text(t) => format!("text {t}"),
        };
        out.push((kind, b.node));
        for c in &b.children {
            shape(c, out);
        }
    }

    fn shape_of(tree: &[LayoutBox]) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        for b in tree {
            shape(b, &mut out);
        }
        out
    }

    fn pairs(items: &[(&str, usize)]) -> Vec<(String, usize)> {
        items.iter().map(|(k, n)| (k.to_string(), *n)).collect()
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
        // The span is inline-level content of the div, so it sits in an
        // anonymous block (M10.4) that names the div, its container.
        let anon = &tree[0].children[0];
        assert_eq!(anon.kind, BoxKind::AnonymousBlock);
        assert_eq!(anon.children[0].kind, BoxKind::Inline);
        // Element boxes name their element; the text box inside the
        // span names the span, its containing element.
        assert_eq!(tree[0].node, 1);
        assert_eq!(anon.node, 1);
        assert_eq!(anon.children[0].node, 2);
        assert_eq!(anon.children[0].children[0].node, 2);
    }

    #[test]
    fn display_none_produces_no_box_at_all() {
        let root = Element::new("div")
            .push(Element::new("p").text("hidden").attr("class", "gone"))
            .push(Element::new("span").text("shown"));
        let tree = build_box_tree(&styled(root, "p { display: none; }"));
        assert_eq!(tree[0].children.len(), 1); // the p is gone entirely, not just empty
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[("anon", 1), ("inline", 3), ("text shown", 3)])
        );
    }

    #[test]
    fn text_runs_become_anonymous_boxes_carrying_the_parent_style() {
        let root = Element::new("p").text("hello");
        let tree = build_box_tree(&styled(root, "p { color: #ff0000; }"));
        let text = &tree[0].children[0].children[0];
        match &text.kind {
            BoxKind::Text(t) => assert_eq!(t, "hello"),
            other => panic!("expected a text box, got {other:?}"),
        }
        assert_eq!(text.style.color, style::computed::Color::rgb8(255, 0, 0));
    }

    #[test]
    fn whitespace_only_text_produces_no_box() {
        let root = Element::new("div").text("   \n  ");
        let tree = build_box_tree(&styled(root, ""));
        assert!(tree[0].children.is_empty(), "not even an anonymous block");
        // A no-break space is content, not white space.
        let root = Element::new("div").text("\u{a0}");
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[("anon", 1), ("text \u{a0}", 1)])
        );
    }

    #[test]
    fn inline_runs_between_blocks_get_one_anonymous_block_each() {
        // <div>a<p>b</p>c<span>d</span><p>e</p></div>: CSS2 §9.2.1.1's
        // own example. Nodes: div 1, p 2, span 3, p 4.
        let root = Element::new("div")
            .text("a")
            .push(Element::new("p").text("b"))
            .text("c")
            .push(Element::new("span").text("d"))
            .push(Element::new("p").text("e"));
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("text a", 1),
                ("block", 2),
                ("anon", 2),
                ("text b", 2),
                ("anon", 1),
                ("text c", 1),
                ("inline", 3),
                ("text d", 3),
                ("block", 4),
                ("anon", 4),
                ("text e", 4),
            ])
        );
        assert!(tree[0].children.iter().all(|c| c.kind.is_block_level()));
    }

    #[test]
    fn an_anonymous_block_inherits_from_its_container_and_nothing_else() {
        let root = Element::new("div").text("x");
        let tree = build_box_tree(&styled(
            root,
            "div { color: #ff0000; text-align: center; padding: 10px; margin: 5px;
                   border: 2px solid #000000; background-color: #00ff00; width: 50px; }",
        ));
        let anon = &tree[0].children[0];
        assert_eq!(anon.kind, BoxKind::AnonymousBlock);
        let s = &anon.style;
        assert_eq!(s.color, style::computed::Color::rgb8(255, 0, 0));
        assert_eq!(s.text_align, style::computed::TextAlign::Center);
        let initial = ComputedStyle::default();
        assert_eq!(s.padding, initial.padding);
        assert_eq!(s.margin, initial.margin);
        assert_eq!(s.used_border_width(), [0.0; 4]);
        assert_eq!(s.background_color, initial.background_color);
        assert_eq!(s.width, initial.width);
    }

    #[test]
    fn a_block_inside_an_inline_is_hoisted_and_the_inline_split() {
        // <div><span>a<p>b</p>c</span></div>: the p becomes a child of
        // the div between two anonymous blocks, each holding one half
        // of the span (CSS2 §9.2.1.1). Nodes: div 1, span 2, p 3.
        let root = Element::new("div").push(
            Element::new("span")
                .text("a")
                .push(Element::new("p").text("b"))
                .text("c"),
        );
        let tree = build_box_tree(&styled(root, "span { color: #0000ff; }"));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("inline", 2),
                ("text a", 2),
                ("block", 3),
                ("anon", 3),
                ("text b", 3),
                ("anon", 1),
                ("inline", 2),
                ("text c", 2),
            ])
        );
        // Both halves keep the span's style.
        let blue = style::computed::Color::rgb8(0, 0, 255);
        assert_eq!(tree[0].children[0].children[0].style.color, blue);
        assert_eq!(tree[0].children[2].children[0].style.color, blue);
    }

    #[test]
    fn a_block_inside_nested_inlines_splits_the_whole_chain() {
        // <div><span><em>a<p>b</p>c</em>d</span></div>. Nodes: div 1,
        // span 2, em 3, p 4. The em half after the p sits inside the
        // span half after the p, as it did before; "d" follows it in
        // the span.
        let root = Element::new("div").push(
            Element::new("span")
                .push(
                    Element::new("em")
                        .text("a")
                        .push(Element::new("p").text("b"))
                        .text("c"),
                )
                .text("d"),
        );
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("inline", 2),
                ("inline", 3),
                ("text a", 3),
                ("block", 4),
                ("anon", 4),
                ("text b", 4),
                ("anon", 1),
                ("inline", 2),
                ("inline", 3),
                ("text c", 3),
                ("text d", 2),
            ])
        );
    }

    #[test]
    fn a_block_at_the_start_or_end_of_an_inline_leaves_an_empty_half() {
        // <div><span><p>b</p></span></div>: CSS2 §9.2.1.1 splits "even
        // if either side is empty". The empty halves are kept — layout
        // treats a line box with nothing in it as not existing (CSS2
        // §9.4.2), so they cost nothing and the structure stays
        // uniform. Nodes: div 1, span 2, p 3.
        let root = Element::new("div").push(Element::new("span").push(Element::new("p").text("b")));
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("inline", 2),
                ("block", 3),
                ("anon", 3),
                ("text b", 3),
                ("anon", 1),
                ("inline", 2),
            ])
        );
    }

    #[test]
    fn two_blocks_inside_one_inline_split_it_three_ways() {
        // <div><span>a<p>b</p>c<p>d</p>e</span></div>. Nodes: div 1,
        // span 2, p 3, p 4.
        let root = Element::new("div").push(
            Element::new("span")
                .text("a")
                .push(Element::new("p").text("b"))
                .text("c")
                .push(Element::new("p").text("d"))
                .text("e"),
        );
        let tree = build_box_tree(&styled(root, ""));
        let kinds: Vec<(String, usize)> = shape_of(&tree[0].children)
            .into_iter()
            .filter(|(k, _)| !k.starts_with("text"))
            .collect();
        assert_eq!(
            kinds,
            pairs(&[
                ("anon", 1),
                ("inline", 2),
                ("block", 3),
                ("anon", 3),
                ("anon", 1),
                ("inline", 2),
                ("block", 4),
                ("anon", 4),
                ("anon", 1),
                ("inline", 2),
            ])
        );
    }

    #[test]
    fn a_block_inside_an_inline_is_itself_a_block_container() {
        // The hoisted block's own children get the same treatment: a
        // span inside it goes into an anonymous block of the p's.
        // Nodes: div 1, span 2, p 3, em 4, p 5.
        let root = Element::new("div").push(
            Element::new("span").push(
                Element::new("p")
                    .push(Element::new("em").text("x"))
                    .push(Element::new("p").text("y")),
            ),
        );
        let tree = build_box_tree(&styled(root, ""));
        let p = &tree[0].children[1];
        assert_eq!((p.kind.clone(), p.node), (BoxKind::Block, 3));
        assert_eq!(
            shape_of(&p.children),
            pairs(&[
                ("anon", 3),
                ("inline", 4),
                ("text x", 4),
                ("block", 5),
                ("anon", 5),
                ("text y", 5),
            ])
        );
    }

    #[test]
    fn a_hidden_block_inside_an_inline_splits_nothing() {
        // display: none generates no box, so the span is not split
        // around it. Nodes: div 1, span 2, p 3.
        let root = Element::new("div").push(
            Element::new("span")
                .text("a")
                .push(Element::new("p").text("b"))
                .text("c"),
        );
        let tree = build_box_tree(&styled(root, "p { display: none; }"));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[("anon", 1), ("inline", 2), ("text a", 2), ("text c", 2)])
        );
    }

    #[test]
    fn an_inline_block_is_inline_level_outside_and_a_block_container_inside() {
        // <div>a<span class=ib>b<p>c</p></span>d</div>: the span sits in
        // the div's one anonymous block like any inline-level box; its
        // own children are block-level, "b" in an anonymous block of
        // the span's. Nodes: div 1, span 2, p 3.
        let root = Element::new("div")
            .text("a")
            .push(
                Element::new("span")
                    .text("b")
                    .push(Element::new("p").text("c")),
            )
            .text("d");
        let tree = build_box_tree(&styled(root, "span { display: inline-block; }"));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("text a", 1),
                ("inline-block", 2),
                ("anon", 2),
                ("text b", 2),
                ("block", 3),
                ("anon", 3),
                ("text c", 3),
                ("text d", 1),
            ])
        );
        assert!(!BoxKind::InlineBlock.is_block_level());
    }

    #[test]
    fn white_space_between_inline_level_boxes_is_kept_and_at_the_ends_dropped() {
        // <div> <span>a</span> <span>b</span> </div>: the space between
        // the spans is the gap between two words and stays as a text
        // box; the spaces at the start and the end would be discarded
        // by any line and are not built. Nodes: div 1, spans 2 and 3.
        let root = Element::new("div")
            .text(" ")
            .push(Element::new("span").text("a"))
            .text(" ")
            .push(Element::new("span").text("b"))
            .text(" ");
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("inline", 2),
                ("text a", 2),
                ("text  ", 1),
                ("inline", 3),
                ("text b", 3),
            ])
        );
        // Inside an inline element white space is kept wherever it is:
        // `<span> </span>` between two inline-blocks is their gap.
        let root = Element::new("div").push(Element::new("span").text(" "));
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[("anon", 1), ("inline", 2), ("text  ", 2)])
        );
        // White space before a block, at the end of a run, is dropped
        // with the run when nothing else is in it.
        let root = Element::new("div")
            .text(" ")
            .push(Element::new("p").text("x"))
            .text(" ");
        let tree = build_box_tree(&styled(root, ""));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[("block", 2), ("anon", 2), ("text x", 2)])
        );
    }

    #[test]
    fn a_float_stays_where_it_is_met_and_splits_nothing() {
        // <div><p>a</p><div class=f>f</div><p>b</p></div>: between
        // blocks, the float is a child of the div like them. Nodes:
        // div 1, p 2, div.f 3, p 4.
        let root = Element::new("div")
            .push(Element::new("p").text("a"))
            .push(Element::new("div").attr("class", "f").text("f"))
            .push(Element::new("p").text("b"));
        let tree = build_box_tree(&styled(root, ".f { float: left; }"));
        let kinds: Vec<(String, usize)> = shape_of(&tree[0].children)
            .into_iter()
            .filter(|(k, _)| !k.starts_with("text"))
            .collect();
        assert_eq!(
            kinds,
            pairs(&[
                ("block", 2),
                ("anon", 2),
                ("float", 3),
                ("anon", 3),
                ("block", 4),
                ("anon", 4),
            ])
        );
        // <div>a<span>b<div class=f>f</div>c</span>d</div>: in the run,
        // inside the span, the float is inline-level content of the span
        // and does not split it (only an in-flow block does, CSS2
        // §9.2.1.1). Nodes: div 1, span 2, div.f 3.
        let root = Element::new("div").text("a").push(
            Element::new("span")
                .text("b")
                .push(Element::new("div").attr("class", "f").text("f"))
                .text("c"),
        );
        let tree = build_box_tree(&styled(root, ".f { float: right; }"));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("text a", 1),
                ("inline", 2),
                ("text b", 2),
                ("float", 3),
                ("anon", 3),
                ("text f", 3),
                ("text c", 2),
            ])
        );
        // A float first, then text: the float is met with the run still
        // empty, so it is a child of the div and the text's run follows.
        let root = Element::new("div")
            .push(Element::new("div").attr("class", "f"))
            .text("x");
        let tree = build_box_tree(&styled(root, ".f { float: left; }"));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[("float", 2), ("anon", 1), ("text x", 1)])
        );
        assert!(!BoxKind::Float.is_block_level());
        // An absolutely positioned box the same way, a float no more
        // (CSS2 §9.7). Nodes: div 1, span 2, div.a 3.
        let root = Element::new("div").text("a").push(
            Element::new("span")
                .text("b")
                .push(Element::new("div").attr("class", "a").text("f")),
        );
        let tree = build_box_tree(&styled(root, ".a { position: absolute; float: left; }"));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[
                ("anon", 1),
                ("text a", 1),
                ("inline", 2),
                ("text b", 2),
                ("absolute", 3),
                ("anon", 3),
                ("text f", 3),
            ])
        );
        let root = Element::new("div")
            .push(Element::new("div").attr("class", "a"))
            .push(Element::new("p").text("x"));
        let tree = build_box_tree(&styled(root, ".a { position: fixed; }"));
        assert_eq!(
            shape_of(&tree[0].children),
            pairs(&[("absolute", 2), ("block", 3), ("anon", 3), ("text x", 3)])
        );
        assert!(!BoxKind::Absolute.is_block_level());
    }

    #[test]
    fn top_level_inline_content_goes_into_an_anonymous_block_of_the_document() {
        // A hand-built document whose root element is inline: the
        // document node (0) is the block container.
        let tree = build_box_tree(&styled(Element::new("span").text("x"), ""));
        assert_eq!(
            shape_of(&tree),
            pairs(&[("anon", 0), ("inline", 1), ("text x", 1)])
        );
    }
}
