//! Floats (CSS2 §9.5, roadmap M11): the float context of one block
//! formatting context, and where the content being laid out is in it.
//!
//! A block formatting context owns the floats placed in it, each as its
//! margin box in the context root's content-box coordinates (the root's
//! content top-left is `(0, 0)`), left or right, in placement order.
//! [`FloatContext::place`] is CSS2 §9.5.1's rules in their order;
//! [`FloatContext::space`] answers a line, or a float being placed, with
//! the room between the floats at a height. A new root (an `overflow`
//! other than `visible`, an inline-block, a float, the initial
//! containing block) starts a fresh, empty context, which is why floats
//! never leak out of one.
//!
//! The flow position is tracked in the root's coordinates alongside the
//! relative positions `block` computes ([`At`]): a float's top is the
//! flow position it is met at, after the margins resolved so far. While
//! the margins above a box are still collapsing (CSS2 §8.3.1 — no
//! border, padding or content has separated them yet), the box's
//! position is not known, and a float met there waits
//! ([`FloatContext::pending`]) until the first content below it fixes
//! the position, at which it is placed: a float in an empty block
//! sits level with the content that ends the collapsing, as in
//! browsers. A float's fragment keeps its place under its parent in the
//! fragment tree; since the root coordinates of a float may be known
//! only after its parent's fragment is built, every float fragment is
//! positioned at the end of the root's layout by [`attach_floats`].

use dom::NodeId;
use style::computed::{Clear, Float};
use style::selector_matching::PseudoElement;

use crate::block::{AdjoiningMargins, Fragment, FragmentKind};

/// The floats of one block formatting context.
#[derive(Debug, Default, Clone)]
pub(crate) struct FloatContext {
    /// The floats placed, in placement order.
    placed: Vec<PlacedFloat>,
    /// Floats met while the flow position was unresolved, in order,
    /// waiting for the content that resolves it.
    pending: Vec<PendingFloat>,
}

/// A float laid out and sized, before it is placed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PendingFloat {
    /// The float's element, which its fragment carries, and the
    /// pseudo-element that generated it, if one did (M16): together
    /// they tell the floats of one context apart, as an element's
    /// `::before` and `::after` name it both.
    pub(crate) node: NodeId,
    pub(crate) generated: Option<PseudoElement>,
    pub(crate) side: Float,
    /// The floats it goes below (CSS2 §9.5.2 on a float).
    pub(crate) clear: Clear,
    /// The margin box's size.
    pub(crate) w: f32,
    pub(crate) h: f32,
    /// Where the border box sits inside the margin box.
    pub(crate) margin_left: f32,
    pub(crate) margin_top: f32,
    /// The move `position: relative` gives the float's fragment once
    /// placed (CSS2 §9.4.3); the context keeps the margin box unmoved,
    /// so nothing else is affected.
    pub(crate) relative: (f32, f32),
    /// The containing block's content box, left and right edges, in
    /// root coordinates: what the float is placed between.
    pub(crate) cb_left: f32,
    pub(crate) cb_right: f32,
}

/// A float placed: its margin box, and its border box's top-left, in
/// root coordinates — the latter where the fragment goes, moved by
/// `position: relative` when the float has it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlacedFloat {
    node: NodeId,
    generated: Option<PseudoElement>,
    side: Float,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    border_x: f32,
    border_y: f32,
}

impl PlacedFloat {
    fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// Whether the float's margin box takes room in the band from `y`
    /// of height `h`. A float of no height takes no vertical room and
    /// nothing avoids it; a query of no height is a point, met by the
    /// floats whose band holds it, their top edge included — what a
    /// line in a font-less container, as tall as its items, asks with.
    fn in_band(&self, y: f32, h: f32) -> bool {
        if self.h <= 0.0 {
            return false;
        }
        if h > 0.0 {
            self.y < y + h && y < self.bottom()
        } else {
            self.y <= y && y < self.bottom()
        }
    }
}

impl FloatContext {
    /// The room between the floats in the band from `y` of height `h`,
    /// within the containing block's `[cb_left, cb_right]`: the left
    /// edge is right of every left float in the band, the right edge
    /// left of every right float there. `right` may end up left of
    /// `left` when the floats overlap each other's space.
    pub(crate) fn space(&self, y: f32, h: f32, cb_left: f32, cb_right: f32) -> (f32, f32) {
        let mut left = cb_left;
        let mut right = cb_right;
        for f in self.placed.iter().filter(|f| f.in_band(y, h)) {
            match f.side {
                Float::Left => left = left.max(f.x + f.w),
                Float::Right => right = right.min(f.x),
                Float::None => unreachable!("a placed float has a side"),
            }
        }
        (left, right)
    }

