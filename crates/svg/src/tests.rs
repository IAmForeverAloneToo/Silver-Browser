//! The render tree built from parsed pages, and its pixels, against the
//! reference browser's (the design note's probes).

use canvas::{Canvas, Matrix};
use dom::Document;
use style::CascadeContext;

use crate::draw::draw_within;
use crate::tree::build_within;
use crate::{Item, MAX_SVG_ELEMENTS, MAX_SVG_LAYERS, Paint, Tree, build, draw};

/// The tree of the first `<svg>` in `markup`, for a viewport `w` by `h`.
fn tree_of(markup: &str, w: f64, h: f64) -> Tree {
    let document = Document::from_node(&html::parse_document(markup).expect("never fails"));
    let styles = style::cascade_document(&document, &[], &CascadeContext::default());
    let svg = document
        .subtree(document.root())
        .find(|&n| document.svg_tag(n) == Some("svg"))
        .expect("an svg");
    build(&document, &styles, svg, w, h)
}

/// The tree of the first `<svg>` in `markup`, at most `limit` elements
/// built.
fn tree_within(markup: &str, limit: usize) -> Tree {
    let document = Document::from_node(&html::parse_document(markup).expect("never fails"));
    let styles = style::cascade_document(&document, &[], &CascadeContext::default());
    let svg = document
        .subtree(document.root())
        .find(|&n| document.svg_tag(n) == Some("svg"))
        .expect("an svg");
    build_within(&document, &styles, svg, (10.0, 10.0), limit)
}

/// `markup`'s first `<svg>` drawn on a canvas `w` by `h`.
fn drawn(markup: &str, w: u32, h: u32) -> Canvas {
    let tree = tree_of(markup, f64::from(w), f64::from(h));
    let mut canvas = Canvas::new(w, h, false);
    draw(&tree, &mut canvas, &Matrix::IDENTITY);
    canvas
}

/// The pixel at `x`, `y`, not premultiplied, as `getImageData` reads it.
fn px(canvas: &Canvas, x: u32, y: u32) -> [u8; 4] {
    let bytes = canvas.bytes().expect("a bitmap");
    let i = ((y * canvas.width() + x) * 4) as usize;
    let [r, g, b, a] = [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]];
    if a == 0 {
        return [0, 0, 0, 0];
    }
    let un = |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)) as u8;
    [un(r), un(g), un(b), a]
}

fn near(got: [u8; 4], want: [u8; 4]) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 3)
}

fn shapes(items: &[Item]) -> usize {
    items
        .iter()
        .map(|item| match item {
            Item::Group(g) => shapes(&g.children),
            Item::Shape(_) => 1,
        })
        .sum()
}

#[test]
fn the_tree_holds_what_draws() {
    let tree = tree_of(
        "<svg width=100 height=50>\
         <rect width=10 height=10></rect>\
         <rect width=10 height=10 style='display: none'></rect>\
         <rect width=0 height=10></rect>\
         <circle r=-1></circle>\
         <g><circle r=5></circle><foo><rect width=1 height=1></rect></foo></g>\
         <g style='display: none'><rect width=1 height=1></rect></g>\
         <ellipse rx=auto ry=auto></ellipse><ellipse rx=4></ellipse>\
         <line x2=10></line><polyline points='0,0 5'></polyline><polygon points='0,0 5,5 0,5'></polygon>\
         <path d='M0 0 L10 10'></path><path d=''></path>\
         <rect width=5 height=5 fill=none></rect>\
         <title>a</title><desc>b</desc><defs><rect width=1 height=1></rect></defs>\
         </svg><rect width=1 height=1></rect>",
        100.0,
        50.0,
    );
    // The first rect, the circle in the group, the ellipse with one
    // radius, the polygon and the path; not the line, which has no
    // fill and no stroke, nor the polyline of one point.
    let kept: Vec<usize> = tree
        .root
        .children
        .iter()
        .map(|i| match i {
            Item::Group(g) => shapes(&g.children),
            Item::Shape(_) => 1,
        })
        .collect();
    assert_eq!(kept, [1, 1, 1, 1, 1], "{:?}", tree.root.children);
    let Item::Shape(first) = &tree.root.children[0] else {
        panic!("a shape");
    };
    assert_eq!(first.bounds, [0.0, 0.0, 10.0, 10.0]);
    assert_eq!(
        first.fill.as_ref().map(|f| f.paint.clone()),
        Some(Paint::Color(canvas::Rgba::BLACK))
    );
    assert!(first.stroke.is_none() && first.anti_alias);
    assert!(tree.elements < MAX_SVG_ELEMENTS);
}

