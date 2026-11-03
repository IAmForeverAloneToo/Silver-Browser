//! Ruby layout (CSS Ruby 1, gap batch H, D77): a ruby container is an
//! inline box of columns (`box_tree`'s ruby module), and each column is
//! one atomic piece of its line, laid out here: its base and its
//! annotation each on one line of their own, the annotation over the
//! base, or under it by `ruby-position`, the column as wide as the
//! wider of the two. The column's baseline is its base's, so that the
//! base sits on the line as the text around it does, and the line grows
//! to hold the annotation as it grows for any taller piece.
//!
//! The narrower of the two is placed across the column's width by its
//! `ruby-align`: at the start for `start`, and centred for `center` and
//! the two values that share the space out (`space-around`, the initial
//! one, and `space-between`), which for the one box it is share it on
//! both sides.

use style::computed::{Display, RubyAlign, RubyPosition, TOP};

use crate::block::{
    AdjoiningMargins, BlockLayout, Edges, box_fragment, carried, layout_inline_block,
    settle_absolutes,
};
use crate::box_tree::{BoxKind, LayoutBox};
use crate::pass::Pass;

/// Whether `b` is a ruby column, which this module lays out.
pub(crate) fn is_column(b: &LayoutBox) -> bool {
    b.anonymous && b.kind == BoxKind::InlineBlock && b.style.display == Display::Ruby
}

/// The column's `(min-content, max-content)` content widths: both the
/// wider of its base's and its annotation's, which do not wrap.
pub(crate) fn content_widths<'a>(b: &'a LayoutBox, pass: &Pass<'a>) -> (f32, f32) {
    let width = column_width(b, pass);
    (width, width)
}

/// The width of the wider of the column's base and annotation, each at
/// its max-content width.
fn column_width<'a>(b: &'a LayoutBox, pass: &Pass<'a>) -> f32 {
    b.children
        .iter()
        .map(|child| crate::intrinsic::contribution(child, pass).1)
        .fold(0.0, f32::max)
}

/// Lays out the column `b` (see the module doc), its content box
/// `content_width` wide, as [`crate::block::layout_contents`] does a
/// block container's.
pub(crate) fn layout_column_contents<'a>(
    b: &'a LayoutBox,
    edges: &Edges,
    cb_width: f32,
    content_width: f32,
    margin_left: f32,
    pass: &Pass<'a>,
) -> BlockLayout<'a> {
    let width = column_width(b, pass).max(content_width);
    let mut laid: Vec<_> = b
        .children
        .iter()
        .map(|child| layout_inline_block(child, width, None, pass))
        .collect();
    let under = b.style.text().ruby_position == RubyPosition::Under;
    // The base first, then the annotation, top to bottom: the other way
    // round when the annotation is over it.
    let order: Vec<usize> = if under || laid.len() < 2 {
        (0..laid.len()).collect()
    } else {
        vec![1, 0]
    };
    let content_x = edges.border[style::computed::LEFT] + edges.padding[style::computed::LEFT];
    let content_y = edges.border[TOP] + edges.padding[TOP];
    let mut y = 0.0f32;
    let mut places = vec![(0.0f32, 0.0f32); laid.len()];
    for &i in &order {
        let child = &laid[i];
        let spare = (width - child.width).max(0.0);
        let x = match b.children[i].style.text().ruby_align {
            RubyAlign::Start => 0.0,
            RubyAlign::Center | RubyAlign::SpaceAround | RubyAlign::SpaceBetween => spare / 2.0,
        };
        places[i] = (x, y);
        y += child.height;
    }
    let content_height = y;
    let height = content_height + edges.vertical();
    let mut root = box_fragment(width + edges.horizontal(), height, b);
    let mut absolutes = Vec::new();
    let mut baseline = None;
    for (i, child) in laid.iter_mut().enumerate() {
        let (x, y) = places[i];
        let fx = content_x + x + child.margin_left + child.relative.0;
        let fy = content_y + y + child.margin_top + child.relative.1;
        if i == 0 {
            baseline = Some(content_y + y + child.ascent);
        }
        absolutes.extend(carried(
            std::mem::take(&mut child.absolutes),
            content_x + x + child.margin_left,
            content_y + y + child.margin_top,
        ));
        root.children.push(child.root.clone().translated(fx, fy));
    }
    let absolutes = settle_absolutes(b, &mut root, &edges.border, absolutes, pass);
    finished(
        b,
        root,
        height,
        cb_width,
        margin_left,
        baseline,
        content_height,
        absolutes,
    )
}