    /// The next height below `y` at which a float ends, where the room
    /// may widen; `None` when no float reaches below `y`.
    pub(crate) fn next_bottom(&self, y: f32) -> Option<f32> {
        self.placed
            .iter()
            .map(PlacedFloat::bottom)
            .filter(|b| *b > y)
            .reduce(f32::min)
    }

    /// The bottom margin edge of the lowest float, what a context
    /// root's `auto` height reaches down to (CSS2 §10.6.7); `None`
    /// with no float placed.
    pub(crate) fn lowest_bottom(&self) -> Option<f32> {
        self.placed.iter().map(PlacedFloat::bottom).reduce(f32::max)
    }

    /// The bottom margin edge of the lowest float a box with `clear`
    /// must be below (CSS2 §9.5.2): the left floats, the right ones,
    /// or all; `None` with nothing to clear.
    pub(crate) fn clear_bottom(&self, clear: Clear) -> Option<f32> {
        self.placed
            .iter()
            .filter(|p| match clear {
                Clear::None => false,
                Clear::Left => p.side == Float::Left,
                Clear::Right => p.side == Float::Right,
                Clear::Both => true,
            })
            .map(PlacedFloat::bottom)
            .reduce(f32::max)
    }

    /// Whether any float is placed: with none, nothing dodges or
    /// clears.
    pub(crate) fn is_empty(&self) -> bool {
        self.placed.is_empty()
    }

    /// Whether the context holds no float at all, placed or waiting: a
    /// box laid out in it then neither meets a float nor is moved by
    /// one, which the layout cache relies on (M13.2).
    pub(crate) fn is_untouched(&self) -> bool {
        self.placed.is_empty() && self.pending.is_empty()
    }

    /// Whether a float still waiting for the flow position is one a
    /// box with `clear` would have to be below: such a box resolves the
    /// position, since it cannot collapse its margins past them.
    pub(crate) fn waits_to_be_cleared(&self, clear: Clear) -> bool {
        self.pending.iter().any(|f| match clear {
            Clear::None => false,
            Clear::Left => f.side == Float::Left,
            Clear::Right => f.side == Float::Right,
            Clear::Both => true,
        })
    }

    /// Places `f` with its margin box's top at `top` or as high below
    /// it as CSS2 §9.5.1 allows, and records it. Its top is never
    /// above an earlier float's (rule 5), nor above the bottom of the
    /// floats its `clear` names (§9.5.2); its side edge is at the
    /// containing block's edge, or past the earlier floats of its side
    /// in the band (rules 1, 2 and 3); when the room between the two
    /// sides' floats is narrower than the float, it moves down to the
    /// next height at which a float ends and tries again (rule 8: as
    /// high as possible, then rule 9: as far to its side as possible);
    /// a float too wide for its containing block with no float beside
    /// it is placed at the block's edge and overflows (rule 7).
    /// Returns the margin box's top-left.
    pub(crate) fn place(&mut self, f: PendingFloat, top: f32) -> (f32, f32) {
        let top = self.clear_bottom(f.clear).map_or(top, |b| top.max(b));
        let mut y = self.placed.iter().map(|p| p.y).fold(top, f32::max);
        let (x, y) = loop {
            let (left, right) = self.space(y, f.h, f.cb_left, f.cb_right);
            let fits = right - left >= f.w;
            let alone = left == f.cb_left && right == f.cb_right;
            if fits || alone {
                let x = match f.side {
                    Float::Right => right - f.w,
                    Float::Left | Float::None => left,
                };
                break (x, y);
            }
            match self.next_bottom(y) {
                Some(next) if next > y => y = next,
                _ => {
                    // No float ends below: the room cannot widen. Only
                    // reachable with a float of no height in the band,
                    // which takes no vertical room by `in_band`, so the
                    // band is `alone` above; kept for a safe exit.
                    let x = match f.side {
                        Float::Right => right - f.w,
                        Float::Left | Float::None => left,
                    };
                    break (x, y);
                }
            }
        };
        self.placed.push(PlacedFloat {
            node: f.node,
            generated: f.generated,
            side: f.side,
            x,
            y,
            w: f.w,
            h: f.h,
            border_x: x + f.margin_left + f.relative.0,
            border_y: y + f.margin_top + f.relative.1,
        });
        (x, y)
    }

    /// Holds `f` until the flow position resolves.
    pub(crate) fn defer(&mut self, f: PendingFloat) {
        self.pending.push(f);
    }

