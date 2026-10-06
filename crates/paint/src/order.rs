//! The paint order (M11.6, steps 2 and 8 of
//! `docs/design/floats-positioning.md`): one walk over the fragment
//! tree that says what is painted, where, in which order and under
//! which clip. [`paint_fragments_in`](crate::paint_fragments_in) turns
//! the steps into display items, and the engine's hit test reads the
//! same steps — the last one under the pointer is the element hit — so
//! paint order and hit order have one definition.
//!
//! The order is CSS2 Appendix E's. The root element's box is the root
//! stacking context, and a positioned box whose `z-index` is not `auto`
//! makes one of its own; so does a box with a transform, a
//! `perspective`, `transform-style: preserve-3d`, an `opacity` below
//! one or a `will-change` naming any of them (css-transforms-1 §2,
//! css-transforms-2 §9, §10, css-color-4 §3.2, css-will-change-1 §2.2;
//! M19), painted at the layer its `z-index` gives it when it is
//! positioned or a flex or grid item, and at the `0` layer, in tree
//! order, otherwise. Within a stacking context: its own background
//! and border; the stacking contexts in it with a negative `z-index`,
//! in `z-index` then tree order; then its in-flow content in three
//! passes over the tree — the backgrounds and borders of its block-level
//! boxes, then its floats, then its inline-level content, inline boxes'
//! backgrounds and borders, text and inline-blocks; then its positioned
//! descendants with `z-index: auto`
//! or `0`, in tree order; and the stacking contexts with a positive
//! `z-index`, in `z-index` then tree order. A float, an inline-block and
//! a positioned box with `z-index: auto` are painted as units, each its
//! own background then its content in the three passes, as if it were a
//! stacking context, except that the positioned boxes and stacking
//! contexts inside it belong to the real stacking context around it. A
//! box painted out of tree order is still under the clips and scroll
//! offsets of its ancestors in the fragment tree, which are those of
//! its containing block's: the walk closes the clips in force down to
//! the ones it shares and opens its own around it.
//!
//! Tree order is the boxes' own, which each fragment carries (M17,
//! `Fragment::tree_order`): layout moves an absolutely positioned box
//! to its containing block's fragment or among the roots, but the
//! layers are sorted by it all the same. A flex or grid container's
//! children are in order-modified document order there, and its items
//! paint as inline-blocks do, as units in the inline pass, an item whose
//! `z-index` is not `auto` making a stacking context painted at its
//! layer, positioned or not (css-flexbox-1 §4.3, css-grid-2 §6.5): grid
//! items in the same cells overlap in that order.
//!
//! A box with `transform-style: preserve-3d` whose parent's is `flat`
//! establishes a 3D rendering context (css-transforms-2 §9, M19.5),
//! and a child with `preserve-3d` extends it: the stacking contexts
//! among the children of either are the context's members, each with
//! its accumulated matrix (§5) — its own, under its parent's
//! `perspective` (§10), under the matrices of the members above it —
//! and they are painted at the root's level, around the root's own
//! plane, in the order of their depth, furthest first, the plane at
//! its own depth: a painter's order, with no splitting where planes
//! cross. A member whose `backface-visibility` is `hidden` and whose
//! accumulated matrix turns it away is not painted at all. Every
//! group's matrix is flattened onto the plane for the steps.
//!
//! Every rectangle a step carries is in the coordinates of the group
//! it is in ([`Step::PushGroup`], M19), and the groups' matrices lead
//! to the viewport's: the document's content is in groups of its own
//! whose matrix is the viewport's scroll offset, so the rectangles
//! inside are in document coordinates, the same from frame to frame as
//! the document scrolls; inside a scroll container its offset (M10.5)
//! is applied as the rectangles are listed, and so is the shift of a
//! `position: sticky` box ([`sticky_shift`], M11), which paint and hit
//! testing thereby agree on. A `position: fixed` box is placed against
//! the viewport and left out of the scroll: it is listed outside the
//! document's groups, in viewport coordinates, cutting the document's
//! content into groups around it — unless it is inside a box that is a
//! containing block for all its descendants, a transformed one (M19),
//! with which it scrolls, or inside a group that is not, an opacity
//! group, where it is listed in the group's coordinates at the place the
//! scroll offset gives it. A clip's rectangle is the effective one,
//! intersected with the clips around it, so a consumer needs only the
//! innermost. Content the viewport cannot show is left out: a box or a
//! line it does not meet has no step, and a clipping box it does not
//! meet is skipped with its whole subtree, since nothing under a clip
//! can show where the clip does not; a clip with no step inside it is
//! left out too.

use std::collections::{HashMap, HashSet};

use dom::NodeId;
use layout::{Fragment, FragmentKind, StackingKey};
use style::computed::{
    BOTTOM, BackfaceVisibility, ComputedStyle, Float, LEFT, LengthPercentageAuto, Overflow,
    Position, RIGHT, TOP, TransformStyle,
};
use style::transform::Matrix;

use crate::{GroupKey, LayerReason, ScrollOffsets, Viewport};
use style::computed::WillChange;

/// An axis-aligned rectangle in viewport coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// Whether the point is inside: the left and top edges are, the
    /// right and bottom ones are not, as `Fragment::contains` has it.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    /// Whether the two rectangles share any area.
    pub fn meets(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && self.x + self.w > other.x
            && self.y < other.y + other.h
            && self.y + self.h > other.y
    }

    /// The common part of the two, empty (never negative) when they do
    /// not meet.
    pub fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.w).min(other.x + other.w);
        let bottom = (self.y + self.h).min(other.y + other.h);
        Rect {
            x,
            y,
            w: (right - x).max(0.0),
            h: (bottom - y).max(0.0),
        }
    }
}

/// One step of the paint order.
#[derive(Debug, Clone, Copy)]
pub enum Step<'a> {
    /// A box's own background and border, over its border box `rect`.
    Box { fragment: &'a Fragment, rect: Rect },
    /// A text run: `rect` is its line box, `baseline` where its glyphs
    /// sit.
    Text {
        fragment: &'a Fragment,
        rect: Rect,
        baseline: f32,
    },
    /// Everything up to the matching [`Step::PopClip`] shows only inside
    /// `rect`, the padding box of the clipping box intersected with the
    /// clips around it (CSS2 §11.1.1). `node` is the element whose box
    /// it is, and `None` for a box an element's `::before` or `::after`
    /// generated (M16): scroll offsets are kept by element, and a
    /// generated box's is its element's own, so it clips and does not
    /// scroll. The clipping box's content is scrolled by its offset;
    /// the box, its clip and its scrollbar are not.
    PushClip { node: Option<NodeId>, rect: Rect },
    /// The end of the innermost [`Step::PushClip`].
    PopClip,
    /// Everything up to the matching [`Step::PopGroup`] is a group
    /// (M19): the stacking context of `fragment`, a box with a
    /// transform or an opacity below one — its own step, its content
    /// and its positioned descendants — or, with no fragment, a piece
    /// of the document's scrolled content. The steps inside are in the
    /// group's own coordinates, those the box would have with no
    /// transform, or document coordinates, and `matrix` maps them to
    /// the coordinates the group sits in: the box's own matrix about
    /// its border box where the walk has it, under its parent's
    /// perspective and the matrices of a 3D rendering context above
    /// it, flattened onto the plane (M19.5); or the viewport's scroll
    /// offset; the group is composited with `opacity`. A box whose
    /// matrix is singular is not displayed at all, and has no steps.
    PushGroup {
        fragment: Option<&'a Fragment>,
        matrix: Matrix,
        opacity: f32,
        /// What names the group across frames: the box's own; the
        /// scroll group of a scroll container whose `will-change` names
        /// `scroll-position`, whose matrix is its offset; or the
        /// document's content, one piece of it.
        key: GroupKey,
        /// Whether the group is a layer, and why (the compositing
        /// note's policy).
        layer: Option<LayerReason>,
    },
    /// The end of the innermost [`Step::PushGroup`].
    PopGroup,
    /// A scroll container's place for its overlay scrollbar (M10.5):
    /// over its padding box `rect`, scrolled to `offset`. An indicator,
    /// not a control: hit testing passes over it.
    Scrollbar {
        fragment: &'a Fragment,
        rect: Rect,
        offset: f32,
    },
}

/// The paint order of `roots` for `viewport`, with each scroll
/// container scrolled as `offsets` says.
pub fn paint_order<'a>(
    roots: &'a [Fragment],
    viewport: Viewport,
    offsets: &dyn ScrollOffsets,
) -> Vec<Step<'a>> {
    paint_order_in(roots, viewport, offsets, 0.0)
}

/// [`paint_order`] with the content `margin` pixels above and below the
/// viewport listed as well (M19): what a scroll brings into view next,
/// drawn ahead of it into the tiles of the document's layer. The
/// margin is in viewport coordinates, around the viewport itself; a
/// `position: fixed` box, which does not scroll, is listed only when
/// the viewport meets it.
///
/// The document's content is in groups keyed [`GroupKey::Document`],
/// numbered from zero in order, whose matrix is the scroll offset
/// negated and whose steps are in document coordinates; a fixed box
/// placed against the viewport is listed between two of them, in
/// viewport coordinates. A group with nothing in it is left out.
pub fn paint_order_in<'a>(
    roots: &'a [Fragment],
    viewport: Viewport,
    offsets: &dyn ScrollOffsets,
    margin: f32,
) -> Vec<Step<'a>> {
    let mut walk = Walk {
        steps: Vec::new(),
        viewport: Rect {
            x: 0.0,
            y: -margin,
            w: viewport.width,
            h: viewport.height + 2.0 * margin,
        },
        window: Rect {
            x: 0.0,
            y: 0.0,
            w: viewport.width,
            h: viewport.height,
        },
        scroll: viewport.scroll_y,
        dx: 0.0,
        dy: 0.0,
        chain: Vec::new(),
        groups: Vec::new(),
        contained: 0,
        fixed: 0,
        segment: None,
        segments: 0,
        adopted: HashMap::new(),
        taken: HashSet::new(),
        offsets,
    };
    // The roots' containing block is the initial one: as wide as the
    // viewport, from the document's top down without end.
    let initial = Rect {
        x: 0.0,
        y: 0.0,
        w: viewport.width,
        h: f32::INFINITY,
    };
    walk.open_segment();
    match roots.split_first() {
        // The first root is the root element's box, the root stacking
        // context; the roots after it — the boxes the initial
        // containing block positions and the fixed ones (M11) — are
        // positioned descendants of the root element that layout moved
        // out of its subtree, and belong to its stacking context.
        Some((root, rest)) if matches!(root.kind, FragmentKind::Box) => {
            walk.stacking(root, initial, rest, Space::FLAT);
        }
        // Not a box tree at all: whatever is there, in flow.
        Some(_) => {
            for phase in PHASES {
                walk.phase(roots, initial, phase, false);
            }
        }
        None => {}
    }
    // A box handed to a stacking context the walk never reached — one
    // inside a clipping box the viewport does not meet — is still
    // positioned by a containing block the viewport may meet: painted
    // last, in its layer's order, rather than lost.
    let mut left: Vec<Positioned<'a>> = walk.adopted.drain().flat_map(|(_, p)| p).collect();
    left.sort_by_key(|p| (p.z(), p.f.tree_order));
    for p in left {
        walk.positioned(p);
    }
    walk.close_segment();
    tidy_segments(walk.steps)
}

/// `steps` with the document's groups tidied: one with nothing in it
/// is left out, two with nothing between them are one, and the ones
/// left are numbered from zero in order. The walk closes a group
/// before a fixed box and opens another after it, whether or not the
/// box or anything after it is listed.
fn tidy_segments(steps: Vec<Step<'_>>) -> Vec<Step<'_>> {
    let is_segment = |step: &Step<'_>| {
        matches!(
            step,
            Step::PushGroup {
                key: GroupKey::Document(_),
                ..
            }
        )
    };
    let mut out: Vec<Step<'_>> = Vec::with_capacity(steps.len());
    // Whether each group open in `out` is a document group, innermost
    // last; and whether the last step of `out` ended one.
    let mut open: Vec<bool> = Vec::new();
    let mut just_closed = false;
    for step in steps {
        match step {
            Step::PushGroup { .. } => {
                let segment = is_segment(&step);
                if segment && just_closed {
                    // The one just ended goes on instead.
                    out.pop();
                    open.push(true);
                    just_closed = false;
                    continue;
                }
                open.push(segment);
                out.push(step);
            }
            Step::PopGroup => {
                let segment = open.pop().unwrap_or(false);
                if segment && out.last().is_some_and(is_segment) {
                    out.pop();
                    just_closed = false;
                    continue;
                }
                out.push(step);
                just_closed = segment;
                continue;
            }
            step => out.push(step),
        }
        just_closed = false;
    }
    let mut n = 0;
    for step in &mut out {
        if let Step::PushGroup {
            key: GroupKey::Document(number),
            ..
        } = step
        {
            *number = n;
            n += 1;
        }
    }
    out
}

