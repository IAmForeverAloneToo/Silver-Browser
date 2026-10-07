//! Pagination, v1 (M20): a laid-out document cut into pages. See
//! `docs/design/media.md`, "Print: pagination, v1".
//!
//! The document is laid out once, as on the screen, at the page area's
//! width; [`paginate`] then decides where the pages break and moves what
//! comes after each break down to the next page, rather than laying the
//! document out in its pages. The pages stand one under another in the
//! document's coordinates, each the height of its page area, so that
//! page `k` is the slice of the paginated document from its `top`.
//!
//! The content is read, in document order, as a sequence of pieces that
//! a page break goes between: the lines of a block container, a box
//! that is never broken inside (monolithic: one whose `overflow` clips,
//! a flex or grid container, a table, one with a transform), and the
//! empty space of a block — a block with no content, and what is below
//! a block's last line or child — which a page may end anywhere in, as
//! the reference browser lets it. The places between two pieces are
//! where a page may break: between two blocks, at the start of a block,
//! between two lines (Fragmentation 3's classes A, B and C). The pieces
//! are filled into pages; when one does not fit, the latest break that
//! breaks none of Fragmentation 3's rules is taken — an `avoid` at the
//! place, a `break-inside: avoid` around it, fewer lines before it than
//! `orphans` or after it than `widows` — a place's breaks inside the
//! boxes starting there, innermost first, coming before the break
//! before them; and if none, the rules are let go in the draft's order
//! (§4.4), the line counts first and the `avoid`s after. A piece too
//! tall for a page is put at the top of one and the pages cut it where
//! they end. A forced break, `page`, `left`, `right`, `recto`
//! or `verso`, begins a page, with a blank one before it when the side
//! is wrong.
//!
//! A break moves everything after it in document order down by its
//! strut, the distance from where that content was to the top of the
//! next page; a box that holds content on both sides of a break grows by
//! it. At an unforced break the margins that meet it are dropped: the
//! content after it starts at the top of the page with the border box of
//! the outermost box starting there; at a forced one the top margin of
//! that box is kept. A float or a line's content moves with its line or
//! its place in document order, an absolutely positioned box with its
//! containing block's top, and a fixed box, in the viewport's
//! coordinates, not at all: it is drawn on every page. What is beside a
//! break is not laid out again.
//!
//! Every walk here is iterative: a deep tree costs heap, not stack.

use style::computed::{BreakBetween, Display, Float, Position, TOP};
use style::page::PagePosition;

use crate::block::{DocumentLayout, Fragment, FragmentKind};

/// How close two lengths must be to count as the same, against the
/// rounding of the arithmetic that placed them.
const EPSILON: f32 = 0.01;

/// A paginated document.
#[derive(Debug, Clone)]
pub struct Paginated {
    /// The document's fragments, moved down past each break, in the
    /// same order and with the same coordinates system as a
    /// [`DocumentLayout`]'s.
    pub roots: Vec<Fragment>,
    /// The pages, in order: slices of the paginated document, each from
    /// its `top` and as tall as its page area.
    pub pages: Vec<PageSlice>,
}

/// One page of a [`Paginated`] document.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSlice {
    /// Where the page area starts in the paginated document.
    pub top: f32,
    /// The page area's height.
    pub height: f32,
    /// What the page selectors ask of the page.
    pub position: PagePosition,
}

/// `layout`, laid out at the page area's width, cut into pages whose
/// areas are `page_height` of each page's position tall. There is at
/// least one page, and the pages reach the bottom of the document's
/// flow. A height below one pixel is taken as one pixel. The layout's
/// fragments are moved, not copied.
pub fn paginate(layout: DocumentLayout, page_height: &dyn Fn(PagePosition) -> f32) -> Paginated {
    let plan = Plan::of(&layout.roots);
    let mut pages = Pages {
        pages: Vec::new(),
        page_height,
    };
    let breaks = plan.fill(&mut pages);
    let bottom = layout.height + Plan::total(&breaks);
    while pages.end() < bottom - EPSILON {
        pages.next(false);
    }
    let mut roots = layout.roots;
    plan.apply(&mut roots, &breaks);
    Paginated {
        roots,
        pages: pages.pages,
    }
}

/// The pages made so far.
struct Pages<'h> {
    pages: Vec<PageSlice>,
    page_height: &'h dyn Fn(PagePosition) -> f32,
}

impl Pages<'_> {
    /// Adds the next page, `blank` or not, and gives its top.
    fn next(&mut self, blank: bool) -> f32 {
        let index = self.pages.len();
        let position = PagePosition {
            first: index == 0,
            // The first page is a right page, the document being left to
            // right.
            left: index % 2 == 1,
            blank,
        };
        let top = self.end();
        let height = (self.page_height)(position);
        let height = if height.is_finite() {
            height.max(1.0)
        } else {
            1.0
        };
        self.pages.push(PageSlice {
            top,
            height,
            position,
        });
        top
    }

    /// The bottom of the last page's area.
    fn end(&self) -> f32 {
        self.pages.last().map_or(0.0, |page| page.top + page.height)
    }

    /// Whether the page after the last would be a left one.
    fn next_is_left(&self) -> bool {
        self.pages.len() % 2 == 1
    }
}

