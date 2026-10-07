//! A list item's marker laid out (Lists 3 §3, M21.6): the fragments a
//! marker is, its text or the shape it is drawn as, and where an outside
//! marker goes, which CSS leaves undefined and this engine takes from
//! the reference browser (docs/design/values.md).
//!
//! An outside marker ends at the list item's border-box start edge, on
//! the left for `ltr` and the right for `rtl`, whatever the list item's
//! padding, its content's margins or `text-align`, moved along by the
//! room floats take on its line from the content box's start edge. On
//! the list item's first line it is laid out as wide as nothing, and the
//! list item moves it there once its content is laid out
//! ([`place_on_line`]); without one it is beside the first baseline of
//! the content, or on a line of its own at the top. A marker's text is
//! drawn in the order its direction's bidi gives it, `1. ` in a
//! right-to-left list as ` .1`, as the reference browser draws it.
//!
//! The shapes, with `A` the marker's font's ascent rounded: a square of
//! side `⌊(⌊2A/3⌋ + 1) / 2⌋`, its top `⌊3(A − ⌊2A/3⌋) / 2⌋` below the
//! top of the ascent; outside, its left edge `⌊2A/3⌋ + 7` before the list
//! item's border edge, and inside, the content following it the side,
//! an em and a pixel after its start. These are the reference browser's
//! numbers, measured in DejaVu Sans at 16 and 32 pixels.

use dom::NodeId;
use style::ComputedStyle;
use style::computed::Direction;
use style::selector_matching::PseudoElement;
use text::{FontCollection, FontMetrics};

use crate::block::{Fragment, FragmentKind};
use crate::box_tree::{BoxKind, LayoutBox};

/// A [`BoxKind::Marker`] box as a line takes it.
pub(crate) struct Marker<'a> {
    pub(crate) b: &'a LayoutBox,
    /// Its style where it is: on a first line, its style there.
    pub(crate) style: &'a ComputedStyle,
    /// Its font's, which the line it is on is at least as tall as.
    pub(crate) metrics: FontMetrics,
    /// The width of what it draws.
    pub(crate) width: f32,
    /// How far it moves the line on: nothing for an outside marker, the
    /// shape and the room after it for an inside one.
    pub(crate) advance: f32,
}

impl<'a> Marker<'a> {
    /// The marker box `b`, styled `style` where it is.
    pub(crate) fn of(b: &'a LayoutBox, style: &'a ComputedStyle, fonts: &FontCollection) -> Self {
        let BoxKind::Marker { outside, shape } = b.kind else {
            unreachable!("a marker box");
        };
        let text = text_of(b);
        let metrics = fonts.metrics_for_text(text.unwrap_or(" "), style.font_size_px);
        let width = match shape {
            Some(_) => Shape::of(&metrics).side,
            None => text.map_or(0.0, |text| measure(text, style, fonts)),
        };
        let advance = match (outside, shape) {
            (true, _) => 0.0,
            (false, Some(_)) => width + style.font_size_px + 1.0,
            (false, None) => width,
        };
        Marker {
            b,
            style,
            metrics,
            width,
            advance,
        }
    }

    /// Whether the marker is outside the list item, as wide as nothing
    /// on its line.
    pub(crate) fn is_outside(&self) -> bool {
        matches!(self.b.kind, BoxKind::Marker { outside: true, .. })
    }

    /// The marker's fragments, starting at `x` on a line whose baseline
    /// is at `baseline` and whose text rises `ascent` above it and falls
    /// `descent` below: its text, a character apart by `letter-spacing`
    /// when that is not zero, or its shape.
    pub(crate) fn fragments(
        &self,
        x: f32,
        baseline: f32,
        ascent: f32,
        descent: f32,
        fonts: &FontCollection,
    ) -> Vec<Fragment> {
        let BoxKind::Marker { shape, .. } = self.b.kind else {
            unreachable!("a marker box");
        };
        let fragment = |x: f32, y: f32, w: f32, h: f32, kind: FragmentKind, ascent: f32| Fragment {
            x,
            y,
            w,
            h,
            style: self.style.clone(),
            kind,
            node: self.b.node,
            generated: Some(PseudoElement::Marker),
            ascent,
            scroll_height: 0.0,
            children: Vec::new(),
            tree_order: self.b.tree_order,
            stacking_parent: None,
            parent: Some(self.b.node),
            in_3d: false,
            decorated: None,
        };
        if let Some(shape) = shape {
            let geometry = Shape::of(&self.metrics);
            let top = baseline - self.metrics.ascent.round() + geometry.top;
            let side = geometry.side;
            return vec![fragment(
                x,
                top,
                side,
                side,
                FragmentKind::Marker(shape),
                0.0,
            )];
        }
        let Some(text) = text_of(self.b) else {
            return Vec::new();
        };
        let direction = crate::inline::to_text_direction(self.style.direction);
        let text = &text::bidi_reorder(text, Some(direction));
        let text_fragment = |x: f32, text: &str, w: f32| {
            let kind = FragmentKind::Text(text.to_string());
            fragment(x, baseline, w, ascent + descent, kind, ascent)
        };
        if self.style.letter_spacing == 0.0 {
            return vec![text_fragment(x, text, self.width)];
        }
        let mut out = Vec::new();
        let mut at = x;
        let mut buf = [0u8; 4];
        for c in text.chars() {
            let c = c.encode_utf8(&mut buf);
            let w = fonts.measure(c, self.style.font_size_px);
            out.push(text_fragment(at, c, w));
            at += w + self.style.letter_spacing;
        }
        out
    }
}

