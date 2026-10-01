//! Block formatting context v1 (roadmap M6.2–M6.7): box tree in,
//! positioned [`Fragment`]s out. Inline-level content (`display:
//! inline`, and text) is delegated to [`crate::inline`] (M7) for real
//! line breaking — see that module's docs for its own v1 scope.
//!
//! Scope, deliberately, matching M6's own "v1":
//!
//! * Only normal-flow, non-replaced block boxes get the real CSS2.1
//!   §10.3.3 width/margin algorithm and §8.3.1 margin collapsing.
//!   Floats, positioning, and tables are M11/M12.
//! * Margins never collapse *through* an inline formatting context —
//!   it always "separates" a block's margin-top/bottom from its
//!   siblings', same as a border or padding would.
//! * `height: <percentage>` isn't resolved (M6.4 defers it to M10); a
//!   percentage `height` already failed to parse back in `style`
//!   (M5/M6 extension), so it's never seen here — this module only ever
//!   sees `Auto` or `Px`.

use style::computed::{
    BorderStyle, ComputedStyle, LengthPercentageAuto, TextAlign, BOTTOM, LEFT, RIGHT, TOP,
};
use text::FontCollection;

use crate::box_tree::{BoxKind, LayoutBox};
use crate::inline::layout_inline;

/// A positioned, sized box ready for painting (roadmap M6.7) — absolute
/// viewport coordinates (`layout::layout` is the only place translation
/// happens; everything internal to this module works in box-relative
/// coordinates and translates on the way back up the tree).
#[derive(Debug, Clone)]
pub struct Fragment {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub style: ComputedStyle,
    pub kind: FragmentKind,
}

#[derive(Debug, Clone)]
pub enum FragmentKind {
    /// A block/inline box's own border box — background and border
    /// (M6.8) paint from `style`, within `(x, y, w, h)`.
    Box,
    /// One text run; `y` is its *baseline* (matching
    /// `paint::DisplayItem::Text`'s existing convention), not its top.
    Text(String),
}

impl Fragment {
    pub(crate) fn translated(mut self, dx: f32, dy: f32) -> Self {
        self.x += dx;
        self.y += dy;
        self
    }
}

/// Lays out `boxes` (the document's top-level content) against the
/// initial containing block — the viewport (M6.6) — and returns their
/// fragments in absolute, viewport-relative coordinates. `fonts` is what
/// M7's real text measurement/line-breaking shapes and measures words
/// with, now resolving a per-script fallback font (M8.1) rather than
/// always the one primary font. Ideally it's the exact
/// `FontCollection` `render` goes on to draw with, so measured line
/// breaks match what's drawn pixel-for-pixel; today `engine::Engine`
/// instead loads a second `FontCollection` independently for this (same
/// deterministic system-font queries as `render`'s own, so in practice
/// it resolves to the same font files, but it's a second load, not a
/// shared handle — sharing one properly is natural M35 perf-track
/// territory, not a v1 requirement).
pub fn layout(boxes: &[LayoutBox], viewport_width: f32, fonts: &FontCollection) -> Vec<Fragment> {
    // No containing element at the document root to inherit `text-align`
    // from — `TextAlign::Left` is that property's initial value anyway.
    let children = layout_children(boxes, viewport_width, TextAlign::Left, fonts);
    // The viewport is the one context with no further ancestor to
    // escape to — `leading_margin` *must* materialize here, exactly
    // once, or it would just vanish (see `layout_children`'s doc).
    children
        .fragments
        .into_iter()
        .map(|f| f.translated(0.0, children.leading_margin))
        .collect()
}

/// `border-{side}-width`, but zero whenever `border-style: none` — a
/// `none` border has zero width for *every* purpose, not just painting
/// (CSS2 §8.5.3); every layout calculation reads border width through
/// this, never `style.border_width` directly.
fn effective_border_width(style: &ComputedStyle) -> [f32; 4] {
    if style.border_style == BorderStyle::None {
        [0.0; 4]
    } else {
        style.border_width
    }
}

