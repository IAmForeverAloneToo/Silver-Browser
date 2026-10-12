//! The render tree built from parsed pages, and its pixels, against the
//! reference browser's (the design note's probes).

use canvas::{Canvas, Matrix};
use dom::Document;
use style::{CascadeContext, Stylesheets};

use crate::draw::draw_within;
use crate::tree::build_within;
use crate::{Item, MAX_SVG_ELEMENTS, MAX_SVG_LAYERS, Paint, Tree, build, draw};

/// The tree of the first `<svg>` in `markup`, for a viewport `w` by `h`.
fn tree_of(markup: &str, w: f64, h: f64) -> Tree {
    let document = Document::from_node(&html::parse_document(markup).expect("never fails"));
    let sheets = Stylesheets::new(Vec::new());
    let context = CascadeContext::default();
    let styles = sheets.cascade(&document, &context);
    let recascade = sheets.recascader(&document, &styles, &context);
    let svg = document
        .subtree(document.root())
        .find(|&n| document.svg_tag(n) == Some("svg"))
        .expect("an svg");
    build(&document, &styles, Some(&recascade), svg, w, h)
}

/// The tree of the first `<svg>` in `markup`, at most `limit` elements
/// built.
fn tree_within(markup: &str, limit: usize, max_depth: usize) -> Tree {
    let document = Document::from_node(&html::parse_document(markup).expect("never fails"));
    let styles = style::cascade_document(&document, &[], &CascadeContext::default());
    let svg = document
        .subtree(document.root())
        .find(|&n| document.svg_tag(n) == Some("svg"))
        .expect("an svg");
    build_within(
        &document,
        &styles,
        None,
        svg,
        (10.0, 10.0),
        (limit, max_depth),
    )
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
    let tree = tree_within(markup, 3, dom::MAX_DEPTH);
    assert_eq!(tree.elements, 3);
    let Item::Group(group) = &tree.root.children[0] else {
        panic!("a group");
    };
    assert_eq!((tree.root.children.len(), group.children.len()), (1, 1));
    assert_eq!(
        tree_within(markup, MAX_SVG_ELEMENTS, dom::MAX_DEPTH).elements,
        5
    );
    // Past the depth, what is deeper is left out: three groups open,
    // the rect in them is not built.
    let deep = "<svg><g><g><g><rect width=1 height=1></rect></g></g></g></svg>";
    assert_eq!(tree_within(deep, MAX_SVG_ELEMENTS, 3).elements, 4);
    assert_eq!(tree_within(deep, MAX_SVG_ELEMENTS, 4).elements, 5);
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

#[test]
fn gradients_interpolate_unpremultiplied_and_spread() {
    // The reference browser's middle of opaque red to transparent blue.
    let canvas = drawn(
        "<svg><linearGradient id=g><stop offset=0 stop-color=red></stop>\
         <stop offset=1 stop-color=blue stop-opacity=0></stop></linearGradient>\
         <rect width=100 height=10 fill='url(#g)'></rect></svg>",
        100,
        10,
    );
    let middle = px(&canvas, 50, 5);
    assert_eq!(middle, [128, 0, 130, 126]);
    assert!(
        near(px(&canvas, 0, 5), [255, 0, 0, 255]),
        "{:?}",
        px(&canvas, 0, 5)
    );
    // Past its end at 60%: padded, repeated, reflected.
    let spread = |method: &str| {
        let canvas = drawn(
            &format!(
                "<svg><linearGradient id=g x2=0.5 spreadMethod={method}>\
                 <stop stop-color=red></stop><stop offset=1 stop-color=blue></stop>\
                 </linearGradient><rect width=100 height=10 fill='url(#g)'></rect></svg>"
            ),
            100,
            10,
        );
        px(&canvas, 60, 5)
    };
    assert!(near(spread("pad"), [0, 0, 255, 255]), "{:?}", spread("pad"));
    let repeat = spread("repeat");
    assert!(repeat[0] > 180 && repeat[2] < 75, "{repeat:?}");
    let reflect = spread("reflect");
    assert!(
        reflect[0] < 75 && reflect[0] > 30 && reflect[2] > 180,
        "{reflect:?}"
    );
    // Offsets out of order are each the one before's: blue from 0.8 on,
    // red before it.
    let canvas = drawn(
        "<svg><linearGradient id=g><stop offset=0.8 stop-color=red></stop>\
         <stop offset=0.2 stop-color=blue></stop></linearGradient>\
         <linearGradient id=d x2=0><stop stop-color=red></stop>\
         <stop offset=1 stop-color=lime></stop></linearGradient>\
         <rect width=100 height=10 fill='url(#g)'></rect>\
         <rect y=10 width=100 height=10 fill='url(#d)'></rect></svg>",
        100,
        20,
    );
    assert!(
        near(px(&canvas, 50, 5), [255, 0, 0, 255]),
        "{:?}",
        px(&canvas, 50, 5)
    );
    assert!(
        near(px(&canvas, 90, 5), [0, 0, 255, 255]),
        "{:?}",
        px(&canvas, 90, 5)
    );
    // A gradient of no length paints its last stop.
    assert!(
        near(px(&canvas, 50, 15), [0, 255, 0, 255]),
        "{:?}",
        px(&canvas, 50, 15)
    );
}

#[test]
fn a_gradient_takes_its_units_transform_and_templates() {
    let stops = "<stop stop-color=red></stop><stop offset=1 stop-color=blue></stop>";
    // The bounding box's units: the rect's own left is red.
    let canvas = drawn(
        &format!(
            "<svg><linearGradient id=g>{stops}</linearGradient>\
             <linearGradient id=u gradientUnits=userSpaceOnUse x2=100>{stops}</linearGradient>\
             <rect x=50 width=50 height=10 fill='url(#g)'></rect>\
             <rect x=50 y=10 width=50 height=10 fill='url(#u)'></rect></svg>"
        ),
        100,
        20,
    );
    assert!(px(&canvas, 51, 5)[0] > 240, "{:?}", px(&canvas, 51, 5));
    let user = px(&canvas, 51, 15);
    assert!(user[0] < 140 && user[2] > 115, "{user:?}");
    // Turned a quarter: red at the top, blue at the bottom.
    let canvas = drawn(
        &format!(
            "<svg><linearGradient id=g gradientTransform='rotate(90 .5 .5)'>{stops}\
             </linearGradient><rect width=20 height=20 fill='url(#g)'></rect></svg>"
        ),
        20,
        20,
    );
    assert!(px(&canvas, 10, 1)[0] > 220 && px(&canvas, 10, 18)[2] > 220);
    // A template's stops and spread, the gradient's own end.
    let canvas = drawn(
        &format!(
            "<svg><linearGradient id=a spreadMethod=repeat x2=2>{stops}</linearGradient>\
             <linearGradient id=b href='#a' x2=0.5></linearGradient>\
             <rect width=100 height=10 fill='url(#b)'></rect></svg>"
        ),
        100,
        10,
    );
    let repeated = px(&canvas, 60, 5);
    assert!(repeated[0] > 180 && repeated[2] < 75, "{repeated:?}");
    // One stop paints its colour, none paints nothing; a box of no
    // height none either.
    let canvas = drawn(
        &format!(
            "<svg><linearGradient id=one><stop stop-color=lime></stop></linearGradient>\
             <linearGradient id=none></linearGradient>\
             <rect width=10 height=10 fill='url(#one)'></rect>\
             <rect x=10 width=10 height=10 fill='url(#none) red'></rect>\
             <linearGradient id=two>{stops}</linearGradient>\
             <line x1=0 y1=15 x2=20 y2=15 stroke='url(#two)' stroke-width=4></line></svg>"
        ),
        20,
        20,
    );
    assert!(near(px(&canvas, 5, 5), [0, 255, 0, 255]));
    assert_eq!(px(&canvas, 15, 5), [0, 0, 0, 0]);
    assert_eq!(px(&canvas, 5, 15), [0, 0, 0, 0]);
}

#[test]
fn a_radial_gradient_goes_from_its_focus_to_its_circle() {
    let canvas = drawn(
        "<svg><radialGradient id=g><stop stop-color=red></stop>\
         <stop offset=1 stop-color=blue></stop></radialGradient>\
         <radialGradient id=z r=0><stop stop-color=red></stop>\
         <stop offset=1 stop-color=lime></stop></radialGradient>\
         <rect width=40 height=40 fill='url(#g)'></rect>\
         <rect x=40 width=40 height=40 fill='url(#z)'></rect></svg>",
        80,
        40,
    );
    assert!(px(&canvas, 20, 20)[0] > 240, "{:?}", px(&canvas, 20, 20));
    assert!(
        near(px(&canvas, 1, 1), [0, 0, 255, 255]),
        "{:?}",
        px(&canvas, 1, 1)
    );
    assert!(near(px(&canvas, 60, 20), [0, 255, 0, 255]));
}

#[test]
fn a_use_instantiates_its_reference_inheriting_from_itself() {
    let markup = "<svg><defs><rect id=r width=10 height=10></rect>\
                  <symbol id=s viewBox='0 0 10 10'><rect width=10 height=10></rect></symbol>\
                  </defs><use href='#r' x=10 fill=red></use>\
                  <use xlink:href='#s' y=10 width=20 height=20 fill=blue></use></svg>";
    let canvas = drawn(markup, 40, 40);
    assert_eq!(px(&canvas, 5, 5), [0, 0, 0, 0], "defs draw nothing");
    assert!(
        near(px(&canvas, 15, 5), [255, 0, 0, 255]),
        "{:?}",
        px(&canvas, 15, 5)
    );
    assert!(
        near(px(&canvas, 19, 29), [0, 0, 255, 255]),
        "{:?}",
        px(&canvas, 19, 29)
    );
    assert_eq!(px(&canvas, 21, 29), [0, 0, 0, 0]);
    // What a point on an instance hits is its use.
    let document = Document::from_node(&html::parse_document(markup).expect("never fails"));
    let uses: Vec<_> = document
        .subtree(document.root())
        .filter(|&n| document.svg_tag(n) == Some("use"))
        .collect();
    let tree = tree_of(markup, 40.0, 40.0);
    let mut nodes = Vec::new();
    collect_nodes(&tree.root.children, &mut nodes);
    assert_eq!(nodes, uses);
    // Within uses within uses, the outermost.
    let markup = "<svg><defs><rect id=r width=10 height=10></rect>\
                  <g id=w><use href='#r'></use></g></defs><use href='#w'></use></svg>";
    let document = Document::from_node(&html::parse_document(markup).expect("never fails"));
    let outer = document
        .subtree(document.root())
        .filter(|&n| document.svg_tag(n) == Some("use"))
        .last();
    let mut nodes = Vec::new();
    collect_nodes(&tree_of(markup, 10.0, 10.0).root.children, &mut nodes);
    assert_eq!(nodes.first().copied(), outer);
    // A reference to itself, to an ancestor, or round a loop draws
    // nothing, and ends.
    let canvas = drawn(
        "<svg><g id=a><use href='#a' x=5></use><rect width=5 height=5></rect></g>\
         <use id=u1 href='#u2'></use><use id=u2 href='#u1'></use></svg>",
        10,
        10,
    );
    assert_eq!(px(&canvas, 2, 2)[3], 255);
    assert_eq!(px(&canvas, 7, 2)[3], 0, "no instance of its ancestor");
}

fn collect_nodes(items: &[Item], out: &mut Vec<dom::NodeId>) {
    for item in items {
        match item {
            Item::Group(g) => collect_nodes(&g.children, out),
            Item::Shape(s) => out.push(s.node),
        }
    }
}

#[test]
fn uses_nest_no_deeper_than_the_bound() {
    // Each group uses the one before; the last use instantiates
    // `depth` groups in all.
    let nested = |depth: usize| {
        let mut markup = String::from("<svg><defs><g id=g0><rect width=5 height=5></rect></g>");
        for i in 1..depth {
            markup.push_str(&format!("<g id=g{i}><use href='#g{}'></use></g>", i - 1));
        }
        markup.push_str(&format!("</defs><use href='#g{}'></use></svg>", depth - 1));
        drawn(&markup, 10, 10)
    };
    assert_eq!(px(&nested(crate::MAX_USE_DEPTH), 2, 2)[3], 255);
    assert_eq!(px(&nested(crate::MAX_USE_DEPTH + 1), 2, 2)[3], 0);
}

#[test]
fn a_nested_svg_is_a_viewport_clipped_unless_its_overflow_shows() {
    let canvas = drawn(
        "<svg><svg x=10 y=10 width=10 height=10><rect width=100 height=100 fill=red></rect>\
         </svg><svg x=30 width=5 height=5 overflow=visible viewBox='0 0 1 1'>\
         <rect width=2 height=2 fill=blue></rect></svg></svg>",
        40,
        40,
    );
    assert!(near(px(&canvas, 15, 15), [255, 0, 0, 255]));
    assert_eq!(px(&canvas, 25, 15), [0, 0, 0, 0]);
    assert_eq!(px(&canvas, 5, 5), [0, 0, 0, 0]);
    // Its viewBox scales by 5, its content shows past it.
    assert!(
        near(px(&canvas, 38, 8), [0, 0, 255, 255]),
        "{:?}",
        px(&canvas, 38, 8)
    );
}

#[test]
fn a_clip_path_clips_to_its_childrens_union() {
    let canvas = drawn(
        "<svg><clipPath id=c><rect width=10 height=10></rect><rect x=20 width=10 height=10>\
         </rect></clipPath><clipPath id=o clipPathUnits=objectBoundingBox>\
         <rect width=.5 height=1></rect></clipPath><clipPath id=e></clipPath>\
         <rect width=40 height=10 clip-path='url(#c)'></rect>\
         <rect y=10 width=40 height=10 clip-path='url(#o)'></rect>\
         <rect y=20 width=40 height=10 clip-path='url(#e)'></rect>\
         <rect y=30 width=40 height=10 clip-path='url(#nothing)'></rect>\
         <line x1=0 y1=45 x2=40 y2=45 stroke=black stroke-width=4 clip-path='url(#o)'></line>\
         <svg y=50 width=20 height=10><rect width=40 height=10 clip-path='url(#c)'></rect>\
         </svg></svg>",
        40,
        60,
    );
    let row = |y| -> Vec<u8> {
        [5, 15, 25, 35]
            .iter()
            .map(|&x| px(&canvas, x, y)[3])
            .collect()
    };
    assert_eq!(row(5), [255, 0, 255, 0], "the union");
    assert_eq!(row(15), [255, 255, 0, 0], "the bounding box's left half");
    assert_eq!(row(25), [0, 0, 0, 0], "an empty clipPath clips all");
    assert_eq!(
        row(35),
        [255, 255, 255, 255],
        "a reference to nothing clips nothing"
    );
    assert_eq!(row(45), [0, 0, 0, 0], "a box of no area clips all");
    assert_eq!(row(55), [255, 0, 0, 0], "within the viewport's clip");
}

#[test]
fn conditions_choose_what_draws() {
    let canvas = drawn(
        "<svg><switch><rect width=10 height=10 fill=red systemLanguage=fr></rect>\
         <rect width=10 height=10 fill=red requiredExtensions=x></rect>\
         <rect width=10 height=10 fill=blue systemLanguage='fr, en-GB'></rect>\
         <rect width=10 height=10 fill=red></rect></switch>\
         <rect x=10 width=10 height=10 systemLanguage=de></rect>\
         <rect x=20 width=10 height=10 systemLanguage=en></rect></svg>",
        30,
        10,
    );
    assert!(
        near(px(&canvas, 5, 5), [0, 0, 255, 255]),
        "{:?}",
        px(&canvas, 5, 5)
    );
    assert_eq!(px(&canvas, 15, 5), [0, 0, 0, 0]);
    assert_eq!(px(&canvas, 25, 5)[3], 255);
}
