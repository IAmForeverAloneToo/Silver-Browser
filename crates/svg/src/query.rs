//! What scripts ask of an SVG element's geometry (SVG 2 sections 5.11,
//! 9.6 and 11.11): its bounding box, its matrix to its outer `<svg>`'s
//! viewport, its text's length, its path's length and points, whether a
//! point is in its fill or its stroke. Answered from a render tree built
//! for the asking, which keeps the shapes that paint nothing, at the
//! outer `<svg>`'s size as its style gives it (a percentage of what
//! layout decides is 300 by 150 here, until scripts can ask layout,
//! G46).

use canvas::{Canvas, Matrix};
use dom::NodeId;
use style::computed::Size;

use crate::syntax::Viewport;
use crate::tree::{
    Context, Item, Tree, build_queries, children_bounds, geometry, item_bounds, use_offset,
};

/// A question about an element's geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Query {
    /// `getBBox()`: `[x, y, width, height]` in its user units, of its
    /// geometry alone (no stroke, no clip).
    BBox,
    /// `getCTM()`: `[a, b, c, d, e, f]` from its user units to its outer
    /// `<svg>`'s viewport.
    Ctm,
    /// `getComputedTextLength()`: its characters' advance.
    TextLength,
    /// `getTotalLength()`.
    TotalLength,
    /// `getPointAtLength()`: the point so far along.
    PointAtLength(f64),
    /// `isPointInFill()`: 1 or 0.
    InFill(f64, f64),
    /// `isPointInStroke()`: 1 or 0.
    InStroke(f64, f64),
}

/// The outer `<svg>` the element `node` is drawn in: itself or its
/// nearest ancestor that is an `<svg>` whose parent is no SVG element.
pub fn outer_svg(context: Context<'_>, node: NodeId) -> Option<NodeId> {
    let document = context.document;
    let mut at = Some(node);
    while let Some(n) = at {
        let parent = document.parent(n);
        if document.svg_tag(n) == Some("svg")
            && parent.is_none_or(|p| document.svg_tag(p).is_none())
        {
            return Some(n);
        }
        document.svg_tag(n)?;
        at = parent;
    }
    None
}

/// The size the outer `<svg>` `svg` is asked at: its style's, else 300
/// by 150.
fn asked_size(context: Context<'_>, svg: NodeId) -> Option<(f64, f64)> {
    let style = context.styles.get(svg)?;
    let side = |size: &Size, default: f64| match size {
        Size::Px(px) => f64::from(*px),
        _ => default,
    };
    Some((side(&style.width, 300.0), side(&style.height, 150.0)))
}

/// The tree the questions about the elements drawn in the outer `<svg>`
/// `svg` are answered from (a caller asking many keeps it while nothing
/// changes): `None` for one with no style.
pub fn query_tree(context: Context<'_>, svg: NodeId) -> Option<Tree> {
    let (width, height) = asked_size(context, svg)?;
    Some(build_queries(context, svg, width, height))
}

/// The answer to `query` about `node`: `None` when it is drawn in no
/// outer `<svg>`, or draws nothing that answers (an element under
/// `display: none`, a path of no point).
pub fn query(context: Context<'_>, node: NodeId, query: Query) -> Option<Vec<f64>> {
    let tree = query_tree(context, outer_svg(context, node)?)?;
    query_in(context, &tree, node, query)
}