/// CSS2 §8.3.1's adjoining-margins formula: the greatest of the positive
/// margins, minus the greatest absolute value among the negative ones
/// (if all margins are non-negative, this is just their max; the
/// "collapsing *through*" cases — an empty block, or escaping past a
/// border/padding-free parent — are built out of repeated pairwise
/// calls to this, not a separate algorithm).
fn collapse_margins(a: f32, b: f32) -> f32 {
    let positive = a.max(0.0).max(b.max(0.0));
    let negative = (-a).max(0.0).max((-b).max(0.0));
    positive - negative
}

/// CSS2.1 §10.3.3: resolves `width`, `margin-left`, and `margin-right`
/// for a normal-flow, non-replaced block box. `non_content_width` is
/// whatever horizontal space border+padding already consume.
fn resolve_width_and_margins(
    style: &ComputedStyle,
    cb_width: f32,
    non_content_width: f32,
) -> (f32, f32, f32) {
    let avail = cb_width - non_content_width;
    let margin_left = style.margin[LEFT];
    let margin_right = style.margin[RIGHT];

    if style.width.is_auto() {
        // Unspecified margins default to 0 (not symmetrical centering —
        // that only happens when `width` itself is NOT auto); auto
        // `width` absorbs whatever's left. The used value of `width`
        // is never negative (CSS2.1 §10.2); margins may be.
        let ml = margin_left.resolve(cb_width, 0.0);
        let mr = margin_right.resolve(cb_width, 0.0);
        return ((avail - ml - mr).max(0.0), ml, mr);
    }

    let w = style.width.resolve(cb_width, 0.0).max(0.0);
    let remaining = avail - w;
    match (margin_left.is_auto(), margin_right.is_auto()) {
        (true, true) => {
            let half = remaining / 2.0; // M6.3: auto-centering
            (w, half, half)
        }
        (true, false) => {
            let mr = margin_right.resolve(cb_width, 0.0);
            (w, remaining - mr, mr)
        }
        (false, true) | (false, false) => {
            // Over-constrained (both specified): per the ltr rule,
            // margin-right is recomputed regardless of its specified
            // value, as if it alone were auto.
            let ml = margin_left.resolve(cb_width, 0.0);
            (w, ml, remaining - ml)
        }
    }
}

/// One box's layout, relative to its own border-box top-left `(0, 0)` —
/// the caller (`layout_children`) translates once collapsed-margin
/// placement among siblings is resolved.
pub(crate) struct BlockLayout {
    pub(crate) fragments: Vec<Fragment>,
    /// Border-box height (margin excluded) — siblings stack purely
    /// vertically, so this is the only dimension the caller needs; the
    /// box's own width is already baked into its fragments' `w`.
    pub(crate) height: f32,
    /// Only `margin_left` matters to the caller: horizontal margins
    /// don't collapse between siblings the way vertical ones do, so
    /// `margin_right` is fully "spent" by this box's own
    /// `resolve_width_and_margins` call and never consulted again.
    pub(crate) margin_left: f32,
    /// This box's margin-top/bottom *as exposed to its siblings* —
    /// collapsed with its first/last child's (one level: see the module
    /// doc), or with each other if the box is empty (CSS2 §8.3.1).
    pub(crate) margin_top: f32,
    pub(crate) margin_bottom: f32,
}

