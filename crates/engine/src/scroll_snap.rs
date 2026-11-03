//! Scroll snapping (CSS Scroll Snap 1, and what of Scroll Snap 2 the
//! reference browser ships), which the engine's frames and a script's
//! scrolls both run, and which `scroll-state(snapped)` (D72) and the snap
//! events read.
//!
//! **Containers and areas.** A scroll container whose `scroll-snap-type`
//! is not `none` (the viewport's is the root element's) rests on a snap
//! position in each of its axes the type names. Its snap areas are the
//! border boxes, as transformed, of the elements inside it whose
//! `scroll-snap-align` is not `none`, grown by their `scroll-margin`, in
//! tree order of the flat tree; an area inside a scroll container inside
//! it is that one's. Each aligns as its `scroll-snap-align`
//! says with the snapport, the scrollport less its `scroll-padding`
//! (sections 4 and 6), its inline `start` the right side right to left.
//!
//! **Selection.** After a scroll, each axis goes to the snap position
//! nearest where the scroll went (section 6.2), one past where it was in
//! the scroll's direction for a relative scroll where there is one;
//! `mandatory` always snaps, `proximity` only within [`PROXIMITY`] of the
//! snapport's size. An area counts only where it would show in the
//! snapport across the other axis, and where the two axes' areas would
//! not show together, the axis nearer its goal keeps its position and the
//! other is chosen again among the areas that show there, as the
//! reference browser does. A relative scroll does not pass an area whose
//! `scroll-snap-stop` is `always` (section 6.4). An area larger than the
//! snapport may rest anywhere it covers it (section 6.2.3), but not where
//! another area straddles the snapport's edge: it moves past the nearer
//! such edge, as the reference browser moves it. `scroll-snap-type: pair`
//! (Scroll Snap 2, as the reference browser takes it) snaps both axes to
//! one area aligned in both, the nearest, or for a relative scroll the
//! one most in its direction.
//!
//! **The area snapped to.** Of the areas aligned at the position chosen,
//! one is the snap target, as the reference browser picks it: the
//! targeted element's, or one holding it; else the focused element's, or
//! one holding it; else the innermost, of the areas inside one another;
//! else one aligned in both axes; else the first in tree order.
//!
//! **Re-snapping.** After a layout that moves the areas, a scroll
//! container that was not scrolled since goes back to the areas it was
//! snapped to (section 5.2) where they still align and show; an axis
//! whose area is gone is snapped afresh where the other puts it.

use std::collections::HashMap;

use dom::NodeId;
use layout::{DocumentLayout, Fragment, FragmentKind};
use style::ComputedStyle;
use style::computed::{Direction, Position, SnapAlign, SnapAxis, SnapStrictness};
use style::selector_matching::PseudoElement;
use style::transform::Matrix;

use crate::scroll_state::Scroller;

/// How near a snap position a `proximity` scroll container's offset must
/// come to snap, as a fraction of its snapport's size in that axis.
const PROXIMITY: f32 = 1.0 / 3.0;

/// How far apart, in px, two offsets may be and still be one position.
const SAME: f32 = 0.5;

/// What each snapping scroll container rests on: the snap areas it is
/// snapped to, across and down, and the offset it was left at.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Snap {
    pub(crate) areas: Targets,
    pub(crate) at: (f32, f32),
}

/// The snap areas a container is snapped to, across and down.
pub(crate) type Targets = [Option<SnapTarget>; 2];

/// A snap area as a container is snapped to it: its element, and the
/// pseudo-element (`::before` or `::after`) whose box it is, if it is
/// one's. The snap events name the element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SnapTarget {
    pub(crate) node: NodeId,
    pub(crate) pseudo: Option<PseudoElement>,
}

/// Each snapping scroll container's [`Snap`].
pub(crate) type Snaps = HashMap<Scroller, Snap>;

/// What a selection prefers among the areas aligned at one position: the
/// element a scroll into view or the document's fragment targets, and
/// the element focused, each with its ancestors in the flat tree.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Prefer<'a> {
    pub(crate) target: &'a [NodeId],
    pub(crate) focus: &'a [NodeId],
}

/// `node` and its ancestors in the flat tree: what a [`Prefer`] holds of
/// a target or a focus.
pub(crate) fn chain(document: &dom::Document, node: Option<NodeId>) -> Vec<NodeId> {
    std::iter::successors(node, |&n| document.flat_parent_element(n)).collect()
}

/// A snap area.
#[derive(Debug, Clone, PartialEq)]
struct Area {
    /// Its element: a `::before`'s or `::after`'s is its element's.
    node: NodeId,
    /// The pseudo-element whose box it is, if it is one's.
    pseudo: Option<PseudoElement>,
    /// The offsets, across and down, at which its start edge and its end
    /// edge meet the snapport's start edge.
    span: [(f32, f32); 2],
    /// The offsets, across and down, that align it as its
    /// `scroll-snap-align` says, where it aligns in that axis.
    align: [Option<f32>; 2],
    /// Whether its `scroll-snap-stop` is `always`.
    stop: bool,
    /// The nearest area whose element holds its own, by its index.
    parent: Option<usize>,
}

/// A snapping scroll container as a layout has it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SnapContainer {
    pub(crate) scroller: Scroller,
    /// Whether its `scroll-behavior` is `smooth` (the root's for the
    /// viewport): what a snap a layout or a style asks for moves it by.
    pub(crate) smooth: bool,
    /// Whether it snaps across and down.
    axes: [bool; 2],
    /// Whether it snaps both axes together (`pair`).
    pair: bool,
    mandatory: bool,
    /// Its snapport's size, across and down.
    port: [f32; 2],
    /// The least and the most it scrolls, across and down: across below
    /// 0 right to left, where the scroll origin is the right edge.
    min: [f32; 2],
    max: [f32; 2],
    /// Its snap areas, in tree order.
    areas: Vec<Area>,
}

/// An offset a selection may take in an axis: the area's, by its index,
/// and the offset.
type Candidate = (usize, f32);

/// What a selection finds in an axis: the offset, and the areas aligned
/// there, by their indexes.
type Found = (f32, Vec<usize>);

/// What a selection in one axis keeps to in the other, where it keeps to
/// anything: the offset there, at which an area must show to count, and
/// the areas aligned there, of which one must show at the area's offset.
type Cross<'a> = Option<(f32, &'a [usize])>;

