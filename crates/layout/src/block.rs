//! Block formatting context v1 (roadmap M6.2–M6.7): box tree in,
//! positioned [`Fragment`]s out. Inline-level content (`display:
//! inline`, and text) is delegated to [`crate::inline`] (M7) for real
//! line breaking — see that module's docs for its own v1 scope.
//!
//! Scope, deliberately, matching M6's own "v1":
//!
//! * Only normal-flow, non-replaced block boxes get the real CSS2.1
//!   §10.3.3 width/margin algorithm and §8.3.1 margin collapsing.
//!   Floats, positioning, and tables are M11/M12.
//! * Margins never collapse *through* an inline formatting context —
//!   it always "separates" a block's margin-top/bottom from its
//!   siblings', same as a border or padding would. An inline run that
//!   produces no line box at all (an empty `<a></a>`) separates
//!   nothing: CSS2 §9.4.2 says such a line box is to be treated as not
//!   existing.
//! * A percentage `height`, `min-height` or `max-height` resolves
//!   against the containing block's height only when that is definite
//!   — a length, or a percentage that itself resolved — and otherwise
//!   counts as `auto`, `0` and `none` (CSS2 §10.5, §10.7, M10.3). The
//!   initial containing block's height is the viewport's when
//!   [`layout_document_in`] is given one.
//! * `box-sizing: border-box` (M10.2) is applied before the CSS2
//!   algorithms run: the given width, height and limits are converted
//!   to content-box values, never below zero. `min-*` and `max-*`
//!   (M10.3) clamp the tentative size, the margins re-resolved with the
//!   clamped width (CSS2 §10.4, §10.7).
//! * A block container's children are all block-level (M10.4): the box
//!   tree builder wraps each run of inline-level content in an
//!   anonymous block and hoists a block out of an inline (CSS2
//!   §9.2.1.1), so [`layout_children`] only stacks. An anonymous block
//!   is laid out as its inline formatting context — no margins, border
//!   or padding of its own, the container's width — and has no fragment
//!   of its own: its lines' fragments go into its parent's.
//! * A box with `overflow` other than `visible`, and an inline-block,
//!   establishes a block formatting context (CSS2 §9.4.1, M10.4): its
//!   margins do not adjoin its children's (§8.3.1: they are not in the
//!   same formatting context), so the first child's top margin and the
//!   last child's bottom margin are real space inside it, and its own
//!   top and bottom margins never adjoin each other — it is never
//!   self-collapsing. Its own margins collapse with its siblings' as
//!   usual.
//! * An inline-block (M10.4, [`layout_inline_block`]) is a block
//!   container laid out like any block inside, at a shrink-to-fit width
//!   when its `width` is `auto` (CSS2 §10.3.9: `min(max(min-content,
//!   available), max-content)`, from [`crate::intrinsic`]), its `auto`
//!   margins zero; what the line gets is its margin box and its
//!   baseline — its last line box's, or its bottom margin edge when it
//!   has none or clips (§10.8.1).
//! * A float (M11, [`crate::float`]) is sized as an inline-block is and
//!   placed by its block formatting context's float context at the
//!   flow position it is met at, after the margins resolved so far; it
//!   takes no room in the flow, and the lines of the context are
//!   shortened beside it ([`crate::inline`]). A context root's `auto`
//!   height reaches the lowest float in it (CSS2 §10.6.7); any other
//!   block's ignores floats, which overflow it. The flow position is
//!   tracked in the root's coordinates ([`At`]) alongside the relative
//!   positions below, which still decide where every fragment goes;
//!   the two agree because both resolve the same sets of margins.
//!
//! Margins that adjoin are carried as a set ([`AdjoiningMargins`]: the
//! largest positive and the largest negative seen so far) and resolved
//! to one value only where the set closes — at a border or padding, a
//! line box, or the viewport. CSS2 §8.3.1's formula (max positive minus
//! max absolute negative) is over the whole set, and folding it two at
//! a time gives the wrong answer for three margins of mixed sign. A
//! self-collapsing box (empty, no border or padding, `height: auto`)
//! contributes its top and bottom margins to one set together with the
//! margins of the siblings around it, which is why it joins the set
//! exactly once rather than being counted on both sides.
//!
//! The output is a tree (M9.1): a block box's [`Fragment`] holds its
//! content's fragments as children, which is what clipping (M9.3) and
//! hit testing (M9.4) walk. Coordinates are document coordinates — the
//! initial containing block's top-left is `(0, 0)` and nothing here
//! knows the viewport's height or scroll position; `paint` applies
//! the scroll offset. [`DocumentLayout::height`] is what the viewport
//! scrolls over.

use style::computed::{
    BOTTOM, BoxSizing, ComputedStyle, Float, LEFT, LengthPercentage, LengthPercentageAuto,
    Position, RIGHT, TOP,
};
use text::FontCollection;

use crate::box_tree::{BoxKind, LayoutBox};
use crate::float::{At, FloatContext, PendingFloat, attach_floats};
use crate::inline::layout_inline;

/// A positioned, sized box ready for painting (roadmap M6.7), one node
/// of the fragment tree (M9.1): a block box's fragment holds the
/// fragments of its content as `children`. Coordinates are absolute
/// document coordinates once [`layout_document`] returns — the initial
/// containing block's top-left is `(0, 0)`, and content further down
/// the page than the viewport is tall simply has a larger `y`; `paint`
/// subtracts the scroll offset. Inside this module a fragment's
/// `children` are relative to its own `(x, y)` until the final pass
/// makes them absolute, so placing a box moves its whole subtree at
/// the cost of one translation.
#[derive(Debug, Clone)]
pub struct Fragment {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub style: ComputedStyle,
    pub kind: FragmentKind,
    /// The element this fragment belongs to, as its `FlatDom` index
    /// (M9.4): what hit testing answers with. A text fragment names its
    /// containing element.
    pub node: usize,
    /// For a text fragment, the distance from the top of its box to the
    /// baseline at `y`: the box runs from `y - ascent` to `y - ascent +
    /// h`, the line's own extent. Zero for a box.
    pub ascent: f32,
    /// For a box whose `overflow` clips (a scroll container, M10.5),
    /// the height of its scrollable overflow area from its padding
    /// box's top (css-overflow-3 §2.2): its content as if its height
    /// were `auto`, or any descendant's box or line reaching lower (one
    /// inside a nested clipping box excepted), then its bottom padding;
    /// at least the padding box's own height. Zero for any other
    /// fragment. [`Fragment::max_scroll`] is what it allows.
    pub scroll_height: f32,
    /// The fragments of this box's content, in tree order: the paint
    /// order is the walk's (M11.6, in `paint`). A text fragment has
    /// none; an inline box's are its line's items inside it.
    pub children: Vec<Fragment>,
}

#[derive(Debug, Clone)]
pub enum FragmentKind {
    /// A block/inline box's own border box — background and border
    /// (M6.8) paint from `style`, within `(x, y, w, h)`.
    Box,
    /// One text run; `y` is its *baseline* (matching
    /// `paint::DisplayItem::Text`'s existing convention), not its top,
    /// and `h` is the line's ascent plus descent, see
    /// [`Fragment::ascent`].
    Text(String),
    /// An inline element's box on one line (CSS2 §10.8, M11.6): one per
    /// line box the element appears on, holding the text, inline-block
    /// and nested inline fragments of that line inside it. `(x, y, w,
    /// h)` is its border box: the content area from the line's baseline
    /// up by the element's font ascent and down by its descent (CSS2
    /// §10.6.1), horizontally from its first item to its last, with its
    /// padding and border around it — the vertical ones without
    /// changing the line's height. A line break slices it
    /// (`box-decoration-break: slice`, the only value): the fragment on
    /// a line the element does not start on has no left margin,
    /// border or padding, and one on a line it does not end on no right
    /// ones, a sliced side's border being `none` in the fragment's
    /// `style`. Its background and border paint from `style` within
    /// its border box, like a block's; `overflow` does not apply to it.
    InlineBox,
}

impl Fragment {
    /// Moves this fragment and, since its children are relative to it
    /// until [`layout_document`]'s final pass, its whole subtree.
    pub(crate) fn translated(mut self, dx: f32, dy: f32) -> Self {
        self.x += dx;
        self.y += dy;
        self
    }

    /// The top edge of the fragment's box: `y` for a box, the top of
    /// the line for a text fragment.
    pub fn top(&self) -> f32 {
        self.y - self.ascent
    }

    /// Whether the fragment clips its content (CSS2 §11.1.1, M9.3): a
    /// block-level or inline-block box whose `overflow` says so. An
    /// inline box's `overflow` does not apply, nor does a text's.
    pub fn clips(&self) -> bool {
        matches!(self.kind, FragmentKind::Box) && self.style.overflow.clips()
    }

    /// Whether the document-space point lies within the fragment's box
    /// (a text fragment's is its line box).
    pub fn contains(&self, px: f32, py: f32) -> bool {
        let top = self.top();
        px >= self.x && px < self.x + self.w && py >= top && py < top + self.h
    }

    /// How far a scroll container's content can scroll (M10.5): its
    /// [`Fragment::scroll_height`] past its padding box's height, never
    /// negative — and zero for a box that does not clip, or a text.
    pub fn max_scroll(&self) -> f32 {
        let [top, _, bottom, _] = self.style.used_border_width();
        (self.scroll_height - (self.h - top - bottom)).max(0.0)
    }
}

/// The bottom edge of `f`'s box, or of anything inside it reaching
/// lower, relative to the origin `f`'s own coordinates are in. Content
/// inside a box that clips does not count: its overflow is its own.
fn extent_bottom(f: &Fragment) -> f32 {
    let own = f.top() + f.h;
    if f.clips() {
        return own;
    }
    f.children
        .iter()
        .map(|c| f.y + extent_bottom(c))
        .fold(own, f32::max)
}

/// A laid-out document (M9.1): the fragment tree of its top-level
/// content and the height that content occupies, which is what the
/// viewport scrolls over.
#[derive(Debug, Clone, Default)]
pub struct DocumentLayout {
    /// The top-level fragments, in document order, each with its
    /// subtree; coordinates are absolute.
    pub roots: Vec<Fragment>,
    /// From the top of the initial containing block to the bottom of
    /// the last margin: the content height plus the leading and
    /// trailing margins the viewport materialises (no margin escapes
    /// past the viewport). Never negative.
    pub height: f32,
    /// Whether a top-level box's height, min-height or max-height is a
    /// percentage of the viewport's height (M10.3), so that this layout
    /// is only good for the viewport height it was given.
    pub depends_on_viewport_height: bool,
}

impl DocumentLayout {
    /// Every fragment in tree order (preorder: a box, then its content).
    pub fn iter(&self) -> Preorder<'_> {
        preorder(&self.roots)
    }
}

/// A preorder walk over fragment subtrees — iterative, so a deep tree
/// costs heap, not stack.
pub struct Preorder<'a> {
    stack: Vec<&'a Fragment>,
}

/// The fragments under `roots` in tree order.
pub fn preorder(roots: &[Fragment]) -> Preorder<'_> {
    Preorder {
        stack: roots.iter().rev().collect(),
    }
}

impl<'a> Iterator for Preorder<'a> {
    type Item = &'a Fragment;

    fn next(&mut self) -> Option<&'a Fragment> {
        let f = self.stack.pop()?;
        self.stack.extend(f.children.iter().rev());
        Some(f)
    }
}

/// Takes the fragments out of their tree in tree order; each comes out
/// with no children. The view the pre-M9 tests were written against.
pub fn flatten(roots: Vec<Fragment>) -> Vec<Fragment> {
    let mut out = Vec::new();
    let mut stack: Vec<Fragment> = roots.into_iter().rev().collect();
    while let Some(mut f) = stack.pop() {
        let children = std::mem::take(&mut f.children);
        stack.extend(children.into_iter().rev());
        out.push(f);
    }
    out
}

/// Makes every coordinate absolute: each fragment moves by `(dx, dy)`
/// and its children by its resulting position.
fn make_absolute(fragments: &mut [Fragment], dx: f32, dy: f32) {
    for f in fragments {
        f.x += dx;
        f.y += dy;
        make_absolute(&mut f.children, f.x, f.y);
    }
}

/// A set of adjoining vertical margins (CSS2 §8.3.1), not yet resolved:
/// the largest positive margin and the largest magnitude among the
/// negative ones. `resolve` gives the collapsed margin the set stands
/// for.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct AdjoiningMargins {
    positive: f32,
    negative: f32,
}

impl AdjoiningMargins {
    /// A set of one margin.
    pub(crate) fn of(margin: f32) -> Self {
        if margin >= 0.0 {
            AdjoiningMargins {
                positive: margin,
                negative: 0.0,
            }
        } else {
            AdjoiningMargins {
                positive: 0.0,
                negative: -margin,
            }
        }
    }

    /// The union of two adjoining sets.
    pub(crate) fn join(self, other: Self) -> Self {
        AdjoiningMargins {
            positive: self.positive.max(other.positive),
            negative: self.negative.max(other.negative),
        }
    }

    /// CSS2 §8.3.1: the collapsed margin is the greatest positive
    /// margin minus the greatest absolute negative one.
    pub(crate) fn resolve(self) -> f32 {
        self.positive - self.negative
    }
}

/// Lays out `boxes` (the document's top-level content) against the
/// initial containing block — the viewport (M6.6) — and returns their
/// fragments in absolute, viewport-relative coordinates. `fonts` is what
/// M7's real text measurement/line-breaking shapes and measures words
/// with, now resolving a per-script fallback font (M8.1) rather than
/// always the one primary font. Ideally it's the exact
/// `FontCollection` `render` goes on to draw with, so measured line
/// breaks match what's drawn pixel-for-pixel; today `engine::Engine`
/// instead loads a second `FontCollection` independently for this (same
/// deterministic system-font queries as `render`'s own, so in practice
/// it resolves to the same font files, but it's a second load, not a
/// shared handle — sharing one properly is natural M35 perf-track
/// territory, not a v1 requirement).
pub fn layout_document(
    boxes: &[LayoutBox],
    viewport_width: f32,
    fonts: &FontCollection,
) -> DocumentLayout {
    layout_document_in(boxes, viewport_width, None, fonts)
}

/// [`layout_document`] with the viewport's height known, so a
/// percentage height on a top-level box resolves against it (the
/// initial containing block's height is the viewport's, CSS2 §10.1).
pub fn layout_document_in(
    boxes: &[LayoutBox],
    viewport_width: f32,
    viewport_height: Option<f32>,
    fonts: &FontCollection,
) -> DocumentLayout {
    // The initial containing block is the outermost block formatting
    // context: its float context is the document's, and its content
    // top is the root of the coordinates floats are placed in.
    let mut floats = FloatContext::default();
    let children = layout_children(
        boxes,
        viewport_width,
        viewport_height,
        &mut floats,
        At::root(),
        fonts,
    );
    let depends_on_viewport_height = viewport_height.is_some()
        && boxes.iter().any(|b| {
            let percent = |v: LengthPercentage| matches!(v, LengthPercentage::Percent(_));
            let percent_or_auto =
                |v: LengthPercentageAuto| matches!(v, LengthPercentageAuto::Percent(_));
            percent_or_auto(b.style.height)
                || percent(b.style.min_height)
                || b.style.max_height.is_some_and(percent)
                || (b.style.position == Position::Relative
                    && (percent_or_auto(b.style.inset[TOP])
                        || percent_or_auto(b.style.inset[BOTTOM])))
        });
    // The viewport is the one context with no further ancestor to
    // escape to — `leading` *must* materialize here, exactly once, or
    // it would just vanish (see `layout_children`'s doc). With content
    // it is already in the content's positions and comes back empty;
    // without, it is the set every box collapsed through, the height
    // of the page, each box sitting where its own top margin put it.
    // So must `trailing`, below the content. A float still waiting for
    // the flow position at the end is placed where the content would
    // have been.
    floats.resolve_pending(children.end.position());
    let height = if children.nothing_materialised {
        children.leading.resolve()
    } else {
        children.content_height + children.trailing.resolve()
    };
    // A page ending in a float is as tall as the float (CSS2 §10.6.7).
    let mut height = floats.lowest_bottom().map_or(height, |b| height.max(b));
    let mut roots = children.fragments;
    // The absolutely positioned boxes no positioned ancestor took are
    // the initial containing block's — the viewport's rectangle at the
    // document's top — and so is every `position: fixed` box, those
    // met inside the others included (M11). Their fragments are the
    // document's last roots; a fixed one is not scrolled with the
    // document (`paint` leaves the viewport's offset out of it), an
    // absolute one is, and the page reaches down to it.
    let mut waiting = children.absolutes;
    let mut depends_on_viewport_height = depends_on_viewport_height;
    while !waiting.is_empty() {
        for d in std::mem::take(&mut waiting) {
            depends_on_viewport_height |= viewport_height.is_some() && uses_cb_height(&d.b.style);
            let (fragment, fixed) =
                layout_absolute(d.b, d.x, d.y, viewport_width, viewport_height, fonts);
            if d.b.style.position == Position::Absolute {
                height = height.max(fragment.y + fragment.h);
            }
            roots.push(fragment);
            waiting.extend(fixed);
        }
    }
    // The floats' fragments get their positions while every fragment
    // is still relative to its parent, as the attaching assumes.
    attach_floats(&mut roots, floats.placed(), 0.0, 0.0, 0.0, 0.0);
    make_absolute(&mut roots, 0.0, 0.0);
    DocumentLayout {
        roots,
        height: height.max(0.0),
        depends_on_viewport_height,
    }
}

/// [`layout_document`]'s fragments in tree order without the tree —
/// the flat view, for callers that only inspect geometry.
pub fn layout(boxes: &[LayoutBox], viewport_width: f32, fonts: &FontCollection) -> Vec<Fragment> {
    flatten(layout_document(boxes, viewport_width, fonts).roots)
}

/// `border-{side}-width`, but zero on a side whose style paints nothing
/// — such a border has zero width for *every* purpose, not just
/// painting (CSS2 §8.5.3); every layout calculation reads border width
/// through this, never `style.border_width` directly.
fn effective_border_width(style: &ComputedStyle) -> [f32; 4] {
    style.used_border_width()
}

/// The specified `width`, `min-width` and `max-width` as content-box
/// widths against `cb_width` (M10.2, M10.3): a percentage resolved, and
/// under `box-sizing: border-box` the padding and border taken out,
/// never below zero. `width` is `None` for `auto`, `max` for `none`.
struct WidthConstraints {
    width: Option<f32>,
    min: f32,
    max: Option<f32>,
}

fn width_constraints(style: &ComputedStyle, cb_width: f32, non_content: f32) -> WidthConstraints {
    let inner = |v: f32| match style.box_sizing {
        BoxSizing::ContentBox => v.max(0.0),
        BoxSizing::BorderBox => (v - non_content).max(0.0),
    };
    WidthConstraints {
        width: match style.width {
            LengthPercentageAuto::Auto => None,
            LengthPercentageAuto::Px(v) => Some(inner(v)),
            LengthPercentageAuto::Percent(p) => Some(inner(cb_width * p / 100.0)),
        },
        min: inner(style.min_width.resolve(cb_width)),
        max: style.max_width.map(|m| inner(m.resolve(cb_width))),
    }
}

/// CSS2.1 §10.3.3 and §10.4: resolves `width`, `margin-left`, and
/// `margin-right` for a normal-flow, non-replaced block box, the
/// tentative width clamped to `max-width` then `min-width` with the
/// margins resolved again for the clamped width. `non_content_width`
/// is whatever horizontal space border+padding already consume.
/// Percentages are of `cb_width`; the room the margin box fills is
/// `available_width` — the containing block's width, or for a block
/// formatting context root beside floats the room between them (CSS2
/// §9.5, M11).
fn resolve_width_and_margins(
    style: &ComputedStyle,
    cb_width: f32,
    available_width: f32,
    non_content_width: f32,
) -> (f32, f32, f32) {
    let c = width_constraints(style, cb_width, non_content_width);
    let tentative =
        |width| tentative_width(style, cb_width, available_width, non_content_width, width);
    let mut used = tentative(c.width);
    if let Some(max) = c.max
        && used.0 > max
    {
        used = tentative(Some(max));
    }
    if used.0 < c.min {
        used = tentative(Some(c.min));
    }
    used
}

