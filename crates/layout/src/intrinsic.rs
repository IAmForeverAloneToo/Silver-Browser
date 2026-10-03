//! Intrinsic widths (M10.4): a box's min-content and max-content
//! contributions — CSS2 §10.3.5's preferred minimum width and preferred
//! width, css-sizing-3 §5's terms — which is what shrink-to-fit sizing
//! of an inline-block (`block::layout_inline_block`) needs. A pass over
//! a subtree that measures words through `inline`'s own item
//! collection, so that what counts as a word, a gap or an unbreakable
//! piece is exactly what layout breaks by.
//!
//! A flex container's widths are its items' (M17,
//! `crate::flex::content_widths`).
//!
//! A length `width` is used as given, a percentage one as `auto` (CSS2
//! §10.3.5), an intrinsic keyword (M17) as the box's own min-content or
//! max-content width, whichever the constraint asks for (`fit-content`
//! both, by constraint); `min-width` and `max-width` clamp the
//! contribution the same way, a percentage `max-width` counting as
//! `none` and a percentage `min-width` as zero; percentage margins and paddings count as zero
//! (css-sizing-3 §5.2.1: a percentage against a size that is being
//! computed). `box-sizing: border-box` applies to the given sizes as it
//! does in layout. A contribution is never negative. A table's widths
//! are its grid's (M12, `crate::table::content_widths`), and its given
//! width is a floor over them, never a ceiling: a table is never
//! narrower than its columns need.

use style::computed::{
    BoxSizing, LEFT, LengthPercentage, LengthPercentageAuto, MaxSize, RIGHT, Size,
};

use crate::box_tree::{BoxKind, LayoutBox};
use crate::inline::{
    collect_first_line_items, collect_inline_items, restyled_on_first_line, run_intrinsic_widths,
};
use crate::pass::Pass;

/// The `(min-content, max-content)` contribution of a block-level box
/// or an inline-block to the box it sits in: its own intrinsic size
/// under its `width`, `min-width` and `max-width`, plus its horizontal
/// padding, border and margins. A table (M12) is never narrower than
/// its own minimum, whatever its `width` and `max-width` say.
pub(crate) fn contribution<'a>(b: &'a LayoutBox, pass: &Pass<'a>) -> (f32, f32) {
    let style = &b.style;
    let (non_content, margins) = fixed_edges(b);
    let inner = |v: f32| match style.box_sizing {
        BoxSizing::ContentBox => v.max(0.0),
        BoxSizing::BorderBox => (v - non_content).max(0.0),
    };
    // The box's own widths, measured once if anything asks: under a
    // min-content constraint a keyword (M17) is its min-content width
    // and `fit-content` too; under a max-content one, its max-content
    // width, `fit-content` too.
    let own = std::cell::OnceCell::new();
    let own = || *own.get_or_init(|| content_widths(b, pass));
    let keyword = |size: Size| match size {
        Size::MinContent => Some((own().0, own().0)),
        Size::MaxContent => Some((own().1, own().1)),
        Size::FitContent => Some(own()),
        Size::Auto | Size::Px(_) | Size::Percent(_) => None,
    };
    let max_keyword = |size: MaxSize| match size {
        MaxSize::MinContent => keyword(Size::MinContent),
        MaxSize::MaxContent => keyword(Size::MaxContent),
        MaxSize::FitContent => keyword(Size::FitContent),
        MaxSize::None | MaxSize::Px(_) | MaxSize::Percent(_) => None,
    };
    // A table's minimum is a floor under every other size it is given.
    let floor = if b.is_table() { own().0 } else { 0.0 };
    let (mut min, mut max) = match style.width {
        Size::Px(w) => (inner(w).max(floor), inner(w).max(floor)),
        width => keyword(width).unwrap_or_else(own),
    };
    let limit = match style.max_width {
        MaxSize::Px(limit) => Some((inner(limit), inner(limit))),
        limit => max_keyword(limit),
    };
    if let Some((under_min, under_max)) = limit {
        min = min.min(under_min.max(floor));
        max = max.min(under_max.max(floor));
    }
    let raise = match style.min_width {
        Size::Px(floor) => Some((inner(floor), inner(floor))),
        floor => keyword(floor),
    };
    if let Some((under_min, under_max)) = raise {
        min = min.max(under_min);
        max = max.max(under_max);
    }
    (
        (min + non_content + margins).max(0.0),
        (max + non_content + margins).max(0.0),
    )
}