/// The snapping scroll containers of `layout` for a viewport of
/// `viewport`'s size: the viewport's, by the root element's
/// `scroll-snap-type`, then every scroll container's whose type is not
/// `none`.
pub(crate) fn snap_containers(layout: &DocumentLayout, viewport: (f32, f32)) -> Vec<SnapContainer> {
    let mut out = Vec::new();
    let Some(root) = layout
        .roots
        .first()
        .filter(|f| matches!(f.kind, FragmentKind::Box))
    else {
        return out;
    };
    if root.style.root && root.style.effects().scroll_snap_type.axis != SnapAxis::None {
        // The viewport's snapport is the viewport less the root's
        // `scroll-padding`; the content is the document's.
        let padding = scroll_padding(&root.style, viewport);
        let port = [
            (viewport.0 - padding[1] - padding[3]).max(0.0),
            (viewport.1 - padding[0] - padding[2]).max(0.0),
        ];
        let mut walk = Walk::new(&root.style, padding, port);
        for f in layout
            .roots
            .iter()
            .filter(|f| f.style.position != Position::Fixed)
        {
            // The root element's content is the viewport's, its own snap
            // type the viewport's.
            if f.style.root {
                for c in &f.children {
                    walk.collect(c, Place::at(f.x, f.y), f.w);
                }
            } else {
                walk.collect(f, Place::at(0.0, 0.0), viewport.0);
            }
        }
        let (max_x, max_y) = crate::scroll_state::viewport_max(layout, viewport);
        out.push(container(
            None,
            &root.style,
            port,
            ([0.0, 0.0], [max_x, max_y]),
            walk.areas,
        ));
    }
    for f in layout.iter() {
        if matches!(f.kind, FragmentKind::Box)
            && f.generated.is_none()
            && !f.style.root
            && f.clips()
            && f.style.overflow().scrolls()
            && f.style.effects().scroll_snap_type.axis != SnapAxis::None
        {
            let [top, right, bottom, left] = f.style.used_border_width();
            let padding_box = ((f.w - left - right).max(0.0), (f.h - top - bottom).max(0.0));
            let padding = scroll_padding(&f.style, padding_box);
            let port = [
                (padding_box.0 - padding[1] - padding[3]).max(0.0),
                (padding_box.1 - padding[0] - padding[2]).max(0.0),
            ];
            let mut walk = Walk::new(&f.style, padding, port);
            for c in &f.children {
                walk.collect(c, Place::at(-left, -top), f.w);
            }
            let (min_x, max_x) = crate::geometry::scroll_range_x(f);
            out.push(container(
                Some(f.node),
                &f.style,
                port,
                ([min_x, 0.0], [max_x, f.max_scroll()]),
                walk.areas,
            ));
        }
    }
    out
}

/// Where each scroll container of `layout`, the viewport's of a viewport
/// of `viewport`'s size among them, starts scrolled to by its
/// `scroll-initial-target` (Scroll Snap 2 section 3): the first element
/// in tree order inside it, not inside a scroll container inside it,
/// whose `scroll-initial-target` is `nearest`, aligned as its
/// `scroll-snap-align` says (at its start where that is `none`), as the
/// reference browser places it. Those with none are not listed.
pub(crate) fn initial_targets(
    layout: &DocumentLayout,
    viewport: (f32, f32),
) -> Vec<(Scroller, (f32, f32))> {
    let mut out = Vec::new();
    if !layout
        .iter()
        .any(|f| f.style.effects().scroll_initial_target)
    {
        return out;
    }
    let clamp = |at: [f32; 2], (min, max): ([f32; 2], [f32; 2])| {
        (
            at[0].clamp(min[0], max[0].max(min[0])),
            at[1].clamp(min[1], max[1].max(min[1])),
        )
    };
    if let Some(root) = layout
        .roots
        .first()
        .filter(|f| matches!(f.kind, FragmentKind::Box) && f.style.root)
    {
        let padding = scroll_padding(&root.style, viewport);
        let port = [
            (viewport.0 - padding[1] - padding[3]).max(0.0),
            (viewport.1 - padding[0] - padding[2]).max(0.0),
        ];
        let mut walk = Walk::new(&root.style, padding, port);
        let found = layout
            .roots
            .iter()
            .filter(|f| f.style.position != Position::Fixed)
            .find_map(|f| {
                if f.style.root {
                    f.children
                        .iter()
                        .find_map(|c| walk.initial(c, Place::at(f.x, f.y), f.w))
                } else {
                    walk.initial(f, Place::at(0.0, 0.0), viewport.0)
                }
            });
        if let Some(at) = found {
            let (max_x, max_y) = crate::scroll_state::viewport_max(layout, viewport);
            out.push((None, clamp(at, ([0.0, 0.0], [max_x, max_y]))));
        }
    }
    for f in layout.iter() {
        if matches!(f.kind, FragmentKind::Box)
            && f.generated.is_none()
            && !f.style.root
            && f.clips()
            && f.style.overflow().scrolls()
        {
            let [top, right, bottom, left] = f.style.used_border_width();
            let padding_box = ((f.w - left - right).max(0.0), (f.h - top - bottom).max(0.0));
            let padding = scroll_padding(&f.style, padding_box);
            let port = [
                (padding_box.0 - padding[1] - padding[3]).max(0.0),
                (padding_box.1 - padding[0] - padding[2]).max(0.0),
            ];
            let mut walk = Walk::new(&f.style, padding, port);
            let found = f
                .children
                .iter()
                .find_map(|c| walk.initial(c, Place::at(-left, -top), f.w));
            if let Some(at) = found {
                let (min_x, max_x) = crate::geometry::scroll_range_x(f);
                out.push((
                    Some(f.node),
                    clamp(at, ([min_x, 0.0], [max_x, f.max_scroll()])),
                ));
            }
        }
    }
    out
}

/// A [`SnapContainer`] of `style`'s snap type.
fn container(
    scroller: Scroller,
    style: &ComputedStyle,
    port: [f32; 2],
    (min, max): ([f32; 2], [f32; 2]),
    areas: Vec<Area>,
) -> SnapContainer {
    let kind = style.effects().scroll_snap_type;
    SnapContainer {
        scroller,
        smooth: style.effects().scroll_behavior_smooth,
        axes: kind.axes(),
        pair: kind.axis == SnapAxis::Pair,
        mandatory: kind.strictness == SnapStrictness::Mandatory,
        port,
        min,
        max: [max[0].max(min[0]), max[1].max(min[1])],
        areas,
    }
}

/// `style`'s `scroll-padding` in px, top, right, bottom and left, its
/// percentages of the scrollport's `size`, across and down.
pub(crate) fn scroll_padding(style: &ComputedStyle, size: (f32, f32)) -> [f32; 4] {
    let padding = &style.effects().scroll_padding;
    std::array::from_fn(|side| {
        let of = if side % 2 == 0 { size.1 } else { size.0 };
        padding[side].resolve(of, 0.0).max(0.0)
    })
}

/// Where a box is in its scroll container's content: at a point, or
/// through a transform on the way there, which maps the box's own
/// coordinates to the content's.
#[derive(Debug, Clone, Copy)]
enum Place {
    At(f32, f32),
    Mapped(Matrix),
}

impl Place {
    fn at(x: f32, y: f32) -> Place {
        Place::At(x, y)
    }

    /// The place of a child of `f`, which is here, at `(c.x, c.y)` in
    /// `f`'s coordinates, with `own` its own transform.
    fn child(self, c: &Fragment, own: Option<Matrix>) -> Place {
        match (self, own) {
            (Place::At(x, y), None) => Place::At(x + c.x, y + c.y),
            (Place::At(x, y), Some(own)) => {
                Place::Mapped(Matrix::translate(x + c.x, y + c.y, 0.0).then(&own))
            }
            (Place::Mapped(m), own) => {
                let m = m.then(&Matrix::translate(c.x, c.y, 0.0));
                Place::Mapped(own.map_or(m, |own| m.then(&own)))
            }
        }
    }