/// The text of a marker box drawn as text.
fn text_of(b: &LayoutBox) -> Option<&str> {
    match &b.children.first()?.kind {
        BoxKind::Text(text) => Some(text),
        _ => None,
    }
}

/// `text`'s width in `style`, `letter-spacing` after each character.
fn measure(text: &str, style: &ComputedStyle, fonts: &FontCollection) -> f32 {
    if style.letter_spacing == 0.0 {
        return fonts.measure(text, style.font_size_px);
    }
    let mut buf = [0u8; 4];
    text.chars()
        .map(|c| fonts.measure(c.encode_utf8(&mut buf), style.font_size_px) + style.letter_spacing)
        .sum()
}

/// A drawn marker's geometry, from its font's ascent.
pub(crate) struct Shape {
    /// The side of the square it is drawn in.
    pub(crate) side: f32,
    /// Its top below the top of the ascent.
    pub(crate) top: f32,
    /// Outside, how far before the list item's border edge it starts.
    pub(crate) before: f32,
}

impl Shape {
    pub(crate) fn of(metrics: &FontMetrics) -> Shape {
        let ascent = metrics.ascent.round();
        let two_thirds = (2.0 * ascent / 3.0).floor();
        Shape {
            side: ((two_thirds + 1.0) / 2.0).floor(),
            top: (3.0 * (ascent - two_thirds) / 2.0).floor(),
            before: two_thirds + 7.0,
        }
    }
}

/// Where an outside marker of `list_item`, a list item whose border box
/// is `border_box_width` wide, starts, from its border box's left edge:
/// a text `width` wide ends at the start edge, and a shape starts its
/// own distance before it, mirrored for `rtl`.
pub(crate) fn outside_left(
    list_item: &ComputedStyle,
    border_box_width: f32,
    width: f32,
    shape: Option<&Shape>,
) -> f32 {
    let rtl = list_item.direction == Direction::Rtl;
    match (shape, rtl) {
        (None, false) => -width,
        (None, true) => border_box_width,
        (Some(shape), false) => -shape.before,
        (Some(shape), true) => border_box_width + shape.before - shape.side,
    }
}

/// Moves the outside marker of the list item `node`, styled
/// `list_item` and `border_box_width` wide, to its place, `shift` along
/// from where [`outside_left`] puts it (where floats push the start of
/// its line), when it is on a line among `children`, the fragments in
/// the list item's border box at `(0, 0)`: the fragments of the marker,
/// which lie together, are moved as one. Whether it was found. The walk
/// stops at the first.
pub(crate) fn place_on_line(
    children: &mut [Fragment],
    node: NodeId,
    list_item: &ComputedStyle,
    border_box_width: f32,
    shift: f32,
    fonts: &FontCollection,
) -> bool {
    let place = Place {
        node,
        list_item,
        border_box_width,
        shift,
        fonts,
    };
    place.within(children, 0.0)
}

/// What [`place_on_line`] moves a marker by.
struct Place<'a> {
    node: NodeId,
    list_item: &'a ComputedStyle,
    border_box_width: f32,
    shift: f32,
    fonts: &'a FontCollection,
}