#[test]
fn a_view_box_scales_the_content_to_the_viewport() {
    let canvas = drawn(
        "<svg viewBox='0 0 10 10'><rect width=5 height=5 fill=red></rect></svg>",
        20,
        20,
    );
    assert!(near(px(&canvas, 9, 9), [255, 0, 0, 255]));
    assert_eq!(px(&canvas, 11, 11), [0, 0, 0, 0]);
    // Met in the middle of a wider viewport.
    let canvas = drawn(
        "<svg viewBox='0 0 10 10'><rect width=10 height=10 fill=red></rect></svg>",
        40,
        20,
    );
    assert_eq!(px(&canvas, 5, 10), [0, 0, 0, 0]);
    assert!(near(px(&canvas, 20, 10), [255, 0, 0, 255]));
    // A viewBox of no area draws nothing.
    let canvas = drawn(
        "<svg viewBox='0 0 0 10'><rect width=10 height=10 fill=red></rect></svg>",
        20,
        20,
    );
    assert_eq!(px(&canvas, 5, 5), [0, 0, 0, 0]);
}

#[test]
fn edges_are_smoothed_as_the_reference_browsers_are() {
    // A rect edge at a half pixel is half covered.
    let canvas = drawn(
        "<svg><rect x=2.5 width=3 height=10 fill=black></rect></svg>",
        10,
        10,
    );
    let alpha: Vec<u8> = (1..7).map(|x| px(&canvas, x, 5)[3]).collect();
    assert!(alpha[0] == 0 && alpha[5] == 0, "{alpha:?}");
    assert!(
        alpha[1].abs_diff(128) <= 2 && alpha[4].abs_diff(128) <= 2,
        "{alpha:?}"
    );
    assert!(alpha[2] == 255 && alpha[3] == 255, "{alpha:?}");
    // A one-pixel line on a whole row covers two rows by half.
    let canvas = drawn(
        "<svg><line x1=0 y1=10 x2=20 y2=10 stroke=black></line></svg>",
        20,
        20,
    );
    let rows: Vec<u8> = (8..12).map(|y| px(&canvas, 5, y)[3]).collect();
    assert!(rows[0] == 0 && rows[3] == 0, "{rows:?}");
    assert!(
        rows[1].abs_diff(128) <= 2 && rows[2].abs_diff(128) <= 2,
        "{rows:?}"
    );
    // `crispEdges` leaves no pixel half covered.
    let canvas = drawn(
        "<svg><rect x=2.5 width=3 height=10 fill=black shape-rendering=crispEdges></rect></svg>",
        10,
        10,
    );
    assert!((1..7).all(|x| matches!(px(&canvas, x, 5)[3], 0 | 255)));
}

