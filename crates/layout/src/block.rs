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
    BOTTOM, BoxSizing, ComputedStyle, Display, LEFT, LengthPercentage, LengthPercentageAuto, RIGHT,
    TOP,
};
use text::FontCollection;

use crate::box_tree::{BoxKind, LayoutBox};
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
    /// The fragments of this box's content, in paint order. A text
    /// fragment has none.
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
    if f.style.overflow.clips() {
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
    /// Every fragment in paint order (preorder: a box, then its content).
    pub fn iter(&self) -> Preorder<'_> {
        preorder(&self.roots)
    }
}

/// A preorder walk over fragment subtrees — iterative, so a deep tree
/// costs heap, not stack.
pub struct Preorder<'a> {
    stack: Vec<&'a Fragment>,
}

/// The fragments under `roots` in paint order.
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

/// Takes the fragments out of their tree in paint order; each comes out
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
    let children = layout_children(boxes, viewport_width, viewport_height, fonts);
    let depends_on_viewport_height = viewport_height.is_some()
        && boxes.iter().any(|b| {
            let percent = |v: LengthPercentage| matches!(v, LengthPercentage::Percent(_));
            matches!(b.style.height, LengthPercentageAuto::Percent(_))
                || percent(b.style.min_height)
                || b.style.max_height.is_some_and(percent)
        });
    // The viewport is the one context with no further ancestor to
    // escape to — `leading` *must* materialize here, exactly once, or
    // it would just vanish (see `layout_children`'s doc). So must
    // `trailing`, below the content; when nothing materialised the two
    // are one set, counted once.
    let shift = children.leading.resolve();
    let height = if children.nothing_materialised {
        shift
    } else {
        shift + children.content_height + children.trailing.resolve()
    };
    let mut roots = children.fragments;
    make_absolute(&mut roots, 0.0, shift);
    DocumentLayout {
        roots,
        height: height.max(0.0),
        depends_on_viewport_height,
    }
}

/// [`layout_document`]'s fragments in paint order without the tree —
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
fn resolve_width_and_margins(
    style: &ComputedStyle,
    cb_width: f32,
    non_content_width: f32,
) -> (f32, f32, f32) {
    let c = width_constraints(style, cb_width, non_content_width);
    let mut used = tentative_width(style, cb_width, non_content_width, c.width);
    if let Some(max) = c.max
        && used.0 > max
    {
        used = tentative_width(style, cb_width, non_content_width, Some(max));
    }
    if used.0 < c.min {
        used = tentative_width(style, cb_width, non_content_width, Some(c.min));
    }
    used
}