impl Place<'_> {
    /// Moves the marker if it is among `children`, which are `left` from
    /// the list item's border box, or among theirs.
    fn within(&self, children: &mut [Fragment], left: f32) -> bool {
        let is_marker =
            |f: &Fragment| f.generated == Some(PseudoElement::Marker) && f.node == self.node;
        if let Some(first) = children.iter().position(is_marker) {
            let count = children[first..]
                .iter()
                .take_while(|f| is_marker(f))
                .count();
            let group = &mut children[first..first + count];
            let start = group.iter().map(|f| f.x).fold(f32::INFINITY, f32::min);
            let end = group
                .iter()
                .map(|f| f.x + f.w)
                .fold(f32::NEG_INFINITY, f32::max);
            let shape = match group[0].kind {
                FragmentKind::Marker(_) => Some(Shape::of(
                    &self
                        .fonts
                        .metrics_for_text(" ", group[0].style.font_size_px),
                )),
                _ => None,
            };
            let to = outside_left(
                self.list_item,
                self.border_box_width,
                end - start,
                shape.as_ref(),
            ) + self.shift;
            let dx = to - (left + start);
            for f in group {
                f.x += dx;
            }
            return true;
        }
        children.iter_mut().any(|f| {
            let x = left + f.x;
            self.within(&mut f.children, x)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::layout_document;
    use crate::box_tree::MarkerShape;
    use crate::testing::{Laid, boxes_of};

    fn fonts() -> FontCollection {
        FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
    }

    /// The layout of the document `html` parses to, under `css_src`, at
    /// 800 pixels wide, the body's margin none.
    fn lay(html: &str, css_src: &str) -> Laid {
        let doc = html::parse_document(html).expect("html parsing never fails");
        let sheet = css::parse_stylesheet(&format!("body {{ margin: 0 }} {css_src}"))
            .expect("test css is valid");
        let (document, boxes) = boxes_of(&doc, &[sheet]);
        let layout = layout_document(&boxes, 800.0, &fonts());
        Laid { document, layout }
    }

    /// The box fragment of the element with the id `id`.
    fn by_id<'l>(laid: &'l Laid, id: &str) -> &'l Fragment {
        laid.iter()
            .find(|f| {
                matches!(f.kind, FragmentKind::Box) && laid.document.attr(f.node, "id") == Some(id)
            })
            .unwrap_or_else(|| panic!("no box #{id}"))
    }

    /// The marker fragments of the element with the id `id`.
    fn marker_of<'l>(laid: &'l Laid, id: &str) -> Vec<&'l Fragment> {
        laid.iter()
            .filter(|f| {
                f.generated == Some(PseudoElement::Marker)
                    && laid.document.attr(f.node, "id") == Some(id)
            })
            .collect()
    }

    /// The text fragment saying `text`.
    fn text<'l>(laid: &'l Laid, text: &str) -> &'l Fragment {
        laid.iter()
            .find(|f| matches!(&f.kind, FragmentKind::Text(t) if t == text))
            .unwrap_or_else(|| panic!("no text {text:?}"))
    }

    #[test]
    fn an_outside_marker_ends_at_the_list_items_border_box_start() {
        // Whatever the list item's padding and border, or its content's
        // margins: the reference browser's place.
        let laid = lay(
            "<ol><li id=a>A</li><li id=b>B</li><li id=c>C</li><li id=d><p>D</p></li></ol>",
            "ol { padding-left: 100px; margin: 0 }
             #b { padding-left: 30px; border-left: 10px solid }
             #c { margin-left: 30px } p { margin: 0 0 0 30px }",
        );
        for (id, number) in [("a", "1. "), ("b", "2. "), ("c", "3. "), ("d", "4. ")] {
            let li = by_id(&laid, id);
            let marker = marker_of(&laid, id);
            assert_eq!(marker.len(), 1, "#{id}");
            assert!(
                matches!(&marker[0].kind, FragmentKind::Text(t) if t == number),
                "#{id}"
            );
            assert_eq!(marker[0].x + marker[0].w, li.x, "#{id}");
            // On the first line, its baseline the line's.
            let content = text(&laid, &id.to_uppercase());
            assert_eq!(marker[0].y, content.y, "#{id}");
        }
        // Right to left, it starts at the right edge.
        let laid = lay(
            "<ol dir=rtl><li id=a>A</li></ol>",
            "ol { padding: 0 100px 0 0; margin: 0 }",
        );
        let li = by_id(&laid, "a");
        let marker = marker_of(&laid, "a");
        assert_eq!(marker[0].x, li.x + li.w);
        // Its text in the order the right-to-left bidi gives it.
        assert!(matches!(&marker[0].kind, FragmentKind::Text(t) if t == " .1"));
    }

    #[test]
    fn an_outside_marker_goes_where_floats_push_its_lines_start() {
        // By the room the floats take on its line from the list item's
        // content box, whatever box the line is in; an empty list item's
        // marker too, on a line of its own; none for a float that does
        // not reach the content box (the reference browser,
        // CSS2/lists/list-style-applies-to-017).
        let laid = lay(
            "<div><i></i><ol><li id=a>A</li></ol></div>\
             <div><i></i><ol><li id=b style='padding-left: 20px'>B</li></ol></div>\
             <div><i></i><ol><li id=c><p>C</p></li></ol></div>\
             <div><i></i><ol><li id=d></li></ol></div>\
             <div><i style='width: 100px'></i><ol><li id=e>E</li></ol></div>",
            "div { overflow: hidden } ol { padding-left: 200px; margin: 0 }
             p { margin: 0 0 0 80px } i { float: left; width: 250px; height: 30px }",
        );
        for (id, end) in [
            ("a", 250.0),
            ("b", 230.0),
            ("c", 250.0),
            ("d", 250.0),
            ("e", 200.0),
        ] {
            let marker = marker_of(&laid, id);
            assert_eq!(marker[0].x + marker[0].w, end, "#{id}");
        }
    }

    #[test]
    fn a_disc_circle_and_square_are_drawn_where_the_reference_browser_draws_them() {
        let fonts = fonts();
        let laid = lay(
            "<ul><li id=a>A</li></ul><ul class=c><li id=b>B</li></ul>\
             <ul class=s><li id=c>C</li></ul><ul class=i><li id=d>D</li></ul>\
             <ul class=big><li id=e>E</li></ul>",
            "ul { margin: 0 } .c { list-style-type: circle } .s { list-style-type: square }
             .i { list-style-position: inside } .big { font-size: 32px }",
        );
        let shape = |id: &str| {
            let marker = marker_of(&laid, id);
            assert_eq!(marker.len(), 1, "#{id}");
            let FragmentKind::Marker(shape) = marker[0].kind else {
                panic!("#{id}'s marker is no shape");
            };
            (shape, marker[0].x, marker[0].y, marker[0].w, marker[0].h)
        };
        let geometry = |size: f32| Shape::of(&fonts.metrics_for_text(" ", size));
        let small = geometry(16.0);
        for (id, kind) in [
            ("a", MarkerShape::Disc),
            ("b", MarkerShape::Circle),
            ("c", MarkerShape::Square),
        ] {
            let li = by_id(&laid, id);
            let ascent = fonts.metrics_for_text(" ", 16.0).ascent.round();
            let baseline = text(&laid, &id.to_uppercase()).y;
            assert_eq!(
                shape(id),
                (
                    kind,
                    li.x - small.before,
                    baseline - ascent + small.top,
                    small.side,
                    small.side
                ),
                "#{id}"
            );
        }
        // Inside, at the content's start, the content after it.
        let li = by_id(&laid, "d");
        assert_eq!(shape("d").1, li.x);
        assert_eq!(text(&laid, "D").x, li.x + small.side + 16.0 + 1.0);
        // Twice the size, twice the square.
        let big = geometry(32.0);
        assert_eq!(big.side, 2.0 * small.side);
        assert_eq!(shape("e").1, by_id(&laid, "e").x - big.before);
    }

    #[test]
    fn a_marker_takes_part_in_its_lines_height() {
        let plain = lay("<ol><li id=a>A</li></ol>", "ol { margin: 0 }");
        let big = lay(
            "<ol><li id=a>A</li></ol>",
            "ol { margin: 0 } li::marker { font-size: 40px }",
        );
        let (plain, big) = (by_id(&plain, "a"), by_id(&big, "a"));
        let metrics = fonts().metrics_for_text("1. ", 40.0);
        assert!(big.h > plain.h);
        assert_eq!(big.h, metrics.ascent + metrics.descent + metrics.line_gap);
    }

    #[test]
    fn without_a_first_line_the_marker_is_beside_the_first_baseline_or_at_the_top() {
        let fonts = fonts();
        // An empty list item is a line tall, its marker on that line.
        let laid = lay(
            "<ol><li id=a></li><li id=b><div style='display: flex'>F</div></li></ol>",
            "ol { margin: 0 }",
        );
        let a = by_id(&laid, "a");
        let m = fonts.metrics_for_text(" ", 16.0);
        assert_eq!(a.h, m.ascent + m.descent + m.line_gap);
        let marker = marker_of(&laid, "a");
        assert_eq!(marker[0].y, a.y + m.ascent);
        assert_eq!(marker[0].x + marker[0].w, a.x);
        // Beside a flex container, on its first baseline.
        let marker = marker_of(&laid, "b");
        assert_eq!(marker[0].y, text(&laid, "F").y);
        assert_eq!(marker[0].x + marker[0].w, by_id(&laid, "b").x);
    }

    #[test]
    fn a_marker_is_in_its_list_items_box_which_clips_it() {
        let laid = lay(
            "<ul><li id=a>A</li></ul>",
            "ul { margin: 0 } li { overflow: hidden }",
        );
        let li = laid
            .roots
            .iter()
            .flat_map(|root| crate::preorder(std::slice::from_ref(root)))
            .find(|f| {
                matches!(f.kind, FragmentKind::Box) && laid.document.attr(f.node, "id") == Some("a")
            })
            .expect("the list item's box");
        assert!(li.clips());
        assert!(
            li.children
                .iter()
                .any(|f| matches!(f.kind, FragmentKind::Marker(MarkerShape::Disc)))
        );
    }
}