/// A run of content a page break goes between or around.
#[derive(Debug, Clone, Copy)]
struct Piece {
    top: f32,
    bottom: f32,
    /// Empty space, which a page may end anywhere in.
    fluid: bool,
}

/// A box that starts at a place: its border box's top, its top margin,
/// kept at a forced break, and whether it has a top border or padding,
/// which a break may come after, inside it (class C).
#[derive(Debug, Clone, Copy)]
struct Start {
    top: f32,
    margin: f32,
    gap: bool,
}

/// The place before a piece, where a page may break: before any of the
/// boxes that start there, or before the piece inside the innermost of
/// them, its level saying which.
#[derive(Debug, Clone, Default)]
struct Place {
    /// The boxes that start here, outermost first.
    boxes: Vec<Start>,
    /// The piece's top, and its top margin when it is a box.
    piece: f32,
    margin: f32,
    /// The level of the outermost box starting here whose
    /// `break-inside` avoids breaks: a break deeper is inside it.
    avoid_from: Option<usize>,
    /// A forced break here, and the side its next page must be on.
    forced: Option<Side>,
    /// The rules a break here breaks: [`AVOID`] and [`LINES`].
    flags: u8,
}

impl Place {
    /// Where the content after a break at `level` starts, and the
    /// margin kept above it at a forced break.
    fn start(&self, level: usize) -> (f32, f32) {
        match self.boxes.get(level) {
            Some(start) => (start.top, start.margin),
            None => (self.piece, self.margin),
        }
    }

    /// The levels a break may take here, the latest first: inside each
    /// box with a gap before what follows it, innermost first, and
    /// before the outermost box when `content_before` says something on
    /// the page is before it.
    fn levels(&self, content_before: bool) -> impl Iterator<Item = usize> + '_ {
        (1..=self.boxes.len())
            .rev()
            .filter(|&level| self.boxes[level - 1].gap)
            .chain(content_before.then_some(0))
    }

    /// The rules a break at `level` breaks.
    fn flags_at(&self, level: usize) -> u8 {
        let inside = self.avoid_from.is_some_and(|from| level > from);
        self.flags | if inside { AVOID } else { 0 }
    }
}

/// A break here is one an `avoid` asks against.
const AVOID: u8 = 1;
/// A break here leaves fewer lines than `orphans` or `widows` say.
const LINES: u8 = 2;

/// The side a forced break asks for its next page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Any,
    Left,
    Right,
}

/// What follows the boxes starting at a place: it moves with a break
/// there at any level.
const ALWAYS: usize = usize::MAX;

/// How a fragment moves: the walk that applies the struts reads these
/// in the order the walk that made them visited the fragments.
#[derive(Debug, Clone, Copy)]
enum Mark {
    /// A block whose content is pieces, starting at place `start` at
    /// `level` there: its top moves with a break there at its level or
    /// above, and its bottom with every break before `end`, the place
    /// after its last piece.
    Flow {
        start: usize,
        level: usize,
        end: usize,
    },
    /// Moves whole, with a break at place `at` at `level` or above.
    Rigid { at: usize, level: usize },
    /// Does not move: a fixed box in the viewport's coordinates.
    Fixed,
}

/// The pieces, places and marks of a document.
struct Plan {
    /// How many of the boxes being read avoid breaks inside them: none
    /// of their empty space is cut.
    avoiding: usize,
    pieces: Vec<Piece>,
    /// One more than the pieces: the place before each, and the end.
    places: Vec<Place>,
    marks: Vec<Mark>,
}

/// A block whose children are being read.
struct Frame<'f> {
    fragment: &'f Fragment,
    next_child: usize,
    /// Its mark's index, set when it is done.
    mark: usize,
    start: usize,
    level: usize,
    /// The lines of the run of inline content being read.
    run: Run,
    /// Whether its `break-inside` avoids breaks inside it.
    avoids: bool,
}

/// A run of inline content: its lines, by their pieces.
#[derive(Default)]
struct Run {
    lines: Vec<usize>,
}

/// Where the breaks fall: the strut and the level of the break at each
/// place, none being a strut of zero.
struct Breaks {
    struts: Vec<f32>,
    levels: Vec<usize>,
}

