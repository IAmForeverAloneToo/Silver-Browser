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
//! makes one of its own. Within a stacking context: its own background
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
//! Every rectangle a step carries is in viewport coordinates: the
//! viewport's scroll offset and, inside a scroll container, its offset
//! (M10.5) are already applied, and so is the shift of a `position:
//! sticky` box ([`sticky_shift`], M11), which paint and hit testing
//! thereby agree on. A clip's rectangle is the effective one,
//! intersected with the clips around it, so a consumer needs only the
//! innermost. Content the viewport cannot show is left out: a box or a
//! line it does not meet has no step, and a clipping box it does not
//! meet is skipped with its whole subtree, since nothing under a clip
//! can show where the clip does not; a clip with no step inside it is
//! left out too.

use dom::NodeId;
use layout::{Fragment, FragmentKind};
use style::computed::{
    BOTTOM, ComputedStyle, Float, LEFT, LengthPercentageAuto, Overflow, Position, RIGHT, TOP,
};

use crate::{ScrollOffsets, Viewport};

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
    let mut walk = Walk {
        steps: Vec::new(),
        viewport: Rect {
            x: 0.0,
            y: 0.0,
            w: viewport.width,
            h: viewport.height,
        },
        dx: 0.0,
        dy: -viewport.scroll_y,
        chain: Vec::new(),
        offsets,
    };
    // The roots' containing block is the initial one: as wide as the
    // viewport, from the document's top down without end.
    let initial = Rect {
        x: 0.0,
        y: -viewport.scroll_y,
        w: viewport.width,
        h: f32::INFINITY,
    };
    match roots.split_first() {
        // The first root is the root element's box, the root stacking
        // context; the roots after it — the boxes the initial
        // containing block positions and the fixed ones (M11) — are
        // positioned descendants of the root element that layout moved
        // out of its subtree, and belong to its stacking context.
        Some((root, rest)) if matches!(root.kind, FragmentKind::Box) => {
            walk.stacking(root, initial, rest);
        }
        // Not a box tree at all: whatever is there, in flow.
        Some(_) => {
            for phase in PHASES {
                walk.phase(roots, initial, phase);
            }
        }
        None => {}
    }
    walk.steps
}

/// The walk's state: where the document is relative to the viewport,
/// and the clips in force.
struct Walk<'a, 'o> {
    steps: Vec<Step<'a>>,
    viewport: Rect,
    /// From a document `x` to the viewport's: the shifts of the sticky
    /// boxes being walked into (M11).
    dx: f32,
    /// From a document `y` to the viewport's: the viewport's scroll
    /// offset and the offsets of the scroll containers being walked
    /// into, negated, and the shifts of the sticky boxes.
    dy: f32,
    /// The clips in force, outermost first: each clipping box being
    /// walked into, as [`Step::PushClip`] names it, with its effective
    /// clip rectangle.
    chain: Vec<(Option<NodeId>, Rect)>,
    offsets: &'o dyn ScrollOffsets,
}

/// A positioned descendant of a stacking context, taken out of tree
/// order to be painted at its layer (CSS2 Appendix E), with the walk's
/// state where the tree has it: the clips and scroll offsets of its
/// ancestors there, and its containing block for a sticky shift.
struct Positioned<'a> {
    f: &'a Fragment,
    containing: Rect,
    chain: Vec<(Option<NodeId>, Rect)>,
    dx: f32,
    dy: f32,
}

