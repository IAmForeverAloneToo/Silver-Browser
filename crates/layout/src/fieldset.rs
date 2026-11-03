//! Fieldsets (HTML section 15.3.13): a fieldset's rendered legend laid
//! out over its top border, the rest of its content below as a block's,
//! as the reference browser lays them out.
//!
//! The box tree builder makes the rendered legend, the fieldset's first
//! child `legend` that neither floats nor is absolutely positioned, a
//! [`BoxKind::Legend`] and the fieldset's first child. Its layout is a
//! float's: shrink-to-fit in the fieldset's content width, a block
//! formatting context root. It goes in the area at the top of the
//! fieldset as tall as the top border or the legend, whichever is the
//! taller: at its top when the legend is, else centred on the border.
//! Its top margin is not counted and its bottom margin is, as in the
//! reference browser. Across, it is at the content box's start past its
//! start margin, at the end with an `auto` start margin, centred with
//! both `auto`; with neither, its `justify-self` (which HTML's `align`
//! gives) places it, not the `text-align` it inherits. The rest of the
//! content is laid out as the fieldset's with that area for its top
//! border: the content box starts below it, and so does an absolutely
//! positioned box's containing block. The fieldset's heights are as if
//! the area were its border, the legend
//! taking room from its content; the box is as tall as it would be with
//! no legend unless the area is the taller. Paint draws the background
//! and the border from the middle of a taller legend down, the border
//! with a gap where the legend is ([`Fragment::legend`]).

use style::computed::{
    AlignPosition, ComputedStyle, Direction, LEFT, LengthPercentageAuto, RIGHT, SelfAlignment, TOP,
};

use crate::block::{
    BlockLayout, Content, Deferred, Edges, Fragment, Imposed, carried, layout_flow,
    layout_inline_block,
};
use crate::box_tree::{BoxKind, LayoutBox};
use crate::cache::LayoutCache;
use crate::pass::Pass;

/// A fieldset's rendered legend laid out, which the fieldset's flow
/// layout puts before the rest of its content.
pub(crate) struct LaidLegend<'a> {
    /// The legend's fragment, placed in the fieldset's border box.
    pub(crate) fragment: Fragment,
    /// The absolutely positioned boxes carried out of the legend,
    /// relative to the fieldset's border box.
    pub(crate) absolutes: Vec<Deferred<'a>>,
    /// How far the legend's area reaches below the top border: what it
    /// takes from the content's height.
    pub(crate) intrusion: f32,
}

/// The rendered legend of `b`, which is its first child when it has
/// one.
pub(crate) fn rendered_legend(b: &LayoutBox) -> Option<&LayoutBox> {
    b.children.first().filter(|c| c.kind == BoxKind::Legend)
}

/// Lays out the fieldset `b`, whose first child is its rendered legend,
/// as [`crate::block::layout_block_contents`] lays out a block
/// container: the legend in the area over its top border, the rest of
/// its children below it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_fieldset_contents<'a>(
    b: &'a LayoutBox,
    edges: &Edges,
    cb_width: f32,
    cb_height: Option<f32>,
    content_width: f32,
    margin_left: f32,
    content: Content<'_>,
    pass: &Pass<'a>,
    cache: Option<&LayoutCache>,
    imposed: Option<Imposed>,
) -> BlockLayout<'a> {
    let legend = &b.children[0];
    let laid = layout_inline_block(legend, content_width, None, pass);
    let border_top = edges.border[TOP];
    let height = laid.root.h;
    let margin_bottom = laid.height - laid.margin_top - height;
    // Centred on a border taller than it, at the top otherwise.
    let y = ((border_top - height) / 2.0).max(0.0);
    let area = (y + height + margin_bottom).max(border_top);
    let margin_right = legend.style.margin[RIGHT].resolve(content_width, 0.0);
    let x = edges.border[LEFT]
        + edges.padding[LEFT]
        + inline_offset(
            &b.style,
            &legend.style,
            laid.root.w,
            (laid.margin_left, margin_right),
            content_width,
        );
    let (dx, dy) = laid.relative;
    let legend = LaidLegend {
        fragment: laid.root.translated(x + dx, y + dy),
        absolutes: carried(laid.absolutes, x + dx, y + dy),
        intrusion: area - border_top,
    };
    let mut border = edges.border;
    border[TOP] = area;
    let edges = Edges {
        border,
        padding: edges.padding,
    };
    layout_flow(
        b,
        &b.children[1..],
        &edges,
        cb_width,
        cb_height,
        content_width,
        margin_left,
        content,
        pass,
        cache,
        imposed,
        Some(legend),
    )
}