/// [`query`] answered from `tree`, the [`query_tree`] of `node`'s outer
/// `<svg>`.
pub fn query_in(context: Context<'_>, tree: &Tree, node: NodeId, query: Query) -> Option<Vec<f64>> {
    let svg = tree.root.node?;
    let (width, height) = (tree.width, tree.height);
    if node == svg {
        return match query {
            Query::BBox => {
                Some(item_bounds(&Item::Group(tree.root.clone())).map_or(vec![0.0; 4], Vec::from))
            }
            Query::Ctm => Some(vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            _ => None,
        };
    }
    let Some((item, parent)) = find(&tree.root.children, &tree.root.transform, node) else {
        // Drawn nowhere (under `display: none`, say): a shape answers
        // with its geometry still, as the reference browser's does.
        let style = context.styles.get(node)?;
        let viewport = Viewport {
            width,
            height,
            font_size: f64::from(style.font_size_px),
            root_font_size: 16.0,
        };
        let tag = context.document.svg_tag(node)?;
        let (path, bounds) = geometry(context.document, node, tag, style, &viewport)?;
        return match query {
            Query::BBox => Some(bounds.to_vec()),
            Query::TotalLength => Some(vec![path.length()]),
            Query::PointAtLength(distance) => {
                path.point_at_length(distance).map(|(x, y)| vec![x, y])
            }
            _ => None,
        };
    };
    match (query, item) {
        (Query::BBox, Item::Shape(shape)) => Some(shape.bounds.to_vec()),
        (Query::BBox, Item::Group(group)) => {
            let mut bounds = children_bounds(&group.children);
            // A use's box is where its `x` and `y` put its instance, as
            // the reference browser has it, though they move it after
            // its transform.
            if context.document.svg_tag(node) == Some("use")
                && let (Some(style), Some([x, y, w, h])) = (context.styles.get(node), bounds)
            {
                let viewport = Viewport {
                    width,
                    height,
                    font_size: f64::from(style.font_size_px),
                    root_font_size: 16.0,
                };
                let (dx, dy) = use_offset(style, &viewport);
                bounds = Some([x + dx, y + dy, w, h]);
            }
            Some(bounds.map_or(vec![0.0; 4], Vec::from))
        }
        (Query::Ctm, Item::Shape(_)) => Some(matrix(&parent)),
        (Query::Ctm, Item::Group(group)) => Some(matrix(&parent.then(&group.transform))),
        (Query::TextLength, Item::Group(group)) => Some(vec![advances(&group.children, None)]),
        (Query::TextLength, Item::Shape(_)) => {
            Some(vec![advances(&tree.root.children, Some(node))])
        }
        (Query::TotalLength, Item::Shape(shape)) => Some(vec![shape.path.length()]),
        (Query::PointAtLength(distance), Item::Shape(shape)) => {
            let (x, y) = shape.path.point_at_length(distance)?;
            Some(vec![x, y])
        }
        (Query::InFill(x, y), Item::Shape(shape)) => {
            let inside = shape
                .fill
                .as_ref()
                .is_some_and(|fill| shape.path.contains(x, y, fill.rule));
            Some(vec![f64::from(u8::from(inside))])
        }
        (Query::InStroke(x, y), Item::Shape(shape)) => {
            let Some(stroke) = &shape.stroke else {
                return Some(vec![0.0]);
            };
            let mut probe = Canvas::new(0, 0, false);
            let state = probe.state_mut();
            state.set_line_width(stroke.width);
            state.line_cap = stroke.cap;
            state.line_join = stroke.join;
            state.set_miter_limit(stroke.miter_limit);
            state.dash.set(&stroke.dashes);
            state.dash.offset = stroke.offset;
            let inside = probe.is_point_in_stroke(Some(&shape.path), x, y);
            Some(vec![f64::from(u8::from(inside))])
        }
        _ => None,
    }
}

fn matrix(m: &Matrix) -> Vec<f64> {
    vec![m.a, m.b, m.c, m.d, m.e, m.f]
}

/// The item `node` draws as, and the matrix from its parent's user units
/// to the viewport's; the first in tree order.
fn find<'t>(items: &'t [Item], ctm: &Matrix, node: NodeId) -> Option<(&'t Item, Matrix)> {
    for item in items {
        match item {
            Item::Shape(shape) if shape.node == node => return Some((item, *ctm)),
            Item::Group(group) if group.node == Some(node) => return Some((item, *ctm)),
            Item::Group(group) => {
                if let Some(found) = find(&group.children, &ctm.then(&group.transform), node) {
                    return Some(found);
                }
            }
            Item::Shape(_) => {}
        }
    }
    None
}

/// The advance of the text runs in `items`, those of `node` alone when
/// it is given.
fn advances(items: &[Item], node: Option<NodeId>) -> f64 {
    items
        .iter()
        .map(|item| match item {
            Item::Group(group) => advances(&group.children, node),
            Item::Shape(shape) if node.is_none_or(|n| n == shape.node) => shape.bounds[2],
            Item::Shape(_) => 0.0,
        })
        .sum()
}