/// What `b`'s horizontal padding and border, and its horizontal margins,
/// add to its content width in a contribution: a percentage is zero, as
/// an `auto` margin is. A table whose borders collapse has no padding,
/// and half its outer grid lines for its border (M12).
pub(crate) fn fixed_edges(b: &LayoutBox) -> (f32, f32) {
    let style = &b.style;
    let fixed = |v: LengthPercentage| match v {
        LengthPercentage::Px(px) => px,
        LengthPercentage::Percent(_) => 0.0,
    };
    let fixed_margin = |v: LengthPercentageAuto| match v {
        LengthPercentageAuto::Px(px) => px,
        LengthPercentageAuto::Percent(_) | LengthPercentageAuto::Auto => 0.0,
    };
    let border = style.used_border_width();
    let non_content = if crate::table::collapses(style) {
        let border = crate::table::collapsed_table_border(b);
        border[LEFT] + border[RIGHT]
    } else {
        border[LEFT] + border[RIGHT] + fixed(style.padding[LEFT]) + fixed(style.padding[RIGHT])
    };
    let margins = fixed_margin(style.margin[LEFT]) + fixed_margin(style.margin[RIGHT]);
    (non_content, margins)
}

/// The `(min-content, max-content)` widths of a block container's
/// content: the widest contribution among its block-level children,
/// an anonymous block's being its inline run's. The floats among them
/// (M11) are pieces of their own for the min-content width, and for
/// the max-content width sit side by side with each other and with
/// the widest in-flow child, since at that width nothing wraps and
/// every line runs beside them (css-sizing-3 §5.1.2's summing of
/// adjacent floats, taken for all the floats of the container: a
/// float below content that has ended it would not add, a case this
/// over-estimates).
///
/// Measured once a pass (M17): the pass keeps them (see
/// [`Pass::widths`]).
pub(crate) fn content_widths<'a>(b: &'a LayoutBox, pass: &Pass<'a>) -> (f32, f32) {
    pass.widths(b, || measure_content_widths(b, pass))
}