    /// The bounds, left, top, right and bottom, of a box of size `w` by
    /// `h` placed here.
    fn bounds(self, w: f32, h: f32) -> (f32, f32, f32, f32) {
        match self {
            Place::At(x, y) => (x, y, x + w, y + h),
            Place::Mapped(m) => m.bounds(0.0, 0.0, w, h).unwrap_or((0.0, 0.0, 0.0, 0.0)),
        }
    }
}

/// The walk that finds a scroll container's snap areas.
struct Walk {
    /// Its `scroll-padding`, top, right, bottom and left.
    padding: [f32; 4],
    port: [f32; 2],
    /// Whether its inline axis runs right to left, where an inline
    /// `start` is the right side.
    rtl: bool,
    areas: Vec<Area>,
}

impl Walk {
    fn new(style: &ComputedStyle, padding: [f32; 4], port: [f32; 2]) -> Walk {
        Walk {
            padding,
            port,
            rtl: style.direction == Direction::Rtl,
            areas: Vec::new(),
        }
    }

    /// Adds the snap areas of `f`, a child of a box placed at `parent`
    /// whose width is `width`, and of what is inside it, in tree order: a
    /// box before what is inside it, the children in order. A scroll
    /// container inside is an area itself, but its content is its own:
    /// one that scrolls in one axis alone (its `overflow` `clip` in the
    /// other) keeps them in both, as the reference browser keeps them.
    fn collect(&mut self, f: &Fragment, parent: Place, width: f32) {
        let mut stack = vec![(f, parent.child(f, f.matrix(width)), None)];
        while let Some((f, place, outer)) = stack.pop() {
            let mut inner = outer;
            if matches!(f.kind, FragmentKind::Box) {
                // An element's box, or a `::before` or `::after`'s, which
                // snaps as its element.
                let own = matches!(
                    f.generated,
                    None | Some(PseudoElement::Before | PseudoElement::After)
                );
                let [block, inline] = f.style.effects().scroll_snap_align;
                if own && (block != SnapAlign::None || inline != SnapAlign::None) {
                    inner = Some(self.areas.len());
                    self.add(f, place, [inline, block], outer);
                }
            }
            if f.clips() && f.style.overflow().scrolls() {
                continue;
            }
            for c in f.children.iter().rev() {
                stack.push((c, place.child(c, c.matrix(f.w)), inner));
            }
        }
    }

    /// The offsets, across and down, that align the first element at or
    /// inside `f`, a child of a box placed at `parent` whose width is
    /// `width`, whose `scroll-initial-target` is `nearest`, not inside a
    /// scroll container inside: aligned as its `scroll-snap-align` says,
    /// at its start where that is `none`.
    fn initial(&mut self, f: &Fragment, parent: Place, width: f32) -> Option<[f32; 2]> {
        let mut stack = vec![(f, parent.child(f, f.matrix(width)))];
        while let Some((f, place)) = stack.pop() {
            let element = matches!(f.kind, FragmentKind::Box) && f.generated.is_none();
            if element && f.style.effects().scroll_initial_target {
                let start = |align: SnapAlign| match align {
                    SnapAlign::None => SnapAlign::Start,
                    other => other,
                };
                let [block, inline] = f.style.effects().scroll_snap_align;
                self.add(f, place, [start(inline), start(block)], None);
                let area = self.areas.pop()?;
                return Some([area.align[0]?, area.align[1]?]);
            }
            if f.clips() && f.style.overflow().scrolls() {
                continue;
            }
            for c in f.children.iter().rev() {
                stack.push((c, place.child(c, c.matrix(f.w))));
            }
        }
        None
    }

    /// Adds the area of `f`, placed at `place`, aligned `[across, down]`.
    fn add(
        &mut self,
        f: &Fragment,
        place: Place,
        [across, down]: [SnapAlign; 2],
        parent: Option<usize>,
    ) {
        let margin = f.style.effects().scroll_margin;
        let (left, top, right, bottom) = place.bounds(f.w, f.h);
        let span = [
            (
                left - margin[3] - self.padding[3],
                right + margin[1] - self.padding[3],
            ),
            (
                top - margin[0] - self.padding[0],
                bottom + margin[2] - self.padding[0],
            ),
        ];
        // Right to left, the inline start is the right side.
        let across = match (self.rtl, across) {
            (true, SnapAlign::Start) => SnapAlign::End,
            (true, SnapAlign::End) => SnapAlign::Start,
            (_, other) => other,
        };
        let port = self.port;
        let align = |axis: usize, align: SnapAlign| {
            let (start, end) = span[axis];
            Some(match align {
                SnapAlign::None => return None,
                SnapAlign::Start => start,
                SnapAlign::End => end - port[axis],
                SnapAlign::Center => (start + end) / 2.0 - port[axis] / 2.0,
            })
        };
        self.areas.push(Area {
            node: f.node,
            pseudo: f.generated,
            span,
            align: [align(0, across), align(1, down)],
            stop: f.style.effects().scroll_snap_stop_always,
            parent,
        });
    }
}

impl Area {
    /// It as a container is snapped to it.
    fn target(&self) -> SnapTarget {
        SnapTarget {
            node: self.node,
            pseudo: self.pseudo,
        }
    }
}

impl SnapContainer {
    /// `at` within its scroll range in `axis`.
    fn clamp(&self, axis: usize, at: f32) -> f32 {
        at.clamp(self.min[axis], self.max[axis])
    }

    /// Whether area `i` shows in the snapport in `axis` with the
    /// container scrolled to `at` there: some of it, an edge alone not,
    /// unless it has no size there.
    fn shows(&self, i: usize, axis: usize, at: f32) -> bool {
        let (start, end) = self.areas[i].span[axis];
        let port = self.port[axis];
        if end - start <= 0.0 {
            return start >= at && start <= at + port;
        }
        start.max(at) < end.min(at + port)
    }

    /// Whether area `i` is inside area `outer`.
    fn inside(&self, i: usize, outer: usize) -> bool {
        std::iter::successors(self.areas[i].parent, |&p| self.areas[p].parent).any(|p| p == outer)
    }

    /// The offsets a selection may take in `axis` for a scroll to
    /// `wanted` there, of the areas that count by `cross`: each one's
    /// aligned offset, and where an area larger than the snapport covers
    /// it nearest `wanted`.
    fn candidates(&self, axis: usize, wanted: f32, cross: Cross<'_>) -> Vec<Candidate> {
        let other = 1 - axis;
        let port = self.port[axis];
        let counts = |i: usize, offset: f32| match cross {
            None => true,
            Some((at, areas)) => {
                self.shows(i, other, at)
                    && (areas.is_empty() || areas.iter().any(|&a| self.shows(a, axis, offset)))
            }
        };
        let mut out = Vec::new();
        for (i, area) in self.areas.iter().enumerate() {
            if let Some(offset) = area.align[axis].map(|offset| self.clamp(axis, offset))
                && counts(i, offset)
            {
                out.push((i, offset));
            }
            let (start, end) = area.span[axis];
            if end - start > port {
                let at = wanted.clamp(start, end - port);
                let at = self.clear(i, axis, at, cross.map(|(at, _)| at));
                let at = self.clamp(axis, at.clamp(start, end - port));
                if counts(i, at) {
                    out.push((i, at));
                }
            }
        }
        out
    }