#[test]
fn a_groups_opacity_composites_the_group_and_a_shapes_its_paint() {
    // Blue over red at half: the group composited once.
    let canvas = drawn(
        "<svg><g opacity=0.5><rect width=10 height=10 fill=red></rect>\
         <rect width=5 height=10 fill=blue></rect></g></svg>",
        10,
        10,
    );
    assert!(
        near(px(&canvas, 2, 5), [0, 0, 255, 128]),
        "{:?}",
        px(&canvas, 2, 5)
    );
    assert!(
        near(px(&canvas, 7, 5), [255, 0, 0, 128]),
        "{:?}",
        px(&canvas, 7, 5)
    );
    // Each its own: they mix.
    let canvas = drawn(
        "<svg><rect width=10 height=10 fill=red opacity=0.5></rect>\
         <rect width=5 height=10 fill=blue opacity=0.5></rect></svg>",
        10,
        10,
    );
    assert!(
        near(px(&canvas, 2, 5), [85, 0, 170, 191]),
        "{:?}",
        px(&canvas, 2, 5)
    );
    // A shape with a fill and a stroke at half is one layer: the stroke
    // over the fill does not show the fill through it.
    let canvas = drawn(
        "<svg><rect x=2 y=2 width=6 height=6 fill=red stroke=blue stroke-width=2 opacity=0.5>\
         </rect></svg>",
        10,
        10,
    );
    assert!(
        near(px(&canvas, 2, 5), [0, 0, 255, 128]),
        "{:?}",
        px(&canvas, 2, 5)
    );
    assert!(
        near(px(&canvas, 5, 5), [255, 0, 0, 128]),
        "{:?}",
        px(&canvas, 5, 5)
    );
}

#[test]
fn shapes_take_their_geometry_and_paints() {
    // A circle's middle, not its corner; currentColor is the colour.
    let canvas = drawn(
        "<svg style='color: rgb(0, 128, 0)'><circle cx=10 cy=10 r=8 fill=currentColor>\
         </circle></svg>",
        20,
        20,
    );
    assert!(near(px(&canvas, 10, 10), [0, 128, 0, 255]));
    assert_eq!(px(&canvas, 1, 1), [0, 0, 0, 0]);
    // A rounded corner leaves the corner out.
    let canvas = drawn(
        "<svg><rect width=20 height=20 rx=8 fill=black></rect></svg>",
        20,
        20,
    );
    assert_eq!(px(&canvas, 0, 0)[3], 0);
    assert_eq!(px(&canvas, 10, 0)[3], 255);
    // Radii past half the size are half of it: a stadium.
    let canvas = drawn(
        "<svg><rect width=20 height=10 rx=30 fill=black></rect></svg>",
        20,
        10,
    );
    assert_eq!(px(&canvas, 0, 0)[3], 0);
    assert_eq!(px(&canvas, 10, 5)[3], 255);
    assert_eq!(px(&canvas, 10, 0)[3], 255);
    // An attribute's `em` is of the element's own font.
    let tree = tree_of(
        "<svg style='font-size: 16px'><line x2=1em stroke=black font-size=4></line></svg>",
        20.0,
        20.0,
    );
    let Item::Shape(line) = &tree.root.children[0] else {
        panic!("a shape");
    };
    assert_eq!(line.bounds, [0.0, 0.0, 4.0, 0.0]);
    // evenodd leaves the inner square out; nonzero fills it.
    let star = "M0 0 H20 V20 H0 Z M5 5 H15 V15 H5 Z";
    let canvas = drawn(
        &format!("<svg><path d='{star}' fill-rule=evenodd></path></svg>"),
        20,
        20,
    );
    assert_eq!(px(&canvas, 10, 10)[3], 0);
    assert_eq!(px(&canvas, 2, 2)[3], 255);
    let canvas = drawn(&format!("<svg><path d='{star}'></path></svg>"), 20, 20);
    assert_eq!(px(&canvas, 10, 10)[3], 255);
    // A reference to nothing paints its fallback, or nothing.
    let canvas = drawn(
        "<svg><rect width=10 height=10 fill='url(#none) blue'></rect>\
         <rect x=10 width=10 height=10 fill='url(#none)'></rect></svg>",
        20,
        10,
    );
    assert!(near(px(&canvas, 5, 5), [0, 0, 255, 255]));
    assert_eq!(px(&canvas, 15, 5), [0, 0, 0, 0]);
}