impl Plan {
    /// Reads `roots`, a document's fragments.
    fn of(roots: &[Fragment]) -> Plan {
        let mut plan = Plan {
            avoiding: 0,
            pieces: Vec::new(),
            places: vec![Place::default()],
            marks: Vec::new(),
        };
        // Places inside a box whose `break-inside` avoids breaks, as a
        // difference: one more at its first inner place, one less at
        // its end.
        let mut avoided: Vec<i32> = vec![0];
        let mut stack: Vec<Frame<'_>> = Vec::new();
        for root in roots {
            match root.style.position {
                // In the viewport's coordinates: on every page.
                Position::Fixed => plan.marks.push(Mark::Fixed),
                // Against the initial containing block, which does not
                // move.
                Position::Absolute => plan.marks.push(Mark::Rigid { at: 0, level: 0 }),
                _ => plan.enter(root, &mut stack, &mut avoided),
            }
            while let Some(frame) = stack.last_mut() {
                let fragment = frame.fragment;
                let Some(child) = fragment.children.get(frame.next_child) else {
                    let frame = stack.pop().expect("a frame is open");
                    plan.leave(frame, &mut avoided);
                    continue;
                };
                frame.next_child += 1;
                let (start, level) = (frame.start, frame.level);
                if child.style.position.is_out_of_flow() && matches!(child.kind, FragmentKind::Box)
                {
                    // Moves with its containing block's top, under whose
                    // fragment layout hangs it.
                    plan.marks.push(Mark::Rigid { at: start, level });
                } else if is_inline_level(child) {
                    let at = plan.line_of(child, &mut stack.last_mut().expect("open").run);
                    plan.marks.push(Mark::Rigid { at, level: ALWAYS });
                } else if child.style.float != Float::None {
                    // Moves with what follows it in document order: the
                    // line it is anchored to.
                    plan.marks.push(Mark::Rigid {
                        at: plan.pieces.len(),
                        level: ALWAYS,
                    });
                } else {
                    plan.end_run(stack.last_mut().expect("open"));
                    plan.enter(child, &mut stack, &mut avoided);
                }
            }
        }
        // Inside a box that avoids breaks.
        avoided.resize(plan.places.len() + 1, 0);
        let mut inside = 0;
        for (place, delta) in plan.places.iter_mut().zip(&avoided) {
            inside += delta;
            if inside > 0 {
                place.flags |= AVOID;
            }
        }
        plan
    }