/// CSS2.1 §10.3.3 for one value of `width` (`None` for `auto`), as the
/// content-box width: `(width, margin_left, margin_right)`.
fn tentative_width(
    style: &ComputedStyle,
    cb_width: f32,
    non_content_width: f32,
    width: Option<f32>,
) -> (f32, f32, f32) {
    let avail = cb_width - non_content_width;
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

/// One box's layout, relative to its own border-box top-left `(0, 0)` —
/// the caller (`layout_children`) translates once collapsed-margin
/// placement among siblings is resolved.
pub(crate) struct BlockLayout {
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
    /// The box's top and bottom margins adjoin each other (an empty
    /// box, or one whose every child collapsed through): `margin_top`
    /// and `margin_bottom` are the same set, to be joined with the
    /// siblings' margins once, and the box has no height.
    pub(crate) self_collapsing: bool,
    /// The baseline of the box's last line box, from its border-box
    /// top (CSS2 §10.8.1: an inline-block's own baseline); `None` when
    /// nothing inside made a line.
    pub(crate) last_baseline: Option<f32>,
}

/// An inline-block laid out for its line (M10.4, M7.7): one atomic item
/// as wide and as tall as its margin box.
pub(crate) struct AtomicLayout {
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
}

/// Whether a box with this style establishes a block formatting context
/// of its own (CSS2 §9.4.1, M10.4): `overflow` other than `visible`, or
/// an inline-block. Floats, absolutely positioned boxes and table cells
/// do too; each arrives with its milestone.
fn establishes_bfc(style: &ComputedStyle) -> bool {
    style.overflow.clips() || style.display == Display::InlineBlock
}

/// A box's border and padding, resolved against its containing block's
/// width (padding's percentages are of that width on every side, CSS2
/// §8.4).
struct Edges {
    border: [f32; 4],
    padding: [f32; 4],
}

impl Edges {
    fn of(style: &ComputedStyle, cb_width: f32) -> Self {
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
/// here: [`layout_children`] lays its lines out directly).
fn layout_block(
    b: &LayoutBox,
    cb_width: f32,
    cb_height: Option<f32>,
    fonts: &FontCollection,
) -> BlockLayout {
    let edges = Edges::of(&b.style, cb_width);
    // margin-right is fully resolved here but never read again — see
    // `BlockLayout::margin_left`'s doc for why that's correct, not an
    // oversight.
    let (content_width, margin_left, _margin_right) =
        resolve_width_and_margins(&b.style, cb_width, edges.horizontal());
    layout_block_contents(
        b,
        &edges,
        cb_width,
        cb_height,
        content_width,
        margin_left,
        fonts,
    )
}

/// Lays out one [`BoxKind::InlineBlock`] box for its line (M10.4):
/// CSS2 §10.3.9 — `auto` margins are zero, an `auto` width is the
/// shrink-to-fit width, `min(max(min-content, available), max-content)`
/// with `available` the containing block's width less the box's own
/// margins, border and padding (§10.3.5), clamped by `max-width` and
/// `min-width` (§10.4); the content then lays out as any block's. The
/// baseline is the last line box's, or the bottom margin edge when
/// there is none or the box clips (§10.8.1).
pub(crate) fn layout_inline_block(
    b: &LayoutBox,
    cb_width: f32,
    cb_height: Option<f32>,
    fonts: &FontCollection,
) -> AtomicLayout {
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
    }
}

/// The body of a block container's layout once its content width and
/// left margin are known: its height, the stacking of its children and
/// the margins it exposes.
#[allow(clippy::too_many_arguments)]
fn layout_block_contents(
    b: &LayoutBox,
    edges: &Edges,
    cb_width: f32,
    cb_height: Option<f32>,
    content_width: f32,
    margin_left: f32,
    fonts: &FontCollection,
) -> BlockLayout {
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

    // An empty block (no children, no border/padding, auto height, not
    // a formatting context root): its own margin-top and margin-bottom
    // collapse with *each other*, escaping through as a single set on
    // both sides at once — as if the box weren't there at all,
    // margin-wise (CSS2 §8.3.1).
    if b.children.is_empty() && can_collapse_through {
        let set = own_top.join(own_bottom);
        return BlockLayout {
            root: box_fragment(content_width, 0.0, b),
            height: 0.0,
            margin_left,
            margin_top: set,
            margin_bottom: set,
            self_collapsing: true,
            last_baseline: None,
        };
    }

    let children = layout_children(&b.children, content_width, specified_height, fonts);
    let content_x = border[LEFT] + padding[LEFT];

    // Every child collapsed through (empty blocks all the way down):
    // their one set and this box's own margins adjoin, and so does the
    // box's top with its bottom.
    if children.nothing_materialised && can_collapse_through {
        let set = own_top.join(children.leading).join(own_bottom);
        let mut root = box_fragment(content_width, 0.0, b);
        root.children = children
            .fragments
            .into_iter()
            .map(|f| f.translated(content_x, 0.0))
            .collect();
        return BlockLayout {
            root,
            height: 0.0,
            margin_left,
            margin_top: set,
            margin_bottom: set,
            self_collapsing: true,
            last_baseline: None,
        };
    }

    // `layout_children` never materializes the *leading* set itself
    // (see its doc) — it's this box's job to decide, now that it knows
    // its own separation, whether that set becomes real space just
    // inside its own border/padding-top (`child_shift`, when there's a
    // border/padding to stop the escape) or keeps escaping outward,
    // joined with this box's own margin-top (when there isn't). This
    // is the step a naive "always materialize inside the parent" design
    // gets wrong: that double-counts the escaping case, because the
    // SAME margin would then also get re-applied one level up, when
    // *this* box gets positioned among *its* own siblings.
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
    if b.style.overflow.clips() {
        // The scrollable overflow area (M10.5, see `Fragment::
        // scroll_height`): the content as if the height were auto, or
        // a descendant reaching lower, from the padding box's top to
        // the bottom padding's end, and never less than the padding
        // box itself. The children are still relative to the first
        // child's top, `child_shift` below the content box's.
        let extent = children
            .fragments
            .iter()
            .map(extent_bottom)
            .fold(0.0, f32::max)
            + child_shift;
        let padding_box = padding[TOP] + content_height + padding[BOTTOM];
        root.scroll_height =
            (padding[TOP] + auto_content_height.max(extent) + padding[BOTTOM]).max(padding_box);
    }
    let content_y = border[TOP] + padding[TOP] + child_shift;
    root.children = children
        .fragments
        .into_iter()
        .map(|f| f.translated(content_x, content_y))
        .collect();

    BlockLayout {
        root,
        height: border_box_height,
        margin_left,
        margin_top,
        margin_bottom,
        self_collapsing: false,
        last_baseline: children.last_baseline.map(|y| content_y + y),
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
struct ChildrenLayout {
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
}

/// Stacks a block container's children (M10.4: all block-level, per the
/// box tree builder's contract — an inline-level box that a hand-built
/// tree puts here is laid out as a run of one). An anonymous block is
/// its inline formatting context: its lines, aligned per its own
/// inherited `text-align` (M7.5), at the container's full width.
fn layout_children(
    children: &[LayoutBox],
    cb_width: f32,
    cb_height: Option<f32>,
    fonts: &FontCollection,
) -> ChildrenLayout {
    let mut fragments = Vec::new();
    let mut y = 0.0f32;
    // The set of margins adjoining BETWEEN two siblings, not yet
    // materialized (never the leading one, which `still_leading` routes
    // to `leading` instead, never adding it to `y`).
    let mut pending = AdjoiningMargins::default();
    let mut still_leading = true;
    let mut leading = AdjoiningMargins::default();
    let mut last_baseline = None;

    for child in children {
        match &child.kind {
            BoxKind::Block => {
                let layout = layout_block(child, cb_width, cb_height, fonts);
                if layout.self_collapsing {
                    // Its one set joins the margins around it and stays
                    // open; the box itself has no height. Its fragment
                    // sits where the set so far resolves — at the
                    // parent's content top while the set is still
                    // escaping (CSS2 §8.3.1: same top border edge as the
                    // parent). Nothing paints or hits a zero-height box
                    // with no border or padding, so the exact position
                    // is unobservable; the set is what matters.
                    let box_y = if still_leading {
                        leading = leading.join(layout.margin_top);
                        y
                    } else {
                        pending = pending.join(layout.margin_top);
                        y + pending.resolve()
                    };
                    let margin_left = layout.margin_left;
                    fragments.push(layout.root.translated(margin_left, box_y));
                    continue;
                }
                if still_leading {
                    leading = leading.join(layout.margin_top);
                    still_leading = false;
                    // `y` stays at 0 — see `leading`'s doc.
                } else {
                    y += pending.join(layout.margin_top).resolve();
                }
                let child_y = y;
                let margin_left = layout.margin_left;
                if let Some(baseline) = layout.last_baseline {
                    last_baseline = Some(child_y + baseline);
                }
                fragments.push(layout.root.translated(margin_left, child_y));
                y = child_y + layout.height;
                pending = layout.margin_bottom;
            }
            BoxKind::AnonymousBlock | BoxKind::Inline | BoxKind::InlineBlock | BoxKind::Text(_) => {
                // One inline formatting context (M7): the anonymous
                // block's children wrap onto lines together.
                let run = match child.kind {
                    BoxKind::AnonymousBlock => child.children.as_slice(),
                    _ => std::slice::from_ref(child),
                };
                let lines = layout_inline(run, cb_width, cb_height, &child.style, fonts);
                if lines.fragments.is_empty() && lines.height == 0.0 {
                    // No line box at all (an empty inline element):
                    // treated as not existing, so it separates no
                    // margins (CSS2 §9.4.2).
                    continue;
                }
                // A line box always separates (margins never collapse
                // through it, same as a border/padding would) — flush
                // whatever's pending (or stop a still-leading escape
                // dead, materializing nothing since there was never a
                // border/padding to place it against).
                if still_leading {
                    still_leading = false; // `leading` stays as joined so far: nothing escapes past real content
                } else {
                    y += pending.resolve();
                }
                pending = AdjoiningMargins::default();
                if let Some(baseline) = lines.last_baseline {
                    last_baseline = Some(y + baseline);
                }
                fragments.extend(lines.fragments.into_iter().map(|f| f.translated(0.0, y)));
                y += lines.height;
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
        // Nothing follows, so only the top (leading) margin is
        // observable at the viewport boundary — but it must be the
        // *collapsed* value (max(10,30)=30), not just margin-top (10).
        assert_eq!(frags[0].y, 30.0);
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
            "section", "article", "nav", "header", "footer", "main", "form", "table",
        ] {
            let frags = fragments_of(Element::new(tag).text("x"), "");
            assert_eq!(box_fragments(&frags).len(), 1, "<{tag}> is a block");
        }
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
}
