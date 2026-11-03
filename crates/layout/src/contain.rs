//! Size containment in layout (CSS Containment 2 section 3.1, CSS
//! Sizing 4 section 5, gap batch H): a box with size containment is
//! sized as if it were empty, or as `contain-intrinsic-size` says, its
//! content laid out inside it all the same; one with inline-size
//! containment so in its width alone. A box whose contents are skipped
//! (`content-visibility`) has size containment, and its last remembered
//! size stands for an `auto` intrinsic size, which the box tree puts in
//! its style from what the engine remembers.

use style::computed::ComputedStyle;

/// The width a box of `style` takes for its content when it has
/// inline-size containment: its `contain-intrinsic-width`, or none.
/// `None` for a box without it, whose content's widths count.
pub(crate) fn content_width(style: &ComputedStyle) -> Option<f32> {
    style
        .contains_inline_size()
        .then(|| style.containment().intrinsic[0].used(None).unwrap_or(0.0))
}

/// The height a box of `style` takes for its content when it has size
/// containment: its `contain-intrinsic-height`, or none. `None` for a
/// box without it, whose content's height counts.
pub(crate) fn content_height(style: &ComputedStyle) -> Option<f32> {
    style
        .contains_size()
        .then(|| style.containment().intrinsic[1].used(None).unwrap_or(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::boxes_of;
    use dom::NodeId;
    use style::computed::{Contain, ContainIntrinsicSize, Display};
    use text::FontCollection;

    /// The border box of each element with an `id` of `html` under `css`
    /// laid out 800 wide: `(id, x, y, w, h)`, in tree order.
    fn laid(html: &str, css: &str) -> Vec<(String, f32, f32, f32, f32)> {
        let fonts =
            FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font");
        let doc = html::parse_document(html).expect("never fails");
        let sheet = css::parse_stylesheet(&format!("body {{ margin: 0 }} {css}")).expect("valid");
        let (document, boxes) = boxes_of(&doc, &[sheet]);
        let layout = crate::layout_document(&boxes, 800.0, &fonts);
        let mut seen: Vec<NodeId> = Vec::new();
        let mut out = Vec::new();
        for placed in layout.placed() {
            let f = placed.fragment;
            if !matches!(f.kind, crate::FragmentKind::Box) || f.generated.is_some() {
                continue;
            }
            let Some(id) = document.attr(f.node, "id") else {
                continue;
            };
            if seen.contains(&f.node) {
                continue;
            }
            seen.push(f.node);
            out.push((id.to_owned(), placed.x, placed.top(), f.w, f.h));
        }
        out
    }

    fn of(laid: &[(String, f32, f32, f32, f32)], id: &str) -> (f32, f32, f32, f32) {
        let (_, x, y, w, h) = laid
            .iter()
            .find(|(i, ..)| i == id)
            .unwrap_or_else(|| panic!("#{id}"));
        (*x, *y, *w, *h)
    }

    #[test]
    fn a_textareas_rows_are_lines_of_its_line_height() {
        // Its rows of its own lines, not of the default font's (gap batch
        // H): 3 of 30px, no padding or border.
        let laid = laid(
            "<textarea id=t rows=3></textarea><textarea id=u></textarea>",
            "textarea { line-height: 30px; padding: 0; border: 0 } #u { line-height: 12px }",
        );
        assert_eq!(of(&laid, "t").3, 90.0);
        assert_eq!(of(&laid, "u").3, 24.0);
    }

    #[test]
    fn size_containment_sizes_a_box_as_if_empty_its_content_laid_out_in_it() {
        let laid = laid(
            "<div id=a><div id=in style='height: 100px'></div></div><div id=after></div>\
             <div id=b><div style='height: 100px'></div></div>\
             <span id=c style='display: inline-block'><span style='display: inline-block; \
             width: 300px; height: 10px'></span></span>\
             <div id=f style='display: flex'><div style='height: 80px; width: 10px'></div></div>\
             <div id=g style='display: grid'><div style='height: 70px'></div></div>\
             <div id=h style='content-visibility: hidden; contain-intrinsic-size: auto 33px'>\
             <div style='height: 100px'></div></div>",
            "#a { contain: size } #b { contain: size; contain-intrinsic-size: 20px 40px }
             #c { contain: inline-size } #f, #g { contain: strict; contain-intrinsic-height: 15px }",
        );
        assert_eq!(of(&laid, "a").3, 0.0);
        // Its content is laid out inside, overflowing it.
        assert_eq!(of(&laid, "in").3, 100.0);
        assert_eq!(of(&laid, "after").1, 0.0);
        // `contain-intrinsic-size` gives the height; the width is the
        // containing block's still.
        assert_eq!((of(&laid, "b").2, of(&laid, "b").3), (800.0, 40.0));
        // Inline-size containment: as narrow as empty, as tall as its
        // content.
        assert_eq!((of(&laid, "c").2, of(&laid, "c").3 > 0.0), (0.0, true));
        assert_eq!(of(&laid, "f").3, 15.0);
        assert_eq!(of(&laid, "g").3, 15.0);
        // Skipped contents: size containment, the length of an `auto`
        // with nothing remembered.
        assert_eq!(of(&laid, "h").3, 33.0);
    }

    #[test]
    fn layout_containment_holds_its_positioned_boxes() {
        let laid = laid(
            "<div style='height: 50px'></div>\
             <div id=c style='contain: layout; margin-left: 30px; height: 20px'>\
             <div id=abs style='position: absolute; left: 5px; top: 5px; width: 1px; height: 1px'>\
             </div><div id=fixed style='position: fixed; left: 7px; top: 7px; width: 1px; \
             height: 1px'></div></div>",
            "",
        );
        assert_eq!((of(&laid, "abs").0, of(&laid, "abs").1), (35.0, 55.0));
        assert_eq!((of(&laid, "fixed").0, of(&laid, "fixed").1), (37.0, 57.0));
    }

    #[test]
    fn a_contained_box_takes_its_intrinsic_size_or_none() {
        let mut style = ComputedStyle {
            display: Display::Block,
            ..ComputedStyle::default()
        };
        assert_eq!(content_width(&style), None);
        assert_eq!(content_height(&style), None);
        style.containment_mut().contain = Contain::SIZE;
        assert_eq!(content_width(&style), Some(0.0));
        assert_eq!(content_height(&style), Some(0.0));
        style.containment_mut().intrinsic = [
            ContainIntrinsicSize::Length(30.0),
            ContainIntrinsicSize::Auto(Some(40.0)),
        ];
        assert_eq!(content_width(&style), Some(30.0));
        assert_eq!(content_height(&style), Some(40.0));
        // Inline-size containment leaves the height to the content.
        style.containment_mut().contain = Contain::INLINE_SIZE;
        assert_eq!(content_width(&style), Some(30.0));
        assert_eq!(content_height(&style), None);
        // A box whose contents are skipped has size containment.
        style.containment_mut().contain = Contain::NONE;
        style.set_skips_contents(true);
        assert_eq!(content_height(&style), Some(40.0));
        // An inline box takes none.
        style.display = Display::Inline;
        assert_eq!(content_height(&style), None);
    }
}
