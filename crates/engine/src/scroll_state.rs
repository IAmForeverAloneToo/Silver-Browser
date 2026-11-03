//! The scroll states scroll-state container queries read (CSS
//! Conditional 5 section 7.4.4, D72), and scroll snapping (CSS Scroll
//! Snap 1), which one of them, `snapped`, asks about.
//!
//! A layout's [`DocumentLayout::container_sizes`] gives the size
//! containers' sizes and the scroll containers' `scrollable` sides; this
//! adds the rest, as the engine's frames and a script's flush both read
//! them: the root element's states, which are the viewport's; the sides
//! the last relative scroll of each scroll container moved it towards
//! (`scrolled`), which the engine records as scrolls happen; the sides
//! each sticky box is held off by its insets (`stuck`), as the paint
//! order shifts it; and the axes each snap target is snapped in.
//!
//! **Snapping.** A scroll container whose `scroll-snap-type` is not
//! `none` (the viewport's is the root element's) rests on a snap
//! position in each of its axes the type names: one of its snap areas,
//! the border boxes of the elements inside it (not inside a scroll
//! container inside it) whose `scroll-snap-align` is not `none` grown by
//! their `scroll-margin`, aligned as that says with its snapport, its
//! scrollport less its `scroll-padding` (sections 4 and 6). After a
//! scroll, each axis goes to the snap position nearest where the scroll
//! went (section 6.2's selection), one past where it was in the scroll's
//! direction for a relative scroll where there is one; `mandatory` always
//! snaps, `proximity` only within [`PROXIMITY`] of the snapport's size.
//! After a layout that moves the areas, a scroll container that was not
//! scrolled since goes back to the areas it was snapped to (section
//! 5.2's re-snapping), or snaps afresh where they are gone. Where two
//! areas give the same position, the first in tree order is the one
//! snapped to. A relative scroll does not pass an area whose
//! `scroll-snap-stop` is `always` on its way (section 6.4): it stops at
//! the first. An area larger than the snapport in an axis may rest
//! anywhere it covers the snapport (section 6.2.3): a scroll that ends
//! there stays where it ends.

use std::collections::HashMap;

use dom::NodeId;
use layout::{DocumentLayout, Fragment, FragmentKind};
use style::ComputedStyle;
use style::computed::{Position, SnapAlign, SnapAxis, SnapStrictness};
use style::container::{ContainerSizes, ScrollState};
use style::selector_matching::PseudoElement;

/// How near a snap position a `proximity` scroll container's offset must
/// come to snap, as a fraction of its snapport's size in that axis.
const PROXIMITY: f32 = 0.3;

/// A scroll container: an element's, or the viewport (`None`).
pub(crate) type Scroller = Option<NodeId>;

/// The direction of the last relative scroll that moved each scroll
/// container in each axis, across and down: -1 left or up, 1 right or
/// down, 0 none yet.
pub(crate) type Scrolled = HashMap<Scroller, [i8; 2]>;

/// What each snapping scroll container rests on: the snap areas it is
/// snapped to, across and down, and the offset it was left at.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Snap {
    pub(crate) areas: [Option<NodeId>; 2],
    pub(crate) at: (f32, f32),
}

/// Each snapping scroll container's [`Snap`].
pub(crate) type Snaps = HashMap<Scroller, Snap>;

/// What the states are read from beside a layout: the viewport's size
/// and scroll offset across and down, each scroll container's offset,
/// and the scrolls and snaps so far.
pub(crate) struct Inputs<'a> {
    pub(crate) viewport: (f32, f32),
    pub(crate) viewport_scroll: (f32, f32),
    pub(crate) offsets: &'a dyn Fn(NodeId) -> (f32, f32),
    pub(crate) scrolled: &'a Scrolled,
    pub(crate) snaps: &'a Snaps,
}

/// The sides, top, right, bottom and left, a direction across and down
/// points to.
fn sides([across, down]: [i8; 2]) -> [bool; 4] {
    [down < 0, across > 0, down > 0, across < 0]
}