/// CSS2.1 §10.3.3 for one value of `width` (`None` for `auto`), as the
/// content-box width: `(width, margin_left, margin_right)`.
fn tentative_width(
    style: &ComputedStyle,
    cb_width: f32,
    available_width: f32,
    non_content_width: f32,
    width: Option<f32>,
) -> (f32, f32, f32) {
    let avail = available_width - non_content_width;
    let margin_left = style.margin[LEFT];
    let margin_right = style.margin[RIGHT];

    let Some(w) = width else {
        // Unspecified margins default to 0 (not symmetrical centering —
        // that only happens when `width` itself is NOT auto); auto
        // `width` absorbs whatever's left. The used value of `width`
        // is never negative (CSS2.1 §10.2); margins may be.
        let ml = margin_left.resolve(cb_width, 0.0);
        let mr = margin_right.resolve(cb_width, 0.0);
        return ((avail - ml - mr).max(0.0), ml, mr);
    };

    let remaining = avail - w;
    // §10.3.3 ¶3: when the non-auto parts already exceed the containing
    // block, every `auto` margin is treated as zero (the box overflows
    // to the right), and the over-constrained rule below recomputes
    // margin-right.
    let fixed_left = if margin_left.is_auto() {
        0.0
    } else {
        margin_left.resolve(cb_width, 0.0)
    };
    let fixed_right = if margin_right.is_auto() {
        0.0
    } else {
        margin_right.resolve(cb_width, 0.0)
    };
    let overflows = fixed_left + fixed_right > remaining;
    match (margin_left.is_auto(), margin_right.is_auto()) {
        (true, true) if !overflows => {
            let half = remaining / 2.0; // M6.3: auto-centering
            (w, half, half)
        }
        (true, false) if !overflows => (w, remaining - fixed_right, fixed_right),
        _ => {
            // Over-constrained (both specified, or overflowing): per the
            // ltr rule, margin-right is recomputed regardless of its
            // specified value, as if it alone were auto.
            (w, fixed_left, remaining - fixed_left)
        }
    }
}

/// The specified `height` as a content-box height, or `None` for
/// `auto` — which a percentage of an indefinite containing-block height
/// is too (CSS2 §10.5).
fn definite_height(style: &ComputedStyle, cb_height: Option<f32>, non_content: f32) -> Option<f32> {
    let inner = |v: f32| match style.box_sizing {
        BoxSizing::ContentBox => v.max(0.0),
        BoxSizing::BorderBox => (v - non_content).max(0.0),
    };
    match style.height {
        LengthPercentageAuto::Auto => None,
        LengthPercentageAuto::Px(v) => Some(inner(v)),
        LengthPercentageAuto::Percent(p) => cb_height.map(|h| inner(h * p / 100.0)),
    }
}

/// `min-height` and `max-height` as content-box heights (CSS2 §10.7):
/// a percentage of an indefinite containing-block height is `0` and
/// `none` respectively.
fn height_bounds(
    style: &ComputedStyle,
    cb_height: Option<f32>,
    non_content: f32,
) -> (f32, Option<f32>) {
    let inner = |v: f32| match style.box_sizing {
        BoxSizing::ContentBox => v.max(0.0),
        BoxSizing::BorderBox => (v - non_content).max(0.0),
    };
    let resolve = |v: LengthPercentage| match v {
        LengthPercentage::Px(px) => Some(inner(px)),
        LengthPercentage::Percent(p) => cb_height.map(|h| inner(h * p / 100.0)),
    };
    (
        resolve(style.min_height).unwrap_or(0.0),
        style.max_height.and_then(resolve),
    )
}

/// An absolutely positioned box met in flow (M11), on its way up to
/// its containing block: the box, and the static position of its
/// margin box's top-left (CSS2 §10.3.7, §10.6.4: where the box would
/// have been in flow), relative to the fragment carrying it — a
/// [`BlockLayout`]'s or [`AtomicLayout`]'s border box, a
/// [`ChildrenLayout`]'s or `InlineLayout`'s content-box origin — and
/// moved along as that fragment is placed, until the nearest positioned
/// ancestor (or the initial containing block; the viewport, for
/// `position: fixed`) lays it out against its padding box.
pub(crate) struct Deferred<'a> {
    pub(crate) b: &'a LayoutBox,
    pub(crate) x: f32,
    pub(crate) y: f32,
}

impl Deferred<'_> {
    /// The same box carried by a fragment placed at `(dx, dy)` in the
    /// one it was relative to.
    pub(crate) fn moved(self, dx: f32, dy: f32) -> Self {
        Deferred {
            b: self.b,
            x: self.x + dx,
            y: self.y + dy,
        }
    }
}

/// Every box of `list` moved by `(dx, dy)`.
pub(crate) fn carried<'a>(list: Vec<Deferred<'a>>, dx: f32, dy: f32) -> Vec<Deferred<'a>> {
    list.into_iter().map(|d| d.moved(dx, dy)).collect()
}

/// One box's layout, relative to its own border-box top-left `(0, 0)` —
/// the caller (`layout_children`) translates once collapsed-margin
/// placement among siblings is resolved.
pub(crate) struct BlockLayout<'a> {
    /// The box's own fragment; its content's fragments are the
    /// children, relative to the box's border-box top-left `(0, 0)`.
    pub(crate) root: Fragment,
    /// Border-box height (margin excluded) — siblings stack purely
    /// vertically, so this is the only dimension the caller needs; the
    /// box's own width is already baked into its fragments' `w`.
    pub(crate) height: f32,
    /// Only `margin_left` matters to the caller: horizontal margins
    /// don't collapse between siblings the way vertical ones do, so
    /// `margin_right` is fully "spent" by this box's own
    /// `resolve_width_and_margins` call and never consulted again.
    pub(crate) margin_left: f32,
    /// This box's margin-top/bottom *as exposed to its siblings* —
    /// joined with its first/last child's where those escape (CSS2
    /// §8.3.1), as a still-open adjoining set.
    pub(crate) margin_top: AdjoiningMargins,
    pub(crate) margin_bottom: AdjoiningMargins,
    /// The margins above the box's top border edge: `margin_top`, or
    /// for a self-collapsing box the set without its own bottom margin,
    /// since its top border edge is "the same as it would have been if
    /// the element had a non-zero bottom border" (CSS2 §8.3.1) — which
    /// a float inside it makes observable (M11).
    pub(crate) top_edge: AdjoiningMargins,
    /// The margins below the box's bottom border edge the same way:
    /// `margin_bottom`, or for a self-collapsing box the set without
    /// its own top margin — what follows a self-collapsing box given
    /// clearance (CSS2 §9.5.2), whose top set is materialised above it.
    pub(crate) bottom_edge: AdjoiningMargins,
    /// The box's top and bottom margins adjoin each other (an empty
    /// box, or one whose every child collapsed through): `margin_top`
    /// and `margin_bottom` are the same set, to be joined with the
    /// siblings' margins once, and the box has no height.
    pub(crate) self_collapsing: bool,
    /// The baseline of the box's last line box, from its border-box
    /// top (CSS2 §10.8.1: an inline-block's own baseline); `None` when
    /// nothing inside made a line.
    pub(crate) last_baseline: Option<f32>,
    /// The absolutely positioned boxes met inside whose containing
    /// block is further up (M11), relative to the border box.
    pub(crate) absolutes: Vec<Deferred<'a>>,
}

/// An inline-block laid out for its line (M10.4, M7.7): one atomic item
/// as wide and as tall as its margin box.
pub(crate) struct AtomicLayout<'a> {
    /// The box's own fragment with its content's, relative to its
    /// border-box top-left `(0, 0)`.
    pub(crate) root: Fragment,
    /// The margin box's width and height.
    pub(crate) width: f32,
    pub(crate) height: f32,
    /// From the margin box's top to the baseline the line aligns it by.
    pub(crate) ascent: f32,
    /// Where the border box sits inside the margin box.
    pub(crate) margin_left: f32,
    pub(crate) margin_top: f32,
    /// The move `position: relative` gives the box (M11), applied to
    /// its fragment where it is placed; the line and the float context
    /// see the box unmoved.
    pub(crate) relative: (f32, f32),
    /// As [`BlockLayout::absolutes`], relative to the border box.
    pub(crate) absolutes: Vec<Deferred<'a>>,
}

/// The move a box with `position: relative` gets once laid out in flow
/// (CSS2 §9.4.3): `left`, else minus `right`, both `auto` no move;
/// `top`, else minus `bottom`. A percentage is of the containing
/// block's width for `left` and `right`, of its height for `top` and
/// `bottom`, and `auto` when that height is not definite (§9.3.2). The
/// flow around the box is not affected: the move is applied to its
/// fragment, and its subtree with it, as it is placed.
pub(crate) fn relative_offset(
    style: &ComputedStyle,
    cb_width: f32,
    cb_height: Option<f32>,
) -> (f32, f32) {
    if style.position != Position::Relative {
        return (0.0, 0.0);
    }
    let horizontal = |v: LengthPercentageAuto| match v {
        LengthPercentageAuto::Auto => None,
        LengthPercentageAuto::Px(px) => Some(px),
        LengthPercentageAuto::Percent(p) => Some(cb_width * p / 100.0),
    };
    let vertical = |v: LengthPercentageAuto| match v {
        LengthPercentageAuto::Auto => None,
        LengthPercentageAuto::Px(px) => Some(px),
        LengthPercentageAuto::Percent(p) => cb_height.map(|h| h * p / 100.0),
    };
    let dx = match (
        horizontal(style.inset[LEFT]),
        horizontal(style.inset[RIGHT]),
    ) {
        (Some(left), _) => left,
        (None, Some(right)) => -right,
        (None, None) => 0.0,
    };
    let dy = match (vertical(style.inset[TOP]), vertical(style.inset[BOTTOM])) {
        (Some(top), _) => top,
        (None, Some(bottom)) => -bottom,
        (None, None) => 0.0,
    };
    (dx, dy)
}

/// Whether a box with this style establishes a block formatting context
/// of its own (CSS2 §9.4.1, M10.4): `overflow` other than `visible`, an
/// atomic inline (an inline-block, an inline table), a float or an
/// absolutely positioned box (M11). Table cells and captions do too;
/// they arrive with their box kinds (M12).
pub(crate) fn establishes_bfc(style: &ComputedStyle) -> bool {
    style.overflow.clips()
        || style.display.is_atomic_inline()
        || style.float != Float::None
        || style.position.is_out_of_flow()
}

/// The float context a block container's content is laid out in: a
/// fresh one of its own when the container is a block formatting
/// context root, else its parent's, with where the container's margin
/// position is in it.
pub(crate) enum Content<'c> {
    Root,
    In {
        floats: &'c mut FloatContext,
        at: At,
    },
}

/// A box's border and padding, resolved against its containing block's
/// width (padding's percentages are of that width on every side, CSS2
/// §8.4).
pub(crate) struct Edges {
    pub(crate) border: [f32; 4],
    pub(crate) padding: [f32; 4],
}

impl Edges {
    pub(crate) fn of(style: &ComputedStyle, cb_width: f32) -> Self {
        Edges {
            border: effective_border_width(style),
            padding: style.padding.map(|p| p.resolve(cb_width)),
        }
    }

    /// What the two sides take out of the width.
    fn horizontal(&self) -> f32 {
        self.border[LEFT] + self.border[RIGHT] + self.padding[LEFT] + self.padding[RIGHT]
    }

    /// What the two sides take out of the height.
    fn vertical(&self) -> f32 {
        self.border[TOP] + self.padding[TOP] + self.padding[BOTTOM] + self.border[BOTTOM]
    }
}

/// Lays out one [`BoxKind::Block`] box (an anonymous block never comes
/// here: [`layout_children`] lays its lines out directly) in the float
/// context `floats`, met at `at`, its margin box filling
/// `available_width` of the containing block (its whole width, or the
/// room between floats for a context root, M11).
fn layout_block<'a>(
    b: &'a LayoutBox,
    cb_width: f32,
    available_width: f32,
    cb_height: Option<f32>,
    floats: &mut FloatContext,
    at: At,
    fonts: &FontCollection,
) -> BlockLayout<'a> {
    let edges = Edges::of(&b.style, cb_width);
    // margin-right is fully resolved here but never read again — see
    // `BlockLayout::margin_left`'s doc for why that's correct, not an
    // oversight.
    let (content_width, margin_left, _margin_right) =
        resolve_width_and_margins(&b.style, cb_width, available_width, edges.horizontal());
    let content = if establishes_bfc(&b.style) {
        Content::Root
    } else {
        Content::In { floats, at }
    };
    layout_block_contents(
        b,
        &edges,
        cb_width,
        cb_height,
        content_width,
        margin_left,
        content,
        fonts,
    )
}

/// Lays out one [`BoxKind::Float`] box (M11): sized as an inline-block
/// is — shrink-to-fit when its `width` is `auto` (CSS2 §10.3.5),
/// `auto` margins zero — its content a block formatting context of its
/// own. Where it goes is the caller's: the margin box the result
/// describes is placed by the float context.
pub(crate) fn layout_float<'a>(
    b: &'a LayoutBox,
    cb_width: f32,
    cb_height: Option<f32>,
    fonts: &FontCollection,
) -> AtomicLayout<'a> {
    layout_inline_block(b, cb_width, cb_height, fonts)
}

/// The float the atomic layout of `b` describes, for a containing
/// block whose content box runs from `cb_left` for `cb_width` in root
/// coordinates.
pub(crate) fn pending_float(
    b: &LayoutBox,
    atomic: &AtomicLayout<'_>,
    cb_left: f32,
    cb_width: f32,
) -> PendingFloat {
    PendingFloat {
        node: b.node,
        side: b.style.float,
        clear: b.style.clear,
        w: atomic.width,
        h: atomic.height,
        margin_left: atomic.margin_left,
        margin_top: atomic.margin_top,
        relative: atomic.relative,
        cb_left,
        cb_right: cb_left + cb_width,
    }
}

/// Lays out one [`BoxKind::InlineBlock`] box for its line (M10.4):
/// CSS2 §10.3.9 — `auto` margins are zero, an `auto` width is the
/// shrink-to-fit width, `min(max(min-content, available), max-content)`
/// with `available` the containing block's width less the box's own
/// margins, border and padding (§10.3.5), clamped by `max-width` and
/// `min-width` (§10.4); the content then lays out as any block's. The
/// baseline is the last line box's, or the bottom margin edge when
/// there is none or the box clips (§10.8.1).
pub(crate) fn layout_inline_block<'a>(
    b: &'a LayoutBox,
    cb_width: f32,
    cb_height: Option<f32>,
    fonts: &FontCollection,
) -> AtomicLayout<'a> {
    let edges = Edges::of(&b.style, cb_width);
    let margin = |side: usize| b.style.margin[side].resolve(cb_width, 0.0);
    let (margin_left, margin_right) = (margin(LEFT), margin(RIGHT));
    let (margin_top, margin_bottom) = (margin(TOP), margin(BOTTOM));
    let c = width_constraints(&b.style, cb_width, edges.horizontal());
    let mut content_width = match c.width {
        Some(w) => w,
        None => {
            let (min_content, max_content) = crate::intrinsic::content_widths(b, fonts);
            let available = (cb_width - margin_left - margin_right - edges.horizontal()).max(0.0);
            min_content.max(available.min(max_content))
        }
    };
    if let Some(max) = c.max {
        content_width = content_width.min(max);
    }
    content_width = content_width.max(c.min);
    let layout = layout_block_contents(
        b,
        &edges,
        cb_width,
        cb_height,
        content_width,
        margin_left,
        Content::Root,
        fonts,
    );
    let height = margin_top + layout.height + margin_bottom;
    let ascent = match layout.last_baseline {
        Some(baseline) if !b.style.overflow.clips() => margin_top + baseline,
        _ => height,
    };
    AtomicLayout {
        width: margin_left + layout.root.w + margin_right,
        root: layout.root,
        height,
        ascent,
        margin_left,
        margin_top,
        relative: relative_offset(&b.style, cb_width, cb_height),
        absolutes: layout.absolutes,
    }
}

/// The body of a block container's layout once its content width and
/// left margin are known: its height, the stacking of its children and
/// the margins it exposes. `content` says which float context the
/// children are laid out in (M11).
#[allow(clippy::too_many_arguments)]
fn layout_block_contents<'a>(
    b: &'a LayoutBox,
    edges: &Edges,
    cb_width: f32,
    cb_height: Option<f32>,
    content_width: f32,
    margin_left: f32,
    content: Content<'_>,
    fonts: &FontCollection,
) -> BlockLayout<'a> {
    let Edges { border, padding } = edges;
    // What keeps this box's margins from adjoining its children's: a
    // border or padding on that side (CSS2 §8.3.1), or the box being a
    // block formatting context root, whose children are in a different
    // formatting context from its own margins (§8.3.1's first
    // condition) — on both sides at once, and its own top and bottom
    // margins never adjoin each other either.
    let bfc_root = establishes_bfc(&b.style);
    let stops_top = bfc_root || border[TOP] > 0.0 || padding[TOP] > 0.0;
    let stops_bottom = bfc_root || border[BOTTOM] > 0.0 || padding[BOTTOM] > 0.0;
    let non_content_height = edges.vertical();
    let specified_height = definite_height(&b.style, cb_height, non_content_height);
    let (min_height, max_height) = height_bounds(&b.style, cb_height, non_content_height);
    // CSS2 §8.3.1: a box collapses its own margins through, or lets a
    // child's bottom margin escape, only with `height: auto` and
    // `min-height: 0`. Vertical margins resolve a percentage against
    // the containing block's *width* (§8.3).
    let height_is_auto = specified_height.is_none() && min_height <= 0.0;
    let own_top = AdjoiningMargins::of(b.style.margin[TOP].resolve(cb_width, 0.0));
    let own_bottom = AdjoiningMargins::of(b.style.margin[BOTTOM].resolve(cb_width, 0.0));
    let non_content_width = edges.horizontal();
    let can_collapse_through = !stops_top && !stops_bottom && height_is_auto;
    let content_x = border[LEFT] + padding[LEFT];

    // An empty block (no children, no border/padding, auto height, not
    // a formatting context root): its own margin-top and margin-bottom
    // collapse with *each other*, escaping through as a single set on
    // both sides at once — as if the box weren't there at all,
    // margin-wise (CSS2 §8.3.1).
    if b.children.is_empty() && can_collapse_through {
        let set = own_top.join(own_bottom);
        return BlockLayout {
            root: box_fragment(content_width + non_content_width, 0.0, b),
            height: 0.0,
            margin_left,
            margin_top: set,
            margin_bottom: set,
            top_edge: own_top,
            bottom_edge: own_bottom,
            self_collapsing: true,
            last_baseline: None,
            absolutes: Vec::new(),
        };
    }

    // Where the children are in the float context's coordinates (M11):
    // a root's content box is the origin of a fresh context; otherwise
    // the box's content box is at its own left edges from the parent's,
    // and its top is known once the margins above it have resolved —
    // at once when this box's top edge separates them from its
    // children's (its own margin joins the pending set, which resolves
    // at its top border edge), else when the first content below does.
    let mut own_floats = FloatContext::default();
    let (floats, at) = match content {
        Content::Root => (&mut own_floats, At::root()),
        Content::In { floats, at } => {
            let x = at.x + margin_left + content_x;
            let at = if stops_top {
                let top = at.anchor + at.strut.join(own_top).resolve() + border[TOP] + padding[TOP];
                At {
                    x,
                    origin: Some(top),
                    anchor: top,
                    strut: AdjoiningMargins::default(),
                }
            } else {
                At {
                    x,
                    origin: None,
                    anchor: at.anchor,
                    strut: at.strut.join(own_top),
                }
            };
            (floats, at)
        }
    };
    let children = layout_children(
        &b.children,
        content_width,
        specified_height,
        floats,
        at,
        fonts,
    );
    if bfc_root {
        // A float still waiting at the end of its context is placed
        // where the content would have been.
        floats.resolve_pending(children.end.position());
    }

    // Every child collapsed through (empty blocks all the way down):
    // their one set and this box's own margins adjoin, and so does the
    // box's top with its bottom.
    if children.nothing_materialised && can_collapse_through {
        let top_edge = own_top.join(children.leading);
        let set = top_edge.join(own_bottom);
        let mut root = box_fragment(content_width + non_content_width, 0.0, b);
        root.children = children
            .fragments
            .into_iter()
            .map(|f| f.translated(content_x, 0.0))
            .collect();
        let absolutes = settle_absolutes(
            b,
            &mut root,
            border,
            carried(children.absolutes, content_x, 0.0),
            fonts,
        );
        return BlockLayout {
            root,
            height: 0.0,
            margin_left,
            margin_top: set,
            margin_bottom: set,
            top_edge,
            bottom_edge: children.trailing.join(own_bottom),
            self_collapsing: true,
            last_baseline: None,
            absolutes,
        };
    }

    // `layout_children` materializes the *leading* set itself only when
    // this box's top separates it (then it is already in the content's
    // positions and `leading` comes back empty) or when nothing in the
    // content did (then it is the gap the content would have followed,
    // `child_shift`, and the content height when the bottom separates
    // too). Otherwise it keeps escaping outward, joined with this box's
    // own margin-top. This is the step a naive "always materialize
    // inside the parent" design gets wrong: that double-counts the
    // escaping case, because the SAME margin would then also get
    // re-applied one level up, when *this* box gets positioned among
    // *its* own siblings.
    let (child_shift, margin_top) = if stops_top {
        (children.leading.resolve(), own_top)
    } else {
        (0.0, own_top.join(children.leading))
    };

    // The content's height when `height` is `auto` (CSS2 §10.6.3): what
    // the box is unless told otherwise, and what a scroll container's
    // content scrolls over regardless.
    let auto_content_height = if children.nothing_materialised {
        // Every child collapsed through: `leading` and `trailing` are
        // one set, used exactly once. With no top separation it
        // escaped through the top (joined into `margin_top` above);
        // with no bottom separation it escapes through the bottom
        // (joined into `margin_bottom` below) and the content height
        // is zero (§10.6.3, last case) even though the empty box
        // itself sits past the gap; only with both does it become the
        // gap inside, counted once.
        if stops_top && stops_bottom {
            child_shift
        } else {
            0.0
        }
    } else {
        let base = children.content_height + child_shift;
        if stops_bottom {
            // The last child's margin-bottom doesn't escape — it
            // becomes real space above the padding/border.
            base + children.trailing.resolve()
        } else {
            base
        }
    };
    // A block formatting context root's content reaches down to the
    // bottom margin edge of the lowest float in it (CSS2 §10.6.7); any
    // other block's ignores its floats, which overflow it.
    let auto_content_height = match floats.lowest_bottom() {
        Some(bottom) if bfc_root => auto_content_height.max(bottom),
        _ => auto_content_height,
    };
    // CSS2 §10.7: the tentative height clamped to `max-height`, then
    // `min-height`.
    let content_height = specified_height.unwrap_or(auto_content_height);
    let content_height = max_height
        .map_or(content_height, |max| content_height.min(max))
        .max(min_height);

    let margin_bottom = if stops_bottom || !height_is_auto {
        own_bottom
    } else {
        own_bottom.join(children.trailing)
    };

    let border_box_width = content_width + non_content_width;
    let border_box_height =
        border[TOP] + padding[TOP] + content_height + padding[BOTTOM] + border[BOTTOM];

    let mut root = box_fragment(border_box_width, border_box_height, b);
    // The content's positions are from the content top: the leading
    // set, where it is this box's, is in them already (or, with nothing
    // materialised, is the gap the self-collapsing boxes sit above).
    let content_y = border[TOP] + padding[TOP];
    root.children = children
        .fragments
        .into_iter()
        .map(|f| f.translated(content_x, content_y))
        .collect();
    if bfc_root {
        // The floats of this context get their positions, under
        // whichever box of the context holds their fragment.
        attach_floats(
            &mut root.children,
            own_floats.placed(),
            0.0,
            0.0,
            content_x,
            border[TOP] + padding[TOP],
        );
    }
    // The absolutely positioned boxes met inside: laid out here when
    // this box is their containing block, carried up otherwise (M11).
    let absolutes = settle_absolutes(
        b,
        &mut root,
        border,
        carried(children.absolutes, content_x, content_y),
        fonts,
    );
    if b.style.overflow.clips() {
        // The scrollable overflow area (M10.5, see `Fragment::
        // scroll_height`): the content as if the height were auto, or
        // a descendant reaching lower, from the padding box's top to
        // the bottom padding's end, and never less than the padding
        // box itself.
        let extent = root.children.iter().map(extent_bottom).fold(0.0, f32::max)
            - border[TOP]
            - padding[TOP];
        let padding_box = padding[TOP] + content_height + padding[BOTTOM];
        root.scroll_height =
            (padding[TOP] + auto_content_height.max(extent) + padding[BOTTOM]).max(padding_box);
    }

    BlockLayout {
        root,
        height: border_box_height,
        margin_left,
        margin_top,
        margin_bottom,
        top_edge: margin_top,
        bottom_edge: margin_bottom,
        self_collapsing: false,
        last_baseline: children.last_baseline.map(|y| content_y + y),
        absolutes,
    }
}

