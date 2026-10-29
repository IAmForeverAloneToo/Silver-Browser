//! Where the laid-out boxes are (D45, D65): each element's boxes in
//! viewport coordinates through the transforms and scroll offsets
//! around them, and what is under a point, from the paint order of a
//! layout; what the frame's observers and hit testing and the layout
//! queries of scripts read.

use std::collections::{HashMap, HashSet};

use dom::NodeId;
use layout::DocumentLayout;
use paint::{Rect, Step, Viewport, padding_box, paint_order};

use crate::scripts;

/// A point on an outer `<svg>`'s drawing (M32): the svg, the point in
/// its content box, and that box's size.
pub(crate) type SvgPoint = (NodeId, (f32, f32), (f32, f32));

/// Each of `targets` in `layout`, at the `viewport`'s scroll offset and
/// the scroll containers' `offsets` (D45): its border box's size, border
/// and padding, its bounds in viewport coordinates through the
/// transforms around it, each of its boxes', and the clips around it;
/// from one walk of the whole paint order, not only what is in view.
/// An element with no box is not there.
pub(crate) fn measure(
    layout: &DocumentLayout,
    viewport: Viewport,
    offsets: &HashMap<NodeId, f32>,
    targets: &HashSet<NodeId>,
) -> HashMap<NodeId, scripts::Measured> {
    let mut out: HashMap<NodeId, scripts::Measured> = HashMap::new();
    // The matrices to viewport coordinates of the groups walked
    // into, and the clips in force, each in viewport coordinates.
    let mut matrices = vec![style::transform::Matrix::IDENTITY];
    let mut clips: Vec<(Option<NodeId>, [f32; 4])> = Vec::new();
    let map = |m: &style::transform::Matrix, r: Rect| -> [f32; 4] {
        let corners = [
            (r.x, r.y),
            (r.x + r.w, r.y),
            (r.x, r.y + r.h),
            (r.x + r.w, r.y + r.h),
        ];
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (x, y) in corners {
            let [px, py, _, w] = m.apply(x, y, 0.0);
            let w = if w.abs() > f32::EPSILON { w } else { 1.0 };
            let (px, py) = (px / w, py / w);
            x0 = x0.min(px);
            y0 = y0.min(py);
            x1 = x1.max(px);
            y1 = y1.max(py);
        }
        [x0, y0, x1 - x0, y1 - y0]
    };
    // Content this far above and below the viewport is listed too:
    // all of any page.
    for step in paint::paint_order_in(&layout.roots, viewport, offsets, 1.0e7) {
        let matrix = *matrices.last().expect("the viewport's");
        match step {
            Step::PushGroup { matrix: inner, .. } => matrices.push(matrix.then(&inner)),
            Step::PopGroup => {
                if matrices.len() > 1 {
                    matrices.pop();
                }
            }
            Step::PushClip { node, rect, .. } => {
                let mut clip = map(&matrix, rect);
                if let Some((_, outer)) = clips.last() {
                    let left = clip[0].max(outer[0]);
                    let top = clip[1].max(outer[1]);
                    let right = (clip[0] + clip[2]).min(outer[0] + outer[2]);
                    let bottom = (clip[1] + clip[3]).min(outer[1] + outer[3]);
                    clip = [left, top, (right - left).max(0.0), (bottom - top).max(0.0)];
                }
                clips.push((node, clip));
            }
            Step::PopClip => {
                clips.pop();
            }
            Step::Box {
                fragment,
                rect,
                cb_width,
            } if fragment.generated.is_none() && targets.contains(&fragment.node) => {
                let bounds = map(&matrix, rect);
                match out.entry(fragment.node) {
                    // An inline element's boxes on its lines: their
                    // union.
                    std::collections::hash_map::Entry::Occupied(mut kept) => {
                        kept.get_mut().rects.push(bounds);
                        let b = &mut kept.get_mut().bounds;
                        let right = (b[0] + b[2]).max(bounds[0] + bounds[2]);
                        let bottom = (b[1] + b[3]).max(bounds[1] + bounds[3]);
                        b[0] = b[0].min(bounds[0]);
                        b[1] = b[1].min(bounds[1]);
                        b[2] = right - b[0];
                        b[3] = bottom - b[1];
                    }
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        let style = &fragment.style;
                        let padding = style.padding.clone().map(|p| p.resolve(cb_width));
                        let margin = style.margin.clone().map(|m| m.resolve(cb_width, 0.0));
                        slot.insert(scripts::Measured {
                            size: (rect.w, rect.h),
                            border: style.used_border_width(),
                            padding,
                            bounds,
                            clips: clips.clone(),
                            rects: vec![bounds],
                            margin,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// What is under the viewport point `(x, y)` of `layout` at the
/// `viewport`'s scroll offset and the scroll containers' `offsets`
/// ([`crate::Engine::hit_test`]): the node and the clipping boxes
/// around it, outermost first; the outer `<svg>` hit and the point in
/// it, for its render tree to answer; and every node under the point,
/// in paint order.
#[allow(clippy::type_complexity)]
pub(crate) fn hit(
    layout: &DocumentLayout,
    viewport: Viewport,
    offsets: &HashMap<NodeId, f32>,
    x: f32,
    y: f32,
) -> (Option<(NodeId, Vec<NodeId>)>, Option<SvgPoint>, Vec<NodeId>) {
    // The clips in force, outermost first, each with how many
    // groups deep it was met: the chain around a hit, which a
    // generated box's clip is not part of, as it does not scroll.
    let mut clips: Vec<(Option<NodeId>, Rect, usize)> = Vec::new();
    // The point in the coordinates of the groups being walked into
    // (M19), outermost first: a group's inverse matrix maps the
    // point outside it to the point inside it.
    let mut points: Vec<(f32, f32)> = vec![(x, y)];
    // How deep inside a group nothing of which is hit: its matrix
    // singular, or the point behind its eye.
    let mut passed = 0u32;
    // Whether the point is in `rect`, and inside the innermost clip
    // of every group around, each in that group's coordinates.
    let under = |rect: &Rect, clips: &[(Option<NodeId>, Rect, usize)], points: &[(f32, f32)]| {
        let (px, py) = points[points.len() - 1];
        if !rect.contains(px, py) {
            return false;
        }
        let mut checked = usize::MAX;
        for (_, clip, depth) in clips.iter().rev() {
            if *depth == checked {
                continue;
            }
            checked = *depth;
            let (cx, cy) = points[(*depth).min(points.len() - 1)];
            if !clip.contains(cx, cy) {
                return false;
            }
        }
        true
    };
    let chain = |clips: &[(Option<NodeId>, Rect, usize)]| {
        clips.iter().filter_map(|(node, _, _)| *node).collect()
    };
    let mut hit = None;
    // Every node hit, in paint order.
    let mut all = Vec::new();
    // When the hit is an outer `<svg>`'s drawing (M32): the svg, the
    // point in it and its size, for its render tree to answer.
    let mut in_svg: Option<SvgPoint> = None;
    for step in paint_order(&layout.roots, viewport, offsets) {
        if passed > 0 {
            match step {
                Step::PushGroup { .. } => passed += 1,
                Step::PopGroup => passed -= 1,
                _ => {}
            }
            continue;
        }
        match step {
            // An inert box, and what is inside it, is passed over
            // (M22.1); a hidden box, but not what is visible in it
            // (D56).
            Step::Box { fragment, .. } | Step::Text { fragment, .. }
                if fragment.style.inert
                    || fragment.style.visibility != style::computed::Visibility::Visible => {}
            Step::Box { fragment, rect, .. } => {
                if under(&rect, &clips, &points) {
                    let (px, py) = points[points.len() - 1];
                    let mut chain: Vec<NodeId> = chain(&clips);
                    if fragment.clips()
                        && fragment.generated.is_none()
                        && padding_box(fragment, rect).contains(px, py)
                    {
                        chain.push(fragment.node);
                    }
                    hit = Some((fragment.node, chain));
                    all.push(fragment.node);
                    in_svg = None;
                }
            }
            Step::Text { fragment, rect, .. } => {
                if under(&rect, &clips, &points) {
                    hit = Some((fragment.node, chain(&clips)));
                    all.push(fragment.node);
                    let (px, py) = points[points.len() - 1];
                    in_svg = match &fragment.kind {
                        layout::FragmentKind::Replaced(layout::Replaced::Svg { node, .. }) => {
                            Some((*node, (px - rect.x, py - rect.y), (rect.w, rect.h)))
                        }
                        _ => None,
                    };
                }
            }
            Step::PushClip { node, rect, .. } => clips.push((node, rect, points.len() - 1)),
            Step::PopClip => {
                clips.pop();
            }
            Step::PushGroup { matrix, .. } => {
                let (px, py) = points[points.len() - 1];
                match matrix.inverse().and_then(|inverse| inverse.project(px, py)) {
                    Some(inside) => points.push(inside),
                    None => passed = 1,
                }
            }
            Step::PopGroup => {
                if points.len() > 1 {
                    points.pop();
                }
            }
            Step::Scrollbar { .. } => {}
        }
    }
    (hit, in_svg, all)
}