/// Lays out one block box. `pub(crate)`: also called by `crate::inline`
/// for the M7.6 block-in-inline case (a block box nested in inline
/// content lays out the same way regardless of what's around it).
pub(crate) fn layout_block(b: &LayoutBox, cb_width: f32, fonts: &FontCollection) -> BlockLayout {
    let border = effective_border_width(&b.style);
    let padding = b.style.padding;
    let has_top_sep = border[TOP] > 0.0 || padding[TOP] > 0.0;
    let has_bottom_sep = border[BOTTOM] > 0.0 || padding[BOTTOM] > 0.0;
    let height_is_auto = b.style.height.is_auto();

    let non_content_width = border[LEFT] + border[RIGHT] + padding[LEFT] + padding[RIGHT];
    // margin-right is fully resolved here but never read again — see
    // `BlockLayout::margin_left`'s doc for why that's correct, not an
    // oversight.
    let (content_width, margin_left, _margin_right) =
        resolve_width_and_margins(&b.style, cb_width, non_content_width);

    // An empty block (no children, no border/padding, auto height):
    // its own margin-top and margin-bottom collapse with *each other*,
    // escaping through as a single value on both sides at once — as if
    // the box weren't there at all, margin-wise (CSS2 §8.3.1).
    if b.children.is_empty() && !has_top_sep && !has_bottom_sep && height_is_auto {
        let collapsed = collapse_margins(b.style.margin[TOP].resolve(0.0, 0.0), b.style.margin[BOTTOM].resolve(0.0, 0.0));
        return BlockLayout {
            fragments: vec![box_fragment(0.0, 0.0, content_width, 0.0, &b.style)],
            height: 0.0,
            margin_left,
            margin_top: collapsed,
            margin_bottom: collapsed,
        };
    }

    let children = layout_children(&b.children, content_width, b.style.text_align, fonts);

    // `layout_children` never materializes the *leading* margin itself
    // (see its doc) — it's this box's job to decide, now that it knows
    // its own separation, whether that margin becomes real space just
    // inside its own border/padding-top (`child_shift`, when there's a
    // border/padding to stop the escape) or keeps escaping outward,
    // combined with this box's own margin-top (when there isn't). This
    // is the step a naive "always materialize inside the parent" design
    // gets wrong: that double-counts the escaping case, because the
    // SAME margin would then also get re-applied one level up, when
    // *this* box gets positioned among *its* own siblings.
    let (child_shift, margin_top) = if has_top_sep {
        (children.leading_margin, b.style.margin[TOP].resolve(0.0, 0.0))
    } else {
        (0.0, collapse_margins(b.style.margin[TOP].resolve(0.0, 0.0), children.leading_margin))
    };

    let content_height = match b.style.height {
        LengthPercentageAuto::Px(h) => h,
        _ /* Auto; a Percent is never produced yet, see the module doc */ => {
            let base = children.content_height + child_shift;
            if has_bottom_sep {
                // The last child's margin-bottom doesn't escape — it
                // becomes real space above the padding/border.
                base + children.trailing_margin
            } else {
                base
            }
        }
    };

    let margin_bottom = if has_bottom_sep || !height_is_auto {
        b.style.margin[BOTTOM].resolve(0.0, 0.0)
    } else {
        collapse_margins(b.style.margin[BOTTOM].resolve(0.0, 0.0), children.trailing_margin)
    };

    let border_box_width = content_width + non_content_width;
    let border_box_height = border[TOP] + padding[TOP] + content_height + padding[BOTTOM] + border[BOTTOM];

    let mut fragments = vec![box_fragment(0.0, 0.0, border_box_width, border_box_height, &b.style)];
    let content_x = border[LEFT] + padding[LEFT];
    let content_y = border[TOP] + padding[TOP] + child_shift;
    fragments.extend(children.fragments.into_iter().map(|f| f.translated(content_x, content_y)));

    BlockLayout { fragments, height: border_box_height, margin_left, margin_top, margin_bottom }
}

fn box_fragment(x: f32, y: f32, w: f32, h: f32, style: &ComputedStyle) -> Fragment {
    Fragment { x, y, w, h, style: style.clone(), kind: FragmentKind::Box }
}

/// The result of stacking a sequence of sibling boxes in normal flow.
struct ChildrenLayout {
    /// Every descendant's fragment, relative to the *parent's*
    /// content-box top-left `(0, 0)` — *not* counting `leading_margin`
    /// (see that field's doc): the first child sits at relative `y = 0`
    /// regardless of its own margin-top.
    fragments: Vec<Fragment>,
    /// Content height counting from the first child's `y = 0` — i.e.
    /// also not counting `leading_margin` — and not counting any
    /// trailing collapsible margin either (the caller decides whether
    /// *that* one becomes real space or escapes further).
    content_height: f32,
    /// The first child's margin-top (already collapsed with whatever of
    /// *its* own children escaped, recursively) — deliberately **not**
    /// baked into `fragments`' positions or `content_height`. Whether it
    /// ever becomes real space depends on information only the caller
    /// has (its own border/padding): a parent with top separation turns
    /// it into a gap just inside its content-box top; a parent without
    /// one collapses it with its *own* margin-top and keeps escaping.
    /// Materializing it here unconditionally would double-count it the
    /// moment it keeps escaping — it would show up once when the
    /// escaping value is eventually placed, *and* again as this
    /// already-baked-in internal gap. Zero when there are no children
    /// (nothing to expose) — safe, since `collapse_margins(x, 0.0) == x`
    /// for any `x`, so a parent combining it with its own margin-top is
    /// unaffected either way.
    leading_margin: f32,
    /// The last child's margin-bottom, exposed the same way, for
    /// parent/last-child collapsing.
    trailing_margin: f32,
}