/// The sizes and scroll states of `layout`'s containers for `inputs`.
pub(crate) fn container_states(layout: &DocumentLayout, inputs: &Inputs<'_>) -> ContainerSizes {
    let mut sizes = layout.container_sizes(inputs.viewport.0);
    let scroll_state = |f: &Fragment| {
        matches!(f.kind, FragmentKind::Box)
            && f.generated.is_none()
            && f.style.container_type.scroll_state
    };
    if !layout.iter().any(scroll_state) {
        return sizes;
    }
    let snapped: HashMap<NodeId, [bool; 2]> = inputs
        .snaps
        .values()
        .flat_map(|snap| {
            snap.areas
                .iter()
                .enumerate()
                .filter_map(|(axis, area)| area.map(|area| (area, axis)))
        })
        .fold(HashMap::new(), |mut map, (area, axis)| {
            map.entry(area).or_insert([false; 2])[axis] = true;
            map
        });
    let sticky = if layout
        .iter()
        .any(|f| scroll_state(f) && f.style.position == Position::Sticky)
    {
        let offsets = Offsets(inputs.offsets);
        paint::order::sticky_shifts(
            &layout.roots,
            paint::Viewport {
                width: inputs.viewport.0,
                height: inputs.viewport.1,
                scroll_y: inputs.viewport_scroll.1,
            },
            &offsets,
        )
    } else {
        HashMap::new()
    };
    let (max_x, max_y) = viewport_max(layout, inputs.viewport);
    let (vx, vy) = inputs.viewport_scroll;
    for f in layout.iter().filter(|f| scroll_state(f)) {
        let mut state = ScrollState::default();
        if f.style.root {
            // The root element's scroll states are the viewport's.
            state.scrollable = [vy > 0.0, vx < max_x, vy < max_y, vx > 0.0];
            state.scrolled = sides(inputs.scrolled.get(&None).copied().unwrap_or_default());
        } else if f.style.overflow().scrolls() {
            // Towards the sides it can still go: across from its scroll
            // origin, the right edge right to left.
            let (x, y) = (inputs.offsets)(f.node);
            let (min_x, max_x) = crate::geometry::scroll_range_x(f);
            let max_y = f.max_scroll();
            state.scrollable = [y > 0.0, x < max_x, y < max_y, x > min_x];
            state.scrolled = sides(
                inputs
                    .scrolled
                    .get(&Some(f.node))
                    .copied()
                    .unwrap_or_default(),
            );
        }
        if let Some(&(sx, sy)) = sticky.get(&f.node) {
            state.stuck = [sy > 0.0, sx < 0.0, sy < 0.0, sx > 0.0];
        }
        if let Some(&axes) = snapped.get(&f.node) {
            state.snapped = axes;
        }
        sizes.insert_scroll(f.node, state);
    }
    sizes
}

/// The furthest the viewport scrolls across and down in `layout`.
pub(crate) fn viewport_max(layout: &DocumentLayout, (width, height): (f32, f32)) -> (f32, f32) {
    (
        (crate::geometry::document_width(layout) - width).max(0.0),
        (crate::geometry::document_height(layout) - height).max(0.0),
    )
}

/// The scroll offsets of [`Inputs::offsets`] as the paint order reads
/// them.
struct Offsets<'a>(&'a dyn Fn(NodeId) -> (f32, f32));

impl paint::ScrollOffsets for Offsets<'_> {
    fn offset(&self, node: NodeId) -> f32 {
        (self.0)(node).1
    }

    fn offset_x(&self, node: NodeId) -> f32 {
        (self.0)(node).0
    }
}

// ---- Snapping -------------------------------------------------------------

/// A snapping scroll container as a layout has it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SnapContainer {
    pub(crate) scroller: Scroller,
    /// Whether its `scroll-behavior` is `smooth` (the root's for the
    /// viewport): what a snap a layout or a style asks for moves it by.
    pub(crate) smooth: bool,
    /// Whether it snaps across and down.
    axes: [bool; 2],
    mandatory: bool,
    /// Its snapport's size, across and down.
    port: (f32, f32),
    /// The furthest it scrolls, across and down, and the least across:
    /// below 0 right to left, where the scroll origin is the right edge.
    max: (f32, f32),
    min_x: f32,
    /// Its snap areas, in tree order: each one's element and the offsets,
    /// across and down, that align it, where it aligns in that axis.
    areas: Vec<(NodeId, [Option<f32>; 2])>,
    /// The areas whose `scroll-snap-stop` is `always`.
    stops: Vec<NodeId>,
    /// The areas larger than the snapport in an axis, with the offsets
    /// between which they cover it there.
    covers: Vec<Covering>,
}