    /// Places every waiting float at `top`, the flow position just
    /// resolved, in the order they were met.
    pub(crate) fn resolve_pending(&mut self, top: f32) {
        for f in std::mem::take(&mut self.pending) {
            self.place(f, top);
        }
    }

    /// The placed floats, for [`attach_floats`].
    pub(crate) fn placed(&self) -> &[PlacedFloat] {
        &self.placed
    }
}

/// Where the content being laid out is in its block formatting context
/// root's content-box coordinates: what a float is placed at and a line
/// is shortened by. Copied down the recursion; the context itself is
/// shared.
#[derive(Debug, Clone, Copy)]
pub(crate) struct At {
    /// Root x of the content box being laid out.
    pub(crate) x: f32,
    /// Root y of the content box's top, once the margins above it have
    /// resolved; `None` while they are still collapsing, when a float
    /// met here waits for the first content below it.
    pub(crate) origin: Option<f32>,
    /// The root y the pending margins hang from: the bottom of the last
    /// content, or the content top of the last box whose top edge
    /// separates its margins from its children's.
    pub(crate) anchor: f32,
    /// The margins pending below `anchor`, not yet resolved.
    pub(crate) strut: AdjoiningMargins,
}

impl At {
    /// The top of a context root's content box.
    pub(crate) fn root() -> Self {
        At {
            x: 0.0,
            origin: Some(0.0),
            anchor: 0.0,
            strut: AdjoiningMargins::default(),
        }
    }

    /// The flow position: where the next content's top would be, with
    /// the margins pending so far resolved.
    pub(crate) fn position(&self) -> f32 {
        self.anchor + self.strut.resolve()
    }
}