/// The walk's state: where the document is relative to the viewport,
/// and the clips in force.
struct Walk<'a, 'o> {
    steps: Vec<Step<'a>>,
    /// What is listed: the viewport and the margin around it.
    viewport: Rect,
    /// The viewport itself, which a fixed box is listed for.
    window: Rect,
    /// The viewport's scroll offset: the document's groups' matrix.
    scroll: f32,
    /// From a document `x` to the coordinates of the group being walked
    /// into: the shifts of the sticky boxes being walked into (M11).
    dx: f32,
    /// From a document `y` to the coordinates of the group being walked
    /// into: the offsets of the scroll containers being walked into,
    /// negated, and the shifts of the sticky boxes; inside a fixed box
    /// placed against the viewport, from the viewport's `y`.
    dy: f32,
    /// The clips in force, outermost first: each clipping box being
    /// walked into, as [`Step::PushClip`] names it, with its effective
    /// clip rectangle and how many groups deep it was met (M19): a clip
    /// is intersected with the ones around it in the same group, whose
    /// coordinates it shares, and the consumers of the steps intersect
    /// across groups themselves, each in its own coordinates.
    chain: Vec<(Option<NodeId>, Rect, usize)>,
    /// The groups being walked into (M19), outermost first: each one's
    /// matrix composed with those around it, from its coordinates to the
    /// viewport's, for what the viewport meets.
    groups: Vec<Matrix>,
    /// How many boxes that are containing blocks for all their
    /// descendants (M19) are being walked into: a `position: fixed` box
    /// inside one scrolls with it.
    contained: u32,
    /// How many `position: fixed` boxes placed against the viewport are
    /// being walked into.
    fixed: u32,
    /// Where the open group of the document's content begins in
    /// `steps`, if one is open (M19).
    segment: Option<usize>,
    /// How many groups of the document's content have been opened.
    segments: u32,
    /// The positioned boxes met under one stacking context that belong
    /// to another, the one their [`Fragment::stacking_parent`] names
    /// (M19), with the walk's state where they were met, waiting for
    /// that context to be painted. A context's own subtree is collected
    /// before its layers are painted, and the context a box belongs to
    /// is always inside the one whose subtree holds its fragment, so
    /// the box is here before its context looks.
    adopted: HashMap<StackingKey, Vec<Positioned<'a>>>,
    /// The members of 3D rendering contexts taken by their roots
    /// (M19.5): a stacking context that extends a context paints none
    /// of them itself.
    taken: HashSet<StackingKey>,
    offsets: &'o dyn ScrollOffsets,
}

/// A positioned descendant of a stacking context, or a stacking context
/// that is not positioned (M19), taken out of tree order to be painted
/// at its layer (CSS2 Appendix E), with the walk's state where the tree
/// has it: the clips and scroll offsets of its ancestors there, its
/// containing block for a sticky shift, whether it is a flex or grid
/// item, whose `z-index` applies, and its place in a 3D rendering
/// context.
struct Positioned<'a> {
    f: &'a Fragment,
    containing: Rect,
    chain: Vec<(Option<NodeId>, Rect, usize)>,
    dx: f32,
    dy: f32,
    contained: u32,
    item: bool,
    space: Space,
}

/// What a box's parent and the 3D rendering context around it give
/// its matrix (M19.5, css-transforms-2 §5, §9, §10): the parent's
/// perspective, about the parent's border box in the walk's
/// coordinates, multiplied before the box's own matrix; whether the box
/// takes part in a 3D rendering context, its parent's used
/// `transform-style` being `preserve-3d`, which makes it a member when
/// it is a stacking context; and the accumulated matrix of the
/// context above the box, the root's full matrix down to the parent's,
/// multiplied before the perspective, the identity outside a context.
/// The parent is the box's parent in the box tree, `Fragment::parent`,
/// not the fragment its own hangs under: an absolutely positioned box
/// is under its containing block's fragment, while its parent's
/// perspective and context are what reach it.
#[derive(Debug, Clone, Copy)]
struct Space {
    perspective: Option<Matrix>,
    in_3d: bool,
    ahead: Matrix,
}

impl Space {
    /// Outside every 3D rendering context, under no perspective.
    const FLAT: Space = Space {
        perspective: None,
        in_3d: false,
        ahead: Matrix::IDENTITY,
    };

    /// What the children of the box `node`, which gives `self`, give a
    /// box under its fragment: `self` for the box's own children, and
    /// nothing for a box whose parent is another.
    fn for_child(self, f: &Fragment, node: NodeId) -> Space {
        if f.parent == Some(node) {
            self
        } else {
            Space::FLAT
        }
    }

    /// The full matrix of the box `f`, at `rect` in the walk's
    /// coordinates with a containing block `cb_width` wide: its own
    /// matrix about its border box, under the parent's perspective,
    /// under the context's accumulated matrix. `None` for a box with
    /// no transform outside any context, whose group there is none of;
    /// inside one, a member with no transform of its own still has the
    /// context's.
    fn full(&self, f: &Fragment, rect: Rect, cb_width: f32) -> Option<Matrix> {
        if f.matrix(cb_width).is_none() && !self.in_3d {
            return None;
        }
        Some(self.accumulated(f, rect, cb_width))
    }

    /// The accumulated matrix of the box `f` (css-transforms-2 §5), as
    /// [`Space::full`] has it, the identity standing in for a box with
    /// no transform: what a box that establishes a 3D rendering context
    /// puts ahead of its children's matrices, its parent's perspective
    /// among it when the box has no transform of its own, as the
    /// reference browser has a `preserve-3d` box under a perspective.
    fn accumulated(&self, f: &Fragment, rect: Rect, cb_width: f32) -> Matrix {
        let mut full = self.ahead;
        if let Some(perspective) = &self.perspective {
            full = full.then(perspective);
        }
        if let Some(m) = f.matrix(cb_width) {
            full = full
                .then(&Matrix::translate(rect.x, rect.y, 0.0))
                .then(&m)
                .then(&Matrix::translate(-rect.x, -rect.y, 0.0));
        }
        full
    }
}

impl Positioned<'_> {
    /// Its `z-index`, `auto` counting as `0` for the order of the
    /// layers, and any value as `0` where the property does not apply,
    /// on a box neither positioned nor an item.
    fn z(&self) -> i32 {
        self.f
            .style
            .z_index
            .filter(|_| self.f.style.position.is_positioned() || self.item)
            .unwrap_or(0)
    }
}

/// Why the group of the box `f`, whose matrix is `matrix` flattened, is
/// a layer, or `None` for a group drawn through (M19, the compositing
/// note's policy): its matrix rotates, skews or is projective, or its
/// `will-change` names a transform property, `opacity`, `perspective`
/// or `transform-style`; a `will-change` naming `contents` makes no
/// layer. A member of a 3D rendering context is no layer for being
/// one: a member whose flattened matrix only scales, a `translateZ()`
/// under a perspective, is drawn through, sharp, where a layer would
/// be resampled.
fn layer_reason(f: &Fragment, matrix: &Matrix) -> Option<LayerReason> {
    let will_change = f.style.effects().will_change;
    if will_change.names(WillChange::CONTENTS) {
        return None;
    }
    if !matrix.is_identity() && !matrix.translates_and_scales_only() {
        return Some(LayerReason::Transformed);
    }
    if will_change.names_any(WillChange::LAYER) {
        return Some(LayerReason::WillChange);
    }
    None
}

/// Whether the box `f` is a stacking context (CSS2 Appendix E, M19): a
/// positioned box or a flex or grid item (`item`) whose `z-index` is not
/// `auto`, or a box whose transform, opacity or will-change properties
/// make one — the transform ones on a transformable box alone, not an
/// inline box (css-transforms-1 §3) — or a box in a 3D rendering
/// context whose `backface-visibility` is `hidden` (M19.5).
fn stacking_context(f: &Fragment, item: bool) -> bool {
    f.style.is_stacking_context_of(item, transformable(f))
        || (f.in_3d && f.style.effects().backface_visibility == BackfaceVisibility::Hidden)
}

/// Whether the fragment is a transformable box's (css-transforms-1 §3):
/// any box but an inline box's.
fn transformable(f: &Fragment) -> bool {
    matches!(f.kind, FragmentKind::Box)
}

/// Appendix E's passes over the in-flow content of a stacking context,
/// in order: the backgrounds and borders of its block-level boxes, its
/// floats, its inline-level content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Blocks,
    Floats,
    Inlines,
}

const PHASES: [Phase; 3] = [Phase::Blocks, Phase::Floats, Phase::Inlines];

/// What a box is to the order.
enum Kind {
    /// Painted at a layer of the stacking context it belongs to.
    Positioned,
    /// Painted as a unit in the floats pass.
    Float,
    /// Painted as a unit in the inline pass.
    InlineBlock,
    /// In flow and block-level: its own background in the blocks pass,
    /// its content in every pass.
    Block,
}

/// Whether the in-flow children of the box `f` are items: it is a flex
/// container (M17) or a grid container (M18).
fn has_items(f: &Fragment) -> bool {
    f.style.display.has_items()
}

fn kind_of(f: &Fragment) -> Kind {
    if f.style.position.is_positioned() {
        Kind::Positioned
    } else if f.style.float != Float::None {
        Kind::Float
    } else if f.style.display.is_atomic_inline() {
        Kind::InlineBlock
    } else {
        Kind::Block
    }
}

/// Which steps a clipping box's content is wrapped in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Emit {
    /// None: the clip is only tracked.
    Nothing,
    /// The clip's push and pop.
    Clip,
    /// The clip's push and pop, with the box's scrollbar before the
    /// pop.
    ClipAndScrollbar,
}