/// An area larger than the snapport in an axis, with the offsets between
/// which it covers it there.
type Covering = (NodeId, [Option<(f32, f32)>; 2]);

/// What a scroll container's content gives it to snap to: its areas, the
/// ones of `scroll-snap-stop: always` and the ones covering the snapport
/// ([`SnapContainer`]'s fields of the names).
type Found = (Vec<(NodeId, [Option<f32>; 2])>, Vec<NodeId>, Vec<Covering>);

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
        let port = (
            (viewport.0 - padding[1] - padding[3]).max(0.0),
            (viewport.1 - padding[0] - padding[2]).max(0.0),
        );
        let mut found: Found = (Vec::new(), Vec::new(), Vec::new());
        for f in layout
            .roots
            .iter()
            .filter(|f| f.style.position != Position::Fixed)
        {
            // The root element's content is the viewport's, its own snap
            // type the viewport's.
            if f.style.root {
                for c in &f.children {
                    collect_areas(c, (f.x + c.x, f.y + c.y), padding, port, &mut found);
                }
            } else {
                collect_areas(f, (f.x, f.y), padding, port, &mut found);
            }
        }
        let max = viewport_max(layout, viewport);
        out.push(container(None, &root.style, port, max, found));
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
            let port = (
                (padding_box.0 - padding[1] - padding[3]).max(0.0),
                (padding_box.1 - padding[0] - padding[2]).max(0.0),
            );
            let mut found: Found = (Vec::new(), Vec::new(), Vec::new());
            for c in &f.children {
                collect_areas(c, (c.x - left, c.y - top), padding, port, &mut found);
            }
            let max = (crate::geometry::max_scroll_x(f), f.max_scroll());
            let min_x = crate::geometry::scroll_range_x(f).0;
            let mut snapping = container(Some(f.node), &f.style, port, max, found);
            snapping.min_x = min_x;
            out.push(snapping);
        }
    }
    out
}

/// A [`SnapContainer`] of `style`'s snap type.
fn container(
    scroller: Scroller,
    style: &ComputedStyle,
    port: (f32, f32),
    max: (f32, f32),
    (areas, stops, covers): Found,
) -> SnapContainer {
    let kind = style.effects().scroll_snap_type;
    SnapContainer {
        scroller,
        smooth: style.effects().scroll_behavior_smooth,
        axes: kind.axes(),
        mandatory: kind.strictness == SnapStrictness::Mandatory,
        port,
        max,
        min_x: 0.0,
        areas,
        stops,
        covers,
    }
}

/// `style`'s `scroll-padding` in px, top, right, bottom and left, its
/// percentages of the scrollport's `size`, across and down.
fn scroll_padding(style: &ComputedStyle, size: (f32, f32)) -> [f32; 4] {
    let padding = &style.effects().scroll_padding;
    std::array::from_fn(|side| {
        let of = if side % 2 == 0 { size.1 } else { size.0 };
        padding[side].resolve(of, 0.0).max(0.0)
    })
}