/// Settles the absolutely positioned boxes carried up to `b`, whose
/// fragment `root` is built and whose border widths are `border`
/// (M11). A positioned box is the containing block of the `position:
/// absolute` boxes below it (CSS2 §10.1): each is laid out against its
/// padding box, from the static position carried, and its fragment
/// goes under `root` after the in-flow content — so an `overflow`
/// box between the two does not clip it, and a scroll container
/// between them does not scroll it. A `position: fixed` box, and
/// anything carried by a box that is not positioned, goes on up;
/// returned relative to `root`'s border box.
fn settle_absolutes<'a>(
    b: &LayoutBox,
    root: &mut Fragment,
    border: &[f32; 4],
    absolutes: Vec<Deferred<'a>>,
    fonts: &FontCollection,
) -> Vec<Deferred<'a>> {
    if !b.style.position.is_positioned() {
        return absolutes;
    }
    let (mine, mut up): (Vec<_>, Vec<_>) = absolutes
        .into_iter()
        .partition(|d| d.b.style.position == Position::Absolute);
    let padding_w = (root.w - border[LEFT] - border[RIGHT]).max(0.0);
    let padding_h = (root.h - border[TOP] - border[BOTTOM]).max(0.0);
    for d in mine {
        let (fragment, fixed) = layout_absolute(
            d.b,
            d.x - border[LEFT],
            d.y - border[TOP],
            padding_w,
            Some(padding_h),
            fonts,
        );
        up.extend(carried(fixed, border[LEFT], border[TOP]));
        root.children
            .push(fragment.translated(border[LEFT], border[TOP]));
    }
    up
}

/// A `<length-percentage> | auto` inset or margin as a used value:
/// `None` for `auto`, a percentage of `of` — or `auto` too when `of`,
/// a containing block's height, is not known (CSS2 §9.3.2, §10.5).
fn used(v: LengthPercentageAuto, of: Option<f32>) -> Option<f32> {
    match v {
        LengthPercentageAuto::Auto => None,
        LengthPercentageAuto::Px(px) => Some(px),
        LengthPercentageAuto::Percent(p) => of.map(|base| base * p / 100.0),
    }
}

/// Whether an absolutely positioned box's layout against the initial
/// containing block depends on the viewport's height: a percentage
/// `top`, `height`, `min-height` or `max-height`, or a `bottom` that
/// is not `auto`.
fn uses_cb_height(style: &ComputedStyle) -> bool {
    let percent = |v: LengthPercentage| matches!(v, LengthPercentage::Percent(_));
    matches!(style.inset[TOP], LengthPercentageAuto::Percent(_))
        || !style.inset[BOTTOM].is_auto()
        || matches!(style.height, LengthPercentageAuto::Percent(_))
        || percent(style.min_height)
        || style.max_height.is_some_and(percent)
}

/// Lays out an absolutely positioned box (M11) against its containing
/// block's padding box, `cb_width` by `cb_height` — `None` when the
/// height is not known, the initial containing block's without a
/// viewport height, which makes `bottom` and a percentage `top` or
/// height `auto` — from the static position `(sx, sy)` its margin box
/// would have had in flow, in the padding box's coordinates. Returns
/// the box's fragment positioned there, with its content, and the
/// `position: fixed` boxes met inside, relative to the padding box.
///
/// Width is CSS2 §10.3.7: `left`, `width` and `right` all `auto` is a
/// shrink-to-fit width at the static position; one `auto` among them
/// is solved from the other two, `auto` margins zero; none `auto`
/// solves the margins — both `auto` centres the box, one `auto` takes
/// the rest — and ignores `right` when over-constrained (this engine
/// being left-to-right throughout). `min-width` and `max-width` clamp
/// the width with the edge solved again (§10.4). Height is §10.6.4 the
/// same way with `top`, `height` and `bottom`, an `auto` height the
/// content's, `bottom` ignored when over-constrained, `min-height` and
/// `max-height` clamping (§10.7). The box is a block formatting context
/// root; its content's percentages are of the padding box.
fn layout_absolute<'a>(
    b: &'a LayoutBox,
    sx: f32,
    sy: f32,
    cb_width: f32,
    cb_height: Option<f32>,
    fonts: &FontCollection,
) -> (Fragment, Vec<Deferred<'a>>) {
    let style = &b.style;
    let edges = Edges::of(style, cb_width);
    let horizontal_edges = edges.horizontal();
    let vertical_edges = edges.vertical();
    let (left, right) = (
        used(style.inset[LEFT], Some(cb_width)),
        used(style.inset[RIGHT], Some(cb_width)),
    );
    let (top, bottom) = (
        used(style.inset[TOP], cb_height),
        used(style.inset[BOTTOM], cb_height),
    );
    // Margins: a percentage is of the containing block's width on every
    // side (CSS2 §8.3).
    let margin = |side: usize| used(style.margin[side], Some(cb_width));
    let (margin_left, margin_right) = (margin(LEFT), margin(RIGHT));
    let (margin_top, margin_bottom) = (margin(TOP), margin(BOTTOM));

    // §10.3.7, for one value of `width` (`None` for `auto`): the left
    // edge, the content width and the two margins.
    let c = width_constraints(style, cb_width, horizontal_edges);
    let shrink = |available: f32| {
        let (min_content, max_content) = crate::intrinsic::content_widths(b, fonts);
        min_content.max(available.max(0.0).min(max_content))
    };
    let solve_width = |width: Option<f32>| -> (f32, f32, f32, f32) {
        let ml = margin_left.unwrap_or(0.0);
        let mr = margin_right.unwrap_or(0.0);
        match (left, width, right) {
            (None, None, None) => {
                let w = shrink(cb_width - sx - ml - horizontal_edges - mr);
                (sx, w, ml, mr)
            }
            (Some(l), Some(w), Some(r)) => {
                let rest = cb_width - l - horizontal_edges - w - r;
                match (margin_left, margin_right) {
                    (None, None) if rest >= 0.0 => (l, w, rest / 2.0, rest / 2.0),
                    (None, None) => (l, w, 0.0, rest),
                    (None, Some(mr)) => (l, w, rest - mr, mr),
                    (Some(ml), None) => (l, w, ml, rest - ml),
                    // Over-constrained: `right` is ignored.
                    (Some(ml), Some(_)) => (l, w, ml, rest - ml),
                }
            }
            (None, None, Some(r)) => {
                let w = shrink(cb_width - r - mr - horizontal_edges - ml);
                (cb_width - r - mr - horizontal_edges - w - ml, w, ml, mr)
            }
            (None, Some(w), None) => (sx, w, ml, mr),
            (Some(l), None, None) => {
                let w = shrink(cb_width - l - ml - horizontal_edges - mr);
                (l, w, ml, mr)
            }
            (None, Some(w), Some(r)) => (cb_width - r - mr - horizontal_edges - w - ml, w, ml, mr),
            (Some(l), None, Some(r)) => {
                let w = (cb_width - l - ml - horizontal_edges - mr - r).max(0.0);
                (l, w, ml, mr)
            }
            (Some(l), Some(w), None) => (l, w, ml, mr),
        }
    };
    let mut solved = solve_width(c.width);
    if let Some(max) = c.max
        && solved.1 > max
    {
        solved = solve_width(Some(max));
    }
    if solved.1 < c.min {
        solved = solve_width(Some(c.min));
    }
    let (left, content_width, margin_left, _margin_right) = solved;

    // The content, as a block formatting context root's, with the
    // style's own height (or the content's); the box's used height
    // may then come from `top` and `bottom` instead.
    let layout = layout_block_contents(
        b,
        &edges,
        cb_width,
        cb_height,
        content_width,
        margin_left,
        Content::Root,
        fonts,
    );
    let mut fragment = layout.root;
    let specified_height = definite_height(style, cb_height, vertical_edges);
    let (min_height, max_height) = height_bounds(style, cb_height, vertical_edges);
    // §10.6.4: the top edge, the border-box height and the margins.
    let solve_height = |height: Option<f32>| -> (f32, f32, f32, f32) {
        let mt = margin_top.unwrap_or(0.0);
        let mb = margin_bottom.unwrap_or(0.0);
        let content = |h: Option<f32>| h.unwrap_or(fragment.h - vertical_edges) + vertical_edges;
        match (top, height, bottom, cb_height) {
            (Some(t), Some(h), Some(btm), Some(cb_h)) => {
                let h = h + vertical_edges;
                let rest = cb_h - t - h - btm;
                match (margin_top, margin_bottom) {
                    (None, None) if rest >= 0.0 => (t, h, rest / 2.0, rest / 2.0),
                    (None, None) => (t, h, 0.0, rest),
                    (None, Some(mb)) => (t, h, rest - mb, mb),
                    (Some(mt), None) => (t, h, mt, rest - mt),
                    // Over-constrained: `bottom` is ignored.
                    (Some(mt), Some(_)) => (t, h, mt, rest - mt),
                }
            }
            (Some(t), None, Some(btm), Some(cb_h)) => {
                let h = (cb_h - t - mt - mb - btm).max(0.0);
                (t, h, mt, mb)
            }
            (None, h, Some(btm), Some(cb_h)) => {
                let h = content(h);
                (cb_h - btm - mb - h - mt, h, mt, mb)
            }
            (Some(t), h, _, _) => (t, content(h), mt, mb),
            (None, h, _, _) => (sy, content(h), mt, mb),
        }
    };
    let mut solved = solve_height(specified_height);
    let clamp = |border_height: f32| {
        let content = border_height - vertical_edges;
        let content = max_height
            .map_or(content, |max| content.min(max))
            .max(min_height);
        content + vertical_edges
    };
    if clamp(solved.1) != solved.1 {
        solved = solve_height(Some(clamp(solved.1) - vertical_edges));
    }
    let (top, border_height, margin_top, _margin_bottom) = solved;
    fragment.h = border_height.max(0.0);
    fragment.x = left + margin_left;
    fragment.y = top + margin_top;
    let fixed = carried(layout.absolutes, fragment.x, fragment.y);
    (fragment, fixed)
}

/// A block-level child laid out and placed horizontally among its
/// container's children (M11): where its top goes when clearance or a
/// float moved it, and where its left edge is.
struct BlockPlacement<'a> {
    layout: BlockLayout<'a>,
    /// The border box's left edge from the content box's: the box's
    /// left margin, plus the room a context root left to a float.
    x: f32,
    /// `Some(top)`, the border box's top in root coordinates, when
    /// clearance (CSS2 §9.5.2) or a context root's move below a float
    /// (§9.5) put it there: the margins pending above it resolve and
    /// stop there, and its own top margin lies below the gap. `None`
    /// when the box's margins collapse with its siblings' as usual.
    separated: Option<f32>,
}

/// Whether a box's top edge separates its margins from its children's
/// (CSS2 §8.3.1): a context root, or a top border or padding.
fn separates_top(style: &ComputedStyle, cb_width: f32) -> bool {
    let edges = Edges::of(style, cb_width);
    establishes_bfc(style) || edges.border[TOP] > 0.0 || edges.padding[TOP] > 0.0
}

/// Lays out a [`BoxKind::Block`] child of a container whose content
/// box is at `at` and places it against the floats (M11): with
/// clearance when its `clear` names floats its hypothetical top border
/// edge is not past (CSS2 §9.5.2 — the top is put level with the
/// lowest such float's bottom margin edge), and, for a block
/// formatting context root, in the room between the floats at its top
/// (§9.5: its border box must not overlap a float's margin box): as
/// far left as that room allows, its `auto` width the room's, moved
/// down to the next height at which a float ends when it does not fit
/// — a box too wide for a room with no float beside it overflows, as
/// in normal flow. The room must hold the box's whole height: when a
/// float lower down narrows it, the narrower room is tried at the same
/// top, so a wider float below a narrow one is still avoided.
///
/// A float waiting for the flow position is placed before a box that
/// resolves it: a box whose `clear` names such a float cannot collapse
/// its margins past it, so the position resolves above the box's own
/// margin, where the margins pending so far end (as clearance would
/// have it); a box whose top edge separates its margins resolves it at
/// its hypothetical top.
fn place_block<'a>(
    child: &'a LayoutBox,
    cb_width: f32,
    cb_height: Option<f32>,
    floats: &mut FloatContext,
    at: At,
    fonts: &FontCollection,
) -> BlockPlacement<'a> {
    let root = establishes_bfc(&child.style);
    let own_top = AdjoiningMargins::of(child.style.margin[TOP].resolve(cb_width, 0.0));
    if at.origin.is_none() {
        if floats.waits_to_be_cleared(child.style.clear) {
            floats.resolve_pending(at.anchor + at.strut.resolve());
        } else if separates_top(&child.style, cb_width) {
            floats.resolve_pending(at.anchor + at.strut.join(own_top).resolve());
        }
    }
    if !root || floats.is_empty() {
        let layout = layout_block(child, cb_width, cb_width, cb_height, floats, at, fonts);
        let above = if layout.self_collapsing {
            layout.top_edge
        } else {
            layout.margin_top
        };
        let hypothetical = at.anchor + at.strut.join(above).resolve();
        let Some(top) = floats
            .clear_bottom(child.style.clear)
            .filter(|bottom| hypothetical < *bottom)
        else {
            return BlockPlacement {
                x: layout.margin_left,
                layout,
                separated: None,
            };
        };
        // Clearance: the box's top border edge is at `top`, the margins
        // above it — its own, and what escaped from its first child —
        // inside the clearance. Its content was laid out as if at the
        // hypothetical top; a root's content does not depend on where
        // the box is, an empty box has none, any other's does (its
        // lines beside floats, its own floats), so it is laid out
        // again from where it really is: the pending margins put so
        // that joining the box's resolve to its top.
        let layout = if root || layout.self_collapsing {
            layout
        } else {
            let inside = At {
                x: at.x,
                origin: None,
                anchor: top - above.resolve(),
                strut: above,
            };
            layout_block(child, cb_width, cb_width, cb_height, floats, inside, fonts)
        };
        return BlockPlacement {
            x: layout.margin_left,
            layout,
            separated: Some(top),
        };
    }
    // A context root's top is its own margin's, no child's escaping
    // through it; from there, the first room that holds it.
    let hypothetical = at.anchor + at.strut.join(own_top).resolve();
    let mut top = floats
        .clear_bottom(child.style.clear)
        .map_or(hypothetical, |bottom| hypothetical.max(bottom));
    let (cb_left, cb_right) = (at.x, at.x + cb_width);
    loop {
        let (mut left, mut right) = floats.space(top, 0.0, cb_left, cb_right);
        let fitted = loop {
            let room = right - left;
            let layout = layout_block(child, cb_width, room, cb_height, floats, at, fonts);
            let alone = left == cb_left && right == cb_right;
            if !alone && layout.margin_left + layout.root.w > room {
                break None;
            }
            let (lower_left, lower_right) = floats.space(top, layout.height, cb_left, cb_right);
            if lower_left == left && lower_right == right {
                break Some(BlockPlacement {
                    x: left - cb_left + layout.margin_left,
                    layout,
                    separated: (top > hypothetical).then_some(top),
                });
            }
            // A float lower down narrows the room: try that room.
            left = lower_left;
            right = lower_right;
        };
        if let Some(placement) = fitted {
            return placement;
        }
        match floats.next_bottom(top) {
            Some(bottom) if bottom > top => top = bottom,
            // Nothing ends below, so no float was beside the box: the
            // room was the whole block, where a box always fits.
            _ => unreachable!("a room with a float in it has a float ending below it"),
        }
    }
}

/// `b`'s own box fragment at its border-box origin, no content yet.
fn box_fragment(w: f32, h: f32, b: &LayoutBox) -> Fragment {
    Fragment {
        x: 0.0,
        y: 0.0,
        w,
        h,
        style: b.style.clone(),
        kind: FragmentKind::Box,
        node: b.node,
        ascent: 0.0,
        scroll_height: 0.0,
        children: Vec::new(),
    }
}

/// The result of stacking a sequence of sibling boxes in normal flow.
struct ChildrenLayout<'a> {
    /// Every descendant's fragment, relative to the *parent's*
    /// content-box top-left `(0, 0)` — *not* counting `leading` (see
    /// that field's doc): the first child sits at relative `y = 0`
    /// regardless of its own margin-top.
    fragments: Vec<Fragment>,
    /// Content height counting from the first child's `y = 0` — i.e.
    /// also not counting `leading` — and not counting any trailing
    /// collapsible margin either (the caller decides whether *that* one
    /// becomes real space or escapes further).
    content_height: f32,
    /// The set of margins that escape past the first child's top
    /// (its own margin-top, joined with whatever of *its* children
    /// escaped, recursively, and with every self-collapsing box in
    /// front of the first real one) — deliberately **not** baked into
    /// `fragments`' positions or `content_height`. Whether it ever
    /// becomes real space depends on information only the caller has
    /// (its own border/padding): a parent with top separation turns it
    /// into a gap just inside its content-box top; a parent without
    /// one joins it with its *own* margin-top and keeps escaping.
    /// Materializing it here unconditionally would double-count it the
    /// moment it keeps escaping — it would show up once when the
    /// escaping value is eventually placed, *and* again as this
    /// already-baked-in internal gap. Empty when there are no children
    /// (nothing to expose).
    leading: AdjoiningMargins,
    /// The set of margins that escape past the last child's bottom,
    /// exposed the same way, for parent/last-child collapsing.
    trailing: AdjoiningMargins,
    /// No child produced a line box or a box with height: every
    /// margin went into one set, and `leading` and `trailing` are that
    /// same set.
    nothing_materialised: bool,
    /// The baseline of the last line box among the children, from the
    /// first child's `y = 0` like `content_height`; `None` when none
    /// made a line.
    last_baseline: Option<f32>,
    /// The flow position after the last child, in the float context's
    /// coordinates (M11): where a float met next would go.
    end: At,
    /// The absolutely positioned boxes met among the children or
    /// carried up by them (M11), relative to the content-box origin.
    absolutes: Vec<Deferred<'a>>,
}