/// Positions every float fragment under `fragments` from where its
/// context placed it. `fragments` are relative to a box at `(px, py)`
/// in a coordinate system whose origin is `(dx, dy)` in the root's: for
/// a root's children, relative to its border box, `(dx, dy)` is its
/// content box's top-left in that border box, and `(px, py)` is zero.
/// A float's own content is another context's and is not entered, nor
/// is any other root's.
pub(crate) fn attach_floats(
    fragments: &mut [Fragment],
    placed: &[PlacedFloat],
    px: f32,
    py: f32,
    dx: f32,
    dy: f32,
) {
    for f in fragments {
        if !matches!(f.kind, FragmentKind::Box) {
            continue;
        }
        if f.style.float != Float::None
            && let Some(p) = placed
                .iter()
                .find(|p| p.node == f.node && p.generated == f.generated)
        {
            f.x = p.border_x + dx - px;
            f.y = p.border_y + dy - py;
        }
        if !crate::block::establishes_bfc(&f.style) {
            attach_floats(&mut f.children, placed, px + f.x, py + f.y, dx, dy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn float(side: Float, w: f32, h: f32) -> PendingFloat {
        PendingFloat {
            node: crate::testing::no_node(),
            generated: None,
            side,
            clear: Clear::None,
            w,
            h,
            margin_left: 0.0,
            margin_top: 0.0,
            relative: (0.0, 0.0),
            cb_left: 0.0,
            cb_right: 100.0,
        }
    }

    #[test]
    fn floats_line_up_on_their_side_and_drop_when_they_do_not_fit() {
        let mut ctx = FloatContext::default();
        // Two 40px left floats side by side, 10 and 20 tall; a third
        // does not fit beside them (rule 2 and 7), nor beside the
        // second alone at 10, and goes below both at 20, as far left as
        // possible there (rule 9).
        assert_eq!(ctx.place(float(Float::Left, 40.0, 10.0), 0.0), (0.0, 0.0));
        assert_eq!(ctx.place(float(Float::Left, 40.0, 20.0), 0.0), (40.0, 0.0));
        assert_eq!(ctx.place(float(Float::Left, 40.0, 10.0), 0.0), (0.0, 20.0));
        // A right float met at 10 is placed no higher than the third's
        // top (rule 5), against the right edge.
        assert_eq!(
            ctx.place(float(Float::Right, 30.0, 10.0), 10.0),
            (70.0, 20.0)
        );
        // A small left float met at 10 goes between the third (to 40)
        // and the right one (from 70), at 20.
        assert_eq!(ctx.place(float(Float::Left, 5.0, 5.0), 10.0), (40.0, 20.0));
        assert_eq!(ctx.lowest_bottom(), Some(30.0));
    }

    #[test]
    fn a_float_is_never_above_an_earlier_one_and_a_wide_one_overflows() {
        let mut ctx = FloatContext::default();
        ctx.place(float(Float::Left, 10.0, 10.0), 30.0);
        // Met at 0, placed at 30 (rule 5), beside the first.
        assert_eq!(ctx.place(float(Float::Left, 10.0, 10.0), 0.0), (10.0, 30.0));
        // Too wide for the block with nothing beside it at 40: at the
        // left edge, overflowing (rule 7); a right one at the right
        // edge, overflowing to the left.
        assert_eq!(ctx.place(float(Float::Left, 150.0, 5.0), 40.0), (0.0, 40.0));
        assert_eq!(
            ctx.place(float(Float::Right, 150.0, 5.0), 45.0),
            (-50.0, 45.0)
        );
        // Too wide beside a float: below it.
        let mut ctx = FloatContext::default();
        ctx.place(float(Float::Left, 10.0, 10.0), 0.0);
        assert_eq!(ctx.place(float(Float::Right, 95.0, 5.0), 0.0), (5.0, 10.0));
    }

    #[test]
    fn the_space_between_floats_and_the_next_bottom() {
        let mut ctx = FloatContext::default();
        ctx.place(float(Float::Left, 20.0, 50.0), 0.0);
        ctx.place(float(Float::Right, 30.0, 20.0), 0.0);
        assert_eq!(ctx.space(0.0, 10.0, 0.0, 100.0), (20.0, 70.0));
        assert_eq!(ctx.space(20.0, 10.0, 0.0, 100.0), (20.0, 100.0));
        assert_eq!(ctx.space(50.0, 10.0, 0.0, 100.0), (0.0, 100.0));
        // A band ending where a float starts does not meet it; a query
        // of no height is a point, met from a float's top edge on.
        assert_eq!(ctx.space(-10.0, 10.0, 0.0, 100.0), (0.0, 100.0));
        assert_eq!(ctx.space(0.0, 0.0, 0.0, 100.0), (20.0, 70.0));
        assert_eq!(ctx.space(20.0, 0.0, 0.0, 100.0), (20.0, 100.0));
        assert_eq!(ctx.space(50.0, 0.0, 0.0, 100.0), (0.0, 100.0));
        assert_eq!(ctx.next_bottom(0.0), Some(20.0));
        assert_eq!(ctx.next_bottom(20.0), Some(50.0));
        assert_eq!(ctx.next_bottom(50.0), None);
        // A float of no height takes no room, from a band or a point.
        ctx.place(float(Float::Left, 100.0, 0.0), 60.0);
        assert_eq!(ctx.space(55.0, 10.0, 0.0, 100.0), (0.0, 100.0));
        assert_eq!(ctx.space(60.0, 0.0, 0.0, 100.0), (0.0, 100.0));
        assert_eq!(ctx.lowest_bottom(), Some(60.0));
        assert_eq!(FloatContext::default().lowest_bottom(), None);
    }

    #[test]
    fn a_float_with_clear_goes_below_the_floats_it_names() {
        let mut ctx = FloatContext::default();
        ctx.place(float(Float::Left, 10.0, 30.0), 0.0);
        ctx.place(float(Float::Right, 10.0, 50.0), 0.0);
        assert_eq!(ctx.clear_bottom(Clear::None), None);
        assert_eq!(ctx.clear_bottom(Clear::Left), Some(30.0));
        assert_eq!(ctx.clear_bottom(Clear::Right), Some(50.0));
        assert_eq!(ctx.clear_bottom(Clear::Both), Some(50.0));
        let cleared = |clear: Clear| PendingFloat {
            clear,
            ..float(Float::Left, 10.0, 10.0)
        };
        assert_eq!(ctx.place(cleared(Clear::Left), 0.0), (0.0, 30.0));
        assert_eq!(ctx.place(cleared(Clear::Both), 0.0), (0.0, 50.0));
        // Already past them: placed where it is met.
        assert_eq!(ctx.place(cleared(Clear::Both), 70.0), (0.0, 70.0));
        assert!(!ctx.is_empty());
        assert!(FloatContext::default().is_empty());
    }

    #[test]
    fn pending_floats_are_placed_together_where_the_flow_resolves() {
        let mut ctx = FloatContext::default();
        ctx.defer(float(Float::Left, 10.0, 10.0));
        ctx.defer(float(Float::Left, 10.0, 10.0));
        assert!(ctx.placed().is_empty());
        ctx.resolve_pending(30.0);
        let placed: Vec<(f32, f32)> = ctx.placed().iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(placed, vec![(0.0, 30.0), (10.0, 30.0)]);
        // Resolved again: nothing waits any more.
        ctx.resolve_pending(60.0);
        assert_eq!(ctx.placed().len(), 2);
    }
}