impl Positioned<'_> {
    /// Its `z-index`, `auto` counting as `0` for the order of the
    /// layers.
    fn z(&self) -> i32 {
        self.f.style.z_index.unwrap_or(0)
    }
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
    /// The effective clip in force, if any.
    fn clip(&self) -> Option<Rect> {
        self.chain.last().map(|(_, rect)| *rect)
    }

    /// A fragment's box in viewport coordinates.
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
        if !rect.meets(&self.viewport) {
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
    /// and does not scroll with the document: the viewport's offset is
    /// left out. A `position: sticky` box is shifted to keep its insets
    /// from the edges of the visible part of its scrollport — the
    /// innermost clip, else the viewport — within its containing block,
    /// whose padding box is `containing`.
    fn shifted(&mut self, f: &'a Fragment, containing: Rect, body: impl FnOnce(&mut Self)) {
        let (dx, dy) = (self.dx, self.dy);
        match f.style.position {
            Position::Fixed => self.dy = 0.0,
            Position::Sticky => {
                let scrollport = self.clip().unwrap_or(self.viewport);
                let (sx, sy) = sticky_shift(&f.style, self.rect_of(f), containing, scrollport);
                self.dx += sx;
                self.dy += sy;
            }
            Position::Static | Position::Relative | Position::Absolute => {}
        }
        body(self);
        self.dx = dx;
        self.dy = dy;
    }

    /// Runs `body` over the content of the box `f`, given its padding
    /// box, the containing block of what is inside: under its clip and
    /// scrolled by its offset when it clips (CSS2 §11.1.1), with the
    /// clip's steps and its scrollbar's as `emit` says. Not at all when
    /// the box clips and the viewport does not meet it: nothing under
    /// its clip can show. A clip with no step inside it is left out.
    fn content(&mut self, f: &'a Fragment, emit: Emit, body: impl FnOnce(&mut Self, Rect)) {
        let rect = self.rect_of(f);
        let clips = f.clips();
        if clips && !rect.meets(&self.viewport) {
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
        self.chain.push((node, effective));
        let offset = node.map_or(0.0, |node| self.offsets.offset(node));
        self.dy -= offset;
        body(self, padding_box);
        self.dy += offset;
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

    /// A box painted as a stacking context (CSS2 Appendix E): its own
    /// step; then, under its clip, the stacking contexts in it with a
    /// negative `z-index`, in `z-index` then tree order; its in-flow
    /// content in the three passes; its positioned descendants with
    /// `z-index: auto` or `0`, in tree order, an `auto` one as a unit, a
    /// `0` one a stacking context; and the stacking contexts with a
    /// positive `z-index`, in `z-index` then tree order. `extra` are
    /// further positioned descendants outside its subtree, after it in
    /// tree order: the roots layout moved out of the root element's.
    fn stacking(&mut self, f: &'a Fragment, containing: Rect, extra: &'a [Fragment]) {
        self.shifted(f, containing, |w| {
            w.own(f);
            let mut positioned = Vec::new();
            w.content(f, Emit::Nothing, |w, padding| {
                w.collect(&f.children, padding, &mut positioned);
            });
            w.collect(extra, containing, &mut positioned);
            w.content(f, Emit::ClipAndScrollbar, |w, padding| {
                w.layers(&f.children, padding, positioned);
            });
        });
    }

    /// Gathers the positioned boxes among `fragments` and their
    /// descendants in tree order, each with the walk's state where the
    /// tree has it, into `out`: a stacking context's own positioned
    /// descendants (Appendix E). The walk goes through everything but a
    /// stacking context, whose positioned descendants are its own.
    fn collect(
        &mut self,
        fragments: &'a [Fragment],
        containing: Rect,
        out: &mut Vec<Positioned<'a>>,
    ) {
        for f in fragments {
            if !matches!(f.kind, FragmentKind::Box | FragmentKind::InlineBox) {
                continue;
            }
            if f.style.position.is_positioned() {
                out.push(Positioned {
                    f,
                    containing,
                    chain: self.chain.clone(),
                    dx: self.dx,
                    dy: self.dy,
                });
                if f.style.z_index.is_some() {
                    continue;
                }
            }
            self.shifted(f, containing, |w| {
                w.content(f, Emit::Nothing, |w, padding| {
                    w.collect(&f.children, padding, out);
                });
            });
        }
    }

    /// The layers of a stacking context's content (Appendix E), `children`
    /// its in-flow content and `positioned` its positioned descendants
    /// in tree order.
    fn layers(
        &mut self,
        children: &'a [Fragment],
        containing: Rect,
        mut positioned: Vec<Positioned<'a>>,
    ) {
        // A stable sort keeps tree order within one `z-index`; `auto`
        // and `0` share the middle.
        positioned.sort_by_key(Positioned::z);
        let mut positioned = positioned.into_iter().peekable();
        while let Some(p) = positioned.next_if(|p| p.z() < 0) {
            self.positioned(p);
        }
        for phase in PHASES {
            self.phase(children, containing, phase);
        }
        for p in positioned {
            self.positioned(p);
        }
    }

    /// One pass over in-flow content: the fragments in tree order, each
    /// as the pass has it, through the content of every in-flow
    /// block-level box.
    fn phase(&mut self, fragments: &'a [Fragment], containing: Rect, phase: Phase) {
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
                    if phase == Phase::Inlines && !f.style.position.is_positioned() {
                        self.own(f);
                        self.phase(&f.children, containing, phase);
                    }
                }
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
                        self.content(f, emit, |w, padding| w.phase(&f.children, padding, phase));
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
                    w.phase(&f.children, padding, phase);
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
        for &(node, rect) in &self.chain[shared..] {
            self.steps.push(Step::PushClip { node, rect });
        }
        let opened = self.chain.len() - shared;
        let (dx, dy) = (self.dx, self.dy);
        (self.dx, self.dy) = (p.dx, p.dy);
        if p.f.style.z_index.is_some() {
            self.stacking(p.f, p.containing, &[]);
        } else {
            self.unit(p.f, p.containing);
        }
        (self.dx, self.dy) = (dx, dy);
        if self.steps.len() == mark + opened {
            // Nothing of it shows: its clips are left out with it.
            self.steps.truncate(mark);
        } else {
            for _ in 0..opened {
                self.steps.push(Step::PopClip);
            }
        }
        for &(node, rect) in &current[shared..] {
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
                Step::Scrollbar { offset, .. } => format!("scrollbar {offset}"),
            })
            .collect()
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
            names(&layout, &steps),
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
    fn rectangles_are_in_viewport_coordinates_and_offsets_apply_inside_the_clip() {
        // The page scrolled by 20, the 100px scroll container (node 2)
        // scrolled by 30: its own box is at document 0, viewport -20;
        // its child (node 3, 200px tall) at viewport -20 - 30. The clip
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
        assert_eq!(rect(2).y, -20.0);
        assert_eq!(rect(3).y, -50.0);
        assert!(matches!(
            steps[2],
            Step::PushClip {
                node,
                rect: Rect {
                    y: -20.0,
                    h: 100.0,
                    ..
                }
            } if node.map(|node| layout.number(node)) == Some(2)
        ));
        // The scrollbar is painted over the content, in the last pass.
        assert_eq!(
            names(&layout, &steps)[2..],
            [
                "clip 2 0 -20 800 100",
                "box 3",
                "pop",
                "clip 2 0 -20 800 100",
                "scrollbar 30",
                "pop"
            ]
            .map(String::from)
        );
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
            names(&layout, &steps),
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
            names(&layout, &steps),
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
            names(&layout, &steps),
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
            names(&layout, &steps),
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
            names(&layout, &steps),
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
            names(&layout, &steps),
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
            names(&layout, &steps),
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
        // held at the viewport's top; scrolled by 290, the container's
        // bottom at 10 holds the box 10px up. Nodes: div 1, s 2.
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
        assert_eq!(rects(100.0), (0.0, 0.0), "held at the top, text with it");
        assert_eq!(
            rects(290.0),
            (-10.0, -10.0),
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
            names(&layout, &steps),
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
}