/// Stacks a block container's children (M10.4: all block-level, per the
/// box tree builder's contract — an inline-level box that a hand-built
/// tree puts here is laid out as a run of one; a float, M11, is placed
/// in the float context at the flow position it is met at and takes no
/// room). An anonymous block is its inline formatting context: its
/// lines, aligned per its own inherited `text-align` (M7.5), at the
/// container's full width, shortened beside floats.
///
/// `at` is the container's content box in its float context (M11):
/// its top is resolved when the container's top edge separates its
/// margins from its children's — then the leading set of margins is
/// materialised here, below the content top, and comes back empty —
/// and unresolved otherwise, when the leading set escapes to the
/// caller and the first content's top is `y = 0`. In both the content's
/// positions are relative to the content top, and the float context's
/// positions, kept in `at`, say the same in the root's coordinates.
fn layout_children<'a>(
    children: &'a [LayoutBox],
    cb_width: f32,
    cb_height: Option<f32>,
    floats: &mut FloatContext,
    at: At,
    fonts: &FontCollection,
) -> ChildrenLayout<'a> {
    let mut fragments = Vec::new();
    let mut absolutes: Vec<Deferred<'a>> = Vec::new();
    let mut y = 0.0f32;
    // The set of margins adjoining BETWEEN two siblings, not yet
    // materialized (never the leading one, which `still_leading` routes
    // to `leading` instead).
    let mut pending = AdjoiningMargins::default();
    let mut still_leading = true;
    let mut leading = AdjoiningMargins::default();
    let mut last_baseline = None;
    // The content top is resolved: the leading set is this container's
    // to materialise, at the first content's top.
    let top_resolved = at.origin.is_some();
    let mut at = at;

    for child in children {
        match &child.kind {
            // A table and each of its parts are laid out as the blocks
            // they are built as inside, stacked, until `layout::table`
            // lays a table's grid out (M12 step 3).
            BoxKind::Block
            | BoxKind::Table
            | BoxKind::RowGroup(_)
            | BoxKind::Row
            | BoxKind::ColumnGroup { .. }
            | BoxKind::Column { .. }
            | BoxKind::Cell { .. }
            | BoxKind::Caption => {
                let BlockPlacement {
                    layout,
                    x,
                    separated,
                } = place_block(child, cb_width, cb_height, floats, at, fonts);
                // `position: relative` moves the fragment, and its
                // subtree with it, from where the flow put it (M11).
                let (dx, dy) = relative_offset(&child.style, cb_width, cb_height);
                let x = x + dx;
                if let Some(top) = separated {
                    // Clearance, or a context root moved below a float:
                    // the margins pending above resolve here (CSS2
                    // §9.5.2: clearance inhibits margin collapsing), the
                    // box's own top margin lies below them, and its top
                    // border edge is at `top`. A self-collapsing box so
                    // placed is not self-collapsing for the margins
                    // above it; only its bottom set goes on.
                    let base = at.anchor + at.strut.resolve();
                    let origin = *at.origin.get_or_insert(base);
                    let child_y = top - origin;
                    still_leading = false;
                    // A float still waiting — inside an empty cleared
                    // box, say — is placed at the resolved position.
                    floats.resolve_pending(top);
                    if layout.self_collapsing {
                        y = child_y;
                        pending = layout.bottom_edge;
                    } else {
                        y = child_y + layout.height;
                        pending = layout.margin_bottom;
                        if let Some(baseline) = layout.last_baseline {
                            last_baseline = Some(child_y + baseline);
                        }
                    }
                    fragments.push(layout.root.translated(x, child_y + dy));
                    absolutes.extend(carried(layout.absolutes, x, child_y + dy));
                    at.anchor = origin + y;
                    at.strut = pending;
                    continue;
                }
                if layout.self_collapsing {
                    // Its one set joins the margins around it and stays
                    // open; the box itself has no height. Its top border
                    // edge is where its top margin, with the margins
                    // pending before it, resolves to (CSS2 §8.3.1: as if
                    // it had a bottom border), and the parent's content
                    // top while the set is still escaping there (the
                    // same top border edge as the parent's); a float
                    // inside it is what makes the position observable.
                    let box_y = if still_leading {
                        let box_y = if top_resolved {
                            leading.join(layout.top_edge).resolve()
                        } else {
                            0.0
                        };
                        leading = leading.join(layout.margin_top);
                        box_y
                    } else {
                        let box_y = y + pending.join(layout.top_edge).resolve();
                        pending = pending.join(layout.margin_top);
                        box_y
                    };
                    at.strut = at.strut.join(layout.margin_top);
                    fragments.push(layout.root.translated(x, box_y + dy));
                    absolutes.extend(carried(layout.absolutes, x, box_y + dy));
                    continue;
                }
                let root_top = at.anchor + at.strut.join(layout.margin_top).resolve();
                if still_leading {
                    leading = leading.join(layout.margin_top);
                    still_leading = false;
                    if top_resolved {
                        y = leading.resolve();
                        leading = AdjoiningMargins::default();
                    }
                    // Else `y` stays at 0 — see `leading`'s doc.
                } else {
                    y += pending.join(layout.margin_top).resolve();
                }
                let child_y = y;
                if let Some(baseline) = layout.last_baseline {
                    last_baseline = Some(child_y + baseline);
                }
                fragments.push(layout.root.translated(x, child_y + dy));
                absolutes.extend(carried(layout.absolutes, x, child_y + dy));
                y = child_y + layout.height;
                pending = layout.margin_bottom;
                if at.origin.is_none() {
                    // The margins above a box that is not self-collapsing
                    // end at its top (CSS2 §8.3.1): the flow position
                    // resolves there, and a float still waiting — met
                    // before the box, or inside it before any content —
                    // is placed level with it.
                    floats.resolve_pending(root_top);
                    at.origin = Some(root_top);
                }
                at.anchor = root_top + layout.height;
                at.strut = pending;
            }
            BoxKind::Float => {
                // Out of flow: sized as an inline-block, placed at the
                // flow position after the margins resolved so far — or
                // held until that position is known — and the flow goes
                // on as if it were not there. Its fragment is pushed in
                // its place and positioned by the root's `attach_floats`.
                // The absolutely positioned boxes it carries are moved
                // to where it was placed — or, while it waits, to where
                // it will be placed in the common case, the content top.
                let atomic = layout_float(child, cb_width, cb_height, fonts);
                let float = pending_float(child, &atomic, at.x, cb_width);
                let (fx, fy) = match at.origin {
                    Some(origin) => {
                        let (px, py) = floats.place(float, at.position());
                        (px - at.x, py - origin)
                    }
                    None => {
                        floats.defer(float);
                        (0.0, 0.0)
                    }
                };
                let (dx, dy) = atomic.relative;
                absolutes.extend(carried(
                    atomic.absolutes,
                    fx + atomic.margin_left + dx,
                    fy + atomic.margin_top + dy,
                ));
                fragments.push(atomic.root);
            }
            BoxKind::Absolute => {
                // Out of flow, waiting for its containing block: its
                // static position is the flow position it is met at,
                // after the margins resolved so far (as a self-collapsing
                // box's top edge is), at the content box's left.
                let flow_y = if still_leading {
                    if top_resolved { leading.resolve() } else { 0.0 }
                } else {
                    y + pending.resolve()
                };
                absolutes.push(Deferred {
                    b: child,
                    x: 0.0,
                    y: flow_y,
                });
            }
            BoxKind::AnonymousBlock | BoxKind::Inline | BoxKind::InlineBlock | BoxKind::Text(_) => {
                // One inline formatting context (M7): the anonymous
                // block's children wrap onto lines together, from the
                // flow position down.
                let run = match child.kind {
                    BoxKind::AnonymousBlock => child.children.as_slice(),
                    _ => std::slice::from_ref(child),
                };
                let root_top = at.position();
                let lines =
                    layout_inline(run, cb_width, cb_height, &child.style, floats, at, fonts);
                if lines.last_baseline.is_none() {
                    // No line box at all (an empty inline element, or
                    // floats alone): treated as not existing, so it
                    // separates no margins (CSS2 §9.4.2). A float's
                    // fragment still goes in its place, and an
                    // absolutely positioned box's static position is
                    // the flow position, as a self-collapsing box's.
                    let flow_y = if still_leading {
                        if top_resolved { leading.resolve() } else { 0.0 }
                    } else {
                        y + pending.resolve()
                    };
                    fragments.extend(lines.fragments);
                    absolutes.extend(carried(lines.absolutes, 0.0, flow_y));
                    continue;
                }
                // A line box always separates (margins never collapse
                // through it, same as a border/padding would) — flush
                // whatever's pending (or stop a still-leading escape
                // dead, materializing nothing since there was never a
                // border/padding to place it against).
                if still_leading {
                    still_leading = false;
                    if top_resolved {
                        y = leading.resolve();
                        leading = AdjoiningMargins::default();
                    }
                    // Else `leading` stays as joined so far: nothing
                    // escapes past real content.
                } else {
                    y += pending.resolve();
                }
                pending = AdjoiningMargins::default();
                if let Some(baseline) = lines.last_baseline {
                    last_baseline = Some(y + baseline);
                }
                fragments.extend(lines.fragments.into_iter().map(|f| f.translated(0.0, y)));
                absolutes.extend(carried(lines.absolutes, 0.0, y));
                y += lines.height;
                at.origin.get_or_insert(root_top);
                at.anchor = root_top + lines.height;
                at.strut = pending;
            }
        }
    }

    ChildrenLayout {
        fragments,
        content_height: y,
        leading,
        trailing: if still_leading { leading } else { pending },
        nothing_materialised: still_leading,
        last_baseline,
        end: at,
        absolutes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dom::Element;

    const VIEWPORT: f32 = 800.0;

    /// The bundled test font, so that every machine measures the same
    /// and no test is skipped for want of a system font.
    fn test_fonts() -> FontCollection {
        FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
    }

    /// Builds the box tree for `root` under `css_src` and lays it out
    /// against [`VIEWPORT`] — the full `style` -> `box_tree` -> `block`
    /// pipeline, exactly as `engine` will drive it (M6.7's integration
    /// point, exercised at the unit level here).
    fn fragments_of(root: Element, css_src: &str) -> Vec<Fragment> {
        let fonts = test_fonts();
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(css_src).expect("test css is valid");
        let styled = style::style_document(&doc, &[sheet]);
        let boxes = crate::build_box_tree(&styled);
        layout(&boxes, VIEWPORT, &fonts)
    }

    fn box_fragments(frags: &[Fragment]) -> Vec<&Fragment> {
        frags
            .iter()
            .filter(|f| matches!(f.kind, FragmentKind::Box))
            .collect()
    }

    #[test]
    fn empty_auto_block_fills_width_and_has_zero_height() {
        let frags = fragments_of(Element::new("div"), "div { display: block; }");
        assert_eq!(frags.len(), 1);
        let f = &frags[0];
        assert_eq!((f.x, f.y, f.w, f.h), (0.0, 0.0, VIEWPORT, 0.0));
    }

    #[test]
    fn padding_and_vertical_margin_percentages_use_the_containing_block_width() {
        // CSS2 §8.3 and §8.4: every margin and padding percentage, the
        // vertical ones included, is of the containing block's width —
        // here 200px, so 10% is 20px on all four sides.
        let root = Element::new("div").push(Element::new("p").text("x"));
        let frags = fragments_of(
            root,
            "div { width: 200px; margin: 0; } p { margin: 10% 0 5% 0; padding: 10%; }",
        );
        let boxes = box_fragments(&frags);
        let (div, p) = (boxes[0], boxes[1]);
        assert_eq!(div.y, 20.0, "p's 10% top margin escapes through the div");
        assert_eq!((p.x, p.y, p.w), (0.0, 20.0, 200.0));
        let text = frags
            .iter()
            .find(|f| matches!(f.kind, FragmentKind::Text(_)))
            .expect("the text");
        assert_eq!(text.x, 20.0, "20px of left padding");
        assert_eq!(text.top(), 40.0, "20px of top padding under the margin");
        let m = test_fonts().primary().metrics(16.0);
        assert_eq!(p.h, 20.0 + m.line_height() + 20.0);
    }

    #[test]
    fn box_sizing_border_box_puts_padding_and_border_inside_the_size() {
        let css = "div { width: 100px; height: 60px; padding: 10px; margin: 0;
                         border: 5px solid #000000; box-sizing: border-box; }";
        let frags = fragments_of(Element::new("div"), css);
        assert_eq!((frags[0].w, frags[0].h), (100.0, 60.0));
        let css = css.replace("border-box", "content-box");
        let frags = fragments_of(Element::new("div"), &css);
        assert_eq!((frags[0].w, frags[0].h), (130.0, 90.0));
        // A border-box size smaller than its padding and border leaves
        // no content, not a negative amount.
        let frags = fragments_of(
            Element::new("div"),
            "div { width: 10px; height: 10px; padding: 10px; margin: 0; box-sizing: border-box; }",
        );
        assert_eq!((frags[0].w, frags[0].h), (20.0, 20.0));
    }

    #[test]
    fn min_and_max_width_clamp_the_width_and_the_margins_follow() {
        // An auto width capped by max-width, centred by its auto margins
        // (CSS2 §10.4: the margins are resolved again for the clamped
        // width).
        let frags = fragments_of(
            Element::new("div"),
            "div { max-width: 200px; margin: 0 auto; height: 10px; }",
        );
        assert_eq!((frags[0].x, frags[0].w), (300.0, 200.0));
        // min-width raises a width that is too small; a percentage
        // max-width is of the containing block.
        let frags = fragments_of(
            Element::new("div"),
            "div { width: 50px; min-width: 100px; height: 10px; margin: 0; }",
        );
        assert_eq!(frags[0].w, 100.0);
        let frags = fragments_of(
            Element::new("div"),
            "div { max-width: 50%; margin: 0 0 0 auto; height: 10px; }",
        );
        assert_eq!((frags[0].x, frags[0].w), (400.0, 400.0));
        // Under border-box the limits measure the border box too.
        let frags = fragments_of(
            Element::new("div"),
            "div { max-width: 100px; padding: 10px; margin: 0; height: 0; box-sizing: border-box; }",
        );
        assert_eq!(frags[0].w, 100.0);
    }

    #[test]
    fn percentage_heights_need_a_definite_containing_block_height() {
        let root = Element::new("div").push(Element::new("p"));
        // The parent's height is definite: 50% of 200 is 100.
        let frags = fragments_of(
            root.clone(),
            "div { height: 200px; margin: 0; } p { height: 50%; margin: 0; }",
        );
        assert_eq!(box_fragments(&frags)[1].h, 100.0);
        // The parent's height is auto: the child's percentage is auto
        // too, and an empty box is nothing tall.
        let frags = fragments_of(
            root.clone(),
            "div { margin: 0; } p { height: 50%; margin: 0; }",
        );
        assert_eq!(box_fragments(&frags)[1].h, 0.0);
        // A chain of definite percentages resolves all the way down.
        let frags = fragments_of(
            Element::new("div").push(Element::new("section").push(Element::new("p"))),
            "div { height: 400px; margin: 0; } section { height: 50%; margin: 0; } p { height: 25%; margin: 0; }",
        );
        assert_eq!(box_fragments(&frags)[2].h, 50.0);
        // The viewport's height is the initial containing block's, when
        // given; a box of that size says so.
        let fonts = test_fonts();
        let doc = dom::Node::document(vec![Element::new("div").into()]);
        let sheet = css::parse_stylesheet("div { height: 50%; margin: 0; }").expect("valid");
        let styled = style::style_document(&doc, &[sheet]);
        let boxes = crate::build_box_tree(&styled);
        let l = layout_document_in(&boxes, VIEWPORT, Some(600.0), &fonts);
        assert_eq!(l.roots[0].h, 300.0);
        assert!(l.depends_on_viewport_height);
        let l = layout_document(&boxes, VIEWPORT, &fonts);
        assert_eq!(l.roots[0].h, 0.0);
        assert!(!l.depends_on_viewport_height);
    }

    #[test]
    fn min_and_max_height_clamp_the_height_and_a_min_height_stops_self_collapsing() {
        // An empty box with a min-height has that height and keeps its
        // own margins apart: the following paragraph sits below it.
        let root = Element::new("div")
            .push(Element::new("div").attr("class", "a"))
            .push(Element::new("p").text("x"));
        let frags = fragments_of(
            root,
            "div, p { margin: 0; } .a { min-height: 50px; margin: 10px 0; } p { margin: 0; }",
        );
        let boxes = box_fragments(&frags);
        let (a, p) = (boxes[1], boxes[2]);
        assert_eq!((a.y, a.h), (10.0, 50.0));
        assert_eq!(p.y, 70.0, "10 above, 50 tall, 10 below");
        // max-height clamps an auto height; the content overflows.
        let frags = fragments_of(
            Element::new("div").text("one two"),
            "div { max-height: 5px; margin: 0; }",
        );
        assert_eq!(frags[0].h, 5.0);
        // A definite height is clamped too, and under border-box the
        // limit measures the border box.
        let frags = fragments_of(
            Element::new("div"),
            "div { height: 500px; max-height: 100px; padding: 10px; margin: 0; box-sizing: border-box; }",
        );
        assert_eq!(frags[0].h, 100.0);
        // A percentage limit under an auto parent is no limit.
        let root = Element::new("div").push(Element::new("p").text("x"));
        let frags = fragments_of(
            root,
            "div { margin: 0; } p { max-height: 10%; min-height: 50%; margin: 0; }",
        );
        let m = test_fonts().primary().metrics(16.0);
        assert_eq!(box_fragments(&frags)[1].h, m.line_height());
    }

    #[test]
    fn a_side_whose_style_paints_nothing_takes_no_space() {
        let frags = fragments_of(
            Element::new("div").push(Element::new("p").text("x")),
            "div { width: 100px; margin: 0; border-width: 5px; border-style: none solid hidden dotted; }
             p { margin: 0; }",
        );
        let boxes = box_fragments(&frags);
        let (div, p) = (boxes[0], boxes[1]);
        assert_eq!((div.w, p.x, p.y, p.w), (110.0, 5.0, 0.0, 100.0));
    }

    #[test]
    fn a_block_inside_an_inline_is_a_sibling_whose_margins_collapse() {
        // <div><p class=a>x</p><span><div class=b>y</div></span><p class=c>z</p></div>
        // The b is hoisted out of the span (CSS2 §9.2.1.1) to sit
        // between a and c; the span's two empty halves have no line box
        // and so separate nothing: b's 30px margins collapse with a's
        // 10 and c's 10 as between any two block siblings. Before M10.4
        // the block laid out inside the inline run, its margins
        // collapsing with nothing.
        let root = Element::new("div")
            .push(Element::new("p").attr("class", "a").text("x"))
            .push(Element::new("span").push(Element::new("div").attr("class", "b").text("y")))
            .push(Element::new("p").attr("class", "c").text("z"));
        let layout = document_of(
            root,
            "div, p { margin: 0; } .a { margin-bottom: 10px; } .b { margin: 30px 0; }
             .c { margin-top: 10px; }",
        );
        // The anonymous blocks and the empty inline halves produce no
        // fragment: the outer div's children are a, b and c.
        let outer = &layout.roots[0];
        let kinds: Vec<usize> = outer.children.iter().map(|f| f.node).collect();
        assert_eq!(
            kinds,
            vec![2, 4, 5],
            "p.a, div.b, p.c; nothing for the span"
        );
        assert!(
            outer
                .children
                .iter()
                .all(|f| matches!(f.kind, FragmentKind::Box))
        );
        let (a, b, c) = (&outer.children[0], &outer.children[1], &outer.children[2]);
        assert_eq!(b.y, a.y + a.h + 30.0, "max(10, 30) between a and b");
        assert_eq!(c.y, b.y + b.h + 30.0, "max(30, 10) between b and c");
    }

    #[test]
    fn text_either_side_of_a_hoisted_block_is_on_its_own_lines() {
        // <div><span>a<p>b</p>c</span></div>: "a" on a line above the p,
        // "c" on a line below it, both fragments the div's own (the
        // anonymous blocks around the p have no fragment).
        let root = Element::new("div").push(
            Element::new("span")
                .text("a")
                .push(Element::new("p").text("b"))
                .text("c"),
        );
        let layout = document_of(root, "div, p { margin: 0; }");
        let m = test_fonts().primary().metrics(16.0);
        let div = &layout.roots[0];
        let text = |t: &str| {
            layout
                .iter()
                .find(|f| matches!(&f.kind, FragmentKind::Text(x) if x == t))
                .unwrap_or_else(|| panic!("text {t}"))
        };
        assert_eq!(text("a").top(), 0.0);
        let p = div
            .children
            .iter()
            .find(|f| matches!(f.kind, FragmentKind::Box))
            .expect("the p's box, a child of the div");
        assert_eq!(p.node, 3);
        assert_eq!(p.y, m.line_height(), "below the line holding a");
        assert_eq!(text("b").top(), p.y);
        assert_eq!(text("c").top(), p.y + p.h, "the line after the p");
        assert_eq!(div.children.len(), 3, "a, the p, c");
        assert_eq!(div.h, 3.0 * m.line_height());
    }

    #[test]
    fn an_overflow_box_is_a_block_formatting_context_root() {
        // Its children's margins stay inside it rather than collapsing
        // with its own (CSS2 §8.3.1: not the same formatting context).
        let root = Element::new("div")
            .attr("class", "o")
            .push(Element::new("p").text("x"));
        let frags = fragments_of(
            root,
            "div { margin: 0; overflow: hidden; } p { margin: 20px 0; }",
        );
        let boxes = box_fragments(&frags);
        let (o, p) = (boxes[0], boxes[1]);
        let m = test_fonts().primary().metrics(16.0);
        assert_eq!(o.y, 0.0, "the p's top margin did not escape");
        assert_eq!(p.y, 20.0, "it is real space inside the box");
        assert_eq!(
            o.h,
            20.0 + m.line_height() + 20.0,
            "and so is the bottom one"
        );

        // An empty one is never self-collapsing: its own top and bottom
        // margins stay apart, and only its bottom margin collapses with
        // the next sibling's top.
        let root = Element::new("div")
            .push(Element::new("div").attr("class", "o"))
            .push(Element::new("p").text("x"));
        let frags = fragments_of(
            root,
            "div, p { margin: 0; } .o { overflow: hidden; margin: 10px 0 30px 0; }
             p { margin-top: 50px; }",
        );
        let boxes = box_fragments(&frags);
        let (o, p) = (boxes[1], boxes[2]);
        assert_eq!((o.y, o.h), (10.0, 0.0));
        assert_eq!(
            p.y,
            10.0 + 50.0,
            "max(30, 50) below the empty box, not 10, 30 and 50 as one set"
        );

        // A child that collapses through it still leaves its collapsed
        // margin inside: the box is as tall as that margin (CSS2
        // §10.6.3, the distance to the last child's bottom margin edge).
        let root = Element::new("div")
            .attr("class", "o")
            .push(Element::new("p"));
        let frags = fragments_of(
            root,
            "div { margin: 0; overflow: auto; } p { margin: 20px 0; }",
        );
        assert_eq!(box_fragments(&frags)[0].h, 20.0);

        // Its own margins collapse with its siblings' as usual.
        let root = Element::new("div")
            .push(Element::new("p").attr("class", "a").text("x"))
            .push(Element::new("div").attr("class", "o").text("y"));
        let frags = fragments_of(
            root,
            "div, p { margin: 0; } .a { margin-bottom: 10px; }
             .o { overflow: scroll; margin-top: 30px; }",
        );
        let boxes = box_fragments(&frags);
        let (a, o) = (boxes[1], boxes[2]);
        assert_eq!(o.y, a.y + a.h + 30.0);
    }

    #[test]
    fn an_inline_block_shrinks_to_fit_its_content() {
        // CSS2 §10.3.5: min(max(min-content, available), max-content).
        let fonts = test_fonts();
        let w = |s: &str| fonts.measure(s, 16.0);
        let space = fonts.measure(" ", 16.0);
        let m = fonts.primary().metrics(16.0);
        let root = Element::new("div").push(Element::new("span").text("aa bb"));
        let css = |width: &str| {
            format!("div {{ margin: 0; width: {width}; }} span {{ display: inline-block; }}")
        };
        // Room enough: the max-content width, one line.
        let frags = fragments_of(root.clone(), &css("500px"));
        let ib = box_fragments(&frags)[1];
        assert_eq!(ib.w, w("aa") + space + w("bb"));
        assert_eq!(ib.h, m.line_height());
        // Less room than the max-content: the available width, and the
        // text wraps inside.
        let frags = fragments_of(root.clone(), &css("30px"));
        let ib = box_fragments(&frags)[1];
        assert!(w("aa") < 30.0 && 30.0 < w("aa") + space + w("bb"));
        assert_eq!(ib.w, 30.0);
        assert_eq!(ib.h, 2.0 * m.line_height());
        // Less room than the min-content: the min-content width, the
        // box overflowing its container.
        let frags = fragments_of(root.clone(), &css("10px"));
        let ib = box_fragments(&frags)[1];
        assert_eq!(ib.w, w("aa").max(w("bb")), "the widest word");
        // A given width is used as given; auto margins are zero; the
        // limits apply.
        let frags = fragments_of(
            root.clone(),
            "div { margin: 0; } span { display: inline-block; width: 70px; margin: 0 auto; }",
        );
        let ib = box_fragments(&frags)[1];
        assert_eq!((ib.x, ib.w), (0.0, 70.0));
        let frags = fragments_of(
            root,
            "div { margin: 0; } span { display: inline-block; min-width: 200px; }",
        );
        assert_eq!(box_fragments(&frags)[1].w, 200.0);
    }

    #[test]
    fn an_inline_block_sits_on_the_baseline_of_its_last_line() {
        // <p>x<span class=ib>one\ntwo</span></p> with the newline
        // preserved: the span's "two" shares the p's baseline with "x",
        // "one" is a line above, and the line is two lines tall (CSS2
        // §10.8.1, M7.7).
        let fonts = test_fonts();
        let m = fonts.primary().metrics(16.0);
        let root = Element::new("p")
            .text("x")
            .push(Element::new("span").text("one\ntwo"));
        let layout = document_of(
            root,
            "p { margin: 0; } span { display: inline-block; white-space: pre; }",
        );
        let text = |t: &str| {
            layout
                .iter()
                .find(|f| matches!(&f.kind, FragmentKind::Text(x) if x == t))
                .unwrap_or_else(|| panic!("text {t}"))
        };
        assert_eq!(text("two").y, text("x").y, "one baseline");
        assert_eq!(text("one").y, text("two").y - m.line_height());
        assert_eq!(text("x").y, m.line_height() + m.ascent);
        assert_eq!(layout.roots[0].h, 2.0 * m.line_height());
        let ib = &layout.roots[0].children[1];
        assert!(matches!(ib.kind, FragmentKind::Box));
        assert_eq!(
            (ib.x, ib.y),
            (fonts.measure("x", 16.0), 0.0),
            "flush after x"
        );
    }

    #[test]
    fn an_inline_block_without_a_line_box_or_that_clips_sits_on_its_bottom_margin_edge() {
        let fonts = test_fonts();
        let m = fonts.primary().metrics(16.0);
        // An empty 20 by 30 box with 5px margins: its bottom margin edge
        // is on the baseline, so its border box ends 5px above it; the
        // line is 40px above the baseline and the strut's descent below.
        let root = Element::new("p")
            .text("x")
            .push(Element::new("span"))
            .text(" y");
        let layout = document_of(
            root,
            "p { margin: 0; } span { display: inline-block; width: 20px; height: 30px; margin: 5px; }",
        );
        let text = |t: &str| {
            layout
                .iter()
                .find(|f| matches!(&f.kind, FragmentKind::Text(x) if x == t))
                .unwrap_or_else(|| panic!("text {t}"))
        };
        let ib = &layout.roots[0].children[1];
        assert_eq!(text("x").y, 40.0, "the baseline: 5 + 30 + 5 above the top");
        assert_eq!(
            (ib.x, ib.y, ib.w, ib.h),
            (fonts.measure("x", 16.0) + 5.0, 5.0, 20.0, 30.0)
        );
        assert_eq!(
            text("y").x,
            ib.x + 20.0 + 5.0 + fonts.measure(" ", 16.0),
            "the margin, then the gap"
        );
        assert_eq!(layout.roots[0].h, 40.0 + m.descent + m.line_gap);
        // One that clips: its bottom margin edge too, so its text sits
        // a descent and a line gap higher than the text outside.
        let root = Element::new("p")
            .text("x")
            .push(Element::new("span").text("one"));
        let layout = document_of(
            root,
            "p { margin: 0; } span { display: inline-block; overflow: hidden; }",
        );
        let text = |t: &str| {
            layout
                .iter()
                .find(|f| matches!(&f.kind, FragmentKind::Text(x) if x == t))
                .unwrap_or_else(|| panic!("text {t}"))
        };
        assert_eq!(text("one").y, text("x").y - m.descent - m.line_gap);
    }

    #[test]
    fn inline_blocks_in_a_row_wrap_like_words_and_keep_their_margins() {
        // Three 40px boxes, 5px margins either side, in a 100px row
        // with no font (so no gap for the white space between them and
        // no strut): two fit per line, the third wraps.
        let ib = || Element::new("span");
        let root = Element::new("div")
            .push(ib())
            .text(" ")
            .push(ib())
            .text(" ")
            .push(ib());
        let frags = fragments_of(
            root,
            "div { margin: 0; width: 100px; font-size: 0; }
             span { display: inline-block; width: 40px; height: 10px; margin: 0 5px; }",
        );
        let boxes = box_fragments(&frags);
        let at = |i: usize| (boxes[i].x, boxes[i].y);
        assert_eq!(at(1), (5.0, 0.0));
        assert_eq!(at(2), (55.0, 0.0));
        assert_eq!(at(3), (5.0, 10.0));
        assert_eq!(boxes[0].h, 20.0);
    }

    #[test]
    fn an_inline_block_lays_its_content_out_as_a_block_container() {
        // Margins inside stay inside (a formatting context root), a
        // percentage width inside is of the inline-block's content
        // width, and a nested block makes the box as wide as its
        // widest content.
        let frags = fragments_of(
            Element::new("div").push(
                Element::new("span")
                    .push(Element::new("p").attr("class", "a"))
                    .push(Element::new("p").attr("class", "b")),
            ),
            "div { margin: 0; } span { display: inline-block; padding: 2px; }
             .a { width: 100px; height: 10px; margin: 7px 0; }
             .b { width: 50%; height: 10px; margin: 0; }",
        );
        let boxes = box_fragments(&frags);
        let (ib, a, b) = (boxes[1], boxes[2], boxes[3]);
        assert_eq!(ib.w, 104.0, "the widest child plus the padding");
        assert_eq!((a.x, a.y), (2.0, 2.0 + 7.0), "the margin stays inside");
        assert_eq!((b.w, b.y), (50.0, 2.0 + 7.0 + 10.0 + 7.0));
        assert_eq!(ib.h, 2.0 + 7.0 + 10.0 + 7.0 + 10.0 + 2.0);
    }

    #[test]
    fn a_clipping_box_records_how_far_its_content_scrolls() {
        // A 50px box with 10px of padding: its padding box is 70px. A
        // 100px child reaches 120 from the padding box's top, with the
        // bottom padding after it.
        let s = |inner: &str| {
            format!(
                "div {{ margin: 0; }} .s {{ overflow: auto; height: 50px; padding: 10px; }} {inner}"
            )
        };
        let frags = fragments_of(
            Element::new("div")
                .attr("class", "s")
                .push(Element::new("p")),
            &s("p { margin: 0; height: 100px; }"),
        );
        let b = box_fragments(&frags)[0];
        assert_eq!((b.scroll_height, b.max_scroll()), (120.0, 50.0));
        // Content that fits: the padding box itself, nothing to scroll.
        let frags = fragments_of(
            Element::new("div")
                .attr("class", "s")
                .push(Element::new("p")),
            &s("p { margin: 0; height: 20px; }"),
        );
        let b = box_fragments(&frags)[0];
        assert_eq!((b.scroll_height, b.max_scroll()), (70.0, 0.0));
        // The last child's bottom margin stays inside a formatting
        // context root and scrolls with it.
        let frags = fragments_of(
            Element::new("div")
                .attr("class", "s")
                .push(Element::new("p")),
            &s("p { margin: 0 0 30px 0; height: 100px; }"),
        );
        assert_eq!(box_fragments(&frags)[0].scroll_height, 150.0);
        // A descendant overflowing a child with a fixed height reaches
        // lower than the content height says.
        let frags = fragments_of(
            Element::new("div")
                .attr("class", "s")
                .push(Element::new("p").push(Element::new("span"))),
            &s("p { margin: 0; height: 10px; } span { display: block; height: 80px; }"),
        );
        assert_eq!(box_fragments(&frags)[0].scroll_height, 100.0);
        // Unless that child clips: its overflow is its own.
        let frags = fragments_of(
            Element::new("div")
                .attr("class", "s")
                .push(Element::new("p").push(Element::new("span"))),
            &s(
                "p { margin: 0; height: 10px; overflow: hidden; } span { display: block; height: 80px; }",
            ),
        );
        assert_eq!(box_fragments(&frags)[0].scroll_height, 70.0);
        // A box that does not clip records nothing and scrolls nowhere;
        // text neither.
        let frags = fragments_of(
            Element::new("div").push(Element::new("p").text("x")),
            "div { height: 5px; margin: 0; }",
        );
        assert!(
            frags
                .iter()
                .all(|f| f.scroll_height == 0.0 && f.max_scroll() == 0.0)
        );
    }

    #[test]
    fn a_hand_built_inline_child_of_a_block_is_laid_out_as_a_run() {
        // The builder never produces this tree (it would wrap the text
        // in an anonymous block); layout takes it anyway.
        let fonts = test_fonts();
        let text = LayoutBox {
            style: ComputedStyle::default(),
            kind: BoxKind::Text("x".to_string()),
            children: Vec::new(),
            node: 0,
        };
        let block = LayoutBox {
            style: ComputedStyle {
                display: style::computed::Display::Block,
                ..ComputedStyle::default()
            },
            kind: BoxKind::Block,
            children: vec![text],
            node: 0,
        };
        let layout = layout_document(&[block], VIEWPORT, &fonts);
        let m = fonts.primary().metrics(16.0);
        assert_eq!(layout.roots[0].h, m.line_height());
        assert!(matches!(
            layout.roots[0].children[0].kind,
            FragmentKind::Text(_)
        ));
    }

    #[test]
    fn fixed_geometry_matches_hand_computed_reference() {
        // The M6.10 acceptance scenario: a hand-computed reference.
        // margin: 10px (all sides — not auto, so the "over-constrained"
        // ltr rule applies to margin-right internally, but that's not
        // independently observable here; only margin-LEFT shows up in
        // `x`, which is what this checks).
        let frags = fragments_of(
            Element::new("div"),
            "div {
                display: block;
                width: 200px; height: 100px;
                margin: 10px;
                padding: 5px;
                border-width: 3px; border-style: solid;
            }",
        );
        assert_eq!(frags.len(), 1);
        let f = &frags[0];
        // border-box: width = 200 + 2*5 (padding) + 2*3 (border) = 216;
        // height = 100 + 2*5 + 2*3 = 116; positioned after margin (10, 10).
        assert_eq!((f.x, f.y, f.w, f.h), (10.0, 10.0, 216.0, 116.0));
    }

    #[test]
    fn auto_width_fills_remaining_space_after_margins() {
        let frags = fragments_of(
            Element::new("div"),
            "div { display: block; margin: 0 50px; }", // width: auto (initial)
        );
        assert_eq!(frags[0].w, VIEWPORT - 100.0);
        assert_eq!(frags[0].x, 50.0);
    }

    #[test]
    fn margin_auto_both_sides_centers() {
        let frags = fragments_of(
            Element::new("div"),
            "div { display: block; width: 200px; margin: 0 auto; }",
        );
        let f = &frags[0];
        assert_eq!(f.w, 200.0);
        assert_eq!(f.x, (VIEWPORT - 200.0) / 2.0);
    }

    #[test]
    fn margin_auto_one_side_pushes_to_the_other_edge() {
        let frags = fragments_of(
            Element::new("div"),
            "div { display: block; width: 200px; margin-left: auto; margin-right: 0px; }",
        );
        // auto absorbs all remaining space on its own side.
        assert_eq!(frags[0].x, VIEWPORT - 200.0);
    }

    #[test]
    fn adjacent_siblings_collapse_to_the_larger_positive_margin() {
        let root = Element::new("div")
            .push(Element::new("p").attr("class", "a"))
            .push(Element::new("p").attr("class", "b"));
        let frags = fragments_of(
            root,
            "div { display: block; } p { display: block; padding: 1px; }
             .a { margin: 10px 0 20px 0; } .b { margin: 30px 0 0 0; }",
        );
        let boxes = box_fragments(&frags);
        // div, p.a, p.b. `padding: 1px` keeps a/b from self-collapsing
        // (CSS2 §8.3.1's *other* collapsing case — tested on its own in
        // `empty_block_self_collapses_top_and_bottom`), so this test
        // isolates pure sibling-to-sibling collapsing.
        assert_eq!(boxes.len(), 3);
        let (div, a, b) = (boxes[0], boxes[1], boxes[2]);
        assert_eq!(div.y, 10.0); // div's own margin-top (0) collapses with .a's (10) escaping through
        assert_eq!(a.y, 10.0); // flush with div (div has no border/padding)
        // Gap between a's border-box bottom and b's top is max(20, 30) = 30, not 20+30=50.
        assert_eq!(b.y, a.y + a.h + 30.0);
    }

    #[test]
    fn adjacent_siblings_mixed_sign_margins_collapse_per_formula() {
        // Non-empty (padding keeps them from self-collapsing) so the
        // *sibling* formula is isolated from the *empty-block* one.
        let root = Element::new("div")
            .push(Element::new("p").attr("class", "a"))
            .push(Element::new("p").attr("class", "b"));
        let frags = fragments_of(
            root,
            "div { display: block; } p { display: block; padding: 1px; }
             .a { margin-bottom: 30px; } .b { margin-top: -10px; }",
        );
        let boxes = box_fragments(&frags);
        let (a, b) = (boxes[1], boxes[2]);
        // collapse(30, -10) = max(30,0) - max(0,10) = 20.
        assert_eq!(b.y, a.y + 2.0 /* a's own border-box height */ + 20.0);
    }

    #[test]
    fn parent_first_child_margin_collapses_without_double_counting() {
        // The exact scenario that caught a real double-counting bug
        // during development: div has no border/padding, so its own
        // margin-top (5) collapses with p's margin-top (25) — the
        // *combined* value (25, the max) must appear exactly once, with
        // div and p ending up flush with each other, not 25 (div's
        // escaped margin) + 25 (p's own, re-applied inside div) = 50.
        let root = Element::new("div").push(Element::new("p"));
        let frags = fragments_of(
            root,
            "div { display: block; margin-top: 5px; }
             p { display: block; margin-top: 25px; }",
        );
        let boxes = box_fragments(&frags);
        let (div, p) = (boxes[0], boxes[1]);
        assert_eq!(div.y, 25.0, "the collapsed (max) margin, not div's own 5px");
        assert_eq!(p.y, 25.0, "flush with div, not 25 (div) + 25 (p) = 50");
    }

    #[test]
    fn parent_with_top_padding_stops_the_collapse() {
        let root = Element::new("div").push(Element::new("p"));
        let frags = fragments_of(
            root,
            "div { display: block; margin-top: 5px; padding-top: 2px; }
             p { display: block; margin-top: 25px; }",
        );
        let boxes = box_fragments(&frags);
        let (div, p) = (boxes[0], boxes[1]);
        assert_eq!(
            div.y, 5.0,
            "div's own margin, uncollapsed — padding stops it"
        );
        // p sits inside div: div's padding-top (2) + p's own margin-top
        // (25, now a real internal gap, not an escaping one).
        assert_eq!(p.y, div.y + 2.0 + 25.0);
    }

    #[test]
    fn parent_last_child_margin_collapses_when_height_is_auto() {
        // A trailing sibling after the collapsing pair makes the
        // escaped bottom margin observable as real spacing.
        let root = Element::new("outer")
            .push(Element::new("div").push(Element::new("p")))
            .push(Element::new("div"));
        let frags = fragments_of(
            root,
            // `margin: 0` resets `p`'s UA margin (ua.css's `p { margin:
            // 16px 0; }` would otherwise still apply to margin-top,
            // unrelated to what this test checks but worth resetting
            // for a fixture that's easy to hand-verify). `padding: 1px`
            // keeps `p` from self-collapsing (its own separate
            // CSS2 §8.3.1 case, tested on its own elsewhere) — this
            // test is specifically about the margin escaping *through*
            // div#1 (which has no border/padding of its own), not about
            // p's internal self-collapse.
            "outer, div, p { display: block; margin: 0; }
             p { padding: 1px; }
             div:first-child { margin-bottom: 5px; }
             div:first-child p { margin-bottom: 25px; }",
        );
        let boxes = box_fragments(&frags);
        // outer, div#1, p, div#2
        assert_eq!(boxes.len(), 4);
        let (div1, p, div2) = (boxes[1], boxes[2], boxes[3]);
        assert_eq!(
            div1.h,
            p.y + p.h,
            "div1's border-box ends exactly where p's content does — the margin escaped, not materialized inside"
        );
        assert_eq!(
            div2.y,
            div1.y + div1.h + 25.0,
            "the collapsed (max) margin between div1 and div2, escaped through div1's bottom"
        );
    }

    #[test]
    fn empty_block_self_collapses_top_and_bottom() {
        let frags = fragments_of(
            Element::new("div"),
            "div { display: block; margin-top: 10px; margin-bottom: 30px; }",
        );
        // The box's top border edge is where it would be if it had a
        // bottom border (CSS2 §8.3.1): below its own top margin, 10;
        // the margins collapse through it to one set of 30, which is
        // the page's height (M11 made the edge observable, through a
        // float inside such a box; before, the box sat at 30).
        assert_eq!(frags[0].y, 10.0);
        let l = document_of(
            Element::new("div"),
            "div { display: block; margin-top: 10px; margin-bottom: 30px; }",
        );
        assert_eq!(l.height, 30.0);
    }

    #[test]
    fn text_run_produces_a_baseline_positioned_fragment() {
        let fonts = test_fonts();
        let frags = fragments_of(
            Element::new("p").text("hello"),
            // `margin: 0` resets ua.css's `p { margin: 16px 0; }` —
            // this test is about the baseline/color relationship, not
            // margins, so keep the fixture independent of them.
            "p { display: block; margin: 0; color: #ff0000; font-size: 20px; }",
        );
        let text = frags
            .iter()
            .find(|f| matches!(f.kind, FragmentKind::Text(_)));
        let f = text.expect("a text fragment");
        match &f.kind {
            FragmentKind::Text(t) => assert_eq!(t, "hello"),
            _ => unreachable!(),
        }
        // The baseline sits at the font's own ascent (M7.1/M7.4) — not
        // simply the font-size, and not a fixed guess: whatever the
        // *actual* loaded system font's metrics say, since that's what
        // `layout_inline` used too.
        assert_eq!(f.y, fonts.primary().metrics(20.0).ascent);
        assert_eq!(f.style.color, style::computed::Color::rgb8(255, 0, 0));
    }

    #[test]
    fn display_none_element_contributes_no_fragment_or_space() {
        let root = Element::new("div")
            .push(Element::new("p").attr("class", "gone").text("x"))
            .push(Element::new("p").attr("class", "shown").text("y"));
        let frags = fragments_of(
            root,
            "div, p { display: block; }
             .gone { display: none; margin: 100px; }",
        );
        let boxes = box_fragments(&frags);
        // div + the one visible p — the hidden one contributes no box
        // (not even a zero-size placeholder) and no margin at all.
        assert_eq!(boxes.len(), 2);
    }

    #[test]
    fn two_level_document_with_margins_padding_and_borders() {
        let fonts = test_fonts();
        // M6.10/M7.8: a fuller acceptance case combining several
        // features at once, still hand-computed exactly — using the
        // *actual* loaded font's metrics for the text parts, same as
        // `text_run_produces_a_baseline_positioned_fragment` above.
        let root = Element::new("section").push(
            Element::new("article")
                .push(Element::new("h1").text("Title"))
                .push(Element::new("p").text("Body")),
        );
        let frags = fragments_of(
            root,
            "section, article, h1, p { display: block; margin: 0; padding: 0; }
             article { padding: 10px; border-width: 2px; border-style: solid; }
             h1 { font-size: 30px; margin-bottom: 15px; }
             p { font-size: 10px; }",
        );
        let boxes = box_fragments(&frags);
        let texts: Vec<&Fragment> = frags
            .iter()
            .filter(|f| matches!(f.kind, FragmentKind::Text(_)))
            .collect();
        // section, article, h1, p: every block box gets its own Box
        // fragment regardless of margin/border/padding collapsing —
        // collapsing only affects *position*, never whether a box
        // fragment exists at all.
        assert_eq!(boxes.len(), 4);
        let article = boxes[1];
        // article's border-box starts at (0,0): no margin anywhere, and
        // section itself has none either.
        assert_eq!((article.x, article.y), (0.0, 0.0));
        // h1's line height/baseline, h1's own font-size (30px).
        let h1_metrics = fonts.primary().metrics(30.0);
        // h1's text baseline: inside article's border(2)+padding(10) = 12.
        let h1_text = texts
            .iter()
            .find(|f| matches!(&f.kind, FragmentKind::Text(t) if t == "Title"))
            .unwrap();
        assert_eq!(h1_text.y, 12.0 + h1_metrics.ascent);
        // p's text baseline: after h1's full line box + h1's own margin-bottom (15) + article's padding-top (already counted above, not again).
        let p_metrics = fonts.primary().metrics(10.0);
        let p_text = texts
            .iter()
            .find(|f| matches!(&f.kind, FragmentKind::Text(t) if t == "Body"))
            .unwrap();
        assert_eq!(
            p_text.y,
            12.0 + h1_metrics.line_height() + 15.0 + p_metrics.ascent
        );
    }

    #[test]
    fn an_empty_block_between_siblings_joins_one_margin_set_not_two() {
        // <p>one</p><p></p><p>two</p> with 10px margins all round: the
        // empty p's top and bottom margins and its neighbours' all
        // adjoin as one set, so "two" sits 10px below "one", not 20 (an
        // earlier version folded the empty box's margin in twice, once
        // as its top and once as its bottom).
        let root = Element::new("div")
            .push(Element::new("p").text("one"))
            .push(Element::new("p"))
            .push(Element::new("p").text("two"));
        let frags = fragments_of(root, "div, p { display: block; } p { margin: 10px 0; }");
        let boxes = box_fragments(&frags);
        assert_eq!(boxes.len(), 4);
        let (first, empty, second) = (boxes[1], boxes[2], boxes[3]);
        assert_eq!(first.y, 10.0);
        assert_eq!(empty.h, 0.0);
        assert_eq!(
            second.y,
            first.y + first.h + 10.0,
            "one collapsed margin between the paragraphs, not two"
        );

        // The same page under the UA sheet alone (`p { margin: 16px 0 }`).
        let root = Element::new("body")
            .push(Element::new("p").text("one"))
            .push(Element::new("p"))
            .push(Element::new("p").text("two"));
        let frags = fragments_of(root, "");
        let boxes = box_fragments(&frags);
        let (first, second) = (boxes[1], boxes[3]);
        assert_eq!(second.y, first.y + first.h + 16.0);
    }

    #[test]
    fn three_adjoining_margins_of_mixed_sign_collapse_as_one_set() {
        // CSS2 §8.3.1: the collapsed margin is the largest positive
        // minus the largest absolute negative over the WHOLE set.
        // {30, -25, 10} resolves to 30 - 25 = 5. Folding pairwise gives
        // collapse(collapse(30, -25), 10) = collapse(5, 10) = 10 (wrong)
        // or, in the other order, collapse(30, collapse(-25, 10)) =
        // collapse(30, -15) = 15 (also wrong).
        let root = Element::new("div")
            .push(Element::new("p").attr("class", "a").text("a"))
            .push(Element::new("p").attr("class", "e"))
            .push(Element::new("p").attr("class", "b").text("b"));
        let frags = fragments_of(
            root,
            "div, p { display: block; margin: 0; }
             .a { margin-bottom: 30px; } .e { margin: -25px 0 0 0; } .b { margin-top: 10px; }",
        );
        let boxes = box_fragments(&frags);
        let (a, b) = (boxes[1], boxes[3]);
        assert_eq!(
            b.y,
            a.y + a.h + 5.0,
            "a's 30, the empty box's -25 and b's 10 as one set"
        );

        // The same three margins meeting parent-first-child style: a
        // parent with margin-top 30 whose first child is an empty block
        // with margin-top -25 and margin-bottom 10.
        let root = Element::new("div")
            .push(Element::new("p").attr("class", "e"))
            .push(Element::new("p").text("b"));
        let frags = fragments_of(
            root,
            "div, p { display: block; margin: 0; }
             div { margin-top: 30px; } .e { margin: -25px 0 10px 0; }",
        );
        let boxes = box_fragments(&frags);
        let (div, b) = (boxes[0], boxes[2]);
        assert_eq!(div.y, 5.0, "the set escapes to the viewport as one value");
        assert_eq!(
            b.y, 5.0,
            "flush with the parent: nothing materialises inside"
        );
    }

    #[test]
    fn a_collapsed_through_set_inside_a_padded_parent_counts_once() {
        // The parent has padding on both sides, so the empty child's
        // margins neither escape nor collapse with the parent's: they
        // collapse with EACH OTHER (one 20px gap), which is the whole
        // content height — not 20 for the top and 20 again for the
        // bottom (CSS2 §8.3.1, §10.6.3).
        let root = Element::new("div").push(Element::new("p"));
        let frags = fragments_of(
            root,
            "div, p { display: block; margin: 0; }
             div { padding: 10px; } p { margin: 20px 0; }",
        );
        let boxes = box_fragments(&frags);
        let (div, p) = (boxes[0], boxes[1]);
        assert_eq!(div.h, 10.0 + 20.0 + 10.0);
        assert_eq!(
            p.y,
            10.0 + 20.0,
            "past the gap, as if it had a bottom border"
        );

        // Padding on top only: the set collapses through to the parent's
        // bottom margin instead. The parent's content height is zero
        // (§10.6.3's last case) and the following sibling sits one
        // collapsed margin below the parent's padding.
        let root = Element::new("div")
            .push(
                Element::new("div")
                    .attr("class", "p")
                    .push(Element::new("p")),
            )
            .push(Element::new("div").attr("class", "n").text("next"));
        let frags = fragments_of(
            root,
            "div, p { display: block; margin: 0; }
             .p { padding-top: 10px; } p { margin: 20px 0; }",
        );
        let boxes = box_fragments(&frags);
        let (padded, next) = (boxes[1], boxes[3]);
        assert_eq!(padded.h, 10.0);
        assert_eq!(next.y, 10.0 + 20.0);

        // Padding at the bottom only: the set escapes through the top
        // and joins the parent's own margin-top.
        let root = Element::new("div").push(Element::new("p"));
        let frags = fragments_of(
            root,
            "div, p { display: block; margin: 0; }
             div { padding-bottom: 10px; } p { margin: 20px 0; }",
        );
        let boxes = box_fragments(&frags);
        let div = boxes[0];
        assert_eq!((div.y, div.h), (20.0, 10.0));
    }

    #[test]
    fn an_inline_with_no_line_box_does_not_separate_margins() {
        // <div style="margin-bottom: 30px"></div><a></a><p style="margin-top: 30px">
        // An empty inline element produces no line box, and CSS2 §9.4.2
        // says such a line box is treated as not existing — so the
        // margins either side of it still adjoin.
        let root = Element::new("div")
            .push(Element::new("div").attr("class", "top"))
            .push(Element::new("a"))
            .push(Element::new("p").text("x"));
        let frags = fragments_of(
            root,
            "div, p { display: block; margin: 0; } a { display: inline; }
             .top { margin-bottom: 30px; } p { margin-top: 30px; }",
        );
        let boxes = box_fragments(&frags);
        // outer div, .top, p (an inline with nothing in it has no box fragment)
        assert_eq!(boxes.len(), 3);
        let (outer, top, p) = (boxes[0], boxes[1], boxes[2]);
        assert_eq!(outer.y, 30.0, "the whole set escapes to the viewport");
        assert_eq!(top.y, 30.0);
        assert_eq!(p.y, 30.0, "flush: the empty inline separated nothing");
    }

    #[test]
    fn over_constrained_auto_margins_are_zero_not_negative() {
        // width + the non-auto margin already exceed the containing
        // block (CSS2.1 §10.3.3): every `auto` becomes zero and the box
        // overflows to the right, rather than `auto` absorbing a
        // negative amount and pulling the box off the left edge.
        let frags = fragments_of(
            Element::new("div"),
            "div { display: block; width: 900px; margin: 0 auto; }",
        );
        assert_eq!((frags[0].x, frags[0].w), (0.0, 900.0));

        let frags = fragments_of(
            Element::new("div"),
            "div { display: block; width: 900px; margin-left: auto; margin-right: 0px; }",
        );
        assert_eq!(frags[0].x, 0.0);

        let frags = fragments_of(
            Element::new("div"),
            "div { display: block; width: 900px; margin-left: 20px; margin-right: auto; }",
        );
        assert_eq!(frags[0].x, 20.0);
    }

    #[test]
    fn a_div_is_a_block_by_the_ua_sheet_alone() {
        // No author `display: block` anywhere: the UA sheet alone must
        // make the common containers blocks, or a page of <div>s (and
        // every HTML5 sectioning element) lays out as inline content.
        let frags = fragments_of(
            Element::new("div").attr("style", "width: 200px; height: 50px; margin-left: 12px"),
            "",
        );
        let boxes = box_fragments(&frags);
        assert_eq!(boxes.len(), 1);
        assert_eq!((boxes[0].x, boxes[0].w, boxes[0].h), (12.0, 200.0, 50.0));

        for tag in [
            "section", "article", "nav", "header", "footer", "main", "form",
        ] {
            let frags = fragments_of(Element::new(tag).text("x"), "");
            assert_eq!(box_fragments(&frags).len(), 1, "<{tag}> is a block");
        }
        // A table is a table (M12): its text gets an anonymous row and
        // cell, three boxes.
        let frags = fragments_of(Element::new("table").text("x"), "");
        assert_eq!(box_fragments(&frags).len(), 3, "<table> is a table");
    }

    /// The tree form of [`fragments_of`].
    fn document_of(root: Element, css_src: &str) -> DocumentLayout {
        let fonts = test_fonts();
        let doc = dom::Node::document(vec![root.into()]);
        let sheet = css::parse_stylesheet(css_src).expect("test css is valid");
        let styled = style::style_document(&doc, &[sheet]);
        let boxes = crate::build_box_tree(&styled);
        layout_document(&boxes, VIEWPORT, &fonts)
    }

    #[test]
    fn fragments_form_a_tree_with_absolute_coordinates_and_node_ids() {
        // <section><p>hi</p></section>: the section's fragment holds the
        // p's, which holds the text's. Nodes: document 0, section 1, p 2.
        let root = Element::new("section").push(Element::new("p").text("hi"));
        let layout = document_of(root, "section { padding: 10px; } p { margin: 5px 0 0 0; }");
        assert_eq!(layout.roots.len(), 1);
        let section = &layout.roots[0];
        assert_eq!(section.node, 1);
        assert_eq!(section.children.len(), 1);
        let p = &section.children[0];
        assert_eq!(p.node, 2);
        assert_eq!(
            (p.x, p.y),
            (10.0, 15.0),
            "absolute: the section's padding plus the p's margin"
        );
        assert_eq!(p.children.len(), 1);
        let text = &p.children[0];
        assert!(matches!(text.kind, FragmentKind::Text(_)));
        assert_eq!(text.node, 2, "a text fragment names its containing element");
        let m = test_fonts().primary().metrics(16.0);
        assert_eq!(text.y, 15.0 + m.ascent, "the baseline, absolute");
        assert_eq!(text.ascent, m.ascent);
        assert_eq!(text.h, m.ascent + m.descent);
        assert_eq!(text.top(), 15.0);
        assert!(text.contains(text.x + 1.0, 16.0));
        assert!(!text.contains(text.x + 1.0, 14.0));
        assert!(!text.contains(text.x - 1.0, 16.0));
        assert!(section.contains(0.0, 0.0));
        // The flat views are preorder: a box, then its content.
        let order: Vec<usize> = layout.iter().map(|f| f.node).collect();
        assert_eq!(order, vec![1, 2, 2]);
        let flat = flatten(layout.roots.clone());
        assert_eq!(flat.len(), 3);
        assert!(flat.iter().all(|f| f.children.is_empty()));
        assert_eq!((flat[1].x, flat[1].y), (10.0, 15.0));
    }

    #[test]
    fn document_height_counts_the_leading_and_trailing_margins() {
        // A box of fixed height with margins above and below: both
        // materialise at the viewport, which nothing escapes past.
        let l = document_of(
            Element::new("div"),
            "div { height: 100px; margin: 10px 0 30px 0; }",
        );
        assert_eq!(l.height, 140.0);
        assert_eq!(l.roots[0].y, 10.0);
        // Nothing but margins: one set, counted once.
        let l = document_of(Element::new("div"), "div { margin: 10px 0 30px 0; }");
        assert_eq!(l.height, 30.0);
        // Two paragraphs under the UA sheet: 16 above, 16 between, 16 below.
        let root = Element::new("body")
            .push(Element::new("p").text("one"))
            .push(Element::new("p").text("two"));
        let l = document_of(root, "");
        let flat = flatten(l.roots.clone());
        let boxes = box_fragments(&flat);
        let last = boxes[2];
        assert_eq!(l.height, last.y + last.h + 16.0);
        // A negative margin cannot make the document negative.
        let l = document_of(Element::new("div"), "div { margin: -10px 0; }");
        assert_eq!(l.height, 0.0);
        // Nothing at all.
        let l = layout_document(&[], VIEWPORT, &test_fonts());
        assert_eq!(l.height, 0.0);
        assert!(l.roots.is_empty());
    }

    /// The box fragment of the element with `class`, anywhere in the
    /// tree, by the class attribute's node: the test knows its nodes.
    fn by_node(layout: &DocumentLayout, node: usize) -> Fragment {
        layout
            .iter()
            .find(|f| matches!(f.kind, FragmentKind::Box) && f.node == node)
            .unwrap_or_else(|| panic!("box of node {node}"))
            .clone()
    }

    fn text_of(layout: &DocumentLayout, t: &str) -> Fragment {
        layout
            .iter()
            .find(|f| matches!(&f.kind, FragmentKind::Text(x) if x == t))
            .unwrap_or_else(|| panic!("text {t}"))
            .clone()
    }

    #[test]
    fn a_float_between_blocks_sits_at_the_flow_position_and_takes_no_room() {
        // <div><p class=a>x</p><div class=f></div><p class=b>y</p></div>
        // with a 10px margin under a and a 30px one over b: the float's
        // top is after a's margin (the margins resolved so far), b's
        // after the collapsed 30, with its line shortened by the float;
        // the div, no context root, is as tall as its two paragraphs,
        // the float overflowing it; the page reaches the float's bottom
        // margin edge (CSS2 §10.6.7). Nodes: div 1, a 2, f 3, b 4.
        let m = test_fonts().primary().metrics(16.0);
        let lh = m.line_height();
        let root = Element::new("div")
            .push(Element::new("p").attr("class", "a").text("x"))
            .push(Element::new("div").attr("class", "f"))
            .push(Element::new("p").attr("class", "b").text("y"));
        let layout = document_of(
            root,
            "div, p { margin: 0; } .a { margin-bottom: 10px; } .b { margin-top: 30px; }
             .f { float: left; width: 50px; height: 40px; margin: 2px 4px; }",
        );
        let (div, a, f, b) = (
            by_node(&layout, 1),
            by_node(&layout, 2),
            by_node(&layout, 3),
            by_node(&layout, 4),
        );
        assert_eq!((a.y, a.h), (0.0, lh));
        assert_eq!((f.x, f.y, f.w, f.h), (4.0, lh + 10.0 + 2.0, 50.0, 40.0));
        assert_eq!(b.y, lh + 30.0);
        assert_eq!(
            text_of(&layout, "y").x,
            58.0,
            "beside the float's margin box"
        );
        assert_eq!(text_of(&layout, "x").x, 0.0);
        assert_eq!(div.h, lh + 30.0 + lh, "the float adds nothing");
        assert_eq!(
            layout.height,
            lh + 10.0 + 44.0,
            "the page reaches the float"
        );
        assert_eq!(
            div.children.iter().map(|c| c.node).collect::<Vec<_>>(),
            vec![2, 3, 4],
            "the float's fragment is where its element is"
        );
    }

    #[test]
    fn a_context_root_reaches_its_floats_and_its_lines_run_between_them() {
        // An overflow box 200px wide with a left float (50 by 40) and a
        // right one (60 by 20), then a row of five 30px inline-blocks
        // in a font-less paragraph: three fit on the first line in the
        // 90px between the floats, two on the next; the box is as tall
        // as the left float (CSS2 §10.6.7). Nodes: div.o 1, l 2, r 3,
        // p 4, spans 5..9.
        let mut p = Element::new("p");
        for _ in 0..5 {
            p = p.push(Element::new("span"));
        }
        let root = Element::new("div")
            .attr("class", "o")
            .push(Element::new("div").attr("class", "l"))
            .push(Element::new("div").attr("class", "r"))
            .push(p);
        let layout = document_of(
            root,
            ".o { overflow: hidden; width: 200px; margin: 0; font-size: 0; }
             .l { float: left; width: 50px; height: 40px; }
             .r { float: right; width: 60px; height: 20px; }
             p { margin: 0; } span { display: inline-block; width: 30px; height: 10px; }",
        );
        let o = by_node(&layout, 1);
        assert_eq!(o.h, 40.0);
        assert_eq!((by_node(&layout, 2).x, by_node(&layout, 2).y), (0.0, 0.0));
        assert_eq!((by_node(&layout, 3).x, by_node(&layout, 3).y), (140.0, 0.0));
        let at = |node: usize| (by_node(&layout, node).x, by_node(&layout, node).y);
        assert_eq!(at(5), (50.0, 0.0));
        assert_eq!(at(6), (80.0, 0.0));
        assert_eq!(at(7), (110.0, 0.0));
        assert_eq!(at(8), (50.0, 10.0), "the next line, still beside both");
        assert_eq!(at(9), (80.0, 10.0));
        // The lines are the paragraph's: 20px of them.
        assert_eq!(by_node(&layout, 4).h, 20.0);
        // A scroll container's scrollable height counts its floats.
        let layout = document_of(
            Element::new("div")
                .attr("class", "s")
                .push(Element::new("div").attr("class", "f")),
            ".s { overflow: auto; height: 10px; margin: 0; } .f { float: left; height: 50px; width: 10px; }",
        );
        assert_eq!(by_node(&layout, 1).scroll_height, 50.0);
    }

    #[test]
    fn a_line_that_fits_nothing_beside_the_floats_moves_below_them() {
        // A 100px box with an 80px float: a 50px inline-block does not
        // fit in the 20px beside it and goes below, at the left edge;
        // one wider than the box itself overflows there as always.
        // Nodes: div 1, f 2, p 3, spans 4 and 5.
        let root = Element::new("div")
            .push(Element::new("div").attr("class", "f"))
            .push(
                Element::new("p")
                    .push(Element::new("span").attr("class", "a"))
                    .push(Element::new("span").attr("class", "b")),
            );
        let layout = document_of(
            root,
            "div { width: 100px; margin: 0; font-size: 0; } p { margin: 0; }
             .f { float: left; width: 80px; height: 30px; }
             span { display: inline-block; height: 10px; }
             .a { width: 50px; } .b { width: 150px; }",
        );
        assert_eq!((by_node(&layout, 4).x, by_node(&layout, 4).y), (0.0, 30.0));
        assert_eq!((by_node(&layout, 5).x, by_node(&layout, 5).y), (0.0, 40.0));
    }

    #[test]
    fn a_float_in_a_run_shortens_its_line_or_goes_below_it() {
        // <p><span class=a/><div class=f/><span class=b/><span class=c/></p>
        // 200px wide, font-less: the float (50 by 20) fits in what is
        // left of the line after a (100), so it is placed at the line's
        // top and the line's content moves beside it; b (40) still
        // fits; c (80) does not and starts the next line, which at 10
        // is still beside the float. Nodes: p 1, a 2, f 3, b 4, c 5.
        let root = Element::new("p")
            .push(Element::new("span").attr("class", "a"))
            .push(Element::new("div").attr("class", "f"))
            .push(Element::new("span").attr("class", "b"))
            .push(Element::new("span").attr("class", "c"));
        let css = "p { width: 200px; margin: 0; font-size: 0; }
                   span { display: inline-block; height: 10px; }
                   .a { width: 100px; } .b { width: 40px; } .c { width: 80px; }
                   .f { float: left; width: 50px; height: 20px; }";
        let layout = document_of(root.clone(), css);
        let at = |node: usize| (by_node(&layout, node).x, by_node(&layout, node).y);
        assert_eq!(at(3), (0.0, 0.0), "the float at the line's left");
        assert_eq!(at(2), (50.0, 0.0), "a, moved beside it");
        assert_eq!(at(4), (150.0, 0.0));
        assert_eq!(at(5), (50.0, 10.0));
        assert_eq!(by_node(&layout, 1).h, 20.0);
        // Too wide for what is left of the line (150 > 100): placed
        // below the line once its height is known, where the next line
        // avoids it — and the line it was met on keeps its full width.
        let layout = document_of(
            root,
            &css.replace("width: 50px; height: 20px", "width: 150px; height: 20px"),
        );
        let at = |node: usize| (by_node(&layout, node).x, by_node(&layout, node).y);
        assert_eq!(at(2), (0.0, 0.0));
        assert_eq!(at(4), (100.0, 0.0), "b still on the first line");
        assert_eq!(at(3), (0.0, 10.0), "the float under the first line");
        assert_eq!(at(5), (0.0, 30.0), "c fits in neither 50px beside it");
    }

    #[test]
    fn a_float_in_an_empty_block_waits_for_the_content_that_resolves_its_position() {
        // <div><p class=a>x</p><div class=w><div class=f/></div><p class=b>y</p></div>
        // The wrapper has no border, padding or content: the margins
        // around it collapse through it, 10 under a with 30 over b, and
        // its float is placed only when b's line resolves where the
        // collapsing ends — level with b, as in browsers. The wrapper's
        // own top edge is where its top margin alone would put it (CSS2
        // §8.3.1). Nodes: div 1, a 2, w 3, f 4, b 5.
        let m = test_fonts().primary().metrics(16.0);
        let lh = m.line_height();
        let root = Element::new("div")
            .push(Element::new("p").attr("class", "a").text("x"))
            .push(
                Element::new("div")
                    .attr("class", "w")
                    .push(Element::new("div").attr("class", "f")),
            )
            .push(Element::new("p").attr("class", "b").text("y"));
        let layout = document_of(
            root,
            "div, p { margin: 0; } .a { margin-bottom: 10px; } .b { margin-top: 30px; }
             .f { float: left; width: 20px; height: 20px; }",
        );
        let (w, f, b) = (
            by_node(&layout, 3),
            by_node(&layout, 4),
            by_node(&layout, 5),
        );
        assert_eq!(b.y, lh + 30.0);
        assert_eq!((f.x, f.y), (0.0, lh + 30.0), "level with b");
        assert_eq!(text_of(&layout, "y").x, 20.0, "b's line is shortened");
        assert_eq!((w.y, w.h), (lh + 10.0, 0.0));
        assert_eq!(w.children.len(), 1, "the float's fragment is the wrapper's");
        // Nothing resolves it before its context ends: it is placed
        // where the content would have been, and the root reaches it.
        let layout = document_of(
            Element::new("div").attr("class", "o").push(
                Element::new("div")
                    .attr("class", "w")
                    .push(Element::new("div").attr("class", "f")),
            ),
            ".o { overflow: hidden; margin: 0; } .w { margin: 0; }
             .f { float: left; width: 20px; height: 20px; }",
        );
        assert_eq!((by_node(&layout, 3).y, by_node(&layout, 1).h), (0.0, 20.0));
        // At the page's end, likewise: the page is as tall as the float.
        let layout = document_of(
            Element::new("div")
                .attr("class", "w")
                .push(Element::new("div").attr("class", "f")),
            ".w { margin: 5px 0; } .f { float: left; width: 20px; height: 20px; }",
        );
        assert_eq!(by_node(&layout, 2).y, 5.0);
        assert_eq!(layout.height, 25.0);
    }

    #[test]
    fn a_float_is_a_context_of_its_own_and_shrinks_to_fit() {
        // A float holding a float: the inner one is placed in the outer
        // float's own context, which the outer float's width shrinks to
        // and its height reaches; the paragraph's line in the outer
        // context is shortened by the outer float alone. Nodes: div.o 1,
        // f 2, g 3, p 4.
        let root = Element::new("div")
            .attr("class", "o")
            .push(
                Element::new("div")
                    .attr("class", "f")
                    .push(Element::new("div").attr("class", "g")),
            )
            .push(Element::new("p").text("x"));
        let layout = document_of(
            root,
            ".o { overflow: hidden; margin: 0; } p { margin: 0; }
             .f { float: left; padding: 3px; } .g { float: left; width: 10px; height: 10px; }",
        );
        let (f, g) = (by_node(&layout, 2), by_node(&layout, 3));
        assert_eq!((f.x, f.y, f.w, f.h), (0.0, 0.0, 16.0, 16.0));
        assert_eq!((g.x, g.y), (3.0, 3.0));
        assert_eq!(text_of(&layout, "x").x, 16.0);
        assert_eq!(f.children.len(), 1);
        // An inline-block shrinks around its float and the text beside
        // it: the float's width plus the text's, on one line.
        let fonts = test_fonts();
        let root = Element::new("div").push(
            Element::new("span")
                .push(Element::new("div").attr("class", "f"))
                .text("aa"),
        );
        let layout = document_of(
            root,
            "div { margin: 0; } span { display: inline-block; }
             .f { float: left; width: 30px; height: 5px; }",
        );
        let ib = by_node(&layout, 2);
        assert_eq!(ib.w, 30.0 + fonts.measure("aa", 16.0));
        assert_eq!(text_of(&layout, "aa").x, 30.0);
        assert_eq!(text_of(&layout, "aa").top(), 0.0);
    }

    #[test]
    fn clear_puts_a_block_below_the_floats_it_names_and_ends_the_collapsing() {
        // A left float 60 tall and a right one 20 tall, then paragraphs:
        // a beside both; b with `clear: right` would be at the line
        // height, not past the right float, so it gets clearance down to
        // 20, still beside the left float; c with `clear: left` and a
        // 5px margin would be at 20 + line + 5, and goes to 60, its
        // margin inside the clearance; d's 50px margin collapses with
        // c's bottom one as usual. Nodes: div 1, f 2, g 3, a 4, b 5,
        // c 6, d 7.
        let m = test_fonts().primary().metrics(16.0);
        let lh = m.line_height();
        let root = Element::new("div")
            .push(Element::new("div").attr("class", "f"))
            .push(Element::new("div").attr("class", "g"))
            .push(Element::new("p").attr("class", "a").text("x"))
            .push(Element::new("p").attr("class", "b").text("y"))
            .push(Element::new("p").attr("class", "c").text("z"))
            .push(Element::new("p").attr("class", "d").text("w"));
        let layout = document_of(
            root,
            "div, p { margin: 0; }
             .f { float: left; width: 30px; height: 60px; }
             .g { float: right; width: 30px; height: 20px; }
             .b { clear: right; } .c { clear: left; margin-top: 5px; }
             .d { margin-top: 50px; }",
        );
        let top = |node: usize| by_node(&layout, node).y;
        assert_eq!(top(4), 0.0);
        assert_eq!(text_of(&layout, "x").x, 30.0);
        assert_eq!(top(5), 20.0, "cleared past the right float");
        assert_eq!(text_of(&layout, "y").x, 30.0, "still beside the left one");
        assert_eq!(top(6), 60.0, "cleared past the left float, margin inside");
        assert_eq!(text_of(&layout, "z").x, 0.0);
        assert_eq!(top(7), 60.0 + lh + 50.0);
        assert_eq!(by_node(&layout, 1).h, 60.0 + lh + 50.0 + lh);
        // Already past the float: no clearance, the margins collapse as
        // they always did (a's 0 with b's 5).
        let layout = document_of(
            Element::new("div")
                .push(Element::new("div").attr("class", "f"))
                .push(Element::new("p").attr("class", "a").text("x"))
                .push(Element::new("p").attr("class", "b").text("y")),
            "div, p { margin: 0; } .f { float: left; width: 30px; height: 10px; }
             .b { clear: left; margin-top: 5px; }",
        );
        assert_eq!(by_node(&layout, 4).y, lh + 5.0);
        // A self-collapsing box given clearance is not self-collapsing
        // for the margins above it: its top set stops at the float's
        // bottom, and only its bottom margin goes on to collapse with
        // the next paragraph's. The float waits for the flow position
        // when the box comes, which the box resolves above its own
        // margin: the float is at the container's top. Nodes: div 1,
        // f 2, e 3, p 4.
        let layout = document_of(
            Element::new("div")
                .push(Element::new("div").attr("class", "f"))
                .push(Element::new("div").attr("class", "e"))
                .push(Element::new("p").text("y")),
            "div, p { margin: 0; } .f { float: left; width: 30px; height: 60px; }
             .e { clear: both; margin: 10px 0; } p { margin-top: 30px; }",
        );
        assert_eq!((by_node(&layout, 3).y, by_node(&layout, 3).h), (60.0, 0.0));
        assert_eq!(
            by_node(&layout, 4).y,
            90.0,
            "max(10, 30) below the cleared box"
        );
        assert_eq!(by_node(&layout, 1).h, 90.0 + lh);
        // Clearance also ends the collapsing through a parent's top: the
        // wrapper's 10px margin collapses through the container's top
        // and the cleared child's 20px does not, so the container, the
        // wrapper and the float waiting for the position are at 10,
        // and the cleared child below the float at 50, its margin
        // inside the clearance. Nodes: div 1, f 2, w 3, c 4.
        let layout = document_of(
            Element::new("div")
                .push(Element::new("div").attr("class", "f"))
                .push(
                    Element::new("div")
                        .attr("class", "w")
                        .push(Element::new("p").attr("class", "c").text("z")),
                ),
            "div, p { margin: 0; } .f { float: left; width: 30px; height: 40px; }
             .w { margin-top: 10px; } .c { clear: left; margin-top: 20px; }",
        );
        assert_eq!(by_node(&layout, 1).y, 10.0);
        assert_eq!(by_node(&layout, 2).y, 10.0, "the float, with the container");
        assert_eq!(
            by_node(&layout, 3).y,
            10.0,
            "the wrapper's own margin alone"
        );
        assert_eq!(
            by_node(&layout, 4).y,
            50.0,
            "the cleared child, below the float"
        );
        assert_eq!(by_node(&layout, 3).h, 40.0 + lh);
    }

    #[test]
    fn a_context_root_beside_floats_takes_the_room_left_or_moves_below() {
        // A 200px container with a 50 by 40 left float and an overflow
        // box: the box's border box keeps out of the float (CSS2 §9.5),
        // its `auto` width the room beside it; a left margin lies
        // inside that room; a box too wide for the room goes below the
        // float at its full width. Nodes: div 1, f 2, o 3.
        let root = Element::new("div")
            .push(Element::new("div").attr("class", "f"))
            .push(Element::new("div").attr("class", "o"));
        let css = "div { margin: 0; width: 200px; }
                   .f { float: left; width: 50px; height: 40px; }
                   .o { overflow: hidden; width: auto; height: 20px; }";
        let at = |layout: &DocumentLayout| {
            let o = by_node(layout, 3);
            (o.x, o.y, o.w)
        };
        assert_eq!(at(&document_of(root.clone(), css)), (50.0, 0.0, 150.0));
        assert_eq!(
            at(&document_of(
                root.clone(),
                &format!("{css} .o {{ margin-left: 10px; }}")
            )),
            (60.0, 0.0, 140.0)
        );
        assert_eq!(
            at(&document_of(
                root.clone(),
                &format!("{css} .o {{ width: 180px; }}")
            )),
            (0.0, 40.0, 180.0)
        );
        assert_eq!(
            at(&document_of(
                root.clone(),
                &css.replace("float: left", "float: right")
            )),
            (0.0, 0.0, 150.0)
        );
        // Moved below a float, the box is placed like a cleared one:
        // the margins above it resolve there, its own below them. Here
        // a paragraph with a 10px bottom margin precedes it: the box
        // would be at the line height plus 10, beside the float, where
        // its 180px do not fit, so it goes to the float's bottom.
        // Nodes: div 1, f 2, a 3, o 4.
        let m = test_fonts().primary().metrics(16.0);
        let lh = m.line_height();
        let layout = document_of(
            Element::new("div")
                .push(Element::new("div").attr("class", "f"))
                .push(Element::new("p").attr("class", "a").text("x"))
                .push(Element::new("div").attr("class", "o")),
            &format!("{css} p {{ margin: 0 0 10px 0; }} .o {{ width: 180px; margin-top: 5px; }}"),
        );
        assert!(lh + 10.0 < 40.0);
        assert_eq!(by_node(&layout, 4).y, 40.0);
        // A wider float below a narrow one: the room at the box's top is
        // not the room for its whole height, so the narrower room is
        // taken. Nodes: div 1, f 2, g 3, o 4.
        let layout = document_of(
            Element::new("div")
                .push(Element::new("div").attr("class", "f"))
                .push(Element::new("div").attr("class", "g"))
                .push(Element::new("div").attr("class", "o")),
            "div { margin: 0; width: 200px; }
             .f { float: left; width: 50px; height: 30px; }
             .g { float: left; width: 60px; height: 20px; clear: left; }
             .o { overflow: hidden; height: 50px; width: auto; }",
        );
        assert_eq!((by_node(&layout, 3).x, by_node(&layout, 3).y), (0.0, 30.0));
        let o = by_node(&layout, 4);
        assert_eq!((o.x, o.y, o.w), (60.0, 0.0, 140.0));
    }

    #[test]
    fn a_relatively_positioned_box_is_moved_with_its_content_and_the_flow_is_not() {
        // <div><p class=r>x</p><p class=n>y</p></div>: r is laid out
        // where it would be and then moved by its offsets, its text
        // with it; n sits where it would have, as if r had not moved.
        // Nodes: div 1, r 2, n 3.
        let m = test_fonts().primary().metrics(16.0);
        let lh = m.line_height();
        let root = Element::new("div")
            .push(Element::new("p").attr("class", "r").text("x"))
            .push(Element::new("p").attr("class", "n").text("y"));
        let css = "div, p { margin: 0; } .r { position: relative; left: 10px; top: 5px; }";
        let layout = document_of(root.clone(), css);
        let (r, n) = (by_node(&layout, 2), by_node(&layout, 3));
        assert_eq!((r.x, r.y), (10.0, 5.0));
        assert_eq!(
            (text_of(&layout, "x").x, text_of(&layout, "x").top()),
            (10.0, 5.0)
        );
        assert_eq!((n.x, n.y), (0.0, lh), "the flow is not affected");
        assert_eq!(by_node(&layout, 1).h, 2.0 * lh);
        // `right` and `bottom` move the other way; `left` wins over
        // `right` when both are given (left-to-right); `auto` is no move.
        let moved = |css: &str| {
            let layout = document_of(root.clone(), css);
            (by_node(&layout, 2).x, by_node(&layout, 2).y)
        };
        assert_eq!(
            moved("div, p { margin: 0; } .r { position: relative; right: 10px; bottom: 5px; }"),
            (-10.0, -5.0)
        );
        assert_eq!(
            moved("div, p { margin: 0; } .r { position: relative; left: 3px; right: 10px; }"),
            (3.0, 0.0)
        );
        assert_eq!(
            moved("div, p { margin: 0; } .r { position: relative; }"),
            (0.0, 0.0)
        );
        assert_eq!(
            moved("div, p { margin: 0; } .r { left: 10px; top: 5px; }"),
            (0.0, 0.0),
            "static: the offsets do nothing"
        );
        // A percentage is of the containing block's width horizontally
        // and of its height vertically, which is `auto` when that height
        // is not definite.
        assert_eq!(
            moved("div, p { margin: 0; } .r { position: relative; left: 10%; top: 10%; }"),
            (80.0, 0.0)
        );
        assert_eq!(
            moved(
                "div, p { margin: 0; } div { height: 200px; } .r { position: relative; top: 10%; }"
            ),
            (0.0, 20.0)
        );
        // A float first, moved from where the context put it: the line
        // beside it is shortened as if it had not moved. On that line,
        // in a font-less container, an inline-block is moved and the
        // next item is not. Nodes: div 1, f 2, a 3, b 4 — the spans
        // and the text in an anonymous block of the div.
        let layout = document_of(
            Element::new("div")
                .push(Element::new("div").attr("class", "f"))
                .push(Element::new("span").attr("class", "a"))
                .push(Element::new("span").attr("class", "b"))
                .text("z"),
            "div { margin: 0; font-size: 0; }
             span { display: inline-block; width: 20px; height: 10px; }
             .a { position: relative; left: 5px; top: 7px; }
             .f { float: left; width: 30px; height: 30px; position: relative; left: 100px; top: 1px; }",
        );
        assert_eq!((by_node(&layout, 2).x, by_node(&layout, 2).y), (100.0, 1.0));
        assert_eq!((by_node(&layout, 3).x, by_node(&layout, 3).y), (35.0, 7.0));
        assert_eq!((by_node(&layout, 4).x, by_node(&layout, 4).y), (50.0, 0.0));
        assert_eq!(
            text_of(&layout, "z").x,
            30.0 + 40.0,
            "beside the unmoved float, after the unmoved line"
        );
    }

    /// The box fragment of `node` and the fragment holding it: its
    /// parent in the tree, or `None` at the root.
    fn parent_of(layout: &DocumentLayout, node: usize) -> Option<usize> {
        fn find(
            fragments: &[Fragment],
            node: usize,
            parent: Option<usize>,
        ) -> Option<Option<usize>> {
            for f in fragments {
                if matches!(f.kind, FragmentKind::Box) && f.node == node {
                    return Some(parent);
                }
                if let Some(found) = find(&f.children, node, Some(f.node)) {
                    return Some(found);
                }
            }
            None
        }
        find(&layout.roots, node, None).unwrap_or_else(|| panic!("box of node {node}"))
    }

    #[test]
    fn an_absolutely_positioned_box_is_laid_out_against_its_containing_block() {
        // <div class=c><p>x</p><div class=a>y</div><p>z</p></div> with
        // the container positioned, 200 by 100 with a 10px border: the
        // box takes no room (z follows x), its offsets are from the
        // container's padding box, and its fragment is the container's
        // last, under it in the tree. Nodes: c 1, p 2, a 3, p 4.
        let m = test_fonts().primary().metrics(16.0);
        let lh = m.line_height();
        let root = Element::new("div")
            .attr("class", "c")
            .push(Element::new("p").text("x"))
            .push(Element::new("div").attr("class", "a").text("y"))
            .push(Element::new("p").text("z"));
        let css = ".c { position: relative; width: 200px; height: 100px; margin: 0;
                        border: 10px solid #000000; }
                   p { margin: 0; }
                   .a { position: absolute; left: 20px; top: 30px; width: 50px; height: 40px; }";
        let layout = document_of(root.clone(), css);
        let a = by_node(&layout, 3);
        assert_eq!((a.x, a.y, a.w, a.h), (30.0, 40.0, 50.0, 40.0));
        assert_eq!(by_node(&layout, 4).y, 10.0 + lh, "z right after x");
        assert_eq!(parent_of(&layout, 3), Some(1));
        let c = by_node(&layout, 1);
        assert_eq!(
            c.children.last().map(|f| f.node),
            Some(3),
            "after the content"
        );
        assert_eq!(text_of(&layout, "y").x, 30.0);
        // `right` and `bottom` measure from the padding box's far edges;
        // `auto` width with both edges given is what is left between.
        let at = |css: &str| {
            let layout = document_of(root.clone(), css);
            let a = by_node(&layout, 3);
            (a.x, a.y, a.w, a.h)
        };
        let base = ".c { position: relative; width: 200px; height: 100px; margin: 0;
                         border: 10px solid #000000; } p { margin: 0; }";
        assert_eq!(
            at(&format!(
                "{base} .a {{ position: absolute; right: 20px; bottom: 10px; width: 50px; height: 40px; }}"
            )),
            (
                10.0 + 200.0 - 20.0 - 50.0,
                10.0 + 100.0 - 10.0 - 40.0,
                50.0,
                40.0
            )
        );
        assert_eq!(
            at(&format!(
                "{base} .a {{ position: absolute; left: 20px; right: 30px; top: 5px; bottom: 15px; }}"
            )),
            (30.0, 15.0, 150.0, 80.0)
        );
        // Over-constrained: `right` and `bottom` are ignored; `auto`
        // margins on both sides centre the box.
        assert_eq!(
            at(&format!(
                "{base} .a {{ position: absolute; left: 20px; right: 30px; width: 50px;
                                       top: 5px; bottom: 15px; height: 10px; }}"
            )),
            (30.0, 15.0, 50.0, 10.0)
        );
        assert_eq!(
            at(&format!(
                "{base} .a {{ position: absolute; left: 0; right: 0; width: 50px;
                                       margin: 0 auto; top: 0; bottom: 0; height: 10px; }}"
            )),
            (10.0 + 75.0, 10.0, 50.0, 10.0)
        );
        // Percentages are of the padding box; a percentage top is of
        // its height, which a positioned ancestor always has.
        assert_eq!(
            at(&format!(
                "{base} .a {{ position: absolute; left: 10%; top: 50%; width: 25%; height: 10%; }}"
            )),
            (30.0, 60.0, 50.0, 10.0)
        );
    }

    #[test]
    fn the_static_position_is_where_the_box_would_have_been_in_flow() {
        // All offsets `auto`: the box sits where it was met, after the
        // margins resolved so far, at the content box's left, at a
        // shrink-to-fit width; a `width` alone keeps the static
        // position; `left` alone uses the static top.
        let m = test_fonts().primary().metrics(16.0);
        let lh = m.line_height();
        let fonts = test_fonts();
        let root = Element::new("div")
            .attr("class", "c")
            .push(Element::new("p").attr("class", "p").text("x"))
            .push(Element::new("div").attr("class", "a").text("yy"))
            .push(Element::new("p").text("z"));
        let base = ".c { position: relative; width: 200px; margin: 0; padding: 5px; }
                    p { margin: 0; } .p { margin-bottom: 20px; }";
        let at = |css: &str| {
            let layout = document_of(root.clone(), css);
            let a = by_node(&layout, 3);
            (a.x, a.y, a.w)
        };
        assert_eq!(
            at(&format!("{base} .a {{ position: absolute; }}")),
            (5.0, 5.0 + lh + 20.0, fonts.measure("yy", 16.0))
        );
        assert_eq!(
            at(&format!("{base} .a {{ position: absolute; width: 30px; }}")),
            (5.0, 5.0 + lh + 20.0, 30.0)
        );
        assert_eq!(
            at(&format!(
                "{base} .a {{ position: absolute; left: 50px; width: 30px; }}"
            )),
            (50.0, 5.0 + lh + 20.0, 30.0),
            "left is from the padding box, which starts at the border box's edge"
        );
        assert_eq!(
            at(&format!("{base} .a {{ position: absolute; top: 1px; }}")),
            (5.0, 1.0, fonts.measure("yy", 16.0)),
            "top from the padding box's top, left the static one"
        );
        // The margins around the box collapse as if it were not there:
        // z sits 20px below x, and the box's static top is after that
        // margin, the flow position when it was met.
        let layout = document_of(
            root.clone(),
            &format!("{base} .a {{ position: absolute; }}"),
        );
        assert_eq!(by_node(&layout, 4).y, 5.0 + lh + 20.0);
        // Met in a run, the static position is the current x on the
        // line and the line's top; the words around it flow on. Nodes:
        // c 1, p 2, a 3 (in the p's run).
        let layout = document_of(
            Element::new("div").attr("class", "c").push(
                Element::new("p")
                    .text("aa ")
                    .push(Element::new("div").attr("class", "a").text("y"))
                    .text("bb"),
            ),
            ".c { position: relative; margin: 0; } p { margin: 0; }
             .a { position: absolute; }",
        );
        let a = by_node(&layout, 3);
        let aa = fonts.measure("aa", 16.0);
        assert_eq!((a.x, a.y), (aa, 0.0));
        assert_eq!(
            text_of(&layout, "bb").x,
            aa + fonts.measure(" ", 16.0),
            "one gap, the box between the words being out of flow"
        );
        // Inside an empty, unpositioned wrapper, the static position is
        // the flow position after the margins resolved so far — the
        // wrapper's own 10px, not the paragraph's 30px that collapses
        // with it later, as browsers have it — and the box is carried
        // up to the positioned container. Nodes: c 1, w 2, a 3, p 4.
        let layout = document_of(
            Element::new("div")
                .attr("class", "c")
                .push(
                    Element::new("div")
                        .attr("class", "w")
                        .push(Element::new("div").attr("class", "a")),
                )
                .push(Element::new("p").text("z")),
            ".c { position: relative; margin: 0; padding: 5px; } .w { margin-top: 10px; }
             p { margin: 30px 0 0 0; } .a { position: absolute; width: 10px; height: 10px; }",
        );
        assert_eq!(by_node(&layout, 3).y, 5.0 + 10.0);
        assert_eq!(
            by_node(&layout, 4).y,
            5.0 + 30.0,
            "the paragraph after the collapsed margins"
        );
        assert_eq!(parent_of(&layout, 3), Some(1));
    }

    #[test]
    fn the_initial_containing_block_takes_the_rest_and_fixed_boxes_go_to_the_root() {
        // No positioned ancestor: the viewport is the containing block,
        // the fragment a root of the document after the content, and
        // the page reaches down to the box; a scroll container between
        // does not get it. Nodes: s 1, a 2 — then f 3 in the second
        // document.
        let fonts = test_fonts();
        let doc = |css: &str, root: Element| {
            let docu = dom::Node::document(vec![root.into()]);
            let sheet = css::parse_stylesheet(css).expect("test css is valid");
            let styled = style::style_document(&docu, &[sheet]);
            let boxes = crate::build_box_tree(&styled);
            layout_document_in(&boxes, VIEWPORT, Some(600.0), &fonts)
        };
        let root = Element::new("div")
            .attr("class", "s")
            .push(Element::new("div").attr("class", "a"));
        let layout = doc(
            ".s { overflow: auto; height: 50px; margin: 0; }
             .a { position: absolute; right: 100px; top: 700px; width: 50px; height: 20px; }",
            root.clone(),
        );
        let a = by_node(&layout, 2);
        assert_eq!((a.x, a.y, a.w, a.h), (650.0, 700.0, 50.0, 20.0));
        assert_eq!(parent_of(&layout, 2), None);
        assert_eq!(layout.roots.last().map(|f| f.node), Some(2));
        assert_eq!(layout.height, 720.0, "the page reaches the box");
        assert_eq!(
            by_node(&layout, 1).scroll_height,
            50.0,
            "not the container's"
        );
        assert!(!layout.depends_on_viewport_height);
        // `bottom` is from the viewport's bottom, and makes the layout
        // depend on the viewport's height; a fixed box is a root too,
        // placed the same way, and does not make the page taller.
        let layout = doc(
            ".s { overflow: auto; height: 50px; margin: 0; }
             .a { position: fixed; bottom: 0; left: 0; width: 50px; height: 20px; }",
            root.clone(),
        );
        assert_eq!((by_node(&layout, 2).x, by_node(&layout, 2).y), (0.0, 580.0));
        assert_eq!(parent_of(&layout, 2), None);
        assert_eq!(layout.height, 50.0);
        assert!(layout.depends_on_viewport_height);
        // A fixed box inside an absolutely positioned one goes to the
        // root all the same, from a static position carried through.
        // Nodes: s 1, a 2, f 3.
        let layout = doc(
            ".s { overflow: auto; height: 50px; margin: 0; }
             .a { position: absolute; left: 100px; top: 100px; width: 50px; height: 20px; }
             .f { position: fixed; width: 10px; height: 10px; }",
            Element::new("div").attr("class", "s").push(
                Element::new("div")
                    .attr("class", "a")
                    .push(Element::new("div").attr("class", "f")),
            ),
        );
        assert_eq!(
            (by_node(&layout, 3).x, by_node(&layout, 3).y),
            (100.0, 100.0)
        );
        assert_eq!(parent_of(&layout, 3), None);
        assert_eq!(parent_of(&layout, 2), None);
        // Without a viewport height, `bottom` cannot be used: the static
        // top instead, and a percentage top is `auto`.
        let layout = document_of(
            Element::new("div")
                .attr("class", "a")
                .push(Element::new("p").text("x")),
            ".a { position: absolute; bottom: 0; top: 10%; width: 50px; height: 20px; margin: 0; }
             p { margin: 0; }",
        );
        assert_eq!(by_node(&layout, 1).y, 0.0);
        assert!(!layout.depends_on_viewport_height);
    }

    #[test]
    fn an_absolutely_positioned_box_is_a_context_root_with_the_content_laid_out_inside() {
        // Its content is laid out at the solved width, its own floats
        // in its own context, its height the content's when `auto`;
        // `height` from `top` and `bottom` leaves the content where it
        // is. Nodes: c 1, a 2, f 3, p 4.
        let m = test_fonts().primary().metrics(16.0);
        let lh = m.line_height();
        let root = Element::new("div").attr("class", "c").push(
            Element::new("div")
                .attr("class", "a")
                .push(Element::new("div").attr("class", "f"))
                .push(Element::new("p").text("x")),
        );
        let layout = document_of(
            root.clone(),
            ".c { position: relative; width: 300px; height: 200px; margin: 0; }
             .a { position: absolute; left: 10px; top: 10px; width: 100px; padding: 5px; }
             .f { float: left; width: 20px; height: 50px; } p { margin: 0; }",
        );
        let a = by_node(&layout, 2);
        assert_eq!((a.x, a.y, a.w), (10.0, 10.0, 110.0));
        assert_eq!(a.h, 5.0 + 50.0 + 5.0, "reaches its float");
        assert_eq!(by_node(&layout, 3).x, 15.0);
        assert_eq!(text_of(&layout, "x").x, 35.0, "beside the float");
        assert_eq!(text_of(&layout, "x").top(), 15.0);
        let layout = document_of(
            root,
            ".c { position: relative; width: 300px; height: 200px; margin: 0; }
             .a { position: absolute; left: 10px; top: 10px; bottom: 10px; width: 100px; }
             .f { float: left; width: 20px; height: 50px; } p { margin: 0; }",
        );
        let a = by_node(&layout, 2);
        assert_eq!(a.h, 180.0);
        assert_eq!(text_of(&layout, "x").top(), 10.0);
        assert!(lh > 0.0);
    }

    /// Every inline box fragment (M11.6) in the document, in tree
    /// order: one per line each inline element appears on.
    fn inline_boxes(layout: &DocumentLayout) -> Vec<Fragment> {
        layout
            .iter()
            .filter(|f| matches!(f.kind, FragmentKind::InlineBox))
            .cloned()
            .collect()
    }

    #[test]
    fn an_inline_element_has_a_box_on_its_line_around_its_items() {
        // <p><span>ab</span></p>, the span with 5px of horizontal
        // padding, a 2px border and 3px margins: the text starts after
        // the start edge, inside the span's box, whose border box lies
        // within the margins and around the content area — above the
        // line's top by the border, which does not change the line.
        let layout = document_of(
            Element::new("p").push(Element::new("span").text("ab")),
            "p { margin: 0 } span { padding: 0 5px; border: 2px solid #000; margin: 0 3px }",
        );
        let fonts = test_fonts();
        let m = fonts.primary().metrics(16.0);
        let ab = fonts.measure("ab", 16.0);
        let text = text_of(&layout, "ab");
        assert_eq!((text.x, text.y), (10.0, m.ascent));
        let boxes = inline_boxes(&layout);
        assert_eq!(boxes.len(), 1);
        let b = &boxes[0];
        assert_eq!(b.node, text.node);
        assert_eq!((b.x, b.w), (3.0, 14.0 + ab));
        assert_eq!((b.y, b.h), (-2.0, 2.0 + m.ascent + m.descent + 2.0));
        assert_eq!(b.children.len(), 1, "the text is inside it");
        assert_eq!(
            b.children[0].x, 10.0,
            "absolute, as the final pass leaves it"
        );
        assert_eq!(layout.height, m.line_height());
    }

    #[test]
    fn an_inline_element_across_two_lines_is_sliced_at_the_break() {
        // The p just too narrow for both words beside the start edge:
        // the span's first box has its start edge and no end edge, the
        // second the reverse, and the sliced sides have no border.
        let fonts = test_fonts();
        let (aaaa, bbbb) = (fonts.measure("aaaa", 16.0), fonts.measure("bbbb", 16.0));
        let width = 10.0 + aaaa + fonts.measure(" ", 16.0) + bbbb - 1.0;
        let layout = document_of(
            Element::new("p").push(Element::new("span").text("aaaa bbbb")),
            &format!(
                "p {{ margin: 0; width: {width}px }}
                 span {{ padding: 0 5px; border: 2px solid #000; margin: 0 3px }}"
            ),
        );
        let m = fonts.primary().metrics(16.0);
        let boxes = inline_boxes(&layout);
        assert_eq!(boxes.len(), 2);
        let (first, second) = (&boxes[0], &boxes[1]);
        use style::computed::BorderStyle;
        assert_eq!((first.x, first.w), (3.0, 7.0 + aaaa));
        assert_eq!(first.style.border_style[LEFT], BorderStyle::Solid);
        assert_eq!(first.style.border_style[RIGHT], BorderStyle::None);
        assert_eq!((second.x, second.w), (0.0, bbbb + 7.0));
        assert_eq!(second.y, first.y + m.line_height());
        assert_eq!(second.style.border_style[LEFT], BorderStyle::None);
        assert_eq!(second.style.border_style[RIGHT], BorderStyle::Solid);
        assert_eq!(
            text_of(&layout, "bbbb").x,
            0.0,
            "no start edge on the second line"
        );
    }

    #[test]
    fn white_space_at_an_inline_elements_edges_collapses_to_one_gap() {
        let fonts = test_fonts();
        let (a, b, sp) = (
            fonts.measure("a", 16.0),
            fonts.measure("b", 16.0),
            fonts.measure(" ", 16.0),
        );
        // `a <span>b</span> c`: a gap before the span and one after it.
        let layout = document_of(
            Element::new("p")
                .text("a ")
                .push(Element::new("span").text("b"))
                .text(" c"),
            "p { margin: 0 }",
        );
        assert_eq!(text_of(&layout, "b").x, a + sp);
        assert_eq!(text_of(&layout, "c").x, a + sp + b + sp);
        // `a <span> b</span>`: the space inside collapses into the gap
        // before the span (css-text-3 §4.1.1).
        let layout = document_of(
            Element::new("p")
                .text("a ")
                .push(Element::new("span").text(" b")),
            "p { margin: 0 }",
        );
        assert_eq!(text_of(&layout, "b").x, a + sp);
        // `<span>a </span>b`: the space at the end of the span is the
        // gap before `b`, rendered after the span's end edge.
        let layout = document_of(
            Element::new("p")
                .push(Element::new("span").text("a "))
                .text("b"),
            "p { margin: 0 } span { padding-right: 4px }",
        );
        assert_eq!(text_of(&layout, "b").x, a + 4.0 + sp);
        assert_eq!(inline_boxes(&layout)[0].w, a + 4.0);
    }

    #[test]
    fn an_unbreakable_piece_moves_to_the_next_line_whole() {
        let fonts = test_fonts();
        let m = fonts.primary().metrics(16.0);
        let w = |t: &str| fonts.measure(t, 16.0);
        // `aa <span>bbbb</span>` just too narrow for both: the span,
        // glued to its word, starts on the second line with it and has
        // no box on the first.
        let width = w("aa") + w(" ") + w("bbbb") - 1.0;
        let layout = document_of(
            Element::new("p")
                .text("aa ")
                .push(Element::new("span").text("bbbb")),
            &format!("p {{ margin: 0; width: {width}px }}"),
        );
        assert_eq!(text_of(&layout, "aa").y, m.ascent);
        let bbbb = text_of(&layout, "bbbb");
        assert_eq!((bbbb.x, bbbb.y), (0.0, m.ascent + m.line_height()));
        let boxes = inline_boxes(&layout);
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].y, m.line_height());
        // `aa <b>xx</b>yy`: `xx` and `yy` are one piece, set flush;
        // when `yy` does not fit, the piece moves back to the gap.
        let width = w("aa") + w(" ") + w("xx") + w("yy") - 1.0;
        let layout = document_of(
            Element::new("p")
                .text("aa ")
                .push(Element::new("b").text("xx"))
                .text("yy"),
            &format!("p {{ margin: 0; width: {width}px }}"),
        );
        let (xx, yy) = (text_of(&layout, "xx"), text_of(&layout, "yy"));
        assert_eq!((xx.x, xx.y), (0.0, m.ascent + m.line_height()));
        assert_eq!((yy.x, yy.y), (w("xx"), m.ascent + m.line_height()));
        assert_eq!(layout.height, 2.0 * m.line_height());
    }

    #[test]
    fn a_relatively_positioned_inline_element_moves_with_its_text() {
        let layout = document_of(
            Element::new("p").push(Element::new("span").text("ab")),
            "p { margin: 0 } span { position: relative; left: 5px; top: 2px }",
        );
        let m = test_fonts().primary().metrics(16.0);
        let b = &inline_boxes(&layout)[0];
        assert_eq!((b.x, b.y), (5.0, 2.0));
        let text = text_of(&layout, "ab");
        assert_eq!((text.x, text.y), (5.0, m.ascent + 2.0));
        assert_eq!(layout.height, m.line_height(), "the flow is not affected");
    }

    #[test]
    fn an_empty_inline_element_makes_a_line_only_with_edges_of_some_width() {
        // CSS2 §9.4.2: a line box with nothing in it but an inline
        // element without margins, padding or borders is no line.
        let layout = document_of(
            Element::new("p").push(Element::new("span")),
            "p { margin: 0 }",
        );
        assert_eq!(layout.height, 0.0);
        assert!(inline_boxes(&layout).is_empty());
        let layout = document_of(
            Element::new("p").push(Element::new("span")),
            "p { margin: 0 } span { padding: 0 5px }",
        );
        let m = test_fonts().primary().metrics(16.0);
        assert_eq!(layout.height, m.line_height());
        let b = &inline_boxes(&layout)[0];
        assert_eq!((b.x, b.w, b.h), (0.0, 10.0, m.ascent + m.descent));
    }
}