    /// Takes in an in-flow block-level box: a piece if it is monolithic
    /// or empty, else a frame whose children are read next.
    fn enter<'f>(&mut self, f: &'f Fragment, stack: &mut Vec<Frame<'f>>, avoided: &mut Vec<i32>) {
        let at = self.pieces.len();
        let basis = stack.last().map_or(f.w, |parent| {
            parent.fragment.content_width(parent.fragment.w)
        });
        let margin = f.style.margin[TOP].resolve(basis, 0.0);
        let fragmentation = f.style.fragmentation();
        self.break_at(at, fragmentation.break_before);
        if is_monolithic(f) {
            self.marks.push(Mark::Rigid { at, level: ALWAYS });
            self.piece(f.y, f.y + f.h, false);
            self.places[at].margin = margin;
            self.break_at(self.pieces.len(), fragmentation.break_after);
            return;
        }
        let gap = f.style.used_border_width()[TOP] + f.style.padding[TOP].resolve(basis) > EPSILON;
        let place = &mut self.places[at];
        let level = place.boxes.len();
        place.boxes.push(Start {
            top: f.y,
            margin,
            gap,
        });
        let mark = self.marks.len();
        self.marks.push(Mark::Flow {
            start: at,
            level,
            end: at,
        });
        let avoids = fragmentation.break_inside.avoids_page();
        if avoids {
            add(avoided, at + 1, 1);
            self.avoiding += 1;
            place.avoid_from.get_or_insert(level);
        }
        stack.push(Frame {
            fragment: f,
            next_child: 0,
            mark,
            start: at,
            level,
            run: Run::default(),
            avoids,
        });
    }

    /// Finishes a block whose children have been read: its last run of
    /// lines, the empty space below its content, or the block itself as
    /// empty space when it has none.
    fn leave(&mut self, mut frame: Frame<'_>, avoided: &mut Vec<i32>) {
        self.end_run(&mut frame);
        let f = frame.fragment;
        let bottom = f.y + f.h;
        let fluid = self.avoiding == 0;
        if self.pieces.len() == frame.start {
            if f.h > EPSILON {
                self.piece(f.y, bottom, fluid);
            }
        } else {
            let last = self.pieces.last().expect("a piece").bottom;
            if bottom > last + EPSILON {
                self.piece(last, bottom, fluid);
            }
        }
        let end = self.pieces.len();
        self.marks[frame.mark] = Mark::Flow {
            start: frame.start,
            level: frame.level,
            end,
        };
        if frame.avoids {
            add(avoided, end.max(frame.start + 1), -1);
            self.avoiding -= 1;
        }
        self.break_at(end, f.style.fragmentation().break_after);
    }

    /// Adds a piece, with the place after it.
    fn piece(&mut self, top: f32, bottom: f32, fluid: bool) -> usize {
        let index = self.pieces.len();
        self.places[index].piece = top;
        self.pieces.push(Piece {
            top,
            bottom: bottom.max(top),
            fluid,
        });
        self.places.push(Place::default());
        index
    }

    /// Takes a `break-before` or `break-after` value at place `at`.
    fn break_at(&mut self, at: usize, value: BreakBetween) {
        let place = &mut self.places[at];
        let side = match value {
            BreakBetween::Page => Some(Side::Any),
            BreakBetween::Left | BreakBetween::Verso => Some(Side::Left),
            BreakBetween::Right | BreakBetween::Recto => Some(Side::Right),
            _ => None,
        };
        if let Some(side) = side {
            // A side asked for wins over none; of two sides, the later.
            if side != Side::Any || place.forced.is_none() {
                place.forced = Some(side);
            }
        } else if value.avoids_page() {
            place.flags |= AVOID;
        }
    }

    /// The line piece an inline-level `item` belongs to, a new one when
    /// it does not overlap the run's last line.
    fn line_of(&mut self, item: &Fragment, run: &mut Run) -> usize {
        let (top, bottom) = inline_extent(item);
        if let Some(&last) = run.lines.last() {
            let line = &mut self.pieces[last];
            if top < line.bottom - EPSILON {
                line.bottom = line.bottom.max(bottom);
                return last;
            }
            // The line box reaches down to the next.
            line.bottom = line.bottom.max(top);
        }
        let index = self.piece(top, bottom, false);
        run.lines.push(index);
        index
    }

    /// Ends the frame's run of lines: the places between them that
    /// leave fewer lines than `orphans` before or `widows` after.
    fn end_run(&mut self, frame: &mut Frame<'_>) {
        let lines = std::mem::take(&mut frame.run.lines);
        let fragmentation = frame.fragment.style.fragmentation();
        let (orphans, widows) = (
            fragmentation.orphans as usize,
            fragmentation.widows as usize,
        );
        let count = lines.len();
        for (before, &piece) in lines.iter().enumerate().skip(1) {
            if before < orphans || count - before < widows {
                self.places[piece].flags |= LINES;
            }
        }
    }

    /// Fills the pieces into `pages`, adding pages as they are needed,
    /// and gives where the breaks fall.
    fn fill(&self, pages: &mut Pages<'_>) -> Breaks {
        let mut breaks = Breaks {
            struts: vec![0.0; self.places.len()],
            levels: vec![0; self.places.len()],
        };
        pages.next(false);
        // What the struts so far add.
        let mut shift = 0.0;
        // The first piece that started on this page, and whether one
        // before it reaches into the page.
        let mut first = 0;
        let mut carried = false;
        let mut i = 0;
        while i < self.pieces.len() {
            let piece = self.pieces[i];
            let place = &self.places[i];
            let top = piece.top + shift;
            let bottom = piece.bottom + shift;
            let end = pages.end();
            let content_before = i > first || carried;
            // A forced break, before the outermost box starting here,
            // with content before it on the page.
            if let Some(side) = place.forced
                && content_before
            {
                let (start, margin) = place.start(0);
                let next = new_page(pages, side);
                breaks.struts[i] = next - (start - margin + shift);
                breaks.levels[i] = 0;
                shift += breaks.struts[i];
                first = i;
                carried = false;
                continue;
            }
            if bottom <= end + EPSILON {
                i += 1;
                continue;
            }
            if piece.fluid && top < end - EPSILON {
                // The page ends inside it.
                while pages.end() < bottom - EPSILON {
                    pages.next(false);
                }
                first = i + 1;
                carried = true;
                i += 1;
                continue;
            }
            // It does not fit: the last place on the page, at its latest
            // level, that breaks the fewest rules.
            let chosen = [0, LINES, AVOID | LINES].into_iter().find_map(|allowed| {
                (first..=i).rev().find_map(|k| {
                    let place = &self.places[k];
                    place
                        .levels(k > first || carried)
                        .find(|&level| place.flags_at(level) & !allowed == 0)
                        .map(|level| (k, level))
                })
            });
            match chosen {
                Some((k, level)) => {
                    // The margins at the break are dropped.
                    let (start, _) = self.places[k].start(level);
                    let next = pages.next(false);
                    breaks.struts[k] = next - (start + shift);
                    breaks.levels[k] = level;
                    shift += breaks.struts[k];
                    first = k;
                    carried = false;
                    i = k;
                }
                None if top >= end - EPSILON => {
                    // Past the page through space no break may go in:
                    // the next page takes it as it stands.
                    pages.next(false);
                    first = i;
                    carried = false;
                }
                None => {
                    // At the top of its page and too tall for it: the
                    // pages cut it.
                    while pages.end() < bottom - EPSILON {
                        pages.next(false);
                    }
                    first = i + 1;
                    carried = true;
                    i += 1;
                }
            }
        }
        breaks
    }

    /// Moves `roots` by `breaks`.
    fn apply(&self, roots: &mut [Fragment], breaks: &Breaks) {
        // `before[k]`: what the breaks at the places before `k` add.
        let mut before = Vec::with_capacity(breaks.struts.len() + 1);
        let mut sum = 0.0;
        before.push(0.0);
        for strut in &breaks.struts {
            sum += strut;
            before.push(sum);
        }
        let place = |k: usize| before.get(k).copied().unwrap_or(sum);
        // What a break at place `at` adds to what is at `level` there.
        let shift = |at: usize, level: usize| {
            let own = match breaks.levels.get(at) {
                Some(&broken) if level >= broken => breaks.struts[at],
                _ => 0.0,
            };
            place(at) + own
        };
        let mut marks = self.marks.iter();
        // Each fragment and its mark in the order they were made: a
        // flow box's children are read after it.
        let mut stack: Vec<&mut Fragment> = roots.iter_mut().rev().collect();
        while let Some(f) = stack.pop() {
            match marks.next().copied().unwrap_or(Mark::Fixed) {
                Mark::Fixed => {}
                Mark::Rigid { at, level } => translate(f, shift(at, level)),
                Mark::Flow { start, level, end } => {
                    let top = shift(start, level);
                    let bottom = if end > start { place(end) } else { top };
                    f.y += top;
                    f.h = (f.h + bottom - top).max(0.0);
                    stack.extend(f.children.iter_mut().rev());
                }
            }
        }
    }

    /// What all the breaks add, the struts' sum.
    fn total(breaks: &Breaks) -> f32 {
        breaks.struts.iter().sum()
    }
}