    /// Where area `i`, which covers the snapport at `at` in `axis`, may
    /// rest nearest `at`: there, unless another area (one that shows at
    /// `cross` in the other axis, where that is given) has an edge inside
    /// the snapport; then past it, its end at the start edge or its start
    /// at the end edge, whichever is nearer, the end on a tie, as the
    /// reference browser keeps a covering area from showing another.
    fn clear(&self, i: usize, axis: usize, at: f32, cross: Option<f32>) -> f32 {
        let port = self.port[axis];
        let mut forward: Option<f32> = None;
        let mut backward: Option<f32> = None;
        for (j, area) in self.areas.iter().enumerate() {
            if j == i || cross.is_some_and(|cross| !self.shows(j, 1 - axis, cross)) {
                continue;
            }
            let (start, end) = area.span[axis];
            // Its end inside the snapport: past it forward; its start
            // inside: before it backward. One wholly inside, both.
            if end > at + SAME && end < at + port - SAME {
                forward = Some(forward.map_or(end, |f| f.max(end)));
            }
            if start > at + SAME && start < at + port - SAME {
                let back = start - port;
                backward = Some(backward.map_or(back, |b| b.min(back)));
            }
        }
        match (forward, backward) {
            (None, None) => at,
            (Some(f), None) => f,
            (None, Some(b)) => b,
            (Some(f), Some(b)) => {
                if (f - at).abs() <= (b - at).abs() {
                    f
                } else {
                    b
                }
            }
        }
    }

    /// The offset in `axis` a scroll to `wanted` from `from` comes to
    /// rest at, `way` its direction there for a relative scroll (0 for
    /// none), among the areas that count by `cross`; and the areas
    /// aligned there. `None` where it does not snap.
    fn search(
        &self,
        axis: usize,
        wanted: f32,
        from: f32,
        way: i8,
        cross: Cross<'_>,
    ) -> Option<Found> {
        let candidates = self.candidates(axis, wanted, cross);
        // Past where it was, or for a wheel's or a touch's there too.
        let past = way.abs() != 2;
        let page = way.abs() == 3;
        let way = f32::from(way.signum());
        let mut ahead: Vec<Candidate> = candidates
            .iter()
            .copied()
            .filter(|&(_, at)| {
                if past {
                    way * (at - from) > SAME
                } else {
                    way != 0.0 && way * (at - from) >= -SAME
                }
            })
            .collect();
        // A page's scroll passes no area it has not shown whole: it goes
        // at most as far as brings the first such area's start (going
        // up, its end) to the snapport's edge, as the reference browser
        // pages.
        if page && let Some(limit) = self.page_limit(axis, from, way) {
            let within: Vec<Candidate> = ahead
                .iter()
                .copied()
                .filter(|&(_, at)| way * (at - limit) <= SAME)
                .collect();
            if !within.is_empty() {
                ahead = within;
            }
        }
        // The first area of `scroll-snap-stop: always` the scroll would
        // pass, or reach, on its way.
        let stop = ahead
            .iter()
            .copied()
            .filter(|&(i, at)| self.areas[i].stop && way * (at - wanted) <= SAME)
            .min_by(|a, b| (way * (a.1 - from)).total_cmp(&(way * (b.1 - from))));
        let pool = match stop {
            Some(stop) => vec![stop],
            None if way != 0.0 && !ahead.is_empty() => ahead,
            None => candidates,
        };
        let nearest =
            pool.iter()
                .map(|&(_, at)| at)
                .fold(None, |best: Option<f32>, at| match best {
                    Some(b) if (b - wanted).abs() <= (at - wanted).abs() => Some(b),
                    _ => Some(at),
                })?;
        if !self.mandatory && (nearest - wanted).abs() > PROXIMITY * self.port[axis] {
            return None;
        }
        let mut aligned: Vec<usize> = pool
            .iter()
            .filter(|&&(_, at)| (at - nearest).abs() <= SAME)
            .map(|&(i, _)| i)
            .collect();
        aligned.dedup();
        // Of the areas larger than the snapport that cover it there, the
        // one inside the others, as the reference browser takes it.
        let port = self.port[axis];
        let covers = |i: usize| {
            let (start, end) = self.areas[i].span[axis];
            end - start > port && start <= nearest + SAME && end >= nearest + port - SAME
        };
        let within = |i: usize, j: usize| {
            let (a, b) = (self.areas[i].span[axis], self.areas[j].span[axis]);
            b.0 <= a.0 && a.1 <= b.1 && (b.0 < a.0 || a.1 < b.1)
        };
        let all = aligned.clone();
        aligned
            .retain(|&j| !(covers(j) && all.iter().any(|&i| i != j && covers(i) && within(i, j))));
        Some((nearest, aligned))
    }

    /// How far a page's scroll in `axis` from `from`, `way` its
    /// direction, may go: to the offset that brings the first area it
    /// has not shown whole, past the snapport's far edge, to the near
    /// one; `None` where there is none.
    fn page_limit(&self, axis: usize, from: f32, way: f32) -> Option<f32> {
        let port = self.port[axis];
        let limits = self.areas.iter().filter_map(|area| {
            let (start, end) = area.span[axis];
            if way > 0.0 {
                (start > from + SAME && end > from + port + SAME).then_some(start)
            } else {
                (end < from + port - SAME && start < from - SAME).then_some(end - port)
            }
        });
        if way > 0.0 {
            limits.min_by(f32::total_cmp)
        } else {
            limits.max_by(f32::total_cmp)
        }
    }