/// Adds the snap areas of `f`, whose border box is at `at` in its scroll
/// container's content, and of what is inside it, to `areas`, aligned in
/// a snapport of `port` inset by `padding`. A scroll container inside
/// is an area itself, but its content is its own, as is that of a box
/// whose `scroll-snap-type` is not `none`. The areas are added
/// in tree order: a box before what is inside it, the children in order;
/// those whose `scroll-snap-stop` is `always` to its stops too, and
/// those larger than the snapport to its covering ones.
fn collect_areas(
    f: &Fragment,
    at: (f32, f32),
    padding: [f32; 4],
    port: (f32, f32),
    (areas, stops, covers): &mut Found,
) {
    let mut stack = vec![(f, at)];
    while let Some((f, (x, y))) = stack.pop() {
        if !matches!(f.kind, FragmentKind::Box) {
            stack.extend(f.children.iter().rev().map(|c| (c, (x + c.x, y + c.y))));
            continue;
        }
        let [block, inline] = f.style.effects().scroll_snap_align;
        // An element's box, or a `::before` or `::after`'s, which snaps as
        // its element.
        let own = matches!(
            f.generated,
            None | Some(PseudoElement::Before | PseudoElement::After)
        );
        if own && (block != SnapAlign::None || inline != SnapAlign::None) {
            let margin = f.style.effects().scroll_margin;
            let area = (
                x - margin[3],
                y - margin[0],
                f.w + margin[1] + margin[3],
                f.h + margin[0] + margin[2],
            );
            let align = |align: SnapAlign, start: f32, size: f32, before: f32, port: f32| {
                Some(match align {
                    SnapAlign::None => return None,
                    SnapAlign::Start => start - before,
                    SnapAlign::End => start + size - before - port,
                    SnapAlign::Center => start + size / 2.0 - before - port / 2.0,
                })
            };
            areas.push((
                f.node,
                [
                    align(inline, area.0, area.2, padding[3], port.0),
                    align(block, area.1, area.3, padding[0], port.1),
                ],
            ));
            if f.style.effects().scroll_snap_stop_always {
                stops.push(f.node);
            }
            // From where its start meets the snapport's to where its end
            // does.
            let cover = |start: f32, size: f32, before: f32, port: f32| {
                (size > port).then_some((start - before, start + size - before - port))
            };
            let covering = [
                cover(area.0, area.2, padding[3], port.0),
                cover(area.1, area.3, padding[0], port.1),
            ];
            if covering.iter().any(Option::is_some) {
                covers.push((f.node, covering));
            }
        }
        // A scroll container's content is its own, and so is that of a
        // box whose `scroll-snap-type` is not `none`, scroll container
        // or not: it captures the areas inside it (section 5.1).
        if (f.clips() && f.style.overflow().scrolls())
            || f.style.effects().scroll_snap_type.axis != SnapAxis::None
        {
            continue;
        }
        stack.extend(f.children.iter().rev().map(|c| (c, (x + c.x, y + c.y))));
    }
}

impl SnapContainer {
    /// Where it comes to rest after a scroll to `wanted`, across and
    /// down, from `from`, in the direction `direction` for a relative
    /// scroll (0 in an axis for none), and the areas it is then snapped
    /// to: an axis it does not snap in, or in which no area is near
    /// enough, stays where it was going, snapped to none.
    pub(crate) fn select(
        &self,
        wanted: (f32, f32),
        from: (f32, f32),
        direction: [i8; 2],
    ) -> ((f32, f32), [Option<NodeId>; 2]) {
        let mut at = [wanted.0, wanted.1];
        let mut snapped = [None, None];
        let wanted = [wanted.0, wanted.1];
        let from = [from.0, from.1];
        let (min, max) = ([self.min_x, 0.0], [self.max.0, self.max.1]);
        let port = [self.port.0, self.port.1];
        for axis in 0..2 {
            if !self.axes[axis] {
                continue;
            }
            // The aligned positions, and where an area larger than the
            // snapport covers it nearest where the scroll goes.
            let candidates: Vec<(NodeId, f32)> = self
                .areas
                .iter()
                .filter_map(|(node, offsets)| {
                    offsets[axis].map(|offset| (*node, offset.clamp(min[axis], max[axis])))
                })
                .chain(self.covers.iter().filter_map(|(node, ranges)| {
                    ranges[axis].map(|(low, high)| {
                        (
                            *node,
                            wanted[axis].clamp(low, high).clamp(min[axis], max[axis]),
                        )
                    })
                }))
                .collect();
            let ahead: Vec<(NodeId, f32)> = candidates
                .iter()
                .copied()
                .filter(|&(_, offset)| f32::from(direction[axis]) * (offset - from[axis]) > 0.5)
                .collect();
            // The first area of `scroll-snap-stop: always` the scroll
            // would pass, or reach, on its way.
            let way = f32::from(direction[axis]);
            let stop = ahead
                .iter()
                .copied()
                .filter(|&(node, offset)| {
                    self.stops.contains(&node) && way * (offset - wanted[axis]) <= 0.5
                })
                .min_by(|a, b| (way * (a.1 - from[axis])).total_cmp(&(way * (b.1 - from[axis]))));
            let pool = match stop {
                Some(stop) => vec![stop],
                None if direction[axis] != 0 && !ahead.is_empty() => ahead,
                None => candidates,
            };
            let nearest =
                pool.into_iter()
                    .fold(None, |best: Option<(NodeId, f32)>, c| match best {
                        Some(b) if (b.1 - wanted[axis]).abs() <= (c.1 - wanted[axis]).abs() => {
                            Some(b)
                        }
                        _ => Some(c),
                    });
            if let Some((node, offset)) = nearest
                && (self.mandatory || (offset - wanted[axis]).abs() <= PROXIMITY * port[axis])
            {
                at[axis] = offset;
                snapped[axis] = Some(node);
            }
        }
        ((at[0], at[1]), snapped)
    }

