//! Hit testing a render tree (SVG 2 section 15.6, `pointer-events:
//! visiblePainted`, the initial value): the topmost shape whose fill, by
//! its rule, or whose stroke holds a point, inside every clip around it.

use canvas::{Canvas, Matrix};
use dom::NodeId;

use crate::tree::{Clip, Group, Item, Shape, Tree};

impl Tree {
    /// The element at `(x, y)` in the viewport's CSS pixels: a shape's
    /// (a `<use>`'s for its instance's), or `None` over nothing painted.
    pub fn hit(&self, x: f64, y: f64) -> Option<NodeId> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        // A context of no bitmap answers whether a point is in a stroke.
        let mut probe = Canvas::new(0, 0, false);
        group(&self.root, &Matrix::IDENTITY, (x, y), &mut probe)
    }
}

fn group(group_: &Group, ctm: &Matrix, at: (f64, f64), probe: &mut Canvas) -> Option<NodeId> {
    let ctm = ctm.then(&group_.transform);
    if let Some(clip) = &group_.clip {
        let (x, y) = ctm.invert()?.apply(at.0, at.1);
        let inside = match clip {
            Clip::Rect([cx, cy, w, h]) => x >= *cx && x <= cx + w && y >= *cy && y <= cy + h,
            Clip::Paths(paths) => paths.iter().any(|(path, rule)| path.contains(x, y, *rule)),
        };
        if !inside {
            return None;
        }
    }
    group_.children.iter().rev().find_map(|item| match item {
        Item::Group(g) => group(g, &ctm, at, probe),
        Item::Shape(s) => shape(s, &ctm, at, probe),
    })
}

fn shape(shape: &Shape, ctm: &Matrix, at: (f64, f64), probe: &mut Canvas) -> Option<NodeId> {
    let (x, y) = ctm.invert()?.apply(at.0, at.1);
    if let Some(fill) = &shape.fill
        && shape.path.contains(x, y, fill.rule)
    {
        return Some(shape.node);
    }
    let stroke = shape.stroke.as_ref()?;
    let state = probe.state_mut();
    state.transform = Matrix::IDENTITY;
    state.set_line_width(stroke.width);
    state.line_cap = stroke.cap;
    state.line_join = stroke.join;
    state.set_miter_limit(stroke.miter_limit);
    state.dash.set(&stroke.dashes);
    state.dash.offset = stroke.offset;
    probe
        .is_point_in_stroke(Some(&shape.path), x, y)
        .then_some(shape.node)
}