impl<'a> Walk<'a, '_> {
    /// Opens a group of the document's content (M19): its steps are in
    /// document coordinates, and its matrix is the scroll offset.
    fn open_segment(&mut self) {
        let matrix = Matrix::translate(0.0, -self.scroll, 0.0);
        self.segment = Some(self.steps.len());
        self.steps.push(Step::PushGroup {
            fragment: None,
            matrix,
            opacity: 1.0,
            key: GroupKey::Document(self.segments),
            layer: Some(LayerReason::Scrolled),
        });
        self.segments += 1;
        self.groups.push(matrix);
    }

    /// Closes the open group of the document's content, if any.
    fn close_segment(&mut self) {
        if self.segment.take().is_some() {
            self.steps.push(Step::PopGroup);
            self.groups.pop();
        }
    }

    /// The viewport in the coordinates of the group being walked into:
    /// the bounds of its image through the inverse of the group's
    /// matrix, or the viewport itself when it has none.
    fn window_in_group(&self) -> Rect {
        let window = self.window;
        self.groups
            .last()
            .and_then(|m| m.inverse())
            .and_then(|inverse| inverse.bounds(window.x, window.y, window.w, window.h))
            .map_or(window, |(x0, y0, x1, y1)| Rect {
                x: x0,
                y: y0,
                w: x1 - x0,
                h: y1 - y0,
            })
    }

    /// The effective clip in force in the group being walked into, if
    /// any: the clips of the groups around are in other coordinates.
    fn clip(&self) -> Option<Rect> {
        let depth = self.groups.len();
        self.chain
            .iter()
            .rev()
            .find(|(_, _, d)| *d == depth)
            .map(|(_, rect, _)| *rect)
    }

    /// Whether what is listed meets `rect`, a rectangle in the
    /// coordinates of the group being walked into: through the group's
    /// matrix (M19). A rectangle whose image cannot be told is taken to
    /// be met. Inside a fixed box, the viewport alone is listed.
    fn visible(&self, rect: Rect) -> bool {
        let listed = if self.fixed > 0 {
            self.window
        } else {
            self.viewport
        };
        match self.to_viewport(rect) {
            Some(rect) => rect.meets(&listed),
            None => true,
        }
    }

    /// `rect`, in the coordinates of the group being walked into, in
    /// viewport coordinates: the bounds of its image through the
    /// group's matrix, or `None` when it has none. Outside every group,
    /// inside a fixed box, they are the same.
    fn to_viewport(&self, rect: Rect) -> Option<Rect> {
        match self.groups.last() {
            None => Some(rect),
            Some(m) => m
                .bounds(rect.x, rect.y, rect.w, rect.h)
                .map(|(x0, y0, x1, y1)| Rect {
                    x: x0,
                    y: y0,
                    w: x1 - x0,
                    h: y1 - y0,
                }),
        }
    }

    /// A fragment's box in the coordinates of the group being walked
    /// into.
    fn rect_of(&self, f: &Fragment) -> Rect {
        Rect {
            x: f.x + self.dx,
            y: f.top() + self.dy,
            w: f.w,
            h: f.h,
        }
    }

    /// A fragment's own step — a box's background and border, a text
    /// run — when the viewport meets it.
    fn own(&mut self, f: &'a Fragment) {
        let rect = self.rect_of(f);
        if !self.visible(rect) {
            return;
        }
        self.steps.push(match f.kind {
            FragmentKind::Box | FragmentKind::InlineBox => Step::Box { fragment: f, rect },
            FragmentKind::Text(_) => Step::Text {
                fragment: f,
                rect,
                baseline: f.y + self.dy,
            },
        });
    }

    /// Runs `body` with the shifts the box `f` gets in force, for it
    /// and its subtree. A `position: fixed` box (M11), a root of the
    /// document's fragments by layout, is placed against the viewport
    /// and does not scroll with the document: it is listed outside the
    /// document's groups, in viewport coordinates, the open group
    /// closed before it and another opened after (M19) — unless the
    /// box is inside a containing block for all descendants, under
    /// whose fragment layout put it, and with which it scrolls; or
    /// inside a group that is no containing block, an opacity group,
    /// which it cannot leave, where it is listed in the group's
    /// coordinates at the place the scroll offset gives it, and scrolls
    /// as the group's content is listed again each frame. A `position:
    /// sticky` box is shifted to keep its insets from the edges of the
    /// visible part of its scrollport — the innermost clip, else the
    /// viewport — within its containing block, whose padding box is
    /// `containing`, all in the group's coordinates.
    fn shifted(&mut self, f: &'a Fragment, containing: Rect, body: impl FnOnce(&mut Self)) {
        let (dx, dy) = (self.dx, self.dy);
        let mut fixed = false;
        match f.style.position {
            Position::Fixed if self.contained == 0 => match self.groups.len() {
                // The document's group alone is open: closed around it.
                0 | 1 => {
                    self.close_segment();
                    self.dx = 0.0;
                    self.dy = 0.0;
                    fixed = true;
                    self.fixed += 1;
                }
                _ => {
                    // Where the viewport is in the group's coordinates:
                    // the groups around translate, since one that does
                    // more is a containing block.
                    let (tx, ty) = self
                        .groups
                        .last()
                        .and_then(|m| m.scale_and_translation())
                        .map_or((0.0, 0.0), |(_, _, tx, ty)| (tx, ty));
                    self.dx = -tx;
                    self.dy = -ty;
                }
            },
            Position::Fixed => {}
            Position::Sticky => {
                let scrollport = self.clip().unwrap_or_else(|| self.window_in_group());
                let rect = self.rect_of(f);
                let (sx, sy) = sticky_shift(&f.style, rect, containing, scrollport);
                self.dx += sx;
                self.dy += sy;
            }
            Position::Static | Position::Relative | Position::Absolute => {}
        }
        body(self);
        self.dx = dx;
        self.dy = dy;
        if fixed {
            self.fixed -= 1;
            self.open_segment();
        }
    }

    /// Runs `body` over the content of the box `f`, given its padding
    /// box, the containing block of what is inside: under its clip and
    /// scrolled by its offset when it clips (CSS2 §11.1.1), with the
    /// clip's steps and its scrollbar's as `emit` says. Not at all when
    /// the box clips and the viewport does not meet it: nothing under
    /// its clip can show. A clip with no step inside it is left out.
    fn content(&mut self, f: &'a Fragment, emit: Emit, body: impl FnOnce(&mut Self, Rect)) {
        let contains_all = f.style.effects().contains_all_of(transformable(f))
            || (f.in_3d && f.style.effects().backface_visibility == BackfaceVisibility::Hidden);
        if contains_all {
            self.contained += 1;
        }
        self.content_within(f, emit, body);
        if contains_all {
            self.contained -= 1;
        }
    }

    /// [`Walk::content`], the box's containment of fixed boxes counted
    /// by the caller.
    fn content_within(&mut self, f: &'a Fragment, emit: Emit, body: impl FnOnce(&mut Self, Rect)) {
        let rect = self.rect_of(f);
        let clips = f.clips();
        if clips && !self.visible(rect) {
            return;
        }
        let padding_box = padding_box(f, rect);
        if !clips {
            body(self, padding_box);
            return;
        }
        let effective = match self.clip() {
            Some(outer) => outer.intersect(&padding_box),
            None => padding_box,
        };
        let mark = self.steps.len();
        let node = f.generated.is_none().then_some(f.node);
        if emit != Emit::Nothing {
            self.steps.push(Step::PushClip {
                node,
                rect: effective,
            });
        }
        self.chain.push((node, effective, self.groups.len()));
        let offset = node.map_or(0.0, |node| self.offsets.offset(node));
        // The scrolled content of a container whose `will-change` names
        // `scroll-position` is a group that is a layer (M19), its matrix
        // the offset, its steps inside unscrolled: a scroll of it is a
        // composite. Any other container's content is moved by the
        // offset as it is listed.
        let scroll_layer = node.filter(|_| {
            f.style
                .effects()
                .will_change
                .names(WillChange::SCROLL_POSITION)
        });
        let opened = match scroll_layer {
            Some(node) => {
                let matrix = Matrix::translate(0.0, -offset, 0.0);
                if emit != Emit::Nothing {
                    self.steps.push(Step::PushGroup {
                        fragment: Some(f),
                        matrix,
                        opacity: 1.0,
                        key: GroupKey::Scroll(node),
                        layer: Some(LayerReason::Scrolled),
                    });
                }
                let composed = self
                    .groups
                    .last()
                    .map_or(matrix, |outer| outer.then(&matrix));
                self.groups.push(composed);
                2
            }
            None => {
                self.dy -= offset;
                1
            }
        };
        body(self, padding_box);
        match scroll_layer {
            Some(_) => {
                self.groups.pop();
                if emit != Emit::Nothing {
                    if self.steps.len() == mark + opened {
                        self.steps.pop();
                    } else {
                        self.steps.push(Step::PopGroup);
                    }
                }
            }
            None => self.dy += offset,
        }
        self.chain.pop();
        if emit == Emit::ClipAndScrollbar && shows_scrollbar(f) {
            self.steps.push(Step::Scrollbar {
                fragment: f,
                rect: padding_box,
                offset,
            });
        }
        if emit != Emit::Nothing {
            if self.steps.len() == mark + 1 {
                self.steps.pop();
            } else {
                self.steps.push(Step::PopClip);
            }
        }
    }

    /// The perspective of the box `f`, at `rect`, as a matrix about its
    /// `perspective-origin` in the walk's coordinates (css-transforms-2
    /// §10), which applies to its children's matrices; `None` when it
    /// has none.
    fn perspective_of(&self, f: &Fragment, rect: Rect) -> Option<Matrix> {
        let effects = f.style.effects();
        effects.perspective.map(|depth| {
            let origin = effects.perspective_origin;
            Matrix::perspective_at(
                depth,
                rect.x + origin.x.resolve(rect.w),
                rect.y + origin.y.resolve(rect.h),
            )
        })
    }

    /// A box painted as a stacking context (CSS2 Appendix E): its own
    /// step; then, under its clip, the stacking contexts in it with a
    /// negative `z-index`, in `z-index` then tree order; its in-flow
    /// content in the three passes; its positioned descendants with
    /// `z-index: auto` or `0`, in tree order, an `auto` one as a unit, a
    /// `0` one a stacking context; and the stacking contexts with a
    /// positive `z-index`, in `z-index` then tree order. `extra` are
    /// further positioned descendants outside its subtree, after it in
    /// tree order: the roots layout moved out of the root element's.
    /// `space` is the box's place under its parent's perspective and
    /// in a 3D rendering context (M19.5): a box that establishes one
    /// paints the context's members around its own plane, by depth.
    fn stacking(&mut self, f: &'a Fragment, containing: Rect, extra: &'a [Fragment], space: Space) {
        self.shifted(f, containing, |w| {
            // The group the context is (M19): its full matrix in the
            // coordinates of the group around, about the box where the
            // walk has it, flattened onto the plane, and its opacity. A
            // singular matrix displays nothing, nor does the back of a
            // member that hides it (css-transforms-2 §12).
            let rect = w.rect_of(f);
            let full = space.full(f, rect, containing.w);
            if full.as_ref().is_some_and(|m| m.inverse().is_none()) {
                return;
            }
            // The 3D rendering context the box establishes, or extends
            // (§9): its children take part, with its full matrix ahead
            // of theirs, under its perspective.
            let preserve = transformable(f)
                && f.style.effects().used_transform_style(f.style.overflow)
                    == TransformStyle::Preserve3d;
            let root_3d = preserve && !space.in_3d;
            // A hidden backface turned away (§12) hides the box's own
            // plane; the members of a context it holds are planes of
            // their own, shown or hidden by their own backfaces.
            let hidden = f.style.effects().backface_visibility == BackfaceVisibility::Hidden
                && full.as_ref().is_some_and(|m| m.at(2, 2) < 0.0);
            if hidden && !preserve {
                return;
            }
            let opacity = f.style.effects().opacity;
            let transformed = full.is_some();
            let full = full.unwrap_or(Matrix::IDENTITY);
            let matrix = full.flattened();
            let layer = layer_reason(f, &matrix);
            let inside = Space {
                perspective: w.perspective_of(f, rect),
                in_3d: preserve,
                ahead: if preserve {
                    space.accumulated(f, rect, containing.w)
                } else {
                    Matrix::IDENTITY
                },
            };
            // A group when the box has a transform, an identity one
            // too, or an opacity below one; or is a layer by its
            // `will-change` alone, the author's word that its matrix or
            // opacity is about to change. It is on the stack of groups
            // while the box's descendants are gathered, so that the
            // clips met then are at the depth they are painted at.
            let grouped = transformed || opacity < 1.0 || layer.is_some();
            let composed = w.groups.last().map_or(matrix, |outer| outer.then(&matrix));
            if grouped {
                w.groups.push(composed);
            }
            let key = f.stacking_key();
            let mut positioned = Vec::new();
            w.content(f, Emit::Nothing, |w, padding| {
                w.collect(
                    &f.children,
                    padding,
                    key,
                    &mut positioned,
                    has_items(f),
                    inside,
                );
            });
            w.collect(extra, containing, key, &mut positioned, false, Space::FLAT);
            // The boxes under other contexts' subtrees that belong to
            // this one (M19).
            positioned.extend(w.adopted.remove(&key).unwrap_or_default());
            // The members of the context the box establishes, painted
            // around its plane by depth, furthest first, and those at
            // the plane's own depth in it, at their layers among its
            // positioned descendants (Appendix E), as the reference
            // browser orders what lies in one plane — when the box's own
            // matrix is 2D, under which the group's matrix and a
            // member's compose as the member's full matrix does; under
            // a 3D one they do not, and the members at the plane's
            // depth go after it. A hidden plane keeps nothing in it.
            let (before, after) = if root_3d {
                let plane = full.depth(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
                // The accumulated matrix, not the group's: a root with
                // no transform under its parent's perspective has no
                // group, and its members are under the perspective.
                let nested = inside.ahead.is_2d() && !hidden;
                let mut before = Vec::new();
                let mut after = Vec::new();
                let mut rest = Vec::new();
                for mut p in positioned {
                    if !(p.space.in_3d && stacking_context(p.f, p.item)) {
                        rest.push(p);
                        continue;
                    }
                    let depth = w.depth_of(&p);
                    if depth < plane {
                        before.push((depth, p));
                    } else if depth > plane || !nested {
                        after.push((depth, p));
                    } else {
                        // Inside the group, which has the box's own
                        // matrix.
                        p.space.ahead = Matrix::IDENTITY;
                        rest.push(p);
                    }
                }
                positioned = rest;
                let by_depth = |a: &(f32, Positioned<'a>), b: &(f32, Positioned<'a>)| {
                    a.0.total_cmp(&b.0)
                        .then_with(|| (a.1.z(), a.1.f.tree_order).cmp(&(b.1.z(), b.1.f.tree_order)))
                };
                before.sort_by(by_depth);
                after.sort_by(by_depth);
                (
                    before.into_iter().map(|(_, p)| p).collect::<Vec<_>>(),
                    after.into_iter().map(|(_, p)| p).collect::<Vec<_>>(),
                )
            } else {
                (Vec::new(), Vec::new())
            };
            // The members before the plane are outside the group.
            if grouped {
                w.groups.pop();
            }
            for p in before {
                w.positioned(p);
            }
            if !hidden {
                let group = grouped.then(|| {
                    w.steps.push(Step::PushGroup {
                        fragment: Some(f),
                        matrix,
                        opacity,
                        key: GroupKey::Box(key),
                        layer,
                    });
                    w.groups.push(composed);
                    w.steps.len()
                });
                w.own(f);
                w.content(f, Emit::ClipAndScrollbar, |w, padding| {
                    w.layers(&f.children, padding, positioned, has_items(f));
                });
                if let Some(inside) = group {
                    w.groups.pop();
                    if w.steps.len() == inside {
                        // Nothing of it shows: the group is left out with it.
                        w.steps.pop();
                    } else {
                        w.steps.push(Step::PopGroup);
                    }
                }
            }
            for p in after {
                w.positioned(p);
            }
        });
    }

    /// The depth of a member of a 3D rendering context (M19.5): that
    /// of its border box's centre under its full matrix.
    fn depth_of(&self, p: &Positioned<'a>) -> f32 {
        let rect = Rect {
            x: p.f.x + p.dx,
            y: p.f.top() + p.dy,
            w: p.f.w,
            h: p.f.h,
        };
        p.space
            .full(p.f, rect, p.containing.w)
            .unwrap_or(Matrix::IDENTITY)
            .depth(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
    }

    /// Gathers the members of a 3D rendering context under the box
    /// `c`, which extends it (M19.5), into `out`: the stacking contexts
    /// among its children, each in `space`, and under those of them
    /// that extend the context in turn, their own; each is taken from
    /// the context that would otherwise paint it, the one whose
    /// subtree holds it. A box that belongs elsewhere by its
    /// [`Fragment::stacking_parent`] is left to that context.
    fn members(
        &mut self,
        c: &'a Fragment,
        containing: Rect,
        out: &mut Vec<Positioned<'a>>,
        space: Space,
    ) {
        self.shifted(c, containing, |w| {
            w.content(c, Emit::Nothing, |w, padding| {
                w.members_among(&c.children, padding, c, out, space, has_items(c));
            });
        });
    }

    /// [`Walk::members`] over `fragments` under the context box `c`:
    /// the stacking contexts among them whose parent is `c`, and those
    /// under the boxes that are no stacking context, an anonymous box
    /// say, whose parent is `c` all the same.
    fn members_among(
        &mut self,
        fragments: &'a [Fragment],
        containing: Rect,
        c: &'a Fragment,
        out: &mut Vec<Positioned<'a>>,
        space: Space,
        items: bool,
    ) {
        let owner = c.stacking_key();
        for f in fragments {
            if !matches!(f.kind, FragmentKind::Box)
                || f.stacking_parent.is_some_and(|parent| parent != owner)
                || self.taken.contains(&f.stacking_key())
            {
                continue;
            }
            if !stacking_context(f, items) {
                self.shifted(f, containing, |w| {
                    w.content(f, Emit::Nothing, |w, padding| {
                        w.members_among(&f.children, padding, c, out, space, has_items(f));
                    });
                });
                continue;
            }
            if f.parent != Some(c.node) {
                continue;
            }
            self.taken.insert(f.stacking_key());
            out.push(Positioned {
                f,
                containing,
                chain: self.chain.clone(),
                dx: self.dx,
                dy: self.dy,
                contained: self.contained,
                item: items,
                space,
            });
            self.members_under(f, containing, out, space);
        }
    }

    /// [`Walk::members`] under the member `f` of a 3D rendering
    /// context, when it extends the context: with its full matrix ahead
    /// of its children's, under its perspective.
    fn members_under(
        &mut self,
        f: &'a Fragment,
        containing: Rect,
        out: &mut Vec<Positioned<'a>>,
        space: Space,
    ) {
        if f.style.effects().used_transform_style(f.style.overflow) != TransformStyle::Preserve3d {
            return;
        }
        let rect = self.rect_of(f);
        let inside = Space {
            perspective: self.perspective_of(f, rect),
            in_3d: true,
            ahead: space
                .full(f, rect, containing.w)
                .unwrap_or(Matrix::IDENTITY),
        };
        self.members(f, containing, out, inside);
    }

    /// Gathers the positioned boxes among `fragments` and their
    /// descendants, each with the walk's state where the fragment tree
    /// has it, into `out`: a stacking context's own positioned
    /// descendants (Appendix E), the flex and grid items among them
    /// with a `z-index`, which make stacking contexts without being
    /// positioned (css-flexbox-1 §4.3, css-grid-2 §6.5), and the boxes
    /// whose transform, opacity or will-change make one (M19); `items`
    /// says whether `fragments` are items, and `owner` names the
    /// stacking context collecting. A box that belongs to another
    /// context, its [`Fragment::stacking_parent`], goes to that one
    /// instead, with whatever is positioned inside it. The walk goes
    /// through everything but a stacking context, whose positioned
    /// descendants are its own — unless it extends a 3D rendering
    /// context the `fragments` take part in, as `space` says, whose
    /// members under it are the context's (M19.5); a member taken by a
    /// context's root is left out.
    fn collect(
        &mut self,
        fragments: &'a [Fragment],
        containing: Rect,
        owner: StackingKey,
        out: &mut Vec<Positioned<'a>>,
        items: bool,
        space: Space,
    ) {
        for f in fragments {
            if !matches!(f.kind, FragmentKind::Box | FragmentKind::InlineBox)
                || self.taken.contains(&f.stacking_key())
            {
                continue;
            }
            let context = stacking_context(f, items);
            let elsewhere = f.stacking_parent.filter(|parent| *parent != owner);
            // The owner's perspective and context reach its own
            // children, wherever their fragments hang.
            let own = space.for_child(f, owner.0);
            if f.style.position.is_positioned() || context {
                let p = Positioned {
                    f,
                    containing,
                    chain: self.chain.clone(),
                    dx: self.dx,
                    dy: self.dy,
                    contained: self.contained,
                    item: items,
                    space: own,
                };
                match elsewhere {
                    Some(parent) => self.adopted.entry(parent).or_default().push(p),
                    None => out.push(p),
                }
                if context {
                    if own.in_3d && elsewhere.is_none() {
                        self.members_under(f, containing, out, own);
                    }
                    continue;
                }
            }
            // The children of a box that is no stacking context are
            // under no perspective and in no 3D rendering context of
            // its own, both making one; the owner's reach its children
            // among them, inside an anonymous box say, which `space`
            // carries on for.
            match elsewhere {
                Some(parent) => {
                    let mut inside = Vec::new();
                    self.shifted(f, containing, |w| {
                        w.content(f, Emit::Nothing, |w, padding| {
                            w.collect(
                                &f.children,
                                padding,
                                parent,
                                &mut inside,
                                has_items(f),
                                Space::FLAT,
                            );
                        });
                    });
                    self.adopted.entry(parent).or_default().extend(inside);
                }
                None => self.shifted(f, containing, |w| {
                    w.content(f, Emit::Nothing, |w, padding| {
                        w.collect(&f.children, padding, owner, out, has_items(f), space);
                    });
                }),
            }
        }
    }

    /// The layers of a stacking context's content (Appendix E), `children`
    /// its in-flow content, flex items as `items` says, and `positioned`
    /// its positioned descendants.
    fn layers(
        &mut self,
        children: &'a [Fragment],
        containing: Rect,
        mut positioned: Vec<Positioned<'a>>,
        items: bool,
    ) {
        // By `z-index`, `auto` and `0` sharing the middle, then in tree
        // order: the boxes' own (M17), as layout put some elsewhere in
        // the fragment tree — an absolutely positioned box under its
        // containing block's fragment, or among the roots.
        positioned.sort_by_key(|p| (p.z(), p.f.tree_order));
        let mut positioned = positioned.into_iter().peekable();
        while let Some(p) = positioned.next_if(|p| p.z() < 0) {
            self.positioned(p);
        }
        for phase in PHASES {
            self.phase(children, containing, phase, items);
        }
        for p in positioned {
            self.positioned(p);
        }
    }

    /// One pass over in-flow content: the fragments in tree order, each
    /// as the pass has it, through the content of every in-flow
    /// block-level box; `items` says whether `fragments` are flex or grid
    /// items.
    fn phase(&mut self, fragments: &'a [Fragment], containing: Rect, phase: Phase, items: bool) {
        for f in fragments {
            match f.kind {
                FragmentKind::Text(_) => {
                    if phase == Phase::Inlines {
                        self.own(f);
                    }
                }
                // An inline element's box on a line (M11.6) and all it
                // holds are inline-level content: its background and
                // border, then its items; a positioned one is painted
                // at its layer.
                FragmentKind::InlineBox => {
                    if phase == Phase::Inlines
                        && !f.style.position.is_positioned()
                        && !stacking_context(f, false)
                    {
                        self.own(f);
                        self.phase(&f.children, containing, phase, false);
                    }
                }
                // A flex or grid item paints as an inline-block does, as
                // a unit in the inline pass, its `float` ignored, in the
                // order-modified document order of the fragments; one
                // with a `z-index` makes a stacking context and is
                // painted at its layer, as a positioned one is
                // (css-flexbox-1 §4.3, css-grid-2 §6.5).
                FragmentKind::Box if items && !f.style.position.is_positioned() => {
                    if phase == Phase::Inlines && !stacking_context(f, true) {
                        self.unit(f, containing);
                    }
                }
                // A stacking context that is not positioned (M19) is
                // painted at its layer too.
                FragmentKind::Box if stacking_context(f, false) => {}
                FragmentKind::Box => match kind_of(f) {
                    Kind::Positioned => {}
                    Kind::Float => {
                        if phase == Phase::Floats {
                            self.unit(f, containing);
                        }
                    }
                    Kind::InlineBlock => {
                        if phase == Phase::Inlines {
                            self.unit(f, containing);
                        }
                    }
                    Kind::Block => {
                        if phase == Phase::Blocks {
                            self.own(f);
                        }
                        // The scrollbar goes over the content, after the
                        // last pass.
                        let emit = if phase == Phase::Inlines {
                            Emit::ClipAndScrollbar
                        } else {
                            Emit::Clip
                        };
                        self.content(f, emit, |w, padding| {
                            w.phase(&f.children, padding, phase, has_items(f));
                        });
                    }
                },
            }
        }
    }

    /// A box painted as a unit, as if it made a stacking context
    /// (Appendix E's floats, inline-blocks and positioned boxes with
    /// `z-index: auto`): its own step, then its content in the three
    /// passes — except the positioned boxes and stacking contexts inside
    /// it, which belong to the real stacking context around it and are
    /// painted at their layers there.
    fn unit(&mut self, f: &'a Fragment, containing: Rect) {
        self.shifted(f, containing, |w| {
            w.own(f);
            w.content(f, Emit::ClipAndScrollbar, |w, padding| {
                for phase in PHASES {
                    w.phase(&f.children, padding, phase, has_items(f));
                }
            });
        });
    }

    /// Paints a positioned descendant at its layer, under the clips and
    /// offsets the tree gives it: the clips in force are closed down to
    /// those it shares, its own opened around it, and the walk's put
    /// back after. A stacking context is painted as one, any other
    /// positioned box as a unit.
    fn positioned(&mut self, p: Positioned<'a>) {
        let current = std::mem::replace(&mut self.chain, p.chain);
        let shared = current
            .iter()
            .zip(&self.chain)
            .take_while(|(a, b)| a == b)
            .count();
        for _ in shared..current.len() {
            self.steps.push(Step::PopClip);
        }
        let mark = self.steps.len();
        for &(node, rect, _) in &self.chain[shared..] {
            self.steps.push(Step::PushClip { node, rect });
        }
        let opened = self.chain.len() - shared;
        let (dx, dy, contained) = (self.dx, self.dy, self.contained);
        (self.dx, self.dy, self.contained) = (p.dx, p.dy, p.contained);
        if stacking_context(p.f, p.item) {
            self.stacking(p.f, p.containing, &[], p.space);
        } else {
            self.unit(p.f, p.containing);
        }
        (self.dx, self.dy, self.contained) = (dx, dy, contained);
        if self.steps.len() == mark + opened {
            // Nothing of it shows: its clips are left out with it.
            self.steps.truncate(mark);
        } else {
            for _ in 0..opened {
                self.steps.push(Step::PopClip);
            }
        }
        for &(node, rect, _) in &current[shared..] {
            self.steps.push(Step::PushClip { node, rect });
        }
        self.chain = current;
    }
}

/// Whether a clipping box shows a scrollbar (M10.5): `overflow:
/// scroll` always, `auto` when there is something to scroll.
fn shows_scrollbar(f: &Fragment) -> bool {
    match f.style.overflow {
        Overflow::Scroll => true,
        Overflow::Auto => f.max_scroll() > 0.0,
        Overflow::Hidden | Overflow::Visible => false,
    }
}

/// The shift a `position: sticky` box gets (css-position-3 §3.4, M11):
/// as little as keeps each of its insets that is not `auto` — `top`,
/// `right`, `bottom`, `left`, a percentage of the scrollport's size —
/// between its border box `rect` and the matching edge of
/// `scrollport`, the visible part of its nearest scroll container (the
/// viewport, without one), and never past `containing`, its containing
/// block, so that the box scrolls away with it. A box pushed one way
/// by its start inset is not pulled back by its end inset: the start
/// edge wins. All rectangles are in viewport coordinates. The
/// containing rectangle taken is the parent's padding box, and it is
/// the border box, not the margin box, that is kept inside it: stated
/// approximations.
pub fn sticky_shift(
    style: &ComputedStyle,
    rect: Rect,
    containing: Rect,
    scrollport: Rect,
) -> (f32, f32) {
    let inset = |side: usize, of: f32| match style.inset[side] {
        LengthPercentageAuto::Auto => None,
        LengthPercentageAuto::Px(px) => Some(px),
        LengthPercentageAuto::Percent(p) => Some(of * p / 100.0),
    };
    // Along one axis: the box from `pos` for `size`, the scrollport
    // from `port` for `port_size`, the containing block from `cb` for
    // `cb_size`.
    let along = |insets: (Option<f32>, Option<f32>),
                 (pos, size): (f32, f32),
                 (port, port_size): (f32, f32),
                 (cb, cb_size): (f32, f32)| {
        if let Some(start) = insets.0 {
            let push = (port + start - pos).max(0.0);
            if push > 0.0 {
                let room = (cb + cb_size - (pos + size)).max(0.0);
                return push.min(room);
            }
        }
        if let Some(end) = insets.1 {
            let pull = ((port + port_size - end) - (pos + size)).min(0.0);
            let room = (cb - pos).min(0.0);
            return pull.max(room);
        }
        0.0
    };
    let dx = along(
        (inset(LEFT, scrollport.w), inset(RIGHT, scrollport.w)),
        (rect.x, rect.w),
        (scrollport.x, scrollport.w),
        (containing.x, containing.w),
    );
    let dy = along(
        (inset(TOP, scrollport.h), inset(BOTTOM, scrollport.h)),
        (rect.y, rect.h),
        (scrollport.y, scrollport.h),
        (containing.y, containing.h),
    );
    (dx, dy)
}

/// The padding box of a box fragment whose border box is `rect`: the
/// border box inset by the border, zero-width on a side whose style
/// paints nothing (CSS2 §8.5.3). What `overflow` clips to (CSS2
/// §11.1.1).
pub fn padding_box(f: &Fragment, rect: Rect) -> Rect {
    let [top, right, bottom, left] = f.style.used_border_width();
    Rect {
        x: rect.x + left,
        y: rect.y + top,
        w: (rect.w - left - right).max(0.0),
        h: (rect.h - top - bottom).max(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Laid, laid_out};
    use dom::Element;

    fn test_fonts() -> text::FontCollection {
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses")
    }

    /// The fragment tree of `root` under `css`, laid out at 800px.
    fn layout_of(root: Element, css: &str) -> Laid {
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(css).expect("test css is valid");
        laid_out(&doc, &[sheet], 800.0, &test_fonts())
    }

    /// The steps as short names, with the node's number for a box and a
    /// push, and the clip rectangle for a push.
    fn names(layout: &Laid, steps: &[Step<'_>]) -> Vec<String> {
        steps
            .iter()
            .map(|s| match s {
                Step::Box { fragment, .. } => format!("box {}", layout.number(fragment.node)),
                Step::Text { fragment, .. } => match &fragment.kind {
                    FragmentKind::Text(t) => format!("text {t}"),
                    FragmentKind::Box | FragmentKind::InlineBox => unreachable!(),
                },
                Step::PushClip { node, rect } => {
                    let node = node.map_or(0, |node| layout.number(node));
                    format!("clip {node} {} {} {} {}", rect.x, rect.y, rect.w, rect.h)
                }
                Step::PopClip => "pop".to_string(),
                Step::PushGroup {
                    fragment,
                    matrix,
                    opacity,
                    key,
                    layer,
                } => {
                    let Some(fragment) = fragment else {
                        let GroupKey::Document(n) = key else {
                            panic!("a group without a box is the document's");
                        };
                        return match matrix.to_2d() {
                            Some([_, _, _, _, _, dy]) if dy != 0.0 => {
                                format!("document {n} at {}", -dy)
                            }
                            _ => format!("document {n}"),
                        };
                    };
                    let node = layout.number(fragment.node);
                    let kind = match key {
                        GroupKey::Scroll(_) => "scroll ",
                        _ => "",
                    };
                    let layer = layer.map_or(String::new(), |l| format!(" {l:?}"));
                    match matrix.to_2d() {
                        Some(m) if !matrix.is_identity() => {
                            format!("{kind}group {node} {opacity} {m:?}{layer}")
                        }
                        _ => format!("{kind}group {node} {opacity}{layer}"),
                    }
                }
                Step::PopGroup => "endgroup".to_string(),
                Step::Scrollbar { offset, .. } => format!("scrollbar {offset}"),
            })
            .collect()
    }

    /// The names of the steps inside the document's group, which is one
    /// and holds every step of a document without fixed boxes (M19).
    fn content(layout: &Laid, steps: &[Step<'_>]) -> Vec<String> {
        let names = names(layout, steps);
        assert_eq!(
            names.first().map(String::as_str),
            Some("document 0"),
            "{names:?}"
        );
        assert_eq!(
            names.last().map(String::as_str),
            Some("endgroup"),
            "{names:?}"
        );
        names[1..names.len() - 1].to_vec()
    }

    #[test]
    fn block_backgrounds_come_before_inline_content_each_pass_under_the_clip() {
        // <div><p>a</p><div class=c><p>b</p></div></div>, the clipping
        // box 50px tall with a 5px border: nodes div 1, p 2, div.c 3,
        // p 4. The text is a child of the p's fragment. The root div is
        // the stacking context: the blocks pass paints every box's
        // background in tree order, the inline pass every text, and
        // each pass wraps the clipping box's content in its clip, after
        // its own background step. `overflow: hidden` has no scrollbar.
        let layout = layout_of(
            Element::new("div").push(Element::new("p").text("a")).push(
                Element::new("div")
                    .attr("class", "c")
                    .push(Element::new("p").text("b")),
            ),
            "div, p { margin: 0; } .c { overflow: hidden; height: 50px; border: 5px solid #000; width: 100px; }",
        );
        let m = test_fonts().primary().metrics(16.0);
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        let clip = format!("clip 3 5 {} 100 50", m.line_height() + 5.0);
        assert_eq!(
            content(&layout, &steps),
            vec![
                "box 1".to_string(),
                "box 2".to_string(),
                "box 3".to_string(),
                clip.clone(),
                "box 4".to_string(),
                "pop".to_string(),
                "text a".to_string(),
                clip,
                "text b".to_string(),
                "pop".to_string(),
            ]
        );
    }

    #[test]
    fn a_generated_box_clips_and_its_elements_offset_does_not_scroll_it() {
        // The scroll container (node 1) scrolled by 10: its `::before`,
        // a clipping block, moves with the container's content, and its
        // own content, its text 20px down, is not moved by the
        // container's offset again. Its clip, the part of its padding
        // box inside the container's, names no element.
        let layout = layout_of(
            Element::new("div")
                .attr("class", "s")
                .push(Element::new("p").attr("class", "t")),
            "div, p { margin: 0 } .s { overflow: auto; height: 100px } .t { height: 200px }
             .s::before { display: block; overflow: hidden; height: 40px; padding-top: 20px;
                          content: 'a' }",
        );
        let offsets: std::collections::HashMap<NodeId, f32> =
            [(layout.element(1), 10.0)].into_iter().collect();
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &offsets);
        let clips: Vec<(Option<usize>, f32)> = steps
            .iter()
            .filter_map(|s| match s {
                Step::PushClip { node, rect } => Some((node.map(|n| layout.number(n)), rect.y)),
                _ => None,
            })
            .collect();
        assert_eq!(clips, [(Some(1), 0.0), (None, 0.0)]);
        let generated = steps
            .iter()
            .find_map(|s| match s {
                Step::Box { fragment, rect } if fragment.generated.is_some() => Some(rect.y),
                _ => None,
            })
            .expect("the generated box has a step");
        assert_eq!(generated, -10.0);
        let text = steps
            .iter()
            .find_map(|s| match s {
                Step::Text { rect, .. } => Some(rect.y),
                _ => None,
            })
            .expect("the text has a step");
        assert_eq!(text, 10.0);
    }

    #[test]
    fn rectangles_are_in_document_coordinates_and_offsets_apply_inside_the_clip() {
        // The page scrolled by 20, the 100px scroll container (node 2)
        // scrolled by 30: the document's group carries the page's
        // offset (M19), so its own box is at document 0 as the walk
        // lists it; its child (node 3, 200px tall) at 0 - 30. The clip
        // rectangle is the container's padding box, not moved by the
        // container's own offset; with no clip around it, the effective
        // clip is that padding box itself.
        let layout = layout_of(
            Element::new("div").push(
                Element::new("div")
                    .attr("class", "s")
                    .push(Element::new("p").attr("class", "t")),
            ),
            "div, p { margin: 0; } .s { overflow: auto; height: 100px; } .t { height: 200px; }",
        );
        let offsets: std::collections::HashMap<NodeId, f32> =
            [(layout.element(2), 30.0)].into_iter().collect();
        let viewport = Viewport {
            width: 800.0,
            height: 600.0,
            scroll_y: 20.0,
        };
        let steps = paint_order(&layout.roots, viewport, &offsets);
        let rect = |node: usize| {
            steps
                .iter()
                .find_map(|s| match s {
                    Step::Box { fragment, rect } if layout.number(fragment.node) == node => {
                        Some(*rect)
                    }
                    _ => None,
                })
                .expect("the box has a step")
        };
        assert_eq!(rect(2).y, 0.0);
        assert_eq!(rect(3).y, -30.0);
        assert!(matches!(
            steps[3],
            Step::PushClip {
                node,
                rect: Rect {
                    y: 0.0,
                    h: 100.0,
                    ..
                }
            } if node.map(|node| layout.number(node)) == Some(2)
        ));
        assert!(matches!(
            steps[0],
            Step::PushGroup {
                fragment: None,
                key: GroupKey::Document(0),
                layer: Some(LayerReason::Scrolled),
                matrix,
                ..
            } if matrix.scale_and_translation() == Some((1.0, 1.0, 0.0, -20.0))
        ));
        // The scrollbar is painted over the content, in the last pass.
        assert_eq!(
            names(&layout, &steps)[3..],
            [
                "clip 2 0 0 800 100",
                "box 3",
                "pop",
                "clip 2 0 0 800 100",
                "scrollbar 30",
                "pop",
                "endgroup"
            ]
            .map(String::from)
        );
        assert_eq!(names(&layout, &steps)[0], "document 0 at 20");
    }

    #[test]
    fn nested_clips_intersect_and_out_of_view_content_is_left_out() {
        // An outer clipping box 50px tall holding an inner one 200px
        // tall at the same place: the inner's effective clip is the
        // outer's 50px. A text 1000px down the page is not in the
        // 600px viewport, and a clipping box there is skipped whole.
        let layout = layout_of(
            Element::new("div")
                .push(
                    Element::new("div")
                        .attr("class", "o")
                        .push(Element::new("div").attr("class", "i").text("in")),
                )
                .push(Element::new("div").attr("class", "far").text("far"))
                .push(
                    Element::new("div")
                        .attr("class", "farclip")
                        .text("clipped far"),
                ),
            "div { margin: 0; } .o { overflow: hidden; height: 50px; }
             .i { overflow: hidden; height: 200px; }
             .far { margin-top: 1000px; } .farclip { overflow: hidden; height: 10px; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        let clips: Vec<(usize, f32)> = steps
            .iter()
            .filter_map(|s| match s {
                Step::PushClip { node, rect } => Some((layout.number(node.unwrap()), rect.h)),
                _ => None,
            })
            .collect();
        // The blocks pass opens the outer clip around the inner's box
        // and leaves the inner's own out, nothing being inside it in
        // that pass; the inline pass opens both around the text.
        assert_eq!(
            clips,
            vec![(2, 50.0), (2, 50.0), (3, 50.0)],
            "the inner clip is cut to the outer"
        );
        assert!(
            !names(&layout, &steps).iter().any(|n| n.contains("far")),
            "{:?}",
            names(&layout, &steps)
        );
        assert_eq!(
            names(&layout, &steps)
                .iter()
                .filter(|n| *n == "pop")
                .count(),
            3
        );
    }

    #[test]
    fn positioned_boxes_paint_after_the_in_flow_content_with_their_content() {
        // <div><div class=p>x</div><div class=b>y</div></div> with .p
        // relative: nodes div 1, p 2, b 3. The positioned box is a unit
        // at the root stacking context's positioned layer, after every
        // in-flow box and text; its own text goes with it.
        let layout = layout_of(
            Element::new("div")
                .push(Element::new("div").attr("class", "p").text("x"))
                .push(Element::new("div").attr("class", "b").text("y")),
            "div { margin: 0; } .p { position: relative; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            ["box 1", "box 3", "text y", "box 2", "text x"].map(String::from)
        );
        // An absolute box the initial containing block positions is a
        // root of the layout after the root element's, and paints at
        // the root stacking context's positioned layer all the same.
        let layout = layout_of(
            Element::new("div")
                .push(Element::new("div").attr("class", "a").text("x"))
                .push(Element::new("div").attr("class", "b").text("y")),
            "div { margin: 0; } .a { position: absolute; }",
        );
        assert_eq!(layout.roots.len(), 2);
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            ["box 1", "box 3", "text y", "box 2", "text x"].map(String::from)
        );
    }

    #[test]
    fn floats_paint_after_block_backgrounds_and_before_inline_content() {
        // <div><div class=f>f</div><div class=b>b</div></div> with .f
        // floated: nodes div 1, f 2, b 3. The float is a unit in the
        // floats pass: after the block's background, before its text.
        let layout = layout_of(
            Element::new("div")
                .push(Element::new("div").attr("class", "f").text("f"))
                .push(Element::new("div").attr("class", "b").text("b")),
            "div { margin: 0; } .f { float: left; width: 50px; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            ["box 1", "box 3", "box 2", "text f", "text b"].map(String::from)
        );
    }

    #[test]
    fn z_index_orders_the_layers_and_a_stacking_context_keeps_its_own() {
        // Six positioned siblings: nodes div 1, then n (z -1) 2, b (in
        // flow) 3, p (z 2) 4, q (z 1) 5, z (z 0) 6, a (auto) 7. Negative
        // first, after the root's own background; the in-flow box; 0
        // and auto together in tree order; then positive in z order.
        let positioned = |class: &str| Element::new("div").attr("class", class);
        let layout = layout_of(
            Element::new("div")
                .push(positioned("n"))
                .push(positioned("b"))
                .push(positioned("p"))
                .push(positioned("q"))
                .push(positioned("z"))
                .push(positioned("a")),
            "div { margin: 0; height: 10px; }
             .n { position: relative; z-index: -1; }
             .p { position: relative; z-index: 2; }
             .q { position: relative; z-index: 1; }
             .z { position: relative; z-index: 0; }
             .a { position: relative; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "box 1", "box 2", "box 3", "box 6", "box 7", "box 5", "box 4"
            ]
            .map(String::from)
        );
        // A stacking context takes its own positioned descendants: the
        // negative child of .c (z 1) paints inside .c, before .d (z 2).
        // Nodes div 1, c 2, i 3, d 4.
        let layout = layout_of(
            Element::new("div")
                .push(positioned("c").push(positioned("i")))
                .push(positioned("d")),
            "div { margin: 0; height: 10px; }
             .c { position: relative; z-index: 1; }
             .i { position: absolute; z-index: -1; width: 10px; }
             .d { position: relative; z-index: 2; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            ["box 1", "box 2", "box 3", "box 4"].map(String::from)
        );
        // A positioned box with `z-index: auto` does not: its negative
        // child belongs to the root's stacking context and paints
        // before everything in flow, its parent included. Nodes div 1,
        // p 2, i 3, b 4.
        let layout = layout_of(
            Element::new("div")
                .push(positioned("p").push(positioned("i")))
                .push(positioned("b")),
            "div { margin: 0; height: 10px; }
             .p { position: relative; }
             .i { position: relative; z-index: -1; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            ["box 1", "box 3", "box 4", "box 2"].map(String::from)
        );
    }

    #[test]
    fn a_positioned_box_in_a_clipping_box_paints_under_that_clip_at_its_layer() {
        // <div><div class=o><div class=p></div></div><div class=b></div>
        // </div>, .o clipping, .p relative: nodes div 1, o 2, p 3, b 4.
        // In the in-flow passes the clipping box has nothing to show
        // and its clip is left out; at the positioned layer the clip
        // is opened around the positioned box.
        let layout = layout_of(
            Element::new("div")
                .push(
                    Element::new("div")
                        .attr("class", "o")
                        .push(Element::new("div").attr("class", "p")),
                )
                .push(Element::new("div").attr("class", "b")),
            "div { margin: 0; height: 10px; } .o { overflow: hidden; height: 50px; }
             .p { position: relative; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "box 1",
                "box 2",
                "box 4",
                "clip 2 0 0 800 50",
                "box 3",
                "pop"
            ]
            .map(String::from)
        );
        // Out of view, nothing of it shows and its clips are left out
        // with it.
        let viewport = Viewport {
            width: 800.0,
            height: 600.0,
            scroll_y: 1000.0,
        };
        assert!(paint_order(&layout.roots, viewport, &()).is_empty());
    }

    #[test]
    fn a_sticky_box_is_shifted_to_keep_its_insets_within_its_containing_block() {
        let sticky = |inset: [LengthPercentageAuto; 4]| ComputedStyle {
            position: Position::Sticky,
            inset,
            ..ComputedStyle::default()
        };
        let auto = LengthPercentageAuto::Auto;
        let px = LengthPercentageAuto::Px;
        let rect = |x, y, w, h| Rect { x, y, w, h };
        let port = rect(0.0, 0.0, 800.0, 600.0);
        let top = sticky([px(10.0), auto, auto, auto]);
        // Already 10px or more below the scrollport's top: no shift.
        assert_eq!(
            sticky_shift(
                &top,
                rect(0.0, 100.0, 50.0, 20.0),
                rect(0.0, -200.0, 800.0, 1000.0),
                port
            ),
            (0.0, 0.0)
        );
        // Scrolled 50px past the top: pushed down to 10, as the
        // containing block, ending far below, allows.
        assert_eq!(
            sticky_shift(
                &top,
                rect(0.0, -50.0, 50.0, 20.0),
                rect(0.0, -200.0, 800.0, 1000.0),
                port
            ),
            (0.0, 60.0)
        );
        // The containing block ends at 0: the box is held with its
        // bottom there, pushed by 30 only.
        assert_eq!(
            sticky_shift(
                &top,
                rect(0.0, -50.0, 50.0, 20.0),
                rect(0.0, -200.0, 800.0, 200.0),
                port
            ),
            (0.0, 30.0)
        );
        // `bottom: 0` pulls a box sticking out below the scrollport up,
        // no further than its containing block's top.
        let bottom = sticky([auto, auto, px(0.0), auto]);
        assert_eq!(
            sticky_shift(
                &bottom,
                rect(0.0, 590.0, 50.0, 20.0),
                rect(0.0, 500.0, 800.0, 400.0),
                port
            ),
            (0.0, -10.0)
        );
        assert_eq!(
            sticky_shift(
                &bottom,
                rect(0.0, 590.0, 50.0, 20.0),
                rect(0.0, 585.0, 800.0, 400.0),
                port
            ),
            (0.0, -5.0)
        );
        // Both given: the start edge wins once it pushes.
        let both = sticky([px(10.0), auto, px(0.0), auto]);
        assert_eq!(
            sticky_shift(
                &both,
                rect(0.0, -50.0, 50.0, 20.0),
                rect(0.0, -200.0, 800.0, 1000.0),
                port
            ),
            (0.0, 60.0)
        );
        // Horizontal the same way; a percentage is of the scrollport.
        let left = sticky([auto, auto, auto, px(5.0)]);
        assert_eq!(
            sticky_shift(
                &left,
                rect(-20.0, 0.0, 10.0, 10.0),
                rect(-100.0, 0.0, 1000.0, 100.0),
                port
            ),
            (25.0, 0.0)
        );
        let percent = sticky([LengthPercentageAuto::Percent(10.0), auto, auto, auto]);
        assert_eq!(
            sticky_shift(
                &percent,
                rect(0.0, 0.0, 50.0, 20.0),
                rect(0.0, 0.0, 800.0, 1000.0),
                port
            ),
            (0.0, 60.0)
        );
        // Every inset `auto`: nothing holds the box, however far out.
        let none = sticky([auto; 4]);
        assert_eq!(
            sticky_shift(
                &none,
                rect(-500.0, -500.0, 50.0, 20.0),
                rect(-600.0, -600.0, 2000.0, 2000.0),
                port
            ),
            (0.0, 0.0)
        );
    }

    #[test]
    fn the_walk_shifts_a_sticky_box_and_its_content_together() {
        // A 300px container holding a 20px sticky box with `top: 0` and
        // a text, the page scrolled by 100: the box and its text are
        // held at the viewport's top, document 100 (M19: the steps are
        // in document coordinates); scrolled by 290, the container's
        // bottom at 300 holds the box 10px up from the viewport's top,
        // at 280. Nodes: div 1, s 2.
        let layout = layout_of(
            Element::new("div").push(Element::new("div").attr("class", "s").text("x")),
            "div { margin: 0; height: 300px; } .s { position: sticky; top: 0; height: 20px; }",
        );
        let rects = |scroll_y: f32| {
            let viewport = Viewport {
                width: 800.0,
                height: 600.0,
                scroll_y,
            };
            let steps = paint_order(&layout.roots, viewport, &());
            let s = steps
                .iter()
                .find_map(|st| match st {
                    Step::Box { fragment, rect } if layout.number(fragment.node) == 2 => {
                        Some(rect.y)
                    }
                    _ => None,
                })
                .expect("the sticky box");
            let text = steps
                .iter()
                .find_map(|st| match st {
                    Step::Text { rect, .. } => Some(rect.y),
                    _ => None,
                })
                .expect("its text");
            (s, text)
        };
        assert_eq!(rects(0.0), (0.0, 0.0));
        assert_eq!(
            rects(100.0),
            (100.0, 100.0),
            "held at the top, text with it"
        );
        assert_eq!(
            rects(290.0),
            (280.0, 280.0),
            "held by the container's bottom"
        );
    }

    #[test]
    fn an_inline_box_paints_before_its_text_in_the_inline_pass() {
        // <div><span>x</span> y</div>: nodes div 1, span 2. The span's
        // background and border step comes in the inline pass, right
        // before its text; the div's own text follows.
        let layout = layout_of(
            Element::new("div")
                .push(Element::new("span").text("x"))
                .text(" y"),
            "div { margin: 0; } span { background-color: #f00; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            ["box 1", "box 2", "text x", "text y"].map(String::from)
        );
    }

    #[test]
    fn rect_arithmetic() {
        let a = Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        };
        let b = Rect {
            x: 5.0,
            y: 5.0,
            w: 10.0,
            h: 10.0,
        };
        assert!(a.contains(0.0, 0.0) && !a.contains(10.0, 0.0) && !a.contains(-1.0, 5.0));
        assert!(a.meets(&b));
        assert_eq!(
            a.intersect(&b),
            Rect {
                x: 5.0,
                y: 5.0,
                w: 5.0,
                h: 5.0
            }
        );
        let c = Rect {
            x: 20.0,
            y: 0.0,
            w: 5.0,
            h: 5.0,
        };
        assert!(!a.meets(&c));
        assert_eq!(a.intersect(&c).w, 0.0);
    }

    #[test]
    fn a_flex_or_grid_item_paints_as_a_unit_in_the_inline_pass() {
        // css-flexbox-1 §4.3, css-grid-2 §6.5: <div class=f><p>a</p>
        // <p>b</p></div>, nodes div 1, p 2 and 3. Each item paints as an
        // inline-block does, its background and then its content, in the
        // inline pass, where blocks paint every background before any
        // text; a `float` on an item changes nothing.
        let page = || {
            Element::new("div")
                .attr("class", "f")
                .push(Element::new("p").text("a"))
                .push(Element::new("p").text("b"))
        };
        let items = ["box 1", "box 2", "text a", "box 3", "text b"];
        let blocks = ["box 1", "box 2", "box 3", "text a", "text b"];
        for (css, expected) in [
            ("div, p { margin: 0 } .f { display: flex }", items),
            (
                "div, p { margin: 0 } .f { display: flex } p { float: left }",
                items,
            ),
            ("div, p { margin: 0 } .f { display: grid }", items),
            (
                "div, p { margin: 0 } .f { display: grid } p { float: left }",
                items,
            ),
            ("div, p { margin: 0 }", blocks),
        ] {
            let layout = layout_of(page(), css);
            let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
            assert_eq!(content(&layout, &steps), expected, "{css}");
        }
    }

    #[test]
    fn a_flex_or_grid_item_with_a_z_index_is_a_stacking_context_at_its_layer() {
        // css-flexbox-1 §4.3, css-grid-2 §6.5: an item whose `z-index` is
        // not `auto` makes a stacking context, positioned or not, painted
        // at its layer: nodes div 1, p.z 2 (`z-index: 1`), p 3, p.n 4
        // (`z-index: -1`). A block that is not an item keeps its place:
        // `z-index` does nothing to a box that is not positioned.
        let page = || {
            Element::new("div")
                .attr("class", "f")
                .push(Element::new("p").attr("class", "z").text("a"))
                .push(Element::new("p").text("b"))
                .push(Element::new("p").attr("class", "n").text("c"))
        };
        let base = "div, p { margin: 0 } .z { z-index: 1 } .n { z-index: -1 }";
        for display in ["flex", "grid"] {
            let layout = layout_of(page(), &format!("{base} .f {{ display: {display} }}"));
            let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
            assert_eq!(
                content(&layout, &steps),
                [
                    "box 1", "box 4", "text c", "box 3", "text b", "box 2", "text a"
                ],
                "{display}"
            );
        }
        let layout = layout_of(page(), base);
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "box 1", "box 2", "box 3", "box 4", "text a", "text b", "text c"
            ]
        );
    }

    #[test]
    fn positioned_boxes_paint_in_tree_order_wherever_layout_put_them() {
        // CSS2 Appendix E: the positioned descendants in tree order. An
        // absolutely positioned box goes up to its containing block's
        // fragment, after the content in flow, or becomes a root of the
        // layout; it still paints before a positioned box after it in the
        // document. Nodes: div 1, div 2, div.a 3, p.r 4.
        let page = || {
            Element::new("div")
                .attr("class", "cb")
                .push(Element::new("div").push(Element::new("div").attr("class", "a").text("a")))
                .push(Element::new("p").attr("class", "r").text("r"))
        };
        let base =
            "div, p { margin: 0 } .a { position: absolute; top: 0 } .r { position: relative }";
        for css in [
            format!("{base} .cb {{ position: relative }}"),
            base.to_string(),
        ] {
            let layout = layout_of(page(), &css);
            let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
            let names = names(&layout, &steps);
            let at = |name: &str| names.iter().position(|n| n == name);
            assert!(at("box 3") < at("box 4"), "{css}: {names:?}");
            assert!(at("text a") < at("box 4"), "{css}: {names:?}");
        }
        // A positioned inline element's box on its line is in tree order
        // too: span.r 5 after div.a 3.
        let page = Element::new("div")
            .attr("class", "cb")
            .push(Element::new("div").push(Element::new("div").attr("class", "a").text("a")))
            .push(Element::new("p").push(Element::new("span").attr("class", "r").text("r")));
        let layout = layout_of(page, &format!("{base} .cb {{ position: relative }}"));
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        let names = names(&layout, &steps);
        let at = |name: &str| names.iter().position(|n| n == name);
        assert!(at("box 3") < at("box 5"), "{names:?}");
    }

    #[test]
    fn positioned_items_and_the_boxes_in_them_paint_in_order_modified_order() {
        // css-flexbox-1 §5.4, css-grid-2 §6.3: order-modified document
        // order. Nodes: div.f 1, div.i 2 (`order: 1`) holding div.a 3, div
        // 4 holding div.a 5: the second item comes first, and the
        // absolutely positioned box in it, laid out against the
        // container, with it.
        for display in ["flex", "grid"] {
            let page = Element::new("div")
                .attr("class", "f")
                .push(
                    Element::new("div")
                        .attr("class", "i")
                        .push(Element::new("div").attr("class", "a").text("x")),
                )
                .push(Element::new("div").push(Element::new("div").attr("class", "a").text("y")));
            let layout = layout_of(
                page,
                &format!(
                    "div {{ margin: 0 }} .f {{ display: {display}; position: relative }}
                 .i {{ order: 1 }} .a {{ position: absolute; top: 0 }}"
                ),
            );
            let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
            let names = names(&layout, &steps);
            let at = |name: &str| names.iter().position(|n| n == name);
            assert!(at("box 5") < at("box 3"), "{display}: {names:?}");
            assert!(at("text y") < at("text x"), "{display}: {names:?}");
        }
    }

    #[test]
    fn a_transform_or_an_opacity_makes_a_stacking_context_painted_at_the_zero_layer() {
        // css-transforms-1 §2, css-color-4 §3.2 (M19): a box with an
        // opacity below one or a transform is a stacking context,
        // painted as one, in a group of its own, at the layer of
        // positioned boxes with `z-index: auto` or `0` in tree order; a
        // `z-index` on one that is not positioned does not apply.
        // <div><p class=o>a</p><p>b</p><p class=t>c</p><p class=z>d</p>
        // </div>: nodes div 1, p.o 2, p 3, p.t 4, p.z 5.
        let layout = layout_of(
            Element::new("div")
                .push(Element::new("p").attr("class", "o").text("a"))
                .push(Element::new("p").text("b"))
                .push(Element::new("p").attr("class", "t").text("c"))
                .push(Element::new("p").attr("class", "z").text("d")),
            "div, p { margin: 0; } .o { opacity: 0.5; } .t { transform: translateX(1px); z-index: 5; }
             .z { position: relative; z-index: 1; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "box 1",
                "box 3",
                "text b",
                "group 2 0.5",
                "box 2",
                "text a",
                "endgroup",
                "group 4 1 [1.0, 0.0, 0.0, 1.0, 1.0, 0.0]",
                "box 4",
                "text c",
                "endgroup",
                "box 5",
                "text d"
            ]
        );
        // A positioned transformed box takes its `z-index`: a negative
        // one goes under the in-flow content, and an identity matrix
        // still makes a group; a flex item's `z-index` applies too.
        let layout = layout_of(
            Element::new("div")
                .push(Element::new("p").attr("class", "n").text("a"))
                .push(Element::new("p").text("b")),
            "div, p { margin: 0; } .n { position: relative; z-index: -1; transform: scale(1); }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "box 1",
                "group 2 1",
                "box 2",
                "text a",
                "endgroup",
                "box 3",
                "text b"
            ]
        );
        let layout = layout_of(
            Element::new("div")
                .push(Element::new("p").attr("class", "n").text("a"))
                .push(Element::new("p").text("b")),
            "div { display: flex; margin: 0; } p { margin: 0; } .n { z-index: -1; opacity: 0.5; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "box 1",
                "group 2 0.5",
                "box 2",
                "text a",
                "endgroup",
                "box 3",
                "text b"
            ]
        );
        // A stacking context keeps its own positioned descendants, even
        // one whose containing block is above it: an absolutely
        // positioned box with `z-index: 9` inside a faded box, which is
        // not positioned, hangs under the root's fragment by layout and
        // still paints inside the faded box's group, under a sibling
        // with `z-index: 1` outside. <div><div class=o><div class=a>a
        // </div></div><p class=z>b</p></div>: nodes div 1, div.o 2,
        // div.a 3, p.z 4; with the root `position: relative` too, so
        // that the box hangs under it rather than among the roots; and
        // with a faded `span` for the box, an inline box being no
        // containing block but a stacking context all the same.
        for root_css in ["", "div { position: relative; }"] {
            let layout = layout_of(
                Element::new("div")
                    .push(
                        Element::new("div")
                            .attr("class", "o")
                            .push(Element::new("div").attr("class", "a").text("a")),
                    )
                    .push(Element::new("p").attr("class", "z").text("b")),
                &format!(
                    "div, p {{ margin: 0; }} .o {{ opacity: 0.5; height: 10px; }}
                     .a {{ position: absolute; z-index: 9; }}
                     .z {{ position: relative; z-index: 1; }} {root_css}"
                ),
            );
            let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
            assert_eq!(
                content(&layout, &steps),
                [
                    "box 1",
                    "group 2 0.5",
                    "box 2",
                    "box 3",
                    "text a",
                    "endgroup",
                    "box 4",
                    "text b"
                ],
                "{root_css:?}"
            );
        }
        // <div><p><span class=o>x<b class=a>a</b></span></p><p class=z>b
        // </p></div>: nodes div 1, p 2, span.o 3, b.a 4, p.z 5. The
        // span's box on its line is painted at the zero layer with the
        // absolutely positioned box inside its group.
        let layout = layout_of(
            Element::new("div")
                .push(
                    Element::new("p").push(
                        Element::new("span")
                            .attr("class", "o")
                            .text("x")
                            .push(Element::new("b").attr("class", "a").text("a")),
                    ),
                )
                .push(Element::new("p").attr("class", "z").text("b")),
            "div, p { margin: 0; } .o { opacity: 0.5; } .a { position: absolute; z-index: 9; }
             .z { position: relative; z-index: 1; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "box 1",
                "box 2",
                "group 3 0.5",
                "box 3",
                "text x",
                "box 4",
                "text a",
                "endgroup",
                "box 5",
                "text b"
            ]
        );
        // A box whose matrix is singular, and all inside it, is not
        // displayed: no steps at all. Nodes div 1, p.s 2, p 3.
        let layout = layout_of(
            Element::new("div")
                .push(Element::new("p").attr("class", "s").text("a"))
                .push(Element::new("p").text("b")),
            "div, p { margin: 0; } .s { transform: scale(0); }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(content(&layout, &steps), ["box 1", "box 3", "text b"]);
        // The group's matrix is about the box where the walk has it, in
        // the document's coordinates whatever the scroll offset: a box
        // at (0, 20) 100 by 50, scaled by 2 about its centre, with the
        // viewport scrolled by 5.
        let layout = layout_of(
            Element::new("div")
                .push(Element::new("p").text("a"))
                .push(Element::new("p").attr("class", "s").text("b")),
            "div, p { margin: 0; height: 20px; } .s { transform: scale(2); width: 100px; height: 50px; }",
        );
        let viewport = Viewport {
            width: 800.0,
            height: 600.0,
            scroll_y: 5.0,
        };
        let steps = paint_order(&layout.roots, viewport, &());
        // Origin at (50, 45) in document coordinates: translate(50, 45)
        // scale(2) translate(-50, -45) is matrix(2, 0, 0, 2, -50, -45).
        assert!(
            names(&layout, &steps)
                .contains(&"group 3 1 [2.0, 0.0, 0.0, 2.0, -50.0, -45.0]".to_string()),
            "{:?}",
            names(&layout, &steps)
        );
    }

    #[test]
    fn a_fixed_box_inside_a_transformed_box_scrolls_with_it() {
        // css-transforms-1 §2 (M19): the transformed box is the fixed
        // box's containing block, so the fixed box scrolls with the
        // page, listed inside the document's group in document
        // coordinates; one outside any transformed box stays put,
        // listed outside the document's groups in viewport coordinates,
        // between the group before it and the one after.
        // <div><div class=w><div class=f></div></div><div class=g></div>
        // </div>: nodes div 1, div.w 2, div.f 3, div.g 4.
        let layout = layout_of(
            Element::new("div")
                .push(
                    Element::new("div")
                        .attr("class", "w")
                        .push(Element::new("div").attr("class", "f")),
                )
                .push(Element::new("div").attr("class", "g")),
            "div { margin: 0; } .w { transform: translateX(0px); height: 50px; }
             .f, .g { position: fixed; top: 10px; left: 0; width: 30px; height: 20px; }",
        );
        let viewport = Viewport {
            width: 800.0,
            height: 600.0,
            scroll_y: 5.0,
        };
        let steps = paint_order(&layout.roots, viewport, &());
        fn rect(layout: &Laid, steps: &[Step<'_>], node: usize) -> Rect {
            steps
                .iter()
                .find_map(|s| match s {
                    Step::Box { fragment, rect } if layout.number(fragment.node) == node => {
                        Some(*rect)
                    }
                    _ => None,
                })
                .expect("the box has a step")
        }
        assert_eq!(
            rect(&layout, &steps, 3).y,
            10.0,
            "scrolled with the page, in document coordinates"
        );
        assert_eq!(
            rect(&layout, &steps, 4).y,
            10.0,
            "fixed to the viewport, in its coordinates"
        );
        assert_eq!(rect(&layout, &steps, 2).y, 0.0);
        assert_eq!(
            names(&layout, &steps),
            [
                "document 0 at 5",
                "box 1",
                "group 2 1",
                "box 2",
                "box 3",
                "endgroup",
                "endgroup",
                "box 4"
            ]
        );
        // The group after a fixed box is a second one, when anything
        // follows: a positioned box after it in tree order. Nodes div
        // 1, div.g 2, div.r 3.
        let layout = layout_of(
            Element::new("div")
                .push(Element::new("div").attr("class", "g"))
                .push(Element::new("div").attr("class", "r")),
            "div { margin: 0; height: 20px; } .r { position: relative; }
             .g { position: fixed; top: 10px; left: 0; width: 30px; height: 20px; }",
        );
        let steps = paint_order(&layout.roots, viewport, &());
        assert_eq!(
            names(&layout, &steps),
            [
                "document 0 at 5",
                "box 1",
                "endgroup",
                "box 2",
                "document 1 at 5",
                "box 3",
                "endgroup"
            ]
        );
        // A fixed box inside a faded box, which is no containing block,
        // cannot leave the faded box's group: it is listed in it, in
        // document coordinates, where the scroll offset puts it. Nodes
        // div 1, div.o 2, div.g 3.
        let layout = layout_of(
            Element::new("div").push(
                Element::new("div")
                    .attr("class", "o")
                    .push(Element::new("div").attr("class", "g")),
            ),
            "div { margin: 0; height: 20px; } .o { opacity: 0.5; }
             .g { position: fixed; top: 10px; left: 0; width: 30px; height: 20px; }",
        );
        let steps = paint_order(&layout.roots, viewport, &());
        assert_eq!(
            names(&layout, &steps),
            [
                "document 0 at 5",
                "box 1",
                "group 2 0.5",
                "box 2",
                "box 3",
                "endgroup",
                "endgroup"
            ]
        );
        assert_eq!(
            rect(&layout, &steps, 3).y,
            15.0,
            "viewport 10, scrolled by 5"
        );
    }

    #[test]
    fn the_viewport_meets_a_box_through_its_matrix_and_a_clip_is_its_groups_own() {
        // M19: a box far below the viewport, translated into it, has
        // its step; one in view, translated out of it, has none. A clip
        // inside a group is intersected with the clips in the same
        // group only: the one outside is in other coordinates, and the
        // consumers of the steps intersect across groups themselves.
        // <div><div class=far></div><div class=away></div></div>: nodes
        // div 1, div.far 2, div.away 3; the margin collapses through the
        // root, which is far below the viewport too.
        let layout = layout_of(
            Element::new("div")
                .push(Element::new("div").attr("class", "far"))
                .push(Element::new("div").attr("class", "away")),
            "div { margin: 0; height: 20px; }
             .far { transform: translateY(-2000px); margin-top: 2000px; }
             .away { transform: translateX(5000px); }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "group 2 1 [1.0, 0.0, 0.0, 1.0, 0.0, -2000.0]",
                "box 2",
                "endgroup"
            ]
        );
        // <div class=o><div class=c><p>x</p></div></div>: nodes div.o 1,
        // div.c 2, p 3. The inner clip is the inner box's padding box,
        // not cut by the outer clip.
        let layout = layout_of(
            Element::new("div").attr("class", "o").push(
                Element::new("div")
                    .attr("class", "c")
                    .push(Element::new("p").text("x")),
            ),
            "div, p { margin: 0; } .o { overflow: hidden; width: 50px; height: 50px; }
             .c { transform: translateX(100px); overflow: hidden; width: 30px; height: 30px;
                  margin-left: 40px; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        let names = names(&layout, &steps);
        assert!(
            names.contains(&"clip 2 40 0 30 30".to_string()),
            "{names:?}"
        );
    }

    #[test]
    fn a_3d_rendering_context_paints_its_members_by_depth_around_its_plane() {
        // css-transforms-2 §9 (M19.5): a box with `transform-style:
        // preserve-3d` establishes a 3D rendering context; the stacking
        // contexts among its children are painted around its own plane
        // by their depth, furthest first, whatever their order in the
        // tree, so a child pushed back goes under the box's own
        // background. The flattened matrix of `translateZ()` alone is
        // the identity. Nodes: div.ctx 1, div.a 2, div.b 3.
        let layout = layout_of(
            Element::new("div")
                .attr("class", "ctx")
                .push(Element::new("div").attr("class", "a").text("a"))
                .push(Element::new("div").attr("class", "b").text("b")),
            "div { margin: 0; height: 10px; } .ctx { transform-style: preserve-3d; }
             .a { transform: translateZ(10px); } .b { transform: translateZ(-10px); }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "group 3 1",
                "box 3",
                "text b",
                "endgroup",
                "box 1",
                "group 2 1",
                "box 2",
                "text a",
                "endgroup"
            ]
        );
        // The context's children in tree order when their depths are
        // the same, after the plane; a flat context keeps tree order
        // whatever the depths.
        for (css, want) in [
            (
                ".ctx { transform-style: preserve-3d; } .a, .b { transform: translateZ(5px); }",
                [
                    "box 1",
                    "group 2 1",
                    "box 2",
                    "text a",
                    "endgroup",
                    "group 3 1",
                    "box 3",
                    "text b",
                    "endgroup",
                ],
            ),
            (
                ".a { transform: translateZ(10px); } .b { transform: translateZ(-10px); }",
                [
                    "box 1",
                    "group 2 1",
                    "box 2",
                    "text a",
                    "endgroup",
                    "group 3 1",
                    "box 3",
                    "text b",
                    "endgroup",
                ],
            ),
        ] {
            let layout = layout_of(
                Element::new("div")
                    .attr("class", "ctx")
                    .push(Element::new("div").attr("class", "a").text("a"))
                    .push(Element::new("div").attr("class", "b").text("b")),
                &format!("div {{ margin: 0; height: 10px; }} {css}"),
            );
            let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
            assert_eq!(content(&layout, &steps), want, "{css}");
        }
    }

    #[test]
    fn a_member_turned_away_with_its_backface_hidden_has_no_steps() {
        // css-transforms-2 §12 (M19.5): nodes div.ctx 1, div.a 2, div.b
        // 3; `a` is turned by a half-turn with its backface hidden and
        // is not painted; `b`, turned the same with its backface shown,
        // is, its flattened matrix a reflection about its centre.
        let layout = layout_of(
            Element::new("div")
                .attr("class", "ctx")
                .push(Element::new("div").attr("class", "a").text("a"))
                .push(Element::new("div").attr("class", "b").text("b")),
            "div { margin: 0; height: 10px; width: 100px; } .ctx { transform-style: preserve-3d; }
             .a, .b { transform: rotateY(180deg); } .a { backface-visibility: hidden; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "box 1",
                "group 3 1 [-1.0, 0.0, 0.0, 1.0, 100.0, 0.0] Transformed",
                "box 3",
                "text b",
                "endgroup"
            ]
        );
        // Outside a 3D context the same: a box's own matrix turns it
        // away.
        let layout = layout_of(
            Element::new("div").push(Element::new("div").attr("class", "a").text("a")),
            "div { margin: 0; height: 10px; width: 100px; }
             .a { transform: rotateY(180deg); backface-visibility: hidden; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(content(&layout, &steps), ["box 1"]);
    }

    #[test]
    fn a_parents_perspective_applies_to_its_childrens_matrices() {
        // css-transforms-2 §10 (M19.5): a 200px box with `perspective:
        // 400px` about its top-left corner, holding a 100px box at (20,
        // 20) moved 200px towards the viewer: the child scales by 400 /
        // (400 - 200) about the corner, as the reference browser places
        // one (300 / (300 - 100) for 300px and 100px: (30, 30), 150
        // square); flattened, the group's matrix is that scale, which
        // is drawn through. Nodes div.p 1, div.a 2.
        let layout = layout_of(
            Element::new("div")
                .attr("class", "p")
                .push(Element::new("div").attr("class", "a")),
            "div { margin: 0; } .p { perspective: 400px; perspective-origin: 0 0;
             width: 160px; height: 160px; padding: 20px; }
             .a { width: 100px; height: 100px; transform: translateZ(200px); }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "box 1",
                "group 2 1 [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]",
                "box 2",
                "endgroup"
            ]
        );
    }

    #[test]
    fn a_parents_perspective_reaches_through_a_context_root_with_no_transform() {
        // css-transforms-2 §5 and §10 (M19.5): a `preserve-3d` box with
        // no transform under a parent with `perspective: 400px` is the
        // root of a context whose children's matrices are under the
        // perspective, as the reference browser draws them: the box
        // moved 200px towards the viewer scales by two about the
        // corner, as it would were it the parent's own child, and the
        // box turned about its middle, at the root's depth, is under
        // the perspective too, not nested in a group the root has none
        // of. Nodes div.p 1, div.r 2, div.a 3, div.b 4.
        let layout = layout_of(
            Element::new("div").attr("class", "p").push(
                Element::new("div")
                    .attr("class", "r")
                    .push(Element::new("div").attr("class", "a"))
                    .push(Element::new("div").attr("class", "b")),
            ),
            "div { margin: 0; } .p { perspective: 400px; perspective-origin: 0 0;
             width: 160px; height: 160px; padding: 20px; }
             .r { transform-style: preserve-3d; }
             .a { width: 100px; height: 100px; transform: translateZ(200px); }
             .b { width: 100px; height: 50px; transform: rotateX(50deg); }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        let names = content(&layout, &steps);
        assert_eq!(names[..2].to_vec(), ["box 1", "box 2"], "{names:?}");
        assert_eq!(
            names[5..].to_vec(),
            [
                "group 3 1 [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]",
                "box 3",
                "endgroup"
            ],
            "{names:?}"
        );
        let turned: Vec<Matrix> = steps
            .iter()
            .filter_map(|s| match s {
                Step::PushGroup {
                    fragment: Some(f),
                    matrix,
                    ..
                } if !matrix.is_2d() => Some(*matrix),
                _ => None,
            })
            .collect();
        assert_eq!(
            turned.len(),
            1,
            "the turned box is under the perspective: {names:?}"
        );
    }

    #[test]
    fn a_preserve_3d_child_extends_the_context_and_a_clipping_one_flattens_it() {
        // css-transforms-2 §9 (M19.5): a child with `preserve-3d`
        // extends its parent's context, so a box inside it pushed back
        // is painted before the root's plane, at the root's level; a
        // child that clips has a used `transform-style` of `flat`, and
        // the box inside it stays in the child's plane. Nodes div.ctx
        // 1, div.mid 2, div.a 3, div.b 4.
        let page = || {
            Element::new("div")
                .attr("class", "ctx")
                .push(
                    Element::new("div")
                        .attr("class", "mid")
                        .push(Element::new("div").attr("class", "a").text("a")),
                )
                .push(Element::new("div").attr("class", "b").text("b"))
        };
        let base = "div { margin: 0; height: 10px; } .ctx, .mid { transform-style: preserve-3d; }
                    .a { transform: translateZ(-10px); } .b { transform: translateZ(5px); }";
        let layout = layout_of(page(), base);
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "group 3 1",
                "box 3",
                "text a",
                "endgroup",
                "box 1",
                "group 2 1",
                "box 2",
                "endgroup",
                "group 4 1",
                "box 4",
                "text b",
                "endgroup"
            ]
        );
        let layout = layout_of(page(), &format!("{base} .mid {{ overflow: hidden; }}"));
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "box 1",
                "group 2 1",
                "box 2",
                "clip 2 0 0 800 10",
                "group 3 1",
                "box 3",
                "text a",
                "endgroup",
                "pop",
                "endgroup",
                "group 4 1",
                "box 4",
                "text b",
                "endgroup"
            ]
        );
    }

    #[test]
    fn members_in_the_plane_of_a_3d_context_keep_their_layers_order() {
        // A member at the plane's own depth is painted among the
        // plane's positioned descendants by `z-index` and tree order,
        // not after them all: a positioned box after it in the tree
        // paints over it, as the reference browser has it. Nodes
        // div.ctx 1, div.m 2, div.p 3.
        let layout = layout_of(
            Element::new("div")
                .attr("class", "ctx")
                .push(Element::new("div").attr("class", "m").text("m"))
                .push(Element::new("div").attr("class", "p").text("p")),
            "div { margin: 0; height: 10px; } .ctx { transform-style: preserve-3d; }
             .m { transform: translateX(0px); } .p { position: relative; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "box 1",
                "group 2 1",
                "box 2",
                "text m",
                "endgroup",
                "box 3",
                "text p"
            ]
        );
    }

    #[test]
    fn perspective_on_an_inline_box_makes_no_stacking_context() {
        // css-transforms-1 §3, css-transforms-2 §10: an inline box is
        // not transformable, so its `perspective` makes no stacking
        // context, and the box with a negative `z-index` inside it goes
        // under the root's content rather than inside a group of the
        // span's. Nodes div 1, span.p 2, div.a 3.
        let layout = layout_of(
            Element::new("div").push(
                Element::new("span")
                    .attr("class", "p")
                    .text("x")
                    .push(Element::new("div").attr("class", "a")),
            ),
            "div { margin: 0; } .p { perspective: 100px; }
             .a { position: absolute; z-index: -1; width: 10px; height: 10px; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            ["box 1", "box 3", "box 2", "text x"]
        );
    }

    #[test]
    fn a_member_with_its_backface_hidden_is_a_stacking_context() {
        // css-transforms-2 §9 (M19.5): in a 3D rendering context, a box
        // whose `backface-visibility` is `hidden` is a stacking context
        // and a containing block, so the box inside it with a negative
        // `z-index` paints inside its group, over the sibling before it.
        // Nodes div.outer 1, div.sibling 2, div.inner 3, div.positioned
        // 4.
        let layout = layout_of(
            Element::new("div")
                .attr("class", "outer")
                .push(Element::new("div").attr("class", "sibling"))
                .push(
                    Element::new("div")
                        .attr("class", "inner")
                        .push(Element::new("div").attr("class", "positioned")),
                ),
            "div { margin: 0; width: 100px; height: 100px; }
             .outer { transform-style: preserve-3d; }
             .inner { backface-visibility: hidden; }
             .positioned { position: absolute; top: -100px; z-index: -1; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            ["box 1", "box 2", "group 3 1", "box 3", "box 4", "endgroup"]
        );
        let rect = |node: usize| {
            steps
                .iter()
                .find_map(|s| match s {
                    Step::Box { fragment, rect } if layout.number(fragment.node) == node => {
                        Some(*rect)
                    }
                    _ => None,
                })
                .expect("the box has a step")
        };
        assert_eq!(rect(4).y, 0.0, "against the hidden backface's box at 100");
    }

    #[test]
    fn a_contexts_membership_and_a_perspective_follow_the_boxes_parent() {
        // css-transforms-2 §9, §10 (M19.5): an absolutely positioned box
        // hangs under its containing block's fragment, but takes part in
        // a 3D rendering context, and is under a perspective, by its
        // parent in the box tree alone. Nodes div.cb 1, div.parent 2,
        // div.abspos 3, div.sibling 4: the box pushed forward further
        // than its sibling is no member, and stays at its layer in the
        // plane, before the sibling, which comes after the plane.
        let layout = layout_of(
            Element::new("div")
                .attr("class", "cb")
                .push(
                    Element::new("div")
                        .attr("class", "parent")
                        .push(Element::new("div").attr("class", "abspos")),
                )
                .push(Element::new("div").attr("class", "sibling")),
            "div { margin: 0; width: 100px; height: 100px; }
             .cb { transform-style: preserve-3d; position: relative; }
             .abspos { position: absolute; top: 0; left: 0; transform: translateZ(20px); }
             .sibling { position: absolute; top: 0; left: 0; transform: translateZ(10px); }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "box 1",
                "box 2",
                "group 3 1",
                "box 3",
                "endgroup",
                "group 4 1",
                "box 4",
                "endgroup"
            ]
        );
        // The perspective of a box reaches its children, not a grandchild
        // whose containing block it is: nodes div.outer 1, div.middle 2,
        // div.inner 3, the inner box's matrix the identity, flattened,
        // not the half scale the perspective would make of it.
        let layout = layout_of(
            Element::new("div").attr("class", "outer").push(
                Element::new("div")
                    .attr("class", "middle")
                    .push(Element::new("div").attr("class", "inner")),
            ),
            "div { margin: 0; width: 100px; height: 100px; }
             .outer { position: relative; perspective: 100px; }
             .inner { transform: translateZ(-100px); position: absolute; top: 0; left: 0; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            ["box 1", "box 2", "group 3 1", "box 3", "endgroup"]
        );
    }

    #[test]
    fn members_in_the_plane_of_a_2d_root_nest_in_its_group_with_their_own_matrices() {
        // M19.5: a root moved by a translation, which is 2D, holds the
        // members at its plane's depth inside its group, each with its
        // own matrix alone, so a member's translation cancels the
        // root's: nodes div.o 1, div.m 2, div.g 3, the last at the
        // origin again.
        let layout = layout_of(
            Element::new("div").attr("class", "o").push(
                Element::new("div")
                    .attr("class", "m")
                    .push(Element::new("div").attr("class", "g")),
            ),
            "div { margin: 0; width: 100px; height: 100px; }
             .o { transform-style: preserve-3d; transform: translateX(200%); }
             .m { transform-style: preserve-3d; }
             .g { transform: translateX(-200px); }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "group 1 1 [1.0, 0.0, 0.0, 1.0, 200.0, 0.0]",
                "box 1",
                "group 2 1",
                "box 2",
                "endgroup",
                "group 3 1 [1.0, 0.0, 0.0, 1.0, -200.0, 0.0]",
                "box 3",
                "endgroup",
                "endgroup"
            ]
        );
    }

    #[test]
    fn a_hidden_plane_turned_away_keeps_the_members_of_its_context() {
        // css-transforms-2 §12 (M19.5): a box turned away with its
        // backface hidden paints nothing of its own plane, while a
        // member of the context it holds, whose backface is shown, is
        // painted, turned with it. Nodes div.o 1, div.w 2.
        let layout = layout_of(
            Element::new("div")
                .attr("class", "o")
                .push(Element::new("div").attr("class", "w")),
            "div { margin: 0; width: 100px; height: 100px; }
             .o { transform: rotateY(180deg); backface-visibility: hidden;
                  transform-style: preserve-3d; }
             .w { will-change: transform; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        assert_eq!(
            content(&layout, &steps),
            [
                "group 2 1 [-1.0, 0.0, 0.0, 1.0, 100.0, 0.0] Transformed",
                "box 2",
                "endgroup"
            ]
        );
    }

    #[test]
    fn a_table_is_transformed_once_with_its_caption() {
        // css-transforms-1 §3 (M19): the transform of a table applies
        // to its wrapper box, which the caption is in, and the table box
        // inside is not transformed again. Nodes table 1, caption 2,
        // tr 3, td 4, with the wrapper's and the table box's steps both
        // the table's.
        let layout = layout_of(
            Element::new("table")
                .push(Element::new("caption").text("c"))
                .push(Element::new("tr").push(Element::new("td").text("d"))),
            "table { transform: translate(100px); } caption, td { padding: 0; }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        let names = content(&layout, &steps);
        assert_eq!(
            names.iter().filter(|n| n.starts_with("group")).count(),
            1,
            "{names:?}"
        );
        assert_eq!(names[0], "group 1 1 [1.0, 0.0, 0.0, 1.0, 100.0, 0.0]");
    }

    #[test]
    fn a_block_hoisted_out_of_an_inline_keeps_the_inline_as_its_parent() {
        // CSS2 §9.2.1.1, css-transforms-2 §9 (M19.5): the block inside
        // a span is hoisted beside the span's halves, but its parent is
        // the span, which is not transformable and holds no 3D
        // rendering context, so the block is no member of the box's
        // around it: it stays in the plane, inside the box's group,
        // with the identity its `translateZ()` flattens to. Nodes
        // div.outer 1, span 2, div.inner 3.
        let layout = layout_of(
            Element::new("div").attr("class", "outer").push(
                Element::new("span")
                    .text("a")
                    .push(Element::new("div").attr("class", "inner").text("b")),
            ),
            "div { margin: 0; } .outer { transform-style: preserve-3d; transform: translateX(10px); }
             .inner { transform: translateZ(5px); }",
        );
        let steps = paint_order(&layout.roots, Viewport::new(800.0, 600.0), &());
        let names = content(&layout, &steps);
        assert_eq!(names[0], "group 1 1 [1.0, 0.0, 0.0, 1.0, 10.0, 0.0]");
        assert_eq!(names[names.len() - 1], "endgroup", "{names:?}");
        assert!(names.contains(&"group 3 1".to_string()), "{names:?}");
        assert_eq!(names.iter().filter(|n| n.starts_with("group")).count(), 2);
    }
}