    /// Where it comes to rest, and the areas it is then snapped to, its
    /// offset going to `target`, across and down: when it rested at
    /// `snap` and was not scrolled since, back on the areas it was
    /// snapped to where they now are; else, after a scroll, where the
    /// scroll goes (in `direction`, a relative one's), and where an area
    /// it snapped to is gone or it snapped to none in an axis it snaps
    /// in, where it is.
    pub(crate) fn settle(
        &self,
        snap: Option<Snap>,
        target: (f32, f32),
        direction: [i8; 2],
    ) -> ((f32, f32), [Option<NodeId>; 2]) {
        match snap {
            Some(snap) if snap.at == target => {
                let now = self.resnap(snap.areas, target);
                if (0..2).all(|axis| !self.axes[axis] || now[axis].is_some()) {
                    (
                        (now[0].unwrap_or(target.0), now[1].unwrap_or(target.1)),
                        snap.areas,
                    )
                } else {
                    self.select(target, target, [0, 0])
                }
            }
            Some(snap) => self.select(target, snap.at, direction),
            None => self.select(target, target, [0, 0]),
        }
    }

    /// Where the areas `snapped` to, across and down, are now: the
    /// offset of each axis whose area is still one, `None` where it is
    /// gone or none was snapped to. An area larger than the snapport that
    /// still covers it at `at`, across and down, keeps it there.
    fn resnap(&self, snapped: [Option<NodeId>; 2], at: (f32, f32)) -> [Option<f32>; 2] {
        let (min, max) = ([self.min_x, 0.0], [self.max.0, self.max.1]);
        let at = [at.0, at.1];
        std::array::from_fn(|axis| {
            let wanted = snapped[axis]?;
            if !self.axes[axis] {
                return None;
            }
            let covering = self.covers.iter().find(|(node, _)| *node == wanted);
            if let Some((low, high)) = covering.and_then(|(_, ranges)| ranges[axis])
                && (low - 0.5..=high + 0.5).contains(&at[axis])
            {
                return Some(at[axis]);
            }
            self.areas
                .iter()
                .find(|(node, _)| *node == wanted)
                .and_then(|(_, offsets)| offsets[axis])
                .map(|offset| offset.clamp(min[axis], max[axis]))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A container snapping down, `mandatory` or not, with a 100px
    /// snapport, scrolling to 1000, its areas aligning at `at`.
    fn down(mandatory: bool, at: &[f32]) -> (SnapContainer, Vec<NodeId>) {
        let nodes: Vec<NodeId> = (0..at.len())
            .map(|i| NodeId::from_bits(i as u64 + 10).expect("an id"))
            .collect();
        let container = SnapContainer {
            scroller: None,
            smooth: false,
            axes: [false, true],
            mandatory,
            port: (100.0, 100.0),
            max: (0.0, 1000.0),
            min_x: 0.0,
            stops: Vec::new(),
            covers: Vec::new(),
            areas: nodes
                .iter()
                .zip(at)
                .map(|(&node, &y)| (node, [None, Some(y)]))
                .collect(),
        };
        (container, nodes)
    }

    #[test]
    fn a_scroll_goes_to_the_nearest_snap_position_or_the_next_one_its_way() {
        let (c, n) = down(true, &[0.0, 200.0, 400.0, 1200.0]);
        // The nearest, the first in tree order on a tie.
        assert_eq!(
            c.select((0.0, 290.0), (0.0, 0.0), [0, 0]),
            ((0.0, 200.0), [None, Some(n[1])])
        );
        assert_eq!(
            c.select((0.0, 300.0), (0.0, 0.0), [0, 0]),
            ((0.0, 200.0), [None, Some(n[1])])
        );
        // A relative scroll: the nearest past where it was, its way.
        assert_eq!(
            c.select((0.0, 230.0), (0.0, 200.0), [0, 1]),
            ((0.0, 400.0), [None, Some(n[2])])
        );
        assert_eq!(
            c.select((0.0, 170.0), (0.0, 200.0), [0, -1]),
            ((0.0, 0.0), [None, Some(n[0])])
        );
        // None that way: the nearest of all.
        assert_eq!(
            c.select((0.0, 999.0), (0.0, 1000.0), [0, 1]),
            ((0.0, 1000.0), [None, Some(n[3])])
        );
        // An area past the end aligns at the end; the axis it does not
        // snap in goes where it was going.
        assert_eq!(
            c.select((37.0, 900.0), (0.0, 0.0), [0, 0]),
            ((37.0, 1000.0), [None, Some(n[3])])
        );
        // Proximity: within 30px of the 100px snapport, else not at all.
        let (p, n) = down(false, &[0.0, 200.0]);
        assert_eq!(
            p.select((0.0, 175.0), (0.0, 0.0), [0, 0]),
            ((0.0, 200.0), [None, Some(n[1])])
        );
        assert_eq!(
            p.select((0.0, 120.0), (0.0, 0.0), [0, 0]),
            ((0.0, 120.0), [None, None])
        );
    }

    #[test]
    fn an_area_larger_than_the_snapport_may_rest_anywhere_it_covers_it() {
        let (mut c, n) = down(true, &[100.0, 600.0]);
        c.covers.push((n[0], [None, Some((100.0, 300.0))]));
        assert_eq!(
            c.select((0.0, 250.0), (0.0, 0.0), [0, 0]),
            ((0.0, 250.0), [None, Some(n[0])])
        );
        assert_eq!(
            c.select((0.0, 380.0), (0.0, 0.0), [0, 0]),
            ((0.0, 300.0), [None, Some(n[0])])
        );
        assert_eq!(
            c.select((0.0, 500.0), (0.0, 0.0), [0, 0]),
            ((0.0, 600.0), [None, Some(n[1])])
        );
    }

    #[test]
    fn a_relative_scroll_stops_at_an_area_that_says_always() {
        let (mut c, n) = down(true, &[0.0, 100.0, 200.0, 300.0]);
        c.stops.push(n[1]);
        // From 0 towards 290: the area at 100 is not passed.
        assert_eq!(
            c.select((0.0, 290.0), (0.0, 0.0), [0, 1]),
            ((0.0, 100.0), [None, Some(n[1])])
        );
        // Not in the way back from 300, nor for a scroll to a place.
        assert_eq!(
            c.select((0.0, 190.0), (0.0, 300.0), [0, -1]),
            ((0.0, 200.0), [None, Some(n[2])])
        );
        assert_eq!(
            c.select((0.0, 290.0), (0.0, 0.0), [0, 0]),
            ((0.0, 300.0), [None, Some(n[3])])
        );
    }

    #[test]
    fn a_container_not_scrolled_since_goes_back_to_its_areas() {
        let (c, n) = down(true, &[0.0, 250.0]);
        // Not scrolled since it rested at 200 on the second: where that
        // is now.
        let snap = Snap {
            areas: [None, Some(n[1])],
            at: (0.0, 200.0),
        };
        assert_eq!(
            c.settle(Some(snap), (0.0, 200.0), [0, 0]),
            ((0.0, 250.0), [None, Some(n[1])])
        );
        // Scrolled since: where the scroll went.
        assert_eq!(
            c.settle(Some(snap), (0.0, 20.0), [0, 0]),
            ((0.0, 0.0), [None, Some(n[0])])
        );
        // Its area gone, or none snapped to: afresh where it is.
        let (gone, m) = down(true, &[0.0, 180.0]);
        assert_eq!(
            gone.settle(Some(snap), (0.0, 200.0), [0, 0]),
            ((0.0, 180.0), [None, Some(m[1])])
        );
        let unsnapped = Snap {
            areas: [None, None],
            at: (0.0, 200.0),
        };
        assert_eq!(
            c.settle(Some(unsnapped), (0.0, 200.0), [0, 0]),
            ((0.0, 250.0), [None, Some(n[1])])
        );
        // The first time: where it is.
        assert_eq!(
            c.settle(None, (0.0, 0.0), [0, 0]),
            ((0.0, 0.0), [None, Some(n[0])])
        );
    }
}