/// [`content_widths`], measured.
fn measure_content_widths<'a>(b: &'a LayoutBox, pass: &Pass<'a>) -> (f32, f32) {
    pass.measuring();
    if b.is_table() {
        return crate::table::content_widths(b, pass);
    }
    if b.is_flex_container() {
        return crate::flex::content_widths(b, pass);
    }
    let fonts = pass.fonts;
    let mut min = 0.0f32;
    let mut in_flow_max = 0.0f32;
    let mut floats_max = 0.0f32;
    for child in &b.children {
        let (child_min, child_max) = match child.kind {
            // A table contributes its grid's widths (M12); a table part
            // a hand-built tree puts in a block container contributes
            // as the block it is laid out as there.
            BoxKind::Block
            | BoxKind::InlineBlock
            | BoxKind::Float
            | BoxKind::Table
            | BoxKind::Flex
            | BoxKind::RowGroup(_)
            | BoxKind::Row
            | BoxKind::ColumnGroup { .. }
            | BoxKind::Column { .. }
            | BoxKind::Cell { .. }
            | BoxKind::Caption => contribution(child, pass),
            BoxKind::Absolute => (0.0, 0.0),
            // A run holding a first formatted line counts it in its
            // styles there (M16), where any differ from its own.
            BoxKind::AnonymousBlock => {
                let items = collect_inline_items(&child.children, fonts);
                let first = child
                    .first_line
                    .as_ref()
                    .filter(|_| restyled_on_first_line(&child.children))
                    .map(|_| collect_first_line_items(&child.children, fonts, None));
                run_intrinsic_widths(&items, first.as_deref(), pass)
            }
            // A hand-built tree's stray inline-level child: a run of one.
            BoxKind::Inline | BoxKind::Text(_) => {
                let items = collect_inline_items(std::slice::from_ref(child), fonts);
                run_intrinsic_widths(&items, None, pass)
            }
        };
        min = min.max(child_min);
        match child.kind {
            BoxKind::Float => floats_max += child_max,
            // Out of flow and beside nothing: no contribution.
            BoxKind::Absolute => {}
            _ => in_flow_max = in_flow_max.max(child_max),
        }
    }
    (min, floats_max + in_flow_max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dom::Element;
    use text::FontCollection;

    fn test_fonts() -> FontCollection {
        FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
    }

    /// The box tree of `root` under `css`.
    fn tree(root: Element, css: &str) -> Vec<LayoutBox> {
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(css).expect("test css is valid");
        crate::testing::boxes_of(&doc, &[sheet]).1
    }

    #[test]
    fn a_block_contributes_its_content_or_its_width_plus_its_edges() {
        let fonts = test_fonts();
        let w = |s: &str| fonts.measure(s, 16.0);
        let space = fonts.measure(" ", 16.0);
        // Content: the widest word and the widest line, plus padding,
        // border and margins.
        let t = tree(
            Element::new("div").text("aaa bb"),
            "div { padding: 0 5px; border-left: 2px solid #000; margin: 0 3px 0 4px; }",
        );
        let (min, max) = contribution(&t[0], &Pass::new(&fonts));
        assert_eq!(min, w("aaa") + 5.0 + 5.0 + 2.0 + 3.0 + 4.0);
        assert_eq!(
            max,
            w("aaa") + space + w("bb") + 5.0 + 5.0 + 2.0 + 3.0 + 4.0
        );
        // A length width is the content size, however wide the content;
        // a percentage width is auto; percentage edges are zero.
        let t = tree(
            Element::new("div").text("aaaa"),
            "div { width: 10px; padding: 0 10%; margin: 0 10%; }",
        );
        assert_eq!(contribution(&t[0], &Pass::new(&fonts)), (10.0, 10.0));
        let t = tree(Element::new("div").text("aaaa"), "div { width: 50%; }");
        assert_eq!(
            contribution(&t[0], &Pass::new(&fonts)),
            (w("aaaa"), w("aaaa"))
        );
        // Under border-box the width includes the edges.
        let t = tree(
            Element::new("div"),
            "div { width: 50px; padding: 0 10px; box-sizing: border-box; }",
        );
        assert_eq!(contribution(&t[0], &Pass::new(&fonts)), (50.0, 50.0));
    }

    #[test]
    fn min_and_max_width_clamp_the_contribution() {
        let fonts = test_fonts();
        let w = |s: &str| fonts.measure(s, 16.0);
        let t = tree(
            Element::new("div").text("aaaa bbbb"),
            "div { max-width: 10px; }",
        );
        assert_eq!(contribution(&t[0], &Pass::new(&fonts)), (10.0, 10.0));
        let t = tree(Element::new("div").text("a"), "div { min-width: 300px; }");
        assert_eq!(contribution(&t[0], &Pass::new(&fonts)), (300.0, 300.0));
        // Percentage limits are no limit and no floor.
        let t = tree(
            Element::new("div").text("aaaa"),
            "div { max-width: 1%; min-width: 90%; }",
        );
        assert_eq!(
            contribution(&t[0], &Pass::new(&fonts)),
            (w("aaaa"), w("aaaa"))
        );
    }

    #[test]
    fn a_keyword_contributes_the_size_it_stands_for() {
        // M17: under each constraint the keyword's own width, the limits
        // likewise, plus the edges as for any width.
        let fonts = test_fonts();
        let w = |s: &str| fonts.measure(s, 16.0);
        let (min, max) = (w("aaaa"), w("aaaa") + fonts.measure(" ", 16.0) + w("bb"));
        let of = |css: &str| {
            contribution(
                &tree(Element::new("div").text("aaaa bb"), css)[0],
                &Pass::new(&fonts),
            )
        };
        assert_eq!(of("div { width: min-content }"), (min, min));
        assert_eq!(of("div { width: max-content }"), (max, max));
        assert_eq!(of("div { width: fit-content }"), (min, max));
        assert_eq!(
            of("div { width: min-content; padding: 0 3px; box-sizing: border-box }"),
            (min + 6.0, min + 6.0)
        );
        assert_eq!(of("div { max-width: min-content }"), (min, min));
        assert_eq!(of("div { width: 1px; min-width: max-content }"), (max, max));
        assert_eq!(of("div { width: 1px; min-width: fit-content }"), (min, max));
        assert_eq!(
            of("div { width: 999px; max-width: fit-content }"),
            (min, max)
        );
        // `min-width: auto` is no floor.
        assert_eq!(of("div { width: 1px; min-width: auto }"), (1.0, 1.0));
    }

    #[test]
    fn a_table_contributes_no_less_than_its_minimum() {
        // A table is never narrower than its columns need (M12), whatever
        // its width and maximum say.
        let fonts = test_fonts();
        // Cells holding boxes 30 and 50 wide: a cell's own `width` is
        // its column's maximum, not a minimum (M12), so the content
        // gives the grid its minimum.
        let table = |css: &str| {
            tree(
                Element::new("table").push(
                    Element::new("tr")
                        .push(Element::new("td").push(Element::new("span").attr("class", "a")))
                        .push(Element::new("td").push(Element::new("span").attr("class", "b"))),
                ),
                &format!(
                    "td {{ padding: 0 }} span {{ display: inline-block }}
                     .a {{ width: 30px }} .b {{ width: 50px }} {css}"
                ),
            )
        };
        let t = table("");
        let (grid_min, _) = content_widths(&t[0], &Pass::new(&fonts));
        assert!(grid_min > 80.0, "{grid_min}");
        for css in [
            "table { width: 10px }",
            "table { max-width: 10px }",
            "table { max-width: min-content; width: 5px }",
        ] {
            let t = table(css);
            assert_eq!(
                contribution(&t[0], &Pass::new(&fonts)),
                (grid_min, grid_min),
                "{css}"
            );
        }
    }

    #[test]
    fn a_block_container_is_as_wide_as_its_widest_child() {
        let fonts = test_fonts();
        let w = |s: &str| fonts.measure(s, 16.0);
        // <div><p>aaaa bb</p><p style="width: 300px"></p>c</div>: the
        // widest min-content is the fixed p, the widest max-content too.
        let t = tree(
            Element::new("div")
                .push(Element::new("p").text("aaaa bb"))
                .push(Element::new("p").attr("style", "width: 300px"))
                .text("c"),
            "p { margin: 0; }",
        );
        assert_eq!(content_widths(&t[0], &Pass::new(&fonts)), (300.0, 300.0));
        // Without the fixed one: the first p's word and line.
        let t = tree(
            Element::new("div")
                .push(Element::new("p").text("aaaa bb"))
                .text("c"),
            "p { margin: 0; }",
        );
        let (min, max) = content_widths(&t[0], &Pass::new(&fonts));
        assert_eq!(min, w("aaaa"));
        assert_eq!(max, w("aaaa") + fonts.measure(" ", 16.0) + w("bb"));
    }

    #[test]
    fn floats_sit_beside_the_content_for_the_max_content_width() {
        let fonts = test_fonts();
        let w = |s: &str| fonts.measure(s, 16.0);
        // Two floats and a paragraph: at the max-content width the
        // floats stand side by side and the paragraph's line runs
        // beside them; the min-content width is the widest piece.
        let t = tree(
            Element::new("div")
                .push(Element::new("div").attr("class", "f"))
                .push(Element::new("div").attr("class", "g"))
                .push(Element::new("p").text("aaaa bb")),
            ".f { float: left; width: 30px; } .g { float: right; width: 20px; } p { margin: 0; }",
        );
        let (min, max) = content_widths(&t[0], &Pass::new(&fonts));
        assert_eq!(min, w("aaaa").max(30.0));
        assert_eq!(
            max,
            30.0 + 20.0 + w("aaaa") + fonts.measure(" ", 16.0) + w("bb")
        );
        // A float met in a run widens that run's line too.
        let t = tree(
            Element::new("div")
                .text("aaaa ")
                .push(Element::new("div").attr("class", "f"))
                .text(" bb"),
            ".f { float: left; width: 30px; }",
        );
        let (min, max) = content_widths(&t[0], &Pass::new(&fonts));
        assert_eq!(min, w("aaaa").max(30.0));
        assert_eq!(max, w("aaaa") + fonts.measure(" ", 16.0) + w("bb") + 30.0);
    }
}