/// Adds `delta` at `index` of a difference array.
fn add(diff: &mut Vec<i32>, index: usize, delta: i32) {
    if diff.len() <= index {
        diff.resize(index + 1, 0);
    }
    diff[index] += delta;
}

/// Adds the page a forced break to `side` begins, with a blank page
/// before it when the next would be on the other side; gives its top.
fn new_page(pages: &mut Pages<'_>, side: Side) -> f32 {
    let wrong = match side {
        Side::Any => false,
        Side::Left => !pages.next_is_left(),
        Side::Right => pages.next_is_left(),
    };
    if wrong {
        pages.next(true);
    }
    pages.next(false)
}

/// Moves `f` and its whole subtree down by `dy`.
fn translate(f: &mut Fragment, dy: f32) {
    if dy == 0.0 {
        return;
    }
    let mut stack = vec![f];
    while let Some(f) = stack.pop() {
        f.y += dy;
        stack.extend(f.children.iter_mut());
    }
}

/// Whether `f` takes part in its block's lines: a text, an inline box,
/// or an atomic inline.
fn is_inline_level(f: &Fragment) -> bool {
    match f.kind {
        FragmentKind::Text(_) | FragmentKind::InlineBox => true,
        FragmentKind::Box => f.style.display.is_atomic_inline(),
    }
}

/// Whether a page never breaks inside the block-level box `f`: one
/// whose `overflow` clips, a flex or grid container, a table, or a
/// transformed box (Fragmentation 3 names scroll containers; the others
/// are this milestone's, docs/design/media.md section 2).
fn is_monolithic(f: &Fragment) -> bool {
    f.clips()
        || f.style.display.is_flex()
        || f.style.display.is_grid()
        || matches!(f.style.display, Display::Table | Display::InlineTable)
        || f.style.effects().contains_all_descendants()
}