/// Where a legend `width` wide with the inline margins `margins`, left
/// and right, goes across a fieldset's content box `room` wide: its
/// border box's left edge from the content box's. `auto` margins place
/// it, else the legend's `justify-self` (HTML section 15.3.13), in the
/// fieldset's direction: `left` and `right` as they say, the rest at
/// the start but `center` and the ends.
fn inline_offset(
    fieldset: &ComputedStyle,
    legend: &ComputedStyle,
    width: f32,
    (left, right): (f32, f32),
    room: f32,
) -> f32 {
    let rtl = fieldset.direction == Direction::Rtl;
    let auto = |side: usize| matches!(legend.margin[side], LengthPercentageAuto::Auto);
    let (start_auto, end_auto) = if rtl {
        (auto(RIGHT), auto(LEFT))
    } else {
        (auto(LEFT), auto(RIGHT))
    };
    let free = room - width - left - right;
    // How far past its start margin from the start of the room.
    let shift = match (start_auto, end_auto) {
        (true, true) => free / 2.0,
        (true, false) => free,
        (false, true) => 0.0,
        (false, false) => match (&legend.grid().justify_self, rtl) {
            (SelfAlignment::Position(AlignPosition::Center, _), _) => free / 2.0,
            (SelfAlignment::Position(AlignPosition::Right, _), false)
            | (SelfAlignment::Position(AlignPosition::Left, _), true)
            | (
                SelfAlignment::Position(
                    AlignPosition::End | AlignPosition::SelfEnd | AlignPosition::FlexEnd,
                    _,
                ),
                _,
            ) => free,
            _ => 0.0,
        },
    };
    if rtl {
        room - right - shift - width
    } else {
        left + shift
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::layout_document;
    use crate::testing::{Laid, boxes_of};
    use text::FontCollection;

    fn fonts() -> FontCollection {
        FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
    }

    /// The layout of the document `html` parses to, under the UA sheet
    /// and `css_src`, at 800 pixels wide, the body's margin none.
    fn lay(html: &str, css_src: &str) -> Laid {
        let doc = html::parse_document(html).expect("html parsing never fails");
        let sheet = css::parse_stylesheet(&format!("body {{ margin: 0 }} {css_src}"))
            .expect("test css is valid");
        let (document, boxes) = boxes_of(&doc, &[sheet]);
        let layout = layout_document(&boxes, 800.0, &fonts());
        Laid::new(document, layout)
    }

    /// The box fragment of the element with the id `id`, at its place
    /// on the page.
    fn by_id<'l>(laid: &'l Laid, id: &str) -> &'l Fragment {
        laid.iter()
            .find(|f| {
                matches!(f.kind, crate::FragmentKind::Box)
                    && laid.document.attr(f.node, "id") == Some(id)
            })
            .unwrap_or_else(|| panic!("no box #{id}"))
    }

    /// `id`'s border box: x, y, width, height.
    fn rect(laid: &Laid, id: &str) -> (f32, f32, f32, f32) {
        let f = by_id(laid, id);
        (f.x, f.y, f.w, f.h)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    /// The UA sheet's fieldset at 16px: a 2px border, 5.6px of padding
    /// at the top, 10px at the bottom and 12px each side, 2px margins
    /// each side; a legend's 2px of padding each side.
    const CSS: &str = "fieldset { width: 300px } legend { width: 80px }";

    #[test]
    fn a_legend_taller_than_the_border_takes_the_top_and_the_content_follows() {
        let laid = lay(
            "<fieldset id=f><legend id=l style='height: 40px'></legend>\
             <div id=d style='height: 20px'></div></fieldset>",
            CSS,
        );
        let f = by_id(&laid, "f");
        assert!(f.legend, "the fieldset's first child is its legend");
        assert_eq!(laid.document.attr(f.children[0].node, "id"), Some("l"));
        let (x, y, w, h) = rect(&laid, "f");
        assert_eq!((x, y, w), (2.0, 0.0, 300.0 + 24.0 + 4.0));
        assert!(close(h, 40.0 + 5.6 + 20.0 + 10.0 + 2.0), "{h}");
        assert_eq!(rect(&laid, "l"), (16.0, 0.0, 84.0, 40.0));
        // The content box starts below the legend.
        let (dx, dy, dw, _) = rect(&laid, "d");
        assert_eq!((dx, dw), (16.0, 300.0));
        assert!(close(dy, 40.0 + 5.6), "{dy}");
    }

    #[test]
    fn a_legend_shorter_than_the_border_is_centred_on_it_and_its_bottom_margin_counts() {
        let laid = lay(
            "<fieldset id=f style='border-top-width: 30px'>\
             <legend id=l style='height: 10px; margin-top: 6px'></legend>\
             <div id=d style='height: 20px'></div></fieldset>\
             <fieldset id=g><legend id=m style='height: 10px; margin: 8px 0 6px'></legend>\
             <div id=e style='height: 20px'></div></fieldset>",
            CSS,
        );
        // Centred, its top margin not counted, as in the reference.
        assert_eq!(rect(&laid, "l").1, 10.0);
        assert!(close(rect(&laid, "d").1, 30.0 + 5.6));
        // Taller than the border with its bottom margin: the area is.
        let top = rect(&laid, "g").1;
        assert_eq!(rect(&laid, "m").1, top);
        assert!(close(rect(&laid, "e").1 - top, 16.0 + 5.6));
    }

    #[test]
    fn auto_margins_and_justify_self_place_the_legend_across() {
        let laid = lay(
            "<fieldset><legend id=a style='margin-left: 20px'></legend></fieldset>\
             <fieldset><legend id=b style='margin: 0 auto'></legend></fieldset>\
             <fieldset><legend id=c style='margin-left: auto'></legend></fieldset>\
             <fieldset><legend id=d align=center></legend></fieldset>\
             <fieldset><legend id=e align=right></legend></fieldset>\
             <fieldset><legend id=f align=center style='margin-right: auto'></legend></fieldset>\
             <fieldset dir=rtl><legend id=g></legend></fieldset>\
             <fieldset dir=rtl><legend id=h align=left></legend></fieldset>             <div align=center><fieldset id=fi><legend id=i></legend></fieldset></div>             <fieldset><legend id=j style='justify-self: end'></legend></fieldset>             <fieldset><legend id=k style='text-align: center'></legend></fieldset>",
            CSS,
        );
        // The content box runs from 16 for 300; the legend is 84 wide.
        let x = |id: &str| rect(&laid, id).0;
        assert_eq!(x("a"), 36.0);
        assert_eq!(x("b"), 16.0 + 108.0);
        assert_eq!(x("c"), 16.0 + 216.0);
        assert_eq!(x("d"), 16.0 + 108.0);
        assert_eq!(x("e"), 16.0 + 216.0);
        assert_eq!(
            x("f"),
            16.0,
            "an auto end margin alone keeps it at the start"
        );
        assert_eq!(x("g"), 16.0 + 216.0, "the start is the right in rtl");
        assert_eq!(x("h"), 16.0, "left is the end in rtl");
        // Neither an inherited nor its own text-align moves it; its
        // justify-self does.
        assert_eq!(x("i") - x("fi"), 14.0, "past the border and padding");
        assert_eq!(x("j"), 16.0 + 216.0);
        assert_eq!(x("k"), 16.0);
    }

    #[test]
    fn the_rendered_legend_is_the_first_in_flow_and_the_others_are_blocks() {
        let laid = lay(
            "<fieldset id=f><legend id=a style='float: left; height: 10px'></legend>\
             <legend id=b style='position: absolute; height: 10px'></legend>\
             <div id=d style='height: 20px'></div>\
             <legend id=c style='height: 10px'></legend>\
             <legend id=e style='height: 10px'></legend></fieldset>",
            CSS,
        );
        let f = by_id(&laid, "f");
        assert!(f.legend);
        assert_eq!(laid.document.attr(f.children[0].node, "id"), Some("c"));
        // The rendered legend at the top, the next one after the div.
        let top = rect(&laid, "f").1;
        assert_eq!(rect(&laid, "c").1, top);
        let (_, dy, ..) = rect(&laid, "d");
        assert!(close(dy, top + 10.0 + 5.6));
        assert_eq!(rect(&laid, "e").1, dy + 20.0);
        // The float is in the content.
        assert_eq!(rect(&laid, "a").1, dy);
    }

    #[test]
    fn the_text_either_side_of_a_rendered_legend_is_one_run() {
        let doc = html::parse_document("<fieldset>one<legend>x</legend>two</fieldset>")
            .expect("html parsing never fails");
        let (_, boxes) = boxes_of(&doc, &[]);
        let fieldset = &boxes[0].children[0].children[0];
        let kinds: Vec<&BoxKind> = fieldset.children.iter().map(|c| &c.kind).collect();
        assert_eq!(kinds, [&BoxKind::Legend, &BoxKind::AnonymousBlock]);
        assert_eq!(
            fieldset.children[1].children.len(),
            2,
            "one run of both texts"
        );
    }

    #[test]
    fn a_legend_takes_its_area_from_the_fieldsets_own_height() {
        let laid = lay(
            "<fieldset id=a style='height: 60px'><legend style='height: 40px'></legend></fieldset>\
             <fieldset id=b style='height: 10px'><legend style='height: 40px'></legend></fieldset>\
             <fieldset id=c style='min-height: 100px'><legend style='height: 40px'></legend>\
             </fieldset>\
             <fieldset id=d style='height: 80px; box-sizing: border-box'>\
             <legend style='height: 40px'></legend><div id=e style='height: 50%'></div></fieldset>",
            CSS,
        );
        let h = |id: &str| rect(&laid, id).3;
        assert!(close(h("a"), 2.0 + 5.6 + 60.0 + 10.0 + 2.0));
        assert!(close(h("b"), 40.0 + 5.6 + 10.0 + 2.0), "no room is left");
        assert!(close(h("c"), 2.0 + 5.6 + 100.0 + 10.0 + 2.0));
        assert_eq!(h("d"), 80.0);
        // A percentage is of the room the legend leaves.
        assert!(close(h("e"), (80.0 - 40.0 - 5.6 - 10.0 - 2.0) / 2.0));
    }

    #[test]
    fn a_fieldset_is_a_context_root_whose_containing_block_starts_below_its_legend() {
        let laid = lay(
            "<fieldset id=a style='border: 0; padding: 0'>\
             <div id=b style='margin-top: 15px; height: 10px'></div></fieldset>\
             <div style='float: left; width: 100px; height: 30px'></div>\
             <fieldset id=c style='width: auto; position: relative'>\
             <legend style='height: 40px'></legend>\
             <div id=d style='position: absolute; top: 0; left: 0'></div></fieldset>",
            CSS,
        );
        // The child's margin stays inside.
        assert_eq!(rect(&laid, "a").3, 25.0);
        assert_eq!(rect(&laid, "b").1, 15.0);
        // Beside the float, its margin over it as in the reference; its
        // absolutely positioned box below the legend's area, at the
        // padding box's left.
        let (cx, cy, cw, _) = rect(&laid, "c");
        assert_eq!((cx, cw), (100.0, 800.0 - 100.0 - 2.0));
        assert_eq!(rect(&laid, "d").0, cx + 2.0);
        assert_eq!(rect(&laid, "d").1, cy + 40.0);
    }

    #[test]
    fn a_fieldset_is_at_least_as_wide_as_its_legend_and_its_content() {
        let laid = lay(
            "<div style='width: 100px'>\
             <fieldset id=a><legend style='width: 150px'></legend></fieldset>\
             <fieldset id=b><div style='width: 200px'></div></fieldset></div>\
             <fieldset id=c style='float: left'><legend style='width: 150px'></legend>\
             <div style='width: 50px'></div></fieldset>",
            "",
        );
        let w = |id: &str| rect(&laid, id).2;
        assert_eq!(w("a"), 154.0 + 24.0 + 4.0);
        assert_eq!(w("b"), 200.0 + 24.0 + 4.0);
        assert_eq!(w("c"), 154.0 + 24.0 + 4.0);
    }
}