#[test]
fn strokes_take_their_width_dashes_order_and_transforms() {
    // Painted stroke first, the fill over it: the inside of the stroke
    // is the fill's.
    let canvas = drawn(
        "<svg><rect x=4 y=4 width=12 height=12 fill=red stroke=blue stroke-width=4 \
         paint-order=stroke></rect></svg>",
        20,
        20,
    );
    assert!(near(px(&canvas, 5, 10), [255, 0, 0, 255]));
    assert!(near(px(&canvas, 3, 10), [0, 0, 255, 255]));
    // Dashes of 4 and 4 along a line: on, off.
    let canvas = drawn(
        "<svg><line x1=0 y1=5 x2=20 y2=5 stroke=black stroke-width=4 stroke-dasharray=4>\
         </line></svg>",
        20,
        10,
    );
    assert_eq!(px(&canvas, 2, 5)[3], 255);
    assert_eq!(px(&canvas, 6, 5)[3], 0);
    assert_eq!(px(&canvas, 10, 5)[3], 255);
    // A transform moves the shape, a group's and its own composed.
    let canvas = drawn(
        "<svg><g transform='translate(10 0)'><rect width=5 height=5 fill=black \
         transform='scale(2)'></rect></g></svg>",
        20,
        20,
    );
    assert_eq!(px(&canvas, 5, 5)[3], 0);
    assert_eq!(px(&canvas, 19, 9)[3], 255);
    // A stroke that does not scale: 2 wide however the content is.
    let canvas = drawn(
        "<svg viewBox='0 0 10 10'><line x1=0 y1=5 x2=10 y2=5 stroke=black stroke-width=2 \
         vector-effect=non-scaling-stroke></line></svg>",
        40,
        40,
    );
    assert_eq!(px(&canvas, 20, 20)[3], 255);
    assert_eq!(px(&canvas, 20, 18)[3], 0);
    assert_eq!(px(&canvas, 20, 22)[3], 0);
}

#[test]
fn the_bounds_hold_what_a_hostile_drawing_asks() {
    // Past its elements, a tree draws what it built: the svg, the group
    // and the first rect are three.
    let markup = "<svg><g><rect width=1 height=1></rect><rect width=2 height=2></rect></g>\
                  <rect width=3 height=3></rect></svg>";
    let tree = tree_within(markup, 3);
    assert_eq!(tree.elements, 3);
    let Item::Group(group) = &tree.root.children[0] else {
        panic!("a group");
    };
    assert_eq!((tree.root.children.len(), group.children.len()), (1, 1));
    assert_eq!(tree_within(markup, MAX_SVG_ELEMENTS).elements, 5);
    // Groups nested past the layers' depth take their opacity to each
    // child: red and blue at half mix there, where a layer shows blue.
    let pair = "<g opacity=0.5><rect width=10 height=10 fill=red></rect>\
                <rect width=10 height=10 fill=blue></rect></g>";
    let nested = |depth: usize| {
        format!(
            "<svg>{}{pair}{}</svg>",
            "<g opacity=0.999>".repeat(depth),
            "</g>".repeat(depth)
        )
    };
    let blue = |canvas: &Canvas| {
        let [r, _, b, _] = px(canvas, 5, 5);
        r < 10 && b > 245
    };
    assert!(blue(&drawn(&nested(MAX_SVG_LAYERS - 1), 10, 10)));
    assert!(!blue(&drawn(&nested(MAX_SVG_LAYERS), 10, 10)));
    // So do they past the layers' pixels: one layer of 100 fits a budget
    // of 100, a second does not.
    let tree = tree_of(&format!("<svg>{pair}{pair}</svg>"), 10.0, 10.0);
    let mut canvas = Canvas::new(10, 10, false);
    draw_within(&tree, &mut canvas, &Matrix::IDENTITY, 100);
    let [r, _, b, _] = px(&canvas, 5, 5);
    assert!(r > 50 && b > 50, "the second pair mixed: {r} {b}");
    let mut canvas = Canvas::new(10, 10, false);
    draw_within(&tree, &mut canvas, &Matrix::IDENTITY, 200);
    assert!(blue(&canvas));
}