/// The extent of an inline-level fragment on its line: a text's line,
/// an atomic inline's border box, and an inline box's content's.
fn inline_extent(item: &Fragment) -> (f32, f32) {
    let own = |f: &Fragment| match f.kind {
        FragmentKind::Text(_) => Some((f.top(), f.top() + f.h)),
        FragmentKind::Box => Some((f.y, f.y + f.h)),
        FragmentKind::InlineBox => None,
    };
    if let Some(extent) = own(item) {
        return extent;
    }
    let mut extent: Option<(f32, f32)> = None;
    let mut stack: Vec<&Fragment> = item.children.iter().collect();
    while let Some(f) = stack.pop() {
        match own(f) {
            Some((top, bottom)) => {
                extent = Some(extent.map_or((top, bottom), |(t, b)| (t.min(top), b.max(bottom))));
            }
            None => stack.extend(f.children.iter()),
        }
    }
    extent.unwrap_or((item.y, item.y + item.h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::boxes_of;
    use text::FontCollection;

    const WIDTH: f32 = 400.0;

    fn fonts() -> FontCollection {
        FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
    }

    /// `html` under `css`, laid out at [`WIDTH`] and paginated into pages
    /// `height` tall, or `first` for the first.
    fn paged(html: &str, css: &str, first: f32, height: f32) -> Paginated {
        let node = ::html::parse_document(html).expect("never fails");
        let sheet = ::css::parse_stylesheet(css).expect("valid");
        let (_, boxes) = boxes_of(&node, &[sheet]);
        let layout = crate::layout_document_in(&boxes, WIDTH, Some(height), &fonts());
        paginate(layout, &|position: PagePosition| {
            if position.first { first } else { height }
        })
    }

    /// The fragments of `paginated` in tree order.
    fn all(paginated: &Paginated) -> Vec<&Fragment> {
        crate::preorder(&paginated.roots).collect()
    }

    fn texts(paginated: &Paginated) -> Vec<&Fragment> {
        all(paginated)
            .into_iter()
            .filter(|f| matches!(f.kind, FragmentKind::Text(_)))
            .collect()
    }

    fn by_class<'p>(paginated: &'p Paginated, class: &str) -> Vec<&'p Fragment> {
        // The tests give each box a distinct background colour as its
        // name: red, green, blue, ... by `class`'s index in NAMES.
        let index = NAMES.iter().position(|n| *n == class).expect("a name");
        let colour = COLOURS[index];
        all(paginated)
            .into_iter()
            .filter(|f| matches!(f.kind, FragmentKind::Box) && f.style.background_color == colour)
            .collect()
    }

    const NAMES: [&str; 4] = ["a", "b", "c", "d"];
    const COLOURS: [style::computed::Color; 4] = [
        style::computed::Color::rgb8(255, 0, 0),
        style::computed::Color::rgb8(0, 128, 0),
        style::computed::Color::rgb8(0, 0, 255),
        style::computed::Color::rgb8(255, 255, 0),
    ];
    const NAMED: &str = ".a { background: red } .b { background: green } \
                         .c { background: blue } .d { background: yellow } ";

    /// The page `y` is on.
    fn page_of(paginated: &Paginated, y: f32) -> usize {
        paginated
            .pages
            .iter()
            .position(|p| y >= p.top - EPSILON && y < p.top + p.height - EPSILON)
            .unwrap_or_else(|| panic!("{y} is on no page"))
    }

    #[test]
    fn empty_space_is_sliced_where_the_page_ends() {
        // The reference browser's own: seven empty blocks 300px tall with
        // 20px margins on pages 1056px tall are cut where the pages end,
        // nothing moved.
        let html = "<div></div>".repeat(7);
        let p = paged(
            &html,
            "body { margin: 0 } div { height: 300px; margin: 20px 0; background: red }",
            1056.0,
            1056.0,
        );
        let tops: Vec<f32> = by_class(&p, "a").iter().map(|f| f.y).collect();
        assert_eq!(tops, [20.0, 340.0, 660.0, 980.0, 1300.0, 1620.0, 1940.0]);
        assert_eq!(p.pages.len(), 3);
        assert_eq!(
            p.pages.iter().map(|p| p.top).collect::<Vec<_>>(),
            [0.0, 1056.0, 2112.0]
        );
        assert!(p.pages[0].position.first && !p.pages[0].position.left);
        assert!(p.pages[1].position.left && !p.pages[2].position.left);
    }

    /// The distance between the lines of the bundled font at 16px.
    fn pitch() -> f32 {
        let p = paged(
            "<p>a</p><p>b</p>",
            "body { margin: 0 } p { margin: 0 }",
            1e4,
            1e4,
        );
        let texts = texts(&p);
        texts[1].top() - texts[0].top()
    }

    #[test]
    fn lines_go_whole_to_the_next_page() {
        let pitch = pitch();
        let words = "word ".repeat(400);
        let height = pitch * 5.5;
        let p = paged(
            &format!("<p>{words}</p>"),
            "body { margin: 0 } p { margin: 0 }",
            height,
            height,
        );
        let texts = texts(&p);
        assert!(texts.len() > 40);
        let mut lines_per_page = vec![0usize; p.pages.len()];
        let mut last_top = f32::NEG_INFINITY;
        for t in &texts {
            let page = page_of(&p, t.top());
            assert!(
                t.top() + t.h <= p.pages[page].top + p.pages[page].height + EPSILON,
                "a line is cut at {}",
                t.top()
            );
            if t.top() != last_top {
                lines_per_page[page] += 1;
                last_top = t.top();
            }
        }
        // Five lines on a page five and a half lines tall, each page
        // starting with a line at its top.
        assert!(
            lines_per_page[..lines_per_page.len() - 1]
                .iter()
                .all(|&n| n == 5)
        );
        for page in &p.pages[1..] {
            assert!(texts.iter().any(|t| (t.top() - page.top).abs() < EPSILON));
        }
        // The paragraph grew by the struts: from the top of the first
        // page to the bottom of its last line.
        let para = all(&p)
            .into_iter()
            .find(|f| {
                f.style.display == Display::Block
                    && f.children
                        .iter()
                        .any(|c| matches!(c.kind, FragmentKind::Text(_)))
            })
            .expect("the paragraph");
        let last = texts.last().unwrap();
        assert!((para.y + para.h - (last.top() + last.h)).abs() < EPSILON);
    }

    #[test]
    fn a_monolithic_box_that_does_not_fit_moves_and_one_too_tall_is_cut() {
        let p = paged(
            "<div class=a></div><div class=b></div><div class=c></div>",
            &format!(
                "{NAMED} body {{ margin: 0 }} div {{ height: 300px; overflow: hidden; margin: 10px 0 }}
                 .c {{ height: 900px }}"
            ),
            500.0,
            500.0,
        );
        let (a, b, c) = (
            by_class(&p, "a")[0],
            by_class(&p, "b")[0],
            by_class(&p, "c")[0],
        );
        assert_eq!(a.y, 10.0);
        // `b` would end at 630: it starts the next page, its top margin
        // dropped.
        assert_eq!(b.y, 500.0);
        // `c` is taller than a page: at the top of the next, and cut.
        assert_eq!(c.y, 1000.0);
        assert_eq!(c.h, 900.0);
        assert_eq!(p.pages.len(), 4);
    }

    #[test]
    fn forced_breaks_keep_the_margin_after_them_and_add_blank_pages_for_a_side() {
        let p = paged(
            "<div class=a></div><div class=b></div><div class=c></div><div class=d></div>",
            &format!(
                "{NAMED} body {{ margin: 0 }} div {{ height: 50px; margin: 30px 0 }}
                 .b {{ break-before: page }} .c {{ break-before: right }}
                 .d {{ page-break-before: left }}"
            ),
            500.0,
            500.0,
        );
        let (a, b, c, d) = (
            by_class(&p, "a")[0],
            by_class(&p, "b")[0],
            by_class(&p, "c")[0],
            by_class(&p, "d")[0],
        );
        assert_eq!(a.y, 30.0);
        assert_eq!(b.y, 530.0, "on the second page, its margin kept");
        // The third page would be a right page: `c` goes there.
        assert_eq!(c.y, 1030.0);
        assert!(!p.pages[2].position.left);
        // The fourth would be left: `d` too, no blank page.
        assert_eq!(d.y, 1530.0);
        assert!(p.pages.iter().all(|page| !page.position.blank));

        // A right page after the first, a right one: a blank left page
        // between them.
        let p = paged(
            "<div class=a></div><div class=b></div>",
            &format!(
                "{NAMED} body {{ margin: 0 }} div {{ height: 50px }} .b {{ break-before: recto }}"
            ),
            500.0,
            500.0,
        );
        assert_eq!(by_class(&p, "b")[0].y, 1000.0);
        assert!(p.pages[1].position.blank && p.pages[1].position.left);
        assert!(!p.pages[2].position.blank && !p.pages[2].position.left);

        // A forced break with nothing before it on the page does nothing.
        let p = paged(
            "<div class=a></div>",
            &format!("{NAMED} body {{ margin: 0 }} div {{ height: 50px; break-before: page }}"),
            500.0,
            500.0,
        );
        assert_eq!(by_class(&p, "a")[0].y, 0.0);
        assert_eq!(p.pages.len(), 1);
    }

    #[test]
    fn orphans_and_widows_move_the_break_and_give_way_when_they_must() {
        // A paragraph after a 40px block, the first page holding all its
        // lines but two.
        let pitch = pitch();
        let html = format!("<div class=a></div><p>{}</p>", "w ".repeat(30));
        let css = |rule: &str| {
            format!(
                "{NAMED} body {{ margin: 0 }} .a {{ height: 40px }}
                 p {{ margin: 0; width: 60px; {rule} }}"
            )
        };
        let lines_on_first = |p: &Paginated| {
            let mut tops: Vec<f32> = texts(p).iter().map(|t| t.top()).collect();
            tops.dedup();
            tops.iter().filter(|&&t| t < p.pages[0].height).count()
        };
        let count = {
            let p = paged(&html, &css(""), 10_000.0, 10_000.0);
            let mut tops: Vec<f32> = texts(&p).iter().map(|t| t.top()).collect();
            tops.dedup();
            tops.len()
        };
        assert!(count >= 8, "{count} lines");
        let height = 40.0 + pitch * (count as f32 - 2.0);
        // `widows: 2`, the initial: all but the last two lines fit.
        assert_eq!(
            lines_on_first(&paged(&html, &css(""), height, 1000.0)),
            count - 2
        );
        // `widows: 4`: two lines more go to the next page.
        assert_eq!(
            lines_on_first(&paged(&html, &css("widows: 4"), height, 1000.0)),
            count - 4
        );
        // `orphans` too great for any break in the paragraph: the whole
        // paragraph moves, the break before it allowed.
        assert_eq!(
            lines_on_first(&paged(&html, &css("orphans: 1000"), height, 1000.0)),
            0
        );
        // And with no break before it allowed either, the line counts
        // give way first.
        let p = paged(
            &html,
            &css("orphans: 1000; break-before: avoid"),
            height,
            1000.0,
        );
        assert_eq!(lines_on_first(&p), count - 2);
    }

    #[test]
    fn avoid_moves_the_break_before_the_box_or_the_pair() {
        let css = |rule: &str| {
            format!(
                "{NAMED} body {{ margin: 0 }} div {{ height: 100px }} .c {{ height: auto }}
                 .c div {{ height: 100px }} {rule}"
            )
        };
        let html =
            "<div class=a></div><div class=b></div><div class=c><div></div><div></div></div>";
        // With no rule, `c` breaks between its children.
        let p = paged(html, &css(""), 350.0, 350.0);
        assert_eq!(by_class(&p, "c")[0].y, 200.0);
        // `break-inside: avoid` moves it whole.
        let p = paged(html, &css(".c { break-inside: avoid }"), 350.0, 350.0);
        assert_eq!(by_class(&p, "c")[0].y, 350.0);
        // `break-before: avoid` on it moves `b` with it.
        let p = paged(
            html,
            &css(".c { break-inside: avoid; break-before: avoid }"),
            350.0,
            350.0,
        );
        assert_eq!(by_class(&p, "b")[0].y, 350.0);
        assert_eq!(by_class(&p, "c")[0].y, 450.0);
        // Nowhere to break but inside: the avoids give way.
        let p = paged(
            html,
            &css(".a, .b { break-after: avoid } .c { break-inside: avoid }"),
            350.0,
            350.0,
        );
        assert_eq!(by_class(&p, "c")[0].y, 200.0);
        // A box with a top padding that avoids breaks inside it moves
        // whole, not from after its padding, where a break would be
        // inside it.
        let p = paged(
            html,
            &css(".c { break-inside: avoid; padding-top: 10px }"),
            350.0,
            350.0,
        );
        assert_eq!(by_class(&p, "c")[0].y, 350.0);
        assert_eq!(by_class(&p, "c")[0].h, 210.0);
    }

    #[test]
    fn fixed_boxes_stay_and_positioned_ones_move_with_their_containing_block() {
        let p = paged(
            "<div class=a></div><div class=b><div class=c></div></div><div class=d></div>",
            &format!(
                "{NAMED} body {{ margin: 0 }} .a {{ height: 400px }}
                 .b {{ height: 200px; position: relative; overflow: visible }}
                 .c {{ position: absolute; top: 10px; height: 10px; width: 10px }}
                 .d {{ position: fixed; top: 5px; height: 10px; width: 10px }}"
            ),
            500.0,
            500.0,
        );
        // `b` does not fit after `a`: it is empty space, cut where the
        // page ends.
        let b = by_class(&p, "b")[0];
        assert_eq!(b.y, 400.0);
        assert_eq!(by_class(&p, "c")[0].y, 410.0);
        assert_eq!(by_class(&p, "d")[0].y, 5.0, "in the viewport's coordinates");

        // A box that moves takes its positioned child with it.
        let p = paged(
            "<div class=a></div><div class=b><div class=c></div></div>",
            &format!(
                "{NAMED} body {{ margin: 0 }} .a {{ height: 400px }}
                 .b {{ height: 200px; position: relative; overflow: hidden }}
                 .c {{ position: absolute; top: 10px; height: 10px; width: 10px }}"
            ),
            500.0,
            500.0,
        );
        assert_eq!(by_class(&p, "b")[0].y, 500.0);
        assert_eq!(by_class(&p, "c")[0].y, 510.0);
    }

    #[test]
    fn a_box_broken_across_pages_grows_by_its_struts() {
        let p = paged(
            "<div class=a><div class=b></div><div class=c></div></div>",
            &format!(
                "{NAMED} body {{ margin: 0 }} .a {{ padding: 10px; border: 2px solid }}
                 .b, .c {{ height: 200px; overflow: hidden }}"
            ),
            300.0,
            300.0,
        );
        let (a, c) = (by_class(&p, "a")[0], by_class(&p, "c")[0]);
        assert_eq!(c.y, 300.0);
        // 12px above `b`, its 200px, the 88px left on the page, `c`'s
        // 200px and 12px below.
        assert_eq!(a.y, 0.0);
        assert_eq!(a.h, 512.0);
    }

    /// A box of `h` at `y`, with `children`, naming no node.
    fn boxed(y: f32, h: f32, children: Vec<Fragment>) -> Fragment {
        Fragment {
            x: 0.0,
            y,
            w: WIDTH,
            h,
            style: style::ComputedStyle {
                display: Display::Block,
                ..Default::default()
            },
            kind: FragmentKind::Box,
            node: crate::testing::no_node(),
            generated: None,
            ascent: 0.0,
            scroll_height: 0.0,
            children,
            tree_order: 0,
            stacking_parent: None,
            parent: None,
            in_3d: false,
        }
    }

    #[test]
    fn a_tree_deeper_than_any_document_costs_no_stack() {
        // A chain of blocks far deeper than the DOM allows, each with a
        // pixel of top padding, with a monolithic box at its bottom that
        // does not fit the page: paginated on a small stack, the box
        // moved to the next page by a break in its parent's padding
        // (class C), and the tree taken apart without recursion.
        let depth = 100_000;
        let ran = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                let mut leaf = boxed(depth as f32, 50.0, Vec::new());
                leaf.style.overflow = style::computed::Overflow::Hidden;
                let mut chain = leaf;
                for level in (0..depth).rev() {
                    let y = level as f32;
                    chain = boxed(y, depth as f32 + 50.0 - y, vec![chain]);
                    // A pixel of padding above each child: a gap a break
                    // may come after.
                    chain.style.padding[TOP] = style::computed::LengthPercentage::Px(1.0);
                }
                let layout = DocumentLayout {
                    roots: vec![chain],
                    height: depth as f32 + 50.0,
                    depends_on_viewport_height: false,
                };
                let height = depth as f32 + 20.0;
                let mut p = paginate(layout, &|_| height);
                let pages = p.pages.len();
                let mut stack = std::mem::take(&mut p.roots);
                let mut deepest = 0.0f32;
                while let Some(mut f) = stack.pop() {
                    deepest = deepest.max(f.y);
                    stack.append(&mut f.children);
                }
                (pages, deepest)
            })
            .expect("a thread")
            .join()
            .expect("no overflow");
        assert_eq!(ran, (2, 100_020.0), "the leaf moved to the second page");
    }

    #[test]
    fn page_heights_follow_the_position() {
        let p = paged(
            &"<div class=a></div>".repeat(4),
            &format!("{NAMED} body {{ margin: 0 }} div {{ height: 100px; overflow: hidden }}"),
            150.0,
            250.0,
        );
        let tops: Vec<f32> = by_class(&p, "a").iter().map(|f| f.y).collect();
        assert_eq!(tops, [0.0, 150.0, 250.0, 400.0]);
        assert_eq!(p.pages[0].height, 150.0);
        assert_eq!(p.pages[1].height, 250.0);
    }
}