/// `text_align` is the containing block's own — it governs how *its*
/// direct inline runs are aligned (M7.5); nested blocks resolve their
/// own via their own `layout_block` call, reading their own style.
fn layout_children(
    children: &[LayoutBox],
    cb_width: f32,
    text_align: TextAlign,
    fonts: &FontCollection,
) -> ChildrenLayout {
    let mut fragments = Vec::new();
    let mut y = 0.0f32;
    // A margin collapsed-but-not-yet-materialized BETWEEN two siblings
    // (never the leading one, which `still_leading` routes to
    // `leading_margin` instead, never adding it to `y`).
    let mut pending_margin = 0.0f32;
    let mut still_leading = true;
    let mut leading_margin = 0.0f32;

    let mut i = 0;
    while i < children.len() {
        match &children[i].kind {
            BoxKind::Block => {
                let layout = layout_block(&children[i], cb_width, fonts);
                if still_leading {
                    leading_margin = layout.margin_top;
                    still_leading = false;
                    // `y` stays at 0 — see `leading_margin`'s doc.
                } else {
                    pending_margin = collapse_margins(pending_margin, layout.margin_top);
                    y += pending_margin;
                }
                let child_y = y;
                fragments.extend(
                    layout
                        .fragments
                        .into_iter()
                        .map(|f| f.translated(layout.margin_left, child_y)),
                );
                y = child_y + layout.height;
                pending_margin = layout.margin_bottom;
                i += 1;
            }
            BoxKind::Inline | BoxKind::Text(_) => {
                // Inline/text content always separates (margins never
                // collapse through it, same as a border/padding would)
                // — flush whatever's pending (or stop a still-leading
                // escape dead, materializing nothing since there was
                // never a border/padding to place it against).
                if still_leading {
                    still_leading = false; // leading_margin stays 0: nothing escapes past real content
                } else {
                    y += pending_margin;
                }
                pending_margin = 0.0;

                // M7: the *whole* run of consecutive inline-level
                // siblings shares one inline formatting context (they
                // can wrap onto the same lines together) — collect it
                // before laying any of it out, rather than treating
                // each sibling as its own forced line break the way M6
                // did.
                let start = i;
                while i < children.len() && !matches!(children[i].kind, BoxKind::Block) {
                    i += 1;
                }
                let (run_fragments, run_height) =
                    layout_inline(&children[start..i], cb_width, text_align, fonts);
                fragments.extend(run_fragments.into_iter().map(|f| f.translated(0.0, y)));
                y += run_height;
            }
        }
    }

    ChildrenLayout { fragments, content_height: y, leading_margin, trailing_margin: pending_margin }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dom::Element;

    const VIEWPORT: f32 = 800.0;

    /// Builds the box tree for `root` under `css_src` and lays it out
    /// against [`VIEWPORT`] — the full `style` -> `box_tree` -> `block`
    /// pipeline, exactly as `engine` will drive it (M6.7's integration
    /// point, exercised at the unit level here). `None` on machines with
    /// no usable system font (M7's text measurement needs one) — callers
    /// skip the test entirely in that case, same pattern `text`'s own
    /// tests use.
    fn fragments_of(root: Element, css_src: &str) -> Option<Vec<Fragment>> {
        let fonts = FontCollection::new().ok()?;
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(css_src).expect("test css is valid");
        let styled = style::style_document(&doc, &[sheet]);
        let boxes = crate::build_box_tree(&styled);
        Some(layout(&boxes, VIEWPORT, &fonts))
    }

    fn box_fragments(frags: &[Fragment]) -> Vec<&Fragment> {
        frags.iter().filter(|f| matches!(f.kind, FragmentKind::Box)).collect()
    }

    #[test]
    fn empty_auto_block_fills_width_and_has_zero_height() {
        let Some(frags) = fragments_of(Element::new("div"), "div { display: block; }") else {
            return;
        };
        assert_eq!(frags.len(), 1);
        let f = &frags[0];
        assert_eq!((f.x, f.y, f.w, f.h), (0.0, 0.0, VIEWPORT, 0.0));
    }

    #[test]
    fn fixed_geometry_matches_hand_computed_reference() {
        // The M6.10 acceptance scenario: a hand-computed reference.
        // margin: 10px (all sides — not auto, so the "over-constrained"
        // ltr rule applies to margin-right internally, but that's not
        // independently observable here; only margin-LEFT shows up in
        // `x`, which is what this checks).
        let Some(frags) = fragments_of(
            Element::new("div"),
            "div {
                display: block;
                width: 200px; height: 100px;
                margin: 10px;
                padding: 5px;
                border-width: 3px; border-style: solid;
            }",
        ) else {
            return;
        };
        assert_eq!(frags.len(), 1);
        let f = &frags[0];
        // border-box: width = 200 + 2*5 (padding) + 2*3 (border) = 216;
        // height = 100 + 2*5 + 2*3 = 116; positioned after margin (10, 10).
        assert_eq!((f.x, f.y, f.w, f.h), (10.0, 10.0, 216.0, 116.0));
    }

    #[test]
    fn auto_width_fills_remaining_space_after_margins() {
        let Some(frags) = fragments_of(
            Element::new("div"),
            "div { display: block; margin: 0 50px; }", // width: auto (initial)
        ) else {
            return;
        };
        assert_eq!(frags[0].w, VIEWPORT - 100.0);
        assert_eq!(frags[0].x, 50.0);
    }

    #[test]
    fn margin_auto_both_sides_centers() {
        let Some(frags) = fragments_of(
            Element::new("div"),
            "div { display: block; width: 200px; margin: 0 auto; }",
        ) else {
            return;
        };
        let f = &frags[0];
        assert_eq!(f.w, 200.0);
        assert_eq!(f.x, (VIEWPORT - 200.0) / 2.0);
    }

    #[test]
    fn margin_auto_one_side_pushes_to_the_other_edge() {
        let Some(frags) = fragments_of(
            Element::new("div"),
            "div { display: block; width: 200px; margin-left: auto; margin-right: 0px; }",
        ) else {
            return;
        };
        // auto absorbs all remaining space on its own side.
        assert_eq!(frags[0].x, VIEWPORT - 200.0);
    }

    #[test]
    fn adjacent_siblings_collapse_to_the_larger_positive_margin() {
        let root = Element::new("div")
            .push(Element::new("p").attr("class", "a"))
            .push(Element::new("p").attr("class", "b"));
        let Some(frags) = fragments_of(
            root,
            "div { display: block; } p { display: block; padding: 1px; }
             .a { margin: 10px 0 20px 0; } .b { margin: 30px 0 0 0; }",
        ) else {
            return;
        };
        let boxes = box_fragments(&frags);
        // div, p.a, p.b. `padding: 1px` keeps a/b from self-collapsing
        // (CSS2 §8.3.1's *other* collapsing case — tested on its own in
        // `empty_block_self_collapses_top_and_bottom`), so this test
        // isolates pure sibling-to-sibling collapsing.
        assert_eq!(boxes.len(), 3);
        let (div, a, b) = (boxes[0], boxes[1], boxes[2]);
        assert_eq!(div.y, 10.0); // div's own margin-top (0) collapses with .a's (10) escaping through
        assert_eq!(a.y, 10.0); // flush with div (div has no border/padding)
        // Gap between a's border-box bottom and b's top is max(20, 30) = 30, not 20+30=50.
        assert_eq!(b.y, a.y + a.h + 30.0);
    }

    #[test]
    fn adjacent_siblings_mixed_sign_margins_collapse_per_formula() {
        // Non-empty (padding keeps them from self-collapsing) so the
        // *sibling* formula is isolated from the *empty-block* one.
        let root = Element::new("div")
            .push(Element::new("p").attr("class", "a"))
            .push(Element::new("p").attr("class", "b"));
        let Some(frags) = fragments_of(
            root,
            "div { display: block; } p { display: block; padding: 1px; }
             .a { margin-bottom: 30px; } .b { margin-top: -10px; }",
        ) else {
            return;
        };
        let boxes = box_fragments(&frags);
        let (a, b) = (boxes[1], boxes[2]);
        // collapse(30, -10) = max(30,0) - max(0,10) = 20.
        assert_eq!(b.y, a.y + 2.0 /* a's own border-box height */ + 20.0);
    }

    #[test]
    fn parent_first_child_margin_collapses_without_double_counting() {
        // The exact scenario that caught a real double-counting bug
        // during development: div has no border/padding, so its own
        // margin-top (5) collapses with p's margin-top (25) — the
        // *combined* value (25, the max) must appear exactly once, with
        // div and p ending up flush with each other, not 25 (div's
        // escaped margin) + 25 (p's own, re-applied inside div) = 50.
        let root = Element::new("div").push(Element::new("p"));
        let Some(frags) = fragments_of(
            root,
            "div { display: block; margin-top: 5px; }
             p { display: block; margin-top: 25px; }",
        ) else {
            return;
        };
        let boxes = box_fragments(&frags);
        let (div, p) = (boxes[0], boxes[1]);
        assert_eq!(div.y, 25.0, "the collapsed (max) margin, not div's own 5px");
        assert_eq!(p.y, 25.0, "flush with div, not 25 (div) + 25 (p) = 50");
    }

    #[test]
    fn parent_with_top_padding_stops_the_collapse() {
        let root = Element::new("div").push(Element::new("p"));
        let Some(frags) = fragments_of(
            root,
            "div { display: block; margin-top: 5px; padding-top: 2px; }
             p { display: block; margin-top: 25px; }",
        ) else {
            return;
        };
        let boxes = box_fragments(&frags);
        let (div, p) = (boxes[0], boxes[1]);
        assert_eq!(div.y, 5.0, "div's own margin, uncollapsed — padding stops it");
        // p sits inside div: div's padding-top (2) + p's own margin-top
        // (25, now a real internal gap, not an escaping one).
        assert_eq!(p.y, div.y + 2.0 + 25.0);
    }

    #[test]
    fn parent_last_child_margin_collapses_when_height_is_auto() {
        // A trailing sibling after the collapsing pair makes the
        // escaped bottom margin observable as real spacing.
        let root = Element::new("outer")
            .push(Element::new("div").push(Element::new("p")))
            .push(Element::new("div"));
        let Some(frags) = fragments_of(
            root,
            // `margin: 0` resets `p`'s UA margin (ua.css's `p { margin:
            // 16px 0; }` would otherwise still apply to margin-top,
            // unrelated to what this test checks but worth resetting
            // for a fixture that's easy to hand-verify). `padding: 1px`
            // keeps `p` from self-collapsing (its own separate
            // CSS2 §8.3.1 case, tested on its own elsewhere) — this
            // test is specifically about the margin escaping *through*
            // div#1 (which has no border/padding of its own), not about
            // p's internal self-collapse.
            "outer, div, p { display: block; margin: 0; }
             p { padding: 1px; }
             div:first-child { margin-bottom: 5px; }
             div:first-child p { margin-bottom: 25px; }",
        ) else {
            return;
        };
        let boxes = box_fragments(&frags);
        // outer, div#1, p, div#2
        assert_eq!(boxes.len(), 4);
        let (div1, p, div2) = (boxes[1], boxes[2], boxes[3]);
        assert_eq!(
            div1.h,
            p.y + p.h,
            "div1's border-box ends exactly where p's content does — the margin escaped, not materialized inside"
        );
        assert_eq!(
            div2.y,
            div1.y + div1.h + 25.0,
            "the collapsed (max) margin between div1 and div2, escaped through div1's bottom"
        );
    }

    #[test]
    fn empty_block_self_collapses_top_and_bottom() {
        let Some(frags) = fragments_of(
            Element::new("div"),
            "div { display: block; margin-top: 10px; margin-bottom: 30px; }",
        ) else {
            return;
        };
        // Nothing follows, so only the top (leading) margin is
        // observable at the viewport boundary — but it must be the
        // *collapsed* value (max(10,30)=30), not just margin-top (10).
        assert_eq!(frags[0].y, 30.0);
    }

    #[test]
    fn text_run_produces_a_baseline_positioned_fragment() {
        let Some(fonts) = FontCollection::new().ok() else {
            return;
        };
        let Some(frags) = fragments_of(
            Element::new("p").text("hello"),
            // `margin: 0` resets ua.css's `p { margin: 16px 0; }` —
            // this test is about the baseline/color relationship, not
            // margins, so keep the fixture independent of them.
            "p { display: block; margin: 0; color: #ff0000; font-size: 20px; }",
        ) else {
            return;
        };
        let text = frags.iter().find(|f| matches!(f.kind, FragmentKind::Text(_)));
        let f = text.expect("a text fragment");
        match &f.kind {
            FragmentKind::Text(t) => assert_eq!(t, "hello"),
            _ => unreachable!(),
        }
        // The baseline sits at the font's own ascent (M7.1/M7.4) — not
        // simply the font-size, and not a fixed guess: whatever the
        // *actual* loaded system font's metrics say, since that's what
        // `layout_inline` used too.
        assert_eq!(f.y, fonts.primary().metrics(20.0).ascent);
        assert_eq!(f.style.color, style::computed::Color::rgb8(255, 0, 0));
    }

    #[test]
    fn display_none_element_contributes_no_fragment_or_space() {
        let root = Element::new("div")
            .push(Element::new("p").attr("class", "gone").text("x"))
            .push(Element::new("p").attr("class", "shown").text("y"));
        let Some(frags) = fragments_of(
            root,
            "div, p { display: block; }
             .gone { display: none; margin: 100px; }",
        ) else {
            return;
        };
        let boxes = box_fragments(&frags);
        // div + the one visible p — the hidden one contributes no box
        // (not even a zero-size placeholder) and no margin at all.
        assert_eq!(boxes.len(), 2);
    }

    #[test]
    fn two_level_document_with_margins_padding_and_borders() {
        let Some(fonts) = FontCollection::new().ok() else {
            return;
        };
        // M6.10/M7.8: a fuller acceptance case combining several
        // features at once, still hand-computed exactly — using the
        // *actual* loaded font's metrics for the text parts, same as
        // `text_run_produces_a_baseline_positioned_fragment` above.
        let root = Element::new("section").push(
            Element::new("article")
                .push(Element::new("h1").text("Title"))
                .push(Element::new("p").text("Body")),
        );
        let Some(frags) = fragments_of(
            root,
            "section, article, h1, p { display: block; margin: 0; padding: 0; }
             article { padding: 10px; border-width: 2px; border-style: solid; }
             h1 { font-size: 30px; margin-bottom: 15px; }
             p { font-size: 10px; }",
        ) else {
            return;
        };
        let boxes = box_fragments(&frags);
        let texts: Vec<&Fragment> =
            frags.iter().filter(|f| matches!(f.kind, FragmentKind::Text(_))).collect();
        // section, article, h1, p: every block box gets its own Box
        // fragment regardless of margin/border/padding collapsing —
        // collapsing only affects *position*, never whether a box
        // fragment exists at all.
        assert_eq!(boxes.len(), 4);
        let article = boxes[1];
        // article's border-box starts at (0,0): no margin anywhere, and
        // section itself has none either.
        assert_eq!((article.x, article.y), (0.0, 0.0));
        // h1's line height/baseline, h1's own font-size (30px).
        let h1_metrics = fonts.primary().metrics(30.0);
        // h1's text baseline: inside article's border(2)+padding(10) = 12.
        let h1_text = texts.iter().find(|f| matches!(&f.kind, FragmentKind::Text(t) if t == "Title")).unwrap();
        assert_eq!(h1_text.y, 12.0 + h1_metrics.ascent);
        // p's text baseline: after h1's full line box + h1's own margin-bottom (15) + article's padding-top (already counted above, not again).
        let p_metrics = fonts.primary().metrics(10.0);
        let p_text = texts.iter().find(|f| matches!(&f.kind, FragmentKind::Text(t) if t == "Body")).unwrap();
        assert_eq!(p_text.y, 12.0 + h1_metrics.line_height() + 15.0 + p_metrics.ascent);
    }
}