    /// `scroll-snap-type: pair`'s selection: the position, across and
    /// down, of an area aligned in both axes, and the areas aligned
    /// there; the nearest `wanted`, or for a relative scroll from `from`
    /// the one most in its direction, then the nearest.
    fn search_pair(
        &self,
        wanted: [f32; 2],
        from: [f32; 2],
        relative: bool,
    ) -> Option<([f32; 2], Vec<usize>)> {
        let candidates: Vec<(usize, [f32; 2])> = self
            .areas
            .iter()
            .enumerate()
            .filter_map(|(i, area)| {
                Some((
                    i,
                    [self.clamp(0, area.align[0]?), self.clamp(1, area.align[1]?)],
                ))
            })
            .collect();
        let distance = |at: [f32; 2]| (at[0] - wanted[0]).hypot(at[1] - wanted[1]);
        let way = [wanted[0] - from[0], wanted[1] - from[1]];
        let length = way[0].hypot(way[1]);
        // How much an offset is in the scroll's direction: the cosine of
        // the angle between them.
        let cosine = |at: [f32; 2]| {
            let to = [at[0] - from[0], at[1] - from[1]];
            (to[0] * way[0] + to[1] * way[1]) / (to[0].hypot(to[1]) * length)
        };
        let ahead: Vec<(usize, [f32; 2])> = if relative && length > 0.0 {
            candidates
                .iter()
                .copied()
                .filter(|&(_, at)| (at[0] - from[0]) * way[0] + (at[1] - from[1]) * way[1] > SAME)
                .collect()
        } else {
            Vec::new()
        };
        let best = if ahead.is_empty() {
            candidates
                .iter()
                .copied()
                .min_by(|a, b| distance(a.1).total_cmp(&distance(b.1)))
        } else {
            ahead.iter().copied().min_by(|a, b| {
                cosine(b.1)
                    .total_cmp(&cosine(a.1))
                    .then(distance(a.1).total_cmp(&distance(b.1)))
            })
        }?;
        let at = best.1;
        if !self.mandatory
            && (0..2).any(|axis| (at[axis] - wanted[axis]).abs() > PROXIMITY * self.port[axis])
        {
            return None;
        }
        let aligned = candidates
            .iter()
            .filter(|&&(_, c)| (c[0] - at[0]).abs() <= SAME && (c[1] - at[1]).abs() <= SAME)
            .map(|&(i, _)| i)
            .collect();
        Some((at, aligned))
    }

    /// Of the areas `aligned` at one position, the snap target, as the
    /// reference browser picks it (see the module's notes): `both` the
    /// areas aligned at the other axis's position, where it snaps there.
    fn pick(&self, aligned: &[usize], both: Option<&[usize]>, prefer: &Prefer<'_>) -> usize {
        let mut pool = aligned.to_vec();
        let keep = |pool: &mut Vec<usize>, wanted: &dyn Fn(usize) -> bool| {
            let kept: Vec<usize> = pool.iter().copied().filter(|&i| wanted(i)).collect();
            if !kept.is_empty() {
                *pool = kept;
            }
        };
        keep(&mut pool, &|i| prefer.target.contains(&self.areas[i].node));
        keep(&mut pool, &|i| prefer.focus.contains(&self.areas[i].node));
        let all = pool.clone();
        keep(&mut pool, &|i| {
            !all.iter().any(|&j| j != i && self.inside(j, i))
        });
        if let Some(both) = both {
            keep(&mut pool, &|i| both.contains(&i));
        }
        pool.into_iter().min().unwrap_or(0)
    }

    /// Where it comes to rest after a scroll to `wanted`, across and
    /// down, from `from`, in the direction `direction` for a relative
    /// scroll, and the areas it is then snapped to: an axis it does not
    /// snap in, or in which no area is near enough, stays where it was
    /// going, snapped to none. A direction is 0 in an axis for none, 1 or
    /// -1 for a key's or a script's scroll, which snaps past where it was
    /// where it can, 2 or -2 for a wheel's or a touch's, which may snap
    /// back to where it was but not behind it, and 3 or -3 for a page's
    /// by a key, which snaps past where it was but passes no area it has
    /// not shown whole, as the reference browser snaps them.
    pub(crate) fn select(
        &self,
        wanted: (f32, f32),
        from: (f32, f32),
        direction: [i8; 2],
        prefer: &Prefer<'_>,
    ) -> ((f32, f32), Targets) {
        let wanted = [wanted.0, wanted.1];
        let from = [from.0, from.1];
        if self.pair {
            return match self.search_pair(wanted, from, direction != [0, 0]) {
                Some((at, aligned)) => {
                    let target = self.areas[self.pick(&aligned, None, prefer)].target();
                    ((at[0], at[1]), [Some(target), Some(target)])
                }
                None => ((wanted[0], wanted[1]), [None, None]),
            };
        }
        let search = |axis: usize, cross: Cross<'_>| {
            self.axes[axis]
                .then(|| self.search(axis, wanted[axis], from[axis], direction[axis], cross))
                .flatten()
        };
        // An axis snaps alone among the areas that show where the other,
        // which it does not snap in, goes; both axes snap each among all
        // the areas first.
        let base = |axis: usize| (!self.axes[1 - axis]).then_some((wanted[1 - axis], &[][..]));
        let mut found = [search(0, base(0)), search(1, base(1))];
        match &found {
            // Where the two axes' areas would not show together, each axis
            // is chosen again among the areas that show where the other
            // put it and that show it, and the nearer of the two pairs
            // that makes is taken, as the reference browser takes it.
            [Some(x), Some(y)] => {
                let together = x.1.iter().any(|&i| self.shows(i, 1, y.0))
                    && y.1.iter().any(|&i| self.shows(i, 0, x.0));
                if !together {
                    let down = search(1, Some((x.0, &x.1)));
                    let across = search(0, Some((y.0, &y.1)));
                    let distance = |at: [f32; 2]| (at[0] - wanted[0]).hypot(at[1] - wanted[1]);
                    match (across, down) {
                        (Some(across), Some(down))
                            if distance([x.0, down.0]) >= distance([across.0, y.0]) =>
                        {
                            found[0] = Some(across);
                        }
                        (_, Some(down)) => found[1] = Some(down),
                        (Some(across), None) => found[0] = Some(across),
                        (None, None) => {}
                    }
                }
            }
            // An axis no area counts in is chosen among those that show
            // where the other snaps.
            [Some(x), None] => {
                let x = (x.0, x.1.clone());
                found[1] = search(1, Some((x.0, &x.1)));
            }
            [None, Some(y)] => {
                let y = (y.0, y.1.clone());
                found[0] = search(0, Some((y.0, &y.1)));
            }
            [None, None] => {}
        }
        let at = [
            found[0].as_ref().map_or(wanted[0], |f| f.0),
            found[1].as_ref().map_or(wanted[1], |f| f.0),
        ];
        let areas = std::array::from_fn(|axis| {
            let (_, aligned) = found[axis].as_ref()?;
            let other = 1 - axis;
            let showing: Vec<usize> = aligned
                .iter()
                .copied()
                .filter(|&i| self.shows(i, other, at[other]))
                .collect();
            let pool = if showing.is_empty() {
                aligned
            } else {
                &showing
            };
            let both = found[other].as_ref().map(|f| f.1.as_slice());
            Some(self.areas[self.pick(pool, both, prefer)].target())
        });
        ((at[0], at[1]), areas)
    }

    /// Where it comes to rest, and the areas it is then snapped to, its
    /// offset going to `target`, across and down: when it rested at
    /// `snap` and was not scrolled since, back on the areas it was
    /// snapped to where they now are (section 5.2): an axis whose area is
    /// gone snapped afresh where the other puts it, and one resting where
    /// an area larger than the snapport covers it snapped afresh where it
    /// is, unless that keeps the area; else, after a scroll, where the
    /// scroll goes (in `direction`, a relative one's).
    pub(crate) fn settle(
        &self,
        snap: Option<Snap>,
        target: (f32, f32),
        direction: [i8; 2],
        prefer: &Prefer<'_>,
    ) -> ((f32, f32), Targets) {
        let snap = match snap {
            Some(snap) if snap.at == target => snap,
            Some(snap) => return self.select(target, snap.at, direction, prefer),
            None => return self.select(target, target, [0, 0], prefer),
        };
        let now = self.resnap(snap.areas, target);
        let fresh = || self.select(target, target, [0, 0], prefer);
        if self.pair && now.iter().any(|now| !matches!(now, Resnap::At(_))) {
            return fresh();
        }
        let mut at = [target.0, target.1];
        let mut areas = [None, None];
        for axis in 0..2 {
            if let Resnap::At(offset) = now[axis] {
                at[axis] = offset;
                areas[axis] = snap.areas[axis];
            }
        }
        let mut afresh: Option<((f32, f32), Targets)> = None;
        for axis in (0..2).filter(|&axis| self.axes[axis]) {
            let other = 1 - axis;
            match now[axis] {
                Resnap::At(_) => {}
                // The other axis keeps its area: this one snaps where that
                // shows, among the areas that show it.
                Resnap::Gone if matches!(now[other], Resnap::At(_)) => {
                    let kept: Vec<usize> = (0..self.areas.len())
                        .filter(|&i| Some(self.areas[i].target()) == snap.areas[other])
                        .collect();
                    if let Some((offset, aligned)) =
                        self.search(axis, at[axis], at[axis], 0, Some((at[other], &kept)))
                    {
                        at[axis] = offset;
                        areas[axis] = Some(self.areas[self.pick(&aligned, None, prefer)].target());
                    }
                }
                now => {
                    let ((x, y), found) = *afresh.get_or_insert_with(fresh);
                    if now == Resnap::Covering && found[axis] == snap.areas[axis] {
                        areas[axis] = snap.areas[axis];
                    } else {
                        at[axis] = [x, y][axis];
                        areas[axis] = found[axis];
                    }
                }
            }
        }
        ((at[0], at[1]), areas)
    }

    /// Where the areas `snapped` to, across and down, are now, as
    /// [`Resnap`] says: an area that still aligns and shows where the
    /// other axis then is holds its axis at its offset.
    fn resnap(&self, snapped: Targets, at: (f32, f32)) -> [Resnap; 2] {
        let at = [at.0, at.1];
        let position = |axis: usize| -> Resnap {
            let Some(wanted) = snapped[axis].filter(|_| self.axes[axis]) else {
                return Resnap::Gone;
            };
            let port = self.port[axis];
            let mut aligned = Resnap::Gone;
            for (i, area) in self.areas.iter().enumerate() {
                if area.target() != wanted {
                    continue;
                }
                let (start, end) = area.span[axis];
                if end - start > port && (start - SAME..=end - port + SAME).contains(&at[axis]) {
                    return Resnap::Covering;
                }
                if aligned == Resnap::Gone
                    && let Some(offset) = area.align[axis]
                {
                    aligned = Resnap::Aligned(i, self.clamp(axis, offset));
                }
            }
            aligned
        };
        let found = [position(0), position(1)];
        let now = std::array::from_fn::<f32, 2, _>(|axis| match found[axis] {
            Resnap::Aligned(_, offset) => offset,
            _ => at[axis],
        });
        std::array::from_fn(|axis| match found[axis] {
            Resnap::Aligned(i, offset) if self.shows(i, 1 - axis, now[1 - axis]) => {
                Resnap::At(offset)
            }
            Resnap::Aligned(..) => Resnap::Gone,
            other => other,
        })
    }
}