/// The [`BlockLayout`] of an atomic box laid out by its own rules, a
/// ruby column's or a formula's: its fragment `root`, its border-box
/// `height`, its margins its own, and `baseline` from its border-box
/// top as every baseline it has.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finished<'a>(
    b: &'a LayoutBox,
    root: crate::Fragment,
    height: f32,
    cb_width: f32,
    margin_left: f32,
    baseline: Option<f32>,
    content_height: f32,
    absolutes: Vec<crate::block::Deferred<'a>>,
) -> BlockLayout<'a> {
    let own_top = AdjoiningMargins::of(b.style.margin[TOP].resolve(cb_width, 0.0));
    let own_bottom =
        AdjoiningMargins::of(b.style.margin[style::computed::BOTTOM].resolve(cb_width, 0.0));
    BlockLayout {
        root,
        height,
        margin_left,
        margin_top: own_top,
        margin_bottom: own_bottom,
        top_edge: own_top,
        bottom_edge: own_bottom,
        self_collapsing: false,
        first_baseline: baseline,
        first_line: None,
        last_baseline: baseline,
        inline_block_baseline: baseline,
        absolutes,
        auto_content_height: content_height,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::block::{Fragment, FragmentKind, layout_document};
    use crate::testing::{Laid, boxes_of};
    use text::FontCollection;

    /// The document `html` parses to, styled under `css` with the body's
    /// margin taken away, laid out against an 800px viewport.
    pub(crate) fn laid(html: &str, css: &str) -> Laid {
        laid_in(text::TEST_FONT, html, css)
    }

    /// [`laid`] in the font `data`.
    pub(crate) fn laid_in(data: &[u8], html: &str, css: &str) -> Laid {
        let fonts = FontCollection::from_bytes(data.to_vec(), 0).expect("the font parses");
        let doc = html::parse_document(html).expect("html parsing never fails");
        let sheet = css::parse_stylesheet(&format!("body {{ margin: 0 }} {css}"))
            .expect("test css is valid");
        let (document, boxes) = boxes_of(&doc, &[sheet]);
        let layout = layout_document(&boxes, 800.0, &fonts);
        Laid::new(document, layout)
    }

    /// The text fragment of `text`.
    pub(crate) fn text<'l>(laid: &'l Laid, text: &str) -> &'l Fragment {
        laid.iter()
            .find(|f| matches!(&f.kind, FragmentKind::Text(t) if t == text))
            .unwrap_or_else(|| panic!("no text {text:?}"))
    }

    /// The box fragment of the first element named `tag`.
    pub(crate) fn element<'l>(laid: &'l Laid, tag: &str) -> &'l Fragment {
        laid.iter()
            .find(|f| {
                matches!(f.kind, FragmentKind::Box) && laid.document.local_name(f.node) == Some(tag)
            })
            .unwrap_or_else(|| panic!("no box of a {tag}"))
    }

    fn centre(f: &Fragment) -> f32 {
        f.x + f.w / 2.0
    }

    #[test]
    fn an_annotation_sits_centred_over_its_base_and_the_line_grows_to_hold_it() {
        let css = "p { font-size: 20px; margin: 0 }";
        let plain = laid("<p>base</p>", css);
        let ruby = laid("<p><ruby>base<rt>note</rt></ruby></p>", css);
        let (base, note) = (text(&ruby, "base"), text(&ruby, "note"));
        assert_eq!(note.style.font_size_px, 10.0, "the UA sheet's 50%");
        assert!(note.top() + note.h <= base.top() + 0.01, "over it");
        assert!((centre(note) - centre(base)).abs() < 0.5, "centred");
        assert!(element(&ruby, "p").h >= element(&plain, "p").h + note.h - 0.01);
        assert!(base.y > text(&plain, "base").y, "the base sits lower");
    }

    #[test]
    fn ruby_position_under_puts_the_annotation_below() {
        let l = laid(
            "<p><ruby>base<rt>note</rt></ruby></p>",
            "p { margin: 0 } ruby { ruby-position: under }",
        );
        let (base, note) = (text(&l, "base"), text(&l, "note"));
        assert!(note.top() >= base.top() + base.h - 0.01);
    }

    #[test]
    fn ruby_align_start_puts_the_narrower_at_the_start() {
        let l = laid(
            "<p><ruby>a wide base<rt>n</rt></ruby></p>",
            "p { margin: 0 } ruby { ruby-align: start }",
        );
        assert_eq!(text(&l, "n").x, text(&l, "a").x);
    }

    #[test]
    fn bases_take_their_annotations_in_order() {
        let l = laid(
            "<p><ruby>aaaa<rt>x</rt>bbbb<rt>y</rt><rp>(</rp></ruby></p>",
            "p { margin: 0 }",
        );
        let (a, b) = (text(&l, "aaaa"), text(&l, "bbbb"));
        assert!((centre(text(&l, "x")) - centre(a)).abs() < 0.5);
        assert!((centre(text(&l, "y")) - centre(b)).abs() < 0.5);
        assert!(a.x + a.w <= b.x + 0.01);
        assert!(
            !l.iter()
                .any(|f| matches!(&f.kind, FragmentKind::Text(t) if t == "(")),
            "rp shows nothing"
        );
        // Bases and then annotations: the first base the first one.
        let l = laid(
            "<p><ruby><rb>aaaa</rb><rb>bbbb</rb><rt>x</rt><rt>y</rt></ruby></p>",
            "p { margin: 0 }",
        );
        let (a, b) = (text(&l, "aaaa"), text(&l, "bbbb"));
        assert!((centre(text(&l, "x")) - centre(a)).abs() < 0.5);
        assert!((centre(text(&l, "y")) - centre(b)).abs() < 0.5);
    }

    #[test]
    fn an_annotation_outside_a_ruby_is_in_one_of_its_own() {
        let l = laid("<p>text <rt>note</rt></p>", "p { margin: 0 }");
        let (base, note) = (text(&l, "text"), text(&l, "note"));
        assert!(note.top() + note.h <= base.top() + base.h);
        assert!(note.x >= base.x + base.w);
    }
}