/// Where an area a scroll container was snapped to is after a layout, in
/// an axis: aligned at an offset there (and, as a resnap finds it, the
/// area by its index), which holds the axis at it; resting where it is
/// larger than the snapport and covers it, which snaps the axis afresh
/// unless that keeps the area; or gone, or not showing where the other
/// axis is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Resnap {
    Aligned(usize, f32),
    At(f32),
    Covering,
    Gone,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The target of `node`'s own area.
    fn on(node: NodeId) -> Option<SnapTarget> {
        Some(SnapTarget { node, pseudo: None })
    }

    fn ids(count: usize) -> Vec<NodeId> {
        (0..count)
            .map(|i| NodeId::from_bits(i as u64 + 10).expect("an id"))
            .collect()
    }

    /// An area of `node` from `x0` to `x1` across and `y0` to `y1` down,
    /// aligned at its start in the axes `align` names, in a container of
    /// a 100px snapport.
    fn area(node: NodeId, (x0, x1): (f32, f32), (y0, y1): (f32, f32), align: [bool; 2]) -> Area {
        Area {
            node,
            pseudo: None,
            span: [(x0, x1), (y0, y1)],
            align: [align[0].then_some(x0), align[1].then_some(y0)],
            stop: false,
            parent: None,
        }
    }

    /// A container of a 100px snapport snapping in `axes`, `mandatory`
    /// or not, scrolling to 1000 each way.
    fn snapper(axes: [bool; 2], mandatory: bool, areas: Vec<Area>) -> SnapContainer {
        SnapContainer {
            scroller: None,
            smooth: false,
            axes,
            pair: false,
            mandatory,
            port: [100.0, 100.0],
            min: [0.0, 0.0],
            max: [1000.0, 1000.0],
            areas,
        }
    }

    /// A container snapping down, `mandatory` or not, with a 100px
    /// snapport, its 50px areas starting at `at`, all showing across.
    fn down(mandatory: bool, at: &[f32]) -> (SnapContainer, Vec<NodeId>) {
        let nodes = ids(at.len());
        let areas = nodes
            .iter()
            .zip(at)
            .map(|(&node, &y)| area(node, (0.0, 50.0), (y, y + 50.0), [false, true]))
            .collect();
        (snapper([false, true], mandatory, areas), nodes)
    }

    const NONE: Prefer<'static> = Prefer {
        target: &[],
        focus: &[],
    };

    #[test]
    fn a_scroll_goes_to_the_nearest_snap_position_or_the_next_one_its_way() {
        let (c, n) = down(true, &[0.0, 200.0, 400.0, 1200.0]);
        // The nearest, the first in tree order on a tie.
        assert_eq!(
            c.select((0.0, 290.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 200.0), [None, on(n[1])])
        );
        assert_eq!(
            c.select((0.0, 300.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 200.0), [None, on(n[1])])
        );
        // A relative scroll: the nearest past where it was, its way.
        assert_eq!(
            c.select((0.0, 230.0), (0.0, 200.0), [0, 1], &NONE),
            ((0.0, 400.0), [None, on(n[2])])
        );
        assert_eq!(
            c.select((0.0, 170.0), (0.0, 200.0), [0, -1], &NONE),
            ((0.0, 0.0), [None, on(n[0])])
        );
        // None that way: the nearest of all.
        assert_eq!(
            c.select((0.0, 999.0), (0.0, 1000.0), [0, 1], &NONE),
            ((0.0, 1000.0), [None, on(n[3])])
        );
        // An area past the end aligns at the end; the axis it does not
        // snap in goes where it was going.
        assert_eq!(
            c.select((37.0, 900.0), (0.0, 0.0), [0, 0], &NONE),
            ((37.0, 1000.0), [None, on(n[3])])
        );
        // Proximity: within 30px of the 100px snapport, else not at all.
        let (p, n) = down(false, &[0.0, 200.0]);
        assert_eq!(
            p.select((0.0, 175.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 200.0), [None, on(n[1])])
        );
        assert_eq!(
            p.select((0.0, 120.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 120.0), [None, None])
        );
    }

    #[test]
    fn an_area_larger_than_the_snapport_rests_where_it_shows_no_part_of_another() {
        // An area from 100 to 800 down, and 50px ones at 150 and 400.
        let n = ids(3);
        let c = snapper(
            [false, true],
            true,
            vec![
                area(n[0], (0.0, 50.0), (100.0, 800.0), [false, true]),
                area(n[1], (0.0, 50.0), (150.0, 200.0), [false, true]),
                area(n[2], (0.0, 50.0), (400.0, 450.0), [false, true]),
            ],
        );
        // Anywhere it covers the snapport and no other shows in part.
        assert_eq!(
            c.select((0.0, 250.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 250.0), [None, on(n[0])])
        );
        // The one at 150 straddles the start edge at 180: past its end.
        assert_eq!(
            c.select((0.0, 180.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 200.0), [None, on(n[0])])
        );
        // The one at 400 straddles the end edge at 320: its start at the
        // end edge, nearer than its aligned position.
        assert_eq!(
            c.select((0.0, 320.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 300.0), [None, on(n[0])])
        );
        // Nearer an aligned position than any covering one: that.
        assert_eq!(
            c.select((0.0, 160.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 150.0), [None, on(n[1])])
        );
        // Past its end it covers no more: its end, 700.
        assert_eq!(
            c.select((0.0, 750.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 700.0), [None, on(n[0])])
        );
    }

    #[test]
    fn a_wheels_scroll_may_snap_back_and_a_pages_passes_no_area_unseen() {
        // Areas at 0, 30 (300px, larger than the snapport) and 200.
        let n = ids(3);
        let c = snapper(
            [false, true],
            true,
            vec![
                area(n[0], (0.0, 50.0), (0.0, 50.0), [false, true]),
                area(n[1], (0.0, 50.0), (30.0, 330.0), [false, true]),
                area(n[2], (0.0, 50.0), (200.0, 250.0), [false, true]),
            ],
        );
        // A key's 10px from 0 goes past where it was; a wheel's may come
        // back to it, the nearer.
        assert_eq!(
            c.select((0.0, 10.0), (0.0, 0.0), [0, 1], &NONE).1,
            [None, on(n[1])]
        );
        assert_eq!(
            c.select((0.0, 10.0), (0.0, 0.0), [0, 2], &NONE),
            ((0.0, 0.0), [None, on(n[0])])
        );
        // A page from 0, to 175: the area at 30 is not shown whole, so it
        // goes no further than to it, though 200 is nearer.
        assert_eq!(
            c.select((0.0, 175.0), (0.0, 0.0), [0, 1], &NONE),
            ((0.0, 200.0), [None, on(n[2])])
        );
        assert_eq!(
            c.select((0.0, 175.0), (0.0, 0.0), [0, 3], &NONE),
            ((0.0, 30.0), [None, on(n[1])])
        );
    }

    #[test]
    fn an_area_inside_the_snapport_keeps_a_covering_one_from_resting_on_it() {
        // An area from 0 to 1000 down and a 50px one at 300, which the
        // snapport at 250 shows whole: before it or past it, the nearer.
        let n = ids(2);
        let c = snapper(
            [false, true],
            true,
            vec![
                area(n[0], (0.0, 50.0), (0.0, 1000.0), [false, false]),
                area(n[1], (0.0, 50.0), (300.0, 350.0), [false, false]),
            ],
        );
        assert_eq!(
            c.select((0.0, 260.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 200.0), [None, on(n[0])])
        );
        assert_eq!(
            c.select((0.0, 330.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 350.0), [None, on(n[0])])
        );
    }

    #[test]
    fn a_container_resting_in_a_covering_area_stays_while_that_holds_it() {
        // At 150 in an area from 0 to 1000, beside one from 120 to 160
        // that straddles the snapport's start and one from 230 to 260 its
        // end: a selection afresh would take 160, but the area it rests
        // in still holds it there.
        let n = ids(3);
        let c = snapper(
            [false, true],
            true,
            vec![
                area(n[0], (0.0, 50.0), (0.0, 1000.0), [false, false]),
                area(n[1], (0.0, 50.0), (120.0, 160.0), [false, false]),
                area(n[2], (0.0, 50.0), (230.0, 260.0), [false, false]),
            ],
        );
        assert_eq!(
            c.select((0.0, 150.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 160.0), [None, on(n[0])])
        );
        let snap = Snap {
            areas: [None, on(n[0])],
            at: (0.0, 150.0),
        };
        assert_eq!(
            c.settle(Some(snap), (0.0, 150.0), [0, 0], &NONE),
            ((0.0, 150.0), [None, on(n[0])])
        );
    }

    #[test]
    fn a_pseudo_elements_area_is_followed_as_its_own() {
        // An element's box at 0 and its `::after`'s at 300: snapped to the
        // second, a layout that moves it to 400 takes the container there.
        let n = ids(1);
        let mut after = area(n[0], (0.0, 50.0), (400.0, 450.0), [false, true]);
        after.pseudo = Some(PseudoElement::After);
        let c = snapper(
            [false, true],
            true,
            vec![area(n[0], (0.0, 50.0), (0.0, 50.0), [false, true]), after],
        );
        let target = SnapTarget {
            node: n[0],
            pseudo: Some(PseudoElement::After),
        };
        let snap = Snap {
            areas: [None, Some(target)],
            at: (0.0, 300.0),
        };
        assert_eq!(
            c.settle(Some(snap), (0.0, 300.0), [0, 0], &NONE),
            ((0.0, 400.0), [None, Some(target)])
        );
    }

    #[test]
    fn a_relative_scroll_stops_at_an_area_that_says_always() {
        let (mut c, n) = down(true, &[0.0, 100.0, 200.0, 300.0]);
        c.areas[1].stop = true;
        // From 0 towards 290: the area at 100 is not passed.
        assert_eq!(
            c.select((0.0, 290.0), (0.0, 0.0), [0, 1], &NONE),
            ((0.0, 100.0), [None, on(n[1])])
        );
        // Not in the way back from 300, nor for a scroll to a place.
        assert_eq!(
            c.select((0.0, 190.0), (0.0, 300.0), [0, -1], &NONE),
            ((0.0, 200.0), [None, on(n[2])])
        );
        assert_eq!(
            c.select((0.0, 290.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 300.0), [None, on(n[3])])
        );
    }

    #[test]
    fn only_areas_that_would_show_are_snapped_to() {
        // Down only: the area at 100 is across at 500, out of the
        // snapport where the scroll leaves it across, at 0.
        let n = ids(2);
        let c = snapper(
            [false, true],
            true,
            vec![
                area(n[0], (500.0, 550.0), (100.0, 150.0), [false, true]),
                area(n[1], (0.0, 50.0), (300.0, 350.0), [false, true]),
            ],
        );
        assert_eq!(
            c.select((0.0, 110.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 300.0), [None, on(n[1])])
        );
        assert_eq!(
            c.select((480.0, 110.0), (0.0, 0.0), [0, 0], &NONE),
            ((480.0, 100.0), [None, on(n[0])])
        );
        // One whose edge alone meets the snapport's does not show: none
        // does there.
        assert_eq!(
            c.select((400.0, 110.0), (0.0, 0.0), [0, 0], &NONE),
            ((400.0, 110.0), [None, None])
        );
        // Both ways: areas at (0, 0), (0, 400) and (400, 0). Each axis
        // alone would take (400, 400), where neither shows; the nearer
        // axis keeps its position, and the other snaps where it shows.
        let n = ids(3);
        let c = snapper(
            [true, true],
            true,
            vec![
                area(n[0], (0.0, 50.0), (0.0, 50.0), [true, true]),
                area(n[1], (0.0, 50.0), (400.0, 450.0), [true, true]),
                area(n[2], (400.0, 450.0), (0.0, 50.0), [true, true]),
            ],
        );
        assert_eq!(
            c.select((250.0, 300.0), (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 400.0), [on(n[1]), on(n[1])])
        );
        assert_eq!(
            c.select((300.0, 250.0), (0.0, 0.0), [0, 0], &NONE),
            ((400.0, 0.0), [on(n[2]), on(n[2])])
        );
    }

    #[test]
    fn of_the_areas_aligned_at_a_position_the_preferred_is_the_target() {
        // At (0, 0): an outer area and an inner one inside it, a later
        // one, one aligned down alone, and one out of the snapport.
        let n = ids(5);
        let (outer, inner, later, down_only, far) = (n[0], n[1], n[2], n[3], n[4]);
        let mut areas = vec![
            area(outer, (0.0, 80.0), (0.0, 80.0), [true, true]),
            area(inner, (0.0, 20.0), (0.0, 20.0), [true, true]),
            area(later, (0.0, 50.0), (0.0, 50.0), [true, true]),
            area(down_only, (0.0, 30.0), (0.0, 30.0), [false, true]),
            area(far, (300.0, 330.0), (0.0, 30.0), [false, true]),
        ];
        areas[1].parent = Some(0);
        let c = snapper([true, true], true, areas.clone());
        // The innermost before the first in tree order, in both axes.
        assert_eq!(
            c.select((10.0, 10.0), (0.0, 0.0), [0, 0], &NONE).1,
            [on(inner), on(inner)]
        );
        // The focused one, or one holding the focus, before it.
        let focus = [down_only];
        let prefer = Prefer {
            target: &[],
            focus: &focus,
        };
        assert_eq!(
            c.select((10.0, 10.0), (0.0, 0.0), [0, 0], &prefer).1,
            [on(inner), on(down_only)]
        );
        // The targeted one before the focused one; one out of the
        // snapport not even then.
        let target = [later, far];
        let prefer = Prefer {
            target: &target,
            focus: &focus,
        };
        assert_eq!(
            c.select((10.0, 10.0), (0.0, 0.0), [0, 0], &prefer).1,
            [on(later), on(later)]
        );
        // With no nesting, one aligned in both axes before the first in
        // tree order, aligned down alone.
        let flat = vec![
            areas[3].clone(),
            area(inner, (0.0, 20.0), (0.0, 20.0), [true, true]),
            areas[2].clone(),
        ];
        let c = snapper([true, true], true, flat);
        assert_eq!(
            c.select((10.0, 10.0), (0.0, 0.0), [0, 0], &NONE).1,
            [on(inner), on(inner)]
        );
    }

    #[test]
    fn a_pair_snaps_both_axes_to_one_area_aligned_in_both() {
        let n = ids(4);
        let mut c = snapper(
            [true, true],
            true,
            vec![
                area(n[0], (100.0, 150.0), (100.0, 150.0), [true, true]),
                area(n[1], (100.0, 150.0), (500.0, 550.0), [true, true]),
                area(n[2], (800.0, 850.0), (100.0, 150.0), [true, true]),
                // Aligned across alone: not a pair's.
                area(n[3], (400.0, 450.0), (400.0, 450.0), [true, false]),
            ],
        );
        c.pair = true;
        assert_eq!(
            c.select((400.0, 400.0), (100.0, 100.0), [0, 0], &NONE),
            ((100.0, 500.0), [on(n[1]), on(n[1])])
        );
        // A relative scroll: the one most in its direction.
        assert_eq!(
            c.select((300.0, 160.0), (100.0, 100.0), [1, 1], &NONE),
            ((800.0, 100.0), [on(n[2]), on(n[2])])
        );
    }

    #[test]
    fn a_container_not_scrolled_since_goes_back_to_its_areas() {
        let (c, n) = down(true, &[0.0, 250.0]);
        // Not scrolled since it rested at 200 on the second: where that
        // is now.
        let snap = Snap {
            areas: [None, on(n[1])],
            at: (0.0, 200.0),
        };
        assert_eq!(
            c.settle(Some(snap), (0.0, 200.0), [0, 0], &NONE),
            ((0.0, 250.0), [None, on(n[1])])
        );
        // Scrolled since: where the scroll went.
        assert_eq!(
            c.settle(Some(snap), (0.0, 20.0), [0, 0], &NONE),
            ((0.0, 0.0), [None, on(n[0])])
        );
        // Its area gone, or none snapped to: afresh where it is.
        let (gone, m) = down(true, &[0.0, 180.0]);
        assert_eq!(
            gone.settle(Some(snap), (0.0, 200.0), [0, 0], &NONE),
            ((0.0, 180.0), [None, on(m[1])])
        );
        let unsnapped = Snap {
            areas: [None, None],
            at: (0.0, 200.0),
        };
        assert_eq!(
            c.settle(Some(unsnapped), (0.0, 200.0), [0, 0], &NONE),
            ((0.0, 250.0), [None, on(n[1])])
        );
        // The first time: where it is.
        assert_eq!(
            c.settle(None, (0.0, 0.0), [0, 0], &NONE),
            ((0.0, 0.0), [None, on(n[0])])
        );
        // An area moved out of the snapport across no longer holds it:
        // the one aligned with it there does.
        let n = ids(2);
        let moved = snapper(
            [false, true],
            true,
            vec![
                area(n[0], (0.0, 50.0), (100.0, 150.0), [false, true]),
                area(n[1], (500.0, 550.0), (100.0, 150.0), [false, true]),
            ],
        );
        let snap = Snap {
            areas: [None, on(n[1])],
            at: (0.0, 100.0),
        };
        assert_eq!(
            moved.settle(Some(snap), (0.0, 100.0), [0, 0], &NONE),
            ((0.0, 100.0), [None, on(n[0])])
        );
    }
}
