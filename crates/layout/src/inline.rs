//! Inline layout: line boxes and greedy line breaking (roadmap M7).
//!
//! Replaces M6's "every text run is its own line" stand-in with real
//! text flow: words measured through `text::FontCollection` (a
//! per-script fallback font, M8.1 — not just the one primary font
//! every word used before it), packed onto lines greedily at
//! whitespace break opportunities (css-text-3 §3's
//! `white-space: normal` collapsing), baseline-aligned across mixed
//! font sizes on the same line (the line's shared baseline sits at the
//! tallest ascent present), and aligned per `text-align`. `white-space`
//! (M8.4) selects a different tokenization; `text-transform` (M8.4) runs
//! before any of it; `letter-spacing`/`word-spacing` (M8.4) adjust the
//! measured/rendered widths. Bidirectional text (M8.3) reorders each
//! resulting line into visual order — via `text::bidi_reorder`, keyed off
//! the computed `direction` — before word-splitting, so wrapping then
//! just packs words left to right same as any other text.
//!
//! Since M10.4 a line also holds atomic inline-level boxes — an
//! `inline-block`, laid out by `block::layout_inline_block` and placed
//! on the line as one item as wide as its margin box, its baseline on
//! the line's (M7.7) — and every line is at least as tall as the strut,
//! the containing block's own font metrics (css-inline-3 §4.1's root
//! inline box), so a line of small text or of inline-blocks alone is
//! laid out the height the container's font implies, as in browsers.
//!
//! White space (css-text-3 §4.1.1): collapsible white space between
//! two words, within a text node or across the boxes of a run, is one
//! gap of a space's width and a soft wrap opportunity; where there was
//! none, as in `a<b>b</b>`, the words are set flush and never broken
//! apart: when the second does not fit, both move to the next line
//! together, back to the last wrap opportunity on the line, as one
//! unbreakable piece (CSS2 §9.4.2). An atomic inline is a wrap
//! opportunity on both sides regardless. Collapsible white space at
//! the start or end of a line is discarded.
//!
//! Inline elements (CSS2 §10.8, M11.6): an inline element gets a
//! [`FragmentKind::InlineBox`] fragment per line it appears on, holding
//! that line's items inside it. Its start edge — left margin, border
//! and padding — takes room on the line it starts on and its end edge
//! on the line it ends on (`box-decoration-break: slice`, the only
//! value); the start edge takes the gap and wrap opportunity a word
//! there would and is glued to what follows it, the end edge is glued
//! to what precedes it. Collapsible white space at the end of an
//! inline element is rendered after its end edge, not inside it: a
//! stated approximation. An empty inline element with edges of some
//! width makes a line; one without makes none (CSS2 §9.4.2).
//!
//! Floats (CSS2 §9.5, M11): a line box beside floats runs between the
//! floats overlapping it — [`FloatContext::space`] for the line's top
//! and the strut's height, asked when the line starts, and asked again
//! when the line moves or a float joins it; `text-align` works within
//! the shortened line. A line that nothing fits in beside the floats
//! moves down to the next height at which a float ends and tries
//! again; with no float beside it, it overflows as it always has. A
//! float met in the run is placed at the current line's top when its
//! margin box fits in what is left of the line, which the line is then
//! shortened by, else below the line once its height is known, where
//! the next line avoids it; met before the run's first line while the
//! flow position is still unresolved, it waits for that line. The
//! line's height for the query is the strut's: a float the line grows
//! into through a taller item is not re-checked (the design note says
//! so; a browser relays the line).
//!
//! Scope, deliberately, matching the milestones' own "v1":
//!
//! * `overflow-wrap: break-word` (a single word wider than the
//!   available width) is **not** implemented — that word just overflows
//!   on its own line, unbroken. Mid-word breaking is a secondary
//!   feature next to greedy breaking itself; real text rarely needs it
//!   outside unbroken URLs/tokens at narrow viewports.
//! * `text-align: justify` isn't a value this engine parses yet
//!   (`style`'s M21 note) — only `left`/`right`/`center` reach here.
//! * Block-in-inline (M7.6, `<span>a<div>b</div>c</span>`) is the box
//!   tree builder's business since M10.4: the block is hoisted out and
//!   the span split around it (CSS2 §9.2.1.1) before layout, so the
//!   boxes that reach here are inline-level only. A block-level box a
//!   hand-built tree puts inside an inline is skipped.
//! * `white-space: pre`/`pre-wrap` preserve literal newlines as forced
//!   breaks (and `pre` disables auto-wrap entirely), a blank line among
//!   them included (it is a line box of the font's height, css-text-3
//!   §4.1.3), but *don't* preserve runs of plain spaces/tabs at their
//!   literal width — those still collapse to a single breakable gap,
//!   same as `normal`. Getting multi-space runs to render at their true
//!   width needs measuring whitespace itself, not just words; the
//!   common real reason to reach for `white-space: pre` — preserving
//!   *line breaks* in something like a code block — works correctly
//!   without that refinement.
//! * Collapsible white space is what css-text-3 §4.1 says it is: the
//!   space, tab, newline, carriage return and form feed. A no-break
//!   space is a character of the word it is in, measured and drawn
//!   with it, never a break opportunity.
//! * `line-height` and `vertical-align` other than the baseline are
//!   M8/M42 territory.

use std::borrow::Cow;

use dom::NodeId;
use style::computed::{
    BOTTOM, BorderStyle, ComputedStyle, Direction, LEFT, RIGHT, TOP, TextAlign, TextTransform,
    WhiteSpace,
};
use style::selector_matching::PseudoElement;
use text::{FontCollection, FontMetrics};

use crate::block::{
    AtomicLayout, Deferred, Edges, Fragment, FragmentKind, carried, layout_float,
    layout_inline_block, pending_float, relative_offset,
};
use crate::box_tree::{BoxKind, FirstLine, LayoutBox};
use crate::float::{At, FloatContext, PendingFloat};
use crate::pass::Pass;

/// One flattened piece of inline content, in document order.
pub(crate) enum InlineItem<'a> {
    Word {
        text: String,
        /// Already includes `letter-spacing`'s contribution (measured
        /// consistently with how [`emit_letter_spaced`] renders it —
        /// see that function's doc for why that's the same code path).
        width: f32,
        style: &'a ComputedStyle,
        /// `white-space: pre`: the gap before this word is never a
        /// line-break opportunity (text flows without wrapping).
        no_wrap: bool,
        /// Collapsible white space preceded this word (in its text, or
        /// in the boxes before it): a gap of a space's width, and a
        /// soft wrap opportunity unless `no_wrap`.
        gap_before: bool,
        /// The element the text belongs to (M9.4), carried into its
        /// fragment.
        node: NodeId,
        /// Whether the text is a generated box's (M16), also carried
        /// into its fragment.
        generated: Option<PseudoElement>,
    },
    /// A literal newline under `white-space: pre`/`pre-wrap`/`pre-line`
    /// — ends the current line unconditionally, however much of
    /// `available_width` is left. On an empty line it *is* the line: a
    /// blank line of the text's own font height.
    ForcedBreak(&'a ComputedStyle),
    /// An atomic inline-level box (M10.4: an inline-block), one item on
    /// the line, as wide as its margin box.
    Atomic {
        b: &'a LayoutBox,
        /// As for a word; a wrap opportunity exists before an atomic
        /// inline either way.
        gap_before: bool,
    },
    /// A float met in the run (M11): out of flow, placed by the float
    /// context beside the line it is met on, and invisible to the
    /// words around it — neither a gap nor a wrap opportunity.
    Float(&'a LayoutBox),
    /// An absolutely positioned box met in the run (M11): out of flow,
    /// invisible to the words around it; its static position is where
    /// an inline box would have gone, the current line's top and the
    /// current x on it (CSS2 §10.3.7's approximation), and it waits for
    /// its containing block.
    Absolute(&'a LayoutBox),
    /// The start of an inline element (M11.6): its left margin, border
    /// and padding, which take room on the line it starts on. It takes
    /// the gap and wrap opportunity a word there would have, and what
    /// follows it is glued to it. `style` is the element's where the
    /// items go: its own, or on the first line (M16).
    Open {
        b: &'a LayoutBox,
        style: &'a ComputedStyle,
        gap_before: bool,
    },
    /// The end of an inline element: its right padding, border and
    /// margin, on the line it ends on, glued to what precedes it; with
    /// the element's style where the items go.
    Close(&'a ComputedStyle),
}

/// One word placed on the line being built, before [`flush_line`]
/// positions it. It borrows the run's [`InlineItem`]s (`'i`), which
/// live for the layout of the run only.
struct LineWord<'i> {
    text: &'i str,
    width: f32,
    style: &'i ComputedStyle,
    node: NodeId,
    generated: Option<PseudoElement>,
}

/// One item on the line being built. The atomic's layout is boxed: it
/// carries a fragment subtree, many times a word's size — and the
/// absolutely positioned boxes it carries borrow the box tree (`'a`),
/// which outlives the run.
enum LineItem<'a, 'i> {
    Word(LineWord<'i>),
    Atomic(Box<AtomicLayout<'a>>),
    /// An inline element's start edge, as wide as its left margin,
    /// border and padding (M11.6), with the element's style on the line
    /// (M16: on the first line, its style there).
    Open(&'a LayoutBox, &'a ComputedStyle, f32),
    /// An inline element's end edge, as wide as its right padding,
    /// border and margin, with the element's style on the line.
    Close(&'a ComputedStyle, f32),
    /// An absolutely positioned box met here (M11): no width, its
    /// static position taken where the line puts it.
    Absolute(&'a LayoutBox),
}

impl LineItem<'_, '_> {
    fn width(&self) -> f32 {
        match self {
            LineItem::Word(w) => w.width,
            LineItem::Atomic(a) => a.width,
            LineItem::Open(_, _, edge) | LineItem::Close(_, edge) => *edge,
            LineItem::Absolute(_) => 0.0,
        }
    }

    /// The style whose font measures the gap before this item.
    fn style(&self) -> &ComputedStyle {
        match self {
            LineItem::Word(w) => w.style,
            LineItem::Atomic(a) => &a.root.style,
            LineItem::Open(_, style, _) | LineItem::Close(style, _) => style,
            LineItem::Absolute(b) => &b.style,
        }
    }

    /// Whether the item is content: a word or an atomic inline, which
    /// a gap can stand before and which makes a line box a line (CSS2
    /// §9.4.2). An inline box's edge is not, nor is an absolutely
    /// positioned box.
    fn is_content(&self) -> bool {
        matches!(self, LineItem::Word(_) | LineItem::Atomic(_))
    }
}

/// An item's place on the line as it is being built: whether a gap
/// precedes it, and whether the line may break before it; `index` is
/// its [`InlineItem`]'s in the run.
struct Placed<'a, 'i> {
    item: LineItem<'a, 'i>,
    index: usize,
    gap_before: bool,
    can_break: bool,
}

/// The lines of one inline formatting context.
pub(crate) struct InlineLayout<'a> {
    /// Relative to the run's container's content-box top-left `(0, 0)`.
    pub(crate) fragments: Vec<Fragment>,
    /// The height of the lines together.
    pub(crate) height: f32,
    /// The baseline of the last line, from the top of the run (CSS2
    /// §10.8.1: what an inline-block's own baseline is), `None` when no
    /// line was made.
    pub(crate) last_baseline: Option<f32>,
    /// The baseline of the first line, the same way (CSS2 §17.5.3: what
    /// a table cell's baseline is, M12).
    pub(crate) first_baseline: Option<f32>,
    /// The absolutely positioned boxes met in the run or carried by its
    /// inline-blocks and floats (M11), relative to the run's top-left.
    pub(crate) absolutes: Vec<Deferred<'a>>,
}

/// Lays a run of inline-level boxes (an anonymous block's children,
/// M10.4) out into line boxes within `cb_width`, the containing
/// block's content width, returning fragments relative to the run's
/// container's content-box top-left `(0, 0)` and the total height
/// consumed — `block::layout_children` translates both into the
/// container's own coordinate space, same as any block child's.
/// `container` is the run's own style (the anonymous block's): its
/// `text-align` aligns the lines, its font is the strut. `cb_height` is
/// the containing block's height when definite, for a percentage
/// height on an inline-block. `floats` and `at` are the float context
/// the run is in and where the run's top is in it (M11): the lines are
/// shortened beside the floats, and floats met in the run are placed.
///
/// `first_line` is the run's when it holds a first formatted line some
/// `::first-line` styles (M16, [`FirstLine`]): the first line is filled
/// with the items in their styles there, its strut is the innermost
/// `::first-line`'s font, and the items that move on to the next line
/// take their own styles again.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_inline<'a>(
    items: &'a [LayoutBox],
    cb_width: f32,
    cb_height: Option<f32>,
    container: &ComputedStyle,
    first_line: Option<&'a FirstLine>,
    floats: &mut FloatContext,
    at: At,
    pass: &Pass<'a>,
) -> InlineLayout<'a> {
    let flat = collect_inline_items(items, pass.fonts);
    let strut = pass.fonts.metrics_for_text(" ", container.font_size_px);
    // The items as on the first line, as far as it can reach; the run's
    // own when nothing in it is restyled there.
    let first_items = first_line
        .filter(|_| restyled_on_first_line(items))
        .map(|_| collect_first_line_items(items, pass.fonts, Some(cb_width)));
    let first = first_line
        .map(|line| (line, first_items.as_deref().unwrap_or(&flat)))
        .filter(|(_, items)| is_prefix(items, &flat))
        .map(|(line, items)| FirstLineRun {
            items,
            chain: &line.chain,
            strut_after: strut,
        });
    let mut lines = Lines {
        cb_width,
        cb_height,
        cb_left: at.x,
        top: at.position(),
        resolved: at.origin.is_some(),
        floats,
        line_left: 0.0,
        available_width: cb_width,
        text_align: container.text_align,
        strut: match (&first, first_line) {
            (Some(_), Some(line)) => pass.fonts.metrics_for_text(" ", line.style.font_size_px),
            _ => strut,
        },
        pass,
        items: &flat,
        first,
        line: Vec::new(),
        line_width: 0.0,
        has_content: false,
        open: Vec::new(),
        y: 0.0,
        fragments: Vec::new(),
        last_baseline: None,
        first_baseline: None,
        deferred: Vec::new(),
        absolutes: Vec::new(),
    };
    // The item before was an atomic inline: a wrap opportunity follows
    // it whether or not white space does.
    let mut after_atomic = false;

    for index in 0..flat.len() {
        match lines.item(index) {
            InlineItem::Word {
                text,
                width,
                style,
                no_wrap,
                gap_before,
                node,
                generated,
            } => {
                let word = LineWord {
                    text: text.as_str(),
                    width: *width,
                    style,
                    node: *node,
                    generated: *generated,
                };
                let can_break = !no_wrap && (*gap_before || after_atomic);
                lines.place(LineItem::Word(word), index, *gap_before, can_break);
                after_atomic = false;
            }
            InlineItem::Atomic { b, gap_before } => {
                let atomic = layout_inline_block(b, cb_width, cb_height, pass);
                lines.place(LineItem::Atomic(Box::new(atomic)), index, *gap_before, true);
                after_atomic = true;
            }
            InlineItem::Float(b) => lines.float(b),
            InlineItem::Absolute(b) => lines.place(LineItem::Absolute(b), index, false, false),
            InlineItem::Open {
                b,
                style,
                gap_before,
            } => {
                let (start, _) = edge_widths(style, cb_width);
                let can_break =
                    style.white_space != WhiteSpace::Pre && (*gap_before || after_atomic);
                lines.place(
                    LineItem::Open(b, style, start),
                    index,
                    *gap_before,
                    can_break,
                );
                after_atomic = false;
            }
            InlineItem::Close(style) => {
                let (_, end) = edge_widths(style, cb_width);
                lines.place(LineItem::Close(style, end), index, false, false);
            }
            InlineItem::ForcedBreak(style) => {
                // `white-space: pre`/`pre-wrap`/`pre-line` (M8.4): a
                // literal newline ends the line unconditionally. A
                // break on a line with nothing on it is a blank line,
                // as tall as the strut and the text it is in
                // (css-text-3 §4.1.3) — a code block's blank lines are
                // not nothing.
                if !lines.flush() {
                    lines.start_line();
                    let m = join(
                        lines.strut,
                        pass.fonts.metrics_for_text(" ", style.font_size_px),
                    );
                    lines.last_baseline = Some(lines.y + m.ascent);
                    lines.first_baseline = lines.first_baseline.or(lines.last_baseline);
                    lines.y += m.line_height();
                    lines.end_first_line();
                }
                after_atomic = false;
            }
        }
    }
    lines.flush();

    InlineLayout {
        fragments: lines.fragments,
        height: lines.y,
        last_baseline: lines.last_baseline,
        first_baseline: lines.first_baseline,
        absolutes: lines.absolutes,
    }
}

/// Whether `first`, a run's items as on the first line, are its `items`
/// one for one as far as they go, as the collector makes them: else
/// (which a debug build asserts never happens) the first line is laid
/// out in the run's own styles rather than across mismatched items.
fn is_prefix(first: &[InlineItem<'_>], items: &[InlineItem<'_>]) -> bool {
    debug_assert!(
        first.len() <= items.len(),
        "the first line's items are the run's"
    );
    first.len() <= items.len()
}

/// A run's first formatted line, which `::first-line` styles (M16).
struct FirstLineRun<'a, 'i> {
    /// The run's items as they are on the first line, one for one, as
    /// far as the line can reach ([`Reach`]).
    items: &'i [InlineItem<'a>],
    /// The `::first-line`s around the line's content, outermost first.
    chain: &'a [LayoutBox],
    /// The strut after the first line: the run's own.
    strut_after: FontMetrics,
}

/// The lines of one run as they are built: the line in progress and
/// the fragments of the lines before it.
struct Lines<'a, 'i, 'f, 'c> {
    /// The containing block's content width, and height when definite.
    cb_width: f32,
    cb_height: Option<f32>,
    /// The containing block's content box's left edge, and the run's
    /// top, in the float context's coordinates (M11).
    cb_left: f32,
    top: f32,
    /// Whether the flow position is known: content before the run, or
    /// the run's first line, resolved it. Until then a float met on an
    /// empty line waits.
    resolved: bool,
    floats: &'c mut FloatContext,
    /// Where the line in progress starts, from the content box's left
    /// edge, and how wide it may be: the room beside the floats.
    line_left: f32,
    available_width: f32,
    text_align: TextAlign,
    /// The containing block's own font metrics, the least any line is:
    /// on the first line, the innermost `::first-line`'s (M16).
    strut: FontMetrics,
    /// The pass the run is laid out in (M17), and its fonts.
    pass: &'f Pass<'a>,
    /// The run's items, and while the first line is being filled the
    /// `::first-line` styling it, if any.
    items: &'i [InlineItem<'a>],
    first: Option<FirstLineRun<'a, 'i>>,
    line: Vec<Placed<'a, 'i>>,
    line_width: f32,
    /// Whether the line in progress has content on it — a word or an
    /// atomic inline — which a gap can follow.
    has_content: bool,
    /// The inline elements open when the line in progress started,
    /// outermost first (M11.6): continued from the line before, sliced.
    open: Vec<&'a LayoutBox>,
    /// The top of the line in progress.
    y: f32,
    fragments: Vec<Fragment>,
    last_baseline: Option<f32>,
    first_baseline: Option<f32>,
    /// Floats met on the line in progress that did not fit beside it,
    /// placed below it once its height is known.
    deferred: Vec<PendingFloat>,
    /// The absolutely positioned boxes met so far (M11), relative to
    /// the run's top-left.
    absolutes: Vec<Deferred<'a>>,
}

impl<'a, 'i> Lines<'a, 'i, '_, '_> {
    /// The run's `index`th item as the line in progress takes it: as on
    /// the first line while that is being filled (M16). The line ends
    /// before its items do, [`Reach`] has it; else (which a debug build
    /// asserts never happens) the rest of it is in the run's own styles.
    fn item(&self, index: usize) -> &'i InlineItem<'a> {
        match self.first.as_ref().and_then(|first| first.items.get(index)) {
            Some(item) => item,
            None => {
                debug_assert!(
                    self.first.is_none(),
                    "the first line reached past its items"
                );
                &self.items[index]
            }
        }
    }

    /// `item`, the run's `index`th, made while the first line was being
    /// filled and placed again now that it is not: made again from the
    /// item as the next line takes it, in its own style. An atomic
    /// inline, which keeps its style, and an absolutely positioned box
    /// are as they were.
    fn settled(&self, item: LineItem<'a, 'i>, index: usize, was_first: bool) -> LineItem<'a, 'i> {
        if !was_first || self.first.is_some() {
            return item;
        }
        match (self.item(index), item) {
            (
                InlineItem::Word {
                    text,
                    width,
                    style,
                    node,
                    generated,
                    ..
                },
                LineItem::Word(_),
            ) => LineItem::Word(LineWord {
                text,
                width: *width,
                style,
                node: *node,
                generated: *generated,
            }),
            (InlineItem::Open { b, style, .. }, LineItem::Open(..)) => {
                LineItem::Open(b, style, edge_widths(style, self.cb_width).0)
            }
            (InlineItem::Close(style), LineItem::Close(..)) => {
                LineItem::Close(style, edge_widths(style, self.cb_width).1)
            }
            (_, item) => item,
        }
    }

    /// A line was made: if it was the first, the lines after it are in
    /// the run's own styles and over its own strut.
    fn end_first_line(&mut self) {
        if let Some(first) = self.first.take() {
            self.strut = first.strut_after;
        }
    }

    /// The line in progress's top in the float context's coordinates.
    fn line_top(&self) -> f32 {
        self.top + self.y
    }

    /// Begins a line at `y`: the flow position is resolved, so a float
    /// waiting for it is placed first, and the line gets the room
    /// beside the floats at its top for the strut's height.
    fn start_line(&mut self) {
        if !self.resolved {
            self.floats.resolve_pending(self.line_top());
            self.resolved = true;
        }
        self.measure_room();
    }

    /// Takes the room beside the floats for the line in progress.
    fn measure_room(&mut self) {
        let (left, right) = self.floats.space(
            self.line_top(),
            self.strut.line_height(),
            self.cb_left,
            self.cb_left + self.cb_width,
        );
        self.line_left = left - self.cb_left;
        self.available_width = (right - left).max(0.0);
    }

    /// Puts `item`, the run's `index`th, on the line in progress, or on
    /// a new one when it does not fit and a break before it is allowed.
    fn place(&mut self, item: LineItem<'a, 'i>, index: usize, gap_before: bool, can_break: bool) {
        if self.line.is_empty() {
            self.start_line();
            // Nothing fits beside the floats: move the line down to
            // where a float ends and try again, until the item fits
            // or no float is beside it, where it overflows as a line
            // too narrow for a word always has (CSS2 §9.5).
            while item.width() > self.available_width && self.available_width < self.cb_width {
                match self.floats.next_bottom(self.line_top()) {
                    Some(bottom) if bottom > self.line_top() => {
                        self.y = bottom - self.top;
                        self.measure_room();
                    }
                    _ => break,
                }
            }
        }
        // A gap stands between two pieces of content: none before the
        // first on the line, and an inline box's edge is not content.
        let gap = if !self.has_content || !gap_before {
            0.0
        } else {
            space_width(item.style(), self.pass.fonts)
        };
        // Greedy line breaking at a soft-wrap opportunity (M7.3): if
        // this item doesn't fit, start a new line — unless the line is
        // still empty (a solo word wider than `available_width` just
        // overflows; see the module doc's overflow-wrap note).
        if !self.line.is_empty() && self.line_width + gap + item.width() > self.available_width {
            let was_first = self.first.is_some();
            if can_break {
                self.flush();
                let item = self.settled(item, index, was_first);
                self.place(item, index, gap_before, can_break);
                return;
            }
            // No break is allowed before it (glued to the word before,
            // after an inline box's start edge, or `white-space: pre`,
            // M8.4): the item is part of the unbreakable piece that
            // ends the line, which moves to the next line whole, back
            // to the last wrap opportunity on this one (CSS2 §9.4.2) —
            // when there is one past the line's start; a piece that is
            // the whole line overflows.
            if let Some(at) = self
                .line
                .iter()
                .rposition(|p| p.can_break)
                .filter(|at| *at > 0)
            {
                let tail: Vec<Placed<'a, 'i>> = self.line.drain(at..).collect();
                (self.line_width, self.has_content) = measure_line(&self.line, self.pass.fonts);
                self.flush();
                for p in tail {
                    let tail_item = self.settled(p.item, p.index, was_first);
                    self.place(tail_item, p.index, p.gap_before, p.can_break);
                }
                let item = self.settled(item, index, was_first);
                self.place(item, index, gap_before, can_break);
                return;
            }
        }
        self.has_content |= item.is_content();
        self.line_width += gap + item.width();
        self.line.push(Placed {
            item,
            index,
            gap_before,
            can_break,
        });
    }

    /// A float met in the run (M11): placed at the line in progress's
    /// top when it fits in what is left of the line — the line is then
    /// shortened by it, its content moved beside it, unless the float
    /// context put it lower — or below the line once that ends; on an
    /// empty line, at the line's top, or held while the flow position
    /// is unresolved. Its fragment goes in its place, positioned by the
    /// root's `attach_floats`.
    fn float(&mut self, b: &'a LayoutBox) {
        let atomic = layout_float(b, self.cb_width, self.cb_height, self.pass);
        let float = pending_float(b, &atomic, self.cb_left, self.cb_width);
        self.fragments.push(atomic.root);
        if self.line.is_empty() {
            if self.resolved {
                self.floats.place(float, self.line_top());
            } else {
                self.floats.defer(float);
            }
        } else if self.line_width + float.w <= self.available_width {
            // Beside the line, which is measured again: no narrower
            // than the content on it, by the check above; or lower,
            // past other floats, where the room is as it was.
            self.floats.place(float, self.line_top());
            self.measure_room();
        } else {
            self.deferred.push(float);
        }
    }

    /// Places the floats that waited for the line just ended, at its
    /// bottom.
    fn place_deferred(&mut self) {
        let top = self.line_top();
        for float in std::mem::take(&mut self.deferred) {
            self.floats.place(float, top);
        }
    }

    /// Ends the line in progress, if there is one: whether a line was
    /// made of it.
    fn flush(&mut self) -> bool {
        let first_line = self.first.as_ref().map(|first| first.chain);
        let baseline = flush_line(
            &mut self.line,
            self.line_width,
            self.line_left,
            self.available_width,
            self.text_align,
            &self.strut,
            self.pass.fonts,
            (self.cb_width, self.cb_height),
            &mut self.y,
            &mut self.fragments,
            &mut self.absolutes,
            &mut self.open,
            first_line,
        );
        self.last_baseline = baseline.or(self.last_baseline);
        self.first_baseline = self.first_baseline.or(baseline);
        self.line_width = 0.0;
        self.has_content = false;
        if baseline.is_some() {
            self.end_first_line();
        }
        self.place_deferred();
        baseline.is_some()
    }
}

/// The width of a line's items with the gaps between them, and whether
/// any is content: what [`Lines::place`] keeps as it goes, measured
/// again for a line cut short.
fn measure_line(line: &[Placed<'_, '_>], fonts: &FontCollection) -> (f32, bool) {
    let mut width = 0.0;
    let mut has_content = false;
    for p in line {
        if has_content && p.gap_before {
            width += space_width(p.item.style(), fonts);
        }
        width += p.item.width();
        has_content |= p.item.is_content();
    }
    (width, has_content)
}

/// The room an inline element's start edge takes on the line it starts
/// on — its left margin, border and padding — and its end edge on the
/// line it ends on (CSS2 §10.8; css-break-4's `box-decoration-break:
/// slice`, the only value). An `auto` margin is zero on an inline.
fn edge_widths(style: &ComputedStyle, cb_width: f32) -> (f32, f32) {
    let edges = Edges::of(style, cb_width);
    let margin = |side: usize| style.margin[side].resolve(cb_width, 0.0);
    (
        margin(LEFT) + edges.border[LEFT] + edges.padding[LEFT],
        edges.padding[RIGHT] + edges.border[RIGHT] + margin(RIGHT),
    )
}

/// The larger of two sets of metrics, side by side.
fn join(a: FontMetrics, b: FontMetrics) -> FontMetrics {
    FontMetrics {
        ascent: a.ascent.max(b.ascent),
        descent: a.descent.max(b.descent),
        line_gap: a.line_gap.max(b.line_gap),
    }
}

/// Flattens an inline-level subtree into [`InlineItem`]s, applying
/// `text-transform` and `white-space`-driven tokenization (M8.4),
/// measuring each word (with `letter-spacing`, M8.4), and marking the
/// gaps where collapsible white space was (css-text-3 §4.1.1), as it
/// goes. The intrinsic-width pass ([`run_intrinsic_widths`]) and layout
/// consume the same items, so they agree on what a word is.
pub(crate) fn collect_inline_items<'a>(
    boxes: &'a [LayoutBox],
    fonts: &FontCollection,
) -> Vec<InlineItem<'a>> {
    collect(boxes, fonts, None)
}

/// [`collect_inline_items`] in the boxes' styles on the first formatted
/// line (M16, [`crate::box_tree::FirstLine`]): the same items, one for
/// one, as what may differ there — fonts, colour, `text-transform`,
/// spacing — changes no word's bounds, measured and drawn in those
/// styles. Only as many as the first line can take, though: a prefix
/// of the run's items, which ends at the first forced break and, with
/// `room` (the containing block's width, [`Reach`]), at the first
/// break opportunity past it.
pub(crate) fn collect_first_line_items<'a>(
    boxes: &'a [LayoutBox],
    fonts: &FontCollection,
    room: Option<f32>,
) -> Vec<InlineItem<'a>> {
    collect(boxes, fonts, Some(Reach { room, taken: 0.0 }))
}

/// Whether a box in the run `boxes` has a style of its own on the first
/// formatted line (M16): where none has, the run's items on the first
/// line are its own. The boxes in an atomic inline are not the run's.
pub(crate) fn restyled_on_first_line(boxes: &[LayoutBox]) -> bool {
    boxes.iter().any(|b| match b.kind {
        BoxKind::Text(_) => b.first_line.is_some(),
        BoxKind::Inline => b.first_line.is_some() || restyled_on_first_line(&b.children),
        _ => false,
    })
}

fn collect<'a>(
    boxes: &'a [LayoutBox],
    fonts: &FontCollection,
    first_line: Option<Reach>,
) -> Vec<InlineItem<'a>> {
    let mut collector = Collector {
        fonts,
        first_line,
        done: false,
        out: Vec::new(),
        space_pending: false,
        gap_placed: false,
        at_word_start: true,
    };
    collector.collect(boxes);
    collector.out
}

/// How far a first formatted line can reach into its run (M16.5): not
/// past a forced break, which ends it, nor past a break opportunity
/// once the items before it are wider together than `room`, the
/// containing block's width. [`layout_inline`] has all those items on
/// the line in progress, but for those a line of no width took, and a
/// line wider than the room for it breaks at the next opportunity:
/// there the first line is made, at the latest.
///
/// That holds while nothing takes room back: no negative width, gap or
/// edge, which would let more on the line, and no atomic inline, whose
/// width only its layout finds. Past one, the room no longer bounds the
/// line, and the items go on to the first forced break; as they do for
/// the intrinsic widths, whose lines are as wide as their content.
#[derive(Clone, Copy)]
struct Reach {
    room: Option<f32>,
    /// The width of the items so far, the gaps between them aside.
    taken: f32,
}

impl Reach {
    /// Counts an item `width` wide, set after a gap in `style` if any,
    /// with a break opportunity before it if `breaks`: whether the first
    /// line has surely ended there, before it. An item that takes room
    /// back turns the bound off first, as the line may then hold it.
    fn take(&mut self, width: f32, style: &ComputedStyle, breaks: bool) -> bool {
        if !(width >= 0.0 && style.word_spacing >= 0.0) {
            self.room = None;
        }
        let ended = breaks && self.room.is_some_and(|room| self.taken > room.max(0.0));
        self.taken += width;
        ended
    }
}

struct Collector<'a, 'f> {
    fonts: &'f FontCollection,
    /// Whether the boxes are styled as on the first formatted line, and
    /// then how far it reaches.
    first_line: Option<Reach>,
    /// The first line has ended: no more items are collected for it.
    done: bool,
    out: Vec<InlineItem<'a>>,
    /// Collapsible white space has been seen since the last item: the
    /// next item gets a gap before it.
    space_pending: bool,
    /// An inline element's start edge took a gap and no content has
    /// followed it: white space seen now collapses into that gap
    /// (css-text-3 §4.1.1, across the boundary of the inline).
    gap_placed: bool,
    /// The next text starts a word: nothing came before it in the run,
    /// or white space or an atomic inline did. A word split across
    /// boxes — by an inline element, or a `::first-letter` (M16) — is
    /// one word to `text-transform: capitalize`.
    at_word_start: bool,
}

impl<'a> Collector<'a, '_> {
    /// `b`'s style where the items go: on the first line, or its own.
    fn style_of(&self, b: &'a LayoutBox) -> &'a ComputedStyle {
        match &b.first_line {
            Some(line) if self.first_line.is_some() => &line.style,
            _ => &b.style,
        }
    }

    /// Adds `item`. On the first line (M16), it counts towards the
    /// line's [`Reach`], and is the last item when the line has ended
    /// before it, or ends at it.
    fn push(&mut self, item: InlineItem<'a>) {
        if let Some(reach) = &mut self.first_line {
            self.done |= match &item {
                InlineItem::Word {
                    width,
                    style,
                    no_wrap,
                    gap_before,
                    ..
                } => reach.take(*width, style, !no_wrap && *gap_before),
                InlineItem::Open {
                    style, gap_before, ..
                } => reach.room.is_some_and(|room| {
                    let breaks = style.white_space != WhiteSpace::Pre && *gap_before;
                    reach.take(edge_widths(style, room).0, style, breaks)
                }),
                InlineItem::Close(style) => reach
                    .room
                    .is_some_and(|room| reach.take(edge_widths(style, room).1, style, false)),
                InlineItem::Atomic { .. } => {
                    reach.room = None;
                    false
                }
                InlineItem::ForcedBreak(_) => true,
                InlineItem::Float(_) | InlineItem::Absolute(_) => false,
            };
        }
        self.out.push(item);
    }

    fn collect(&mut self, boxes: &'a [LayoutBox]) {
        for b in boxes {
            if self.done {
                return;
            }
            match &b.kind {
                BoxKind::Text(text) => self.text(b, text),
                BoxKind::Inline => {
                    let gap_before = std::mem::take(&mut self.space_pending);
                    self.gap_placed |= gap_before;
                    let style = self.style_of(b);
                    self.push(InlineItem::Open {
                        b,
                        style,
                        gap_before,
                    });
                    // Collects nothing when the line ended at the start
                    // edge, and the end edge is past the end as well.
                    self.collect(&b.children);
                    if self.done {
                        return;
                    }
                    self.push(InlineItem::Close(style));
                }
                // An inline-block, or a table or a flex container in a
                // run, which is an inline table (M12) or an inline flex
                // container (M17): atomic on the line.
                BoxKind::InlineBlock | BoxKind::Table | BoxKind::Flex => {
                    let gap_before = std::mem::take(&mut self.space_pending);
                    self.gap_placed = false;
                    self.at_word_start = true;
                    self.push(InlineItem::Atomic { b, gap_before });
                }
                // A float or an absolutely positioned box is invisible
                // to the white space around it: a gap pending before it
                // is still pending after it.
                BoxKind::Float => self.push(InlineItem::Float(b)),
                BoxKind::Absolute => self.push(InlineItem::Absolute(b)),
                // The box tree builder hoists a block out of inline
                // content (CSS2 §9.2.1.1) and keeps a table's parts in
                // tables, so none reaches here from a parsed document;
                // see the module doc.
                BoxKind::Block
                | BoxKind::AnonymousBlock
                | BoxKind::RowGroup(_)
                | BoxKind::Row
                | BoxKind::ColumnGroup { .. }
                | BoxKind::Column { .. }
                | BoxKind::Cell { .. }
                | BoxKind::Caption => {}
            }
        }
    }

    fn text(&mut self, b: &'a LayoutBox, text: &str) {
        let style = self.style_of(b);
        let transformed = apply_text_transform(text, style.text_transform, self.at_word_start);
        if let Some(last) = text.chars().next_back() {
            self.at_word_start = last.is_whitespace();
        }
        let no_wrap = style.white_space == WhiteSpace::Pre;
        // `pre`/`pre-wrap`/`pre-line` all keep literal newlines as
        // forced breaks; only `normal` collapses them away like any
        // other whitespace (see the module doc for what's *not*
        // preserved: multi-space run width).
        let preserves_newlines = style.white_space != WhiteSpace::Normal;
        // Always split on literal newlines, even under `normal` (where
        // they're not a forced break — just whitespace like any other,
        // a gap). This is also what keeps `\n` from ever reaching
        // `bidi_reorder`: `unicode-bidi` treats it as its own paragraph
        // separator and `bidi_reorder` only returns the first paragraph
        // (see its doc's "level 1" scope cut), so a line containing an
        // embedded `\n` would silently lose everything after it.
        let base_direction = to_text_direction(style.direction);
        for (i, line) in transformed.split('\n').enumerate() {
            if self.done {
                return;
            }
            if i > 0 {
                if preserves_newlines {
                    self.push(InlineItem::ForcedBreak(style));
                    self.space_pending = false;
                    self.gap_placed = false;
                    if self.done {
                        return;
                    }
                } else {
                    self.space_pending = !self.gap_placed;
                }
            }
            if line.is_empty() {
                continue;
            }
            // Bidi reordering (M8.3), one paragraph per literal line.
            // `direction` is always a definite Ltr/Rtl by the time it's
            // computed (there's no `dir="auto"` yet — see `cascade`'s
            // note), so the base direction passed here is always
            // explicit, matching real UAs: an RTL run shapes
            // right-to-left via UAX#9 embedding regardless, but the
            // *paragraph's* own direction only follows content when
            // nothing set it, which this engine doesn't yet do.
            let reordered = text::bidi_reorder(line, Some(base_direction));
            // A word runs from a non-space to the next space; it takes
            // the gap pending when it starts, and every space seen,
            // before, between or after, leaves one pending.
            let mut word: Option<(usize, bool)> = None;
            for (i, c) in reordered.char_indices() {
                if is_css_space(c) {
                    if let Some((start, gap_before)) = word.take() {
                        self.word(b, style, &reordered[start..i], no_wrap, gap_before);
                        if self.done {
                            return;
                        }
                    }
                    self.space_pending = !self.gap_placed;
                } else if word.is_none() {
                    word = Some((i, std::mem::take(&mut self.space_pending)));
                }
            }
            if let Some((start, gap_before)) = word {
                self.word(b, style, &reordered[start..], no_wrap, gap_before);
            }
        }
    }

    fn word(
        &mut self,
        b: &'a LayoutBox,
        style: &'a ComputedStyle,
        text: &str,
        no_wrap: bool,
        gap_before: bool,
    ) {
        self.gap_placed = false;
        self.push(InlineItem::Word {
            text: text.to_string(),
            width: measure_word(text, style, self.fonts),
            style,
            no_wrap,
            gap_before,
            node: b.node,
            generated: b.generated,
        });
    }
}

/// The min-content and max-content widths of one inline formatting
/// context (CSS2 §10.3.5's preferred minimum width and preferred
/// width; M10.4): the widest unbreakable piece — a word, the words a
/// preserved space or no space joins, an atomic inline's min-content
/// contribution — and the widest line when nothing but a forced break
/// breaks one. The same rules [`layout_inline`] breaks by. A float in
/// the run (M11) is a piece of its own for the min-content width, and
/// widens the line it is met on by its max-content contribution, as
/// it would sit beside the line's content.
///
/// With `first`, the items as they are on the first formatted line
/// (M16, one for one with `items` as far as its first forced break,
/// [`collect_first_line_items`] with no room), the first line is
/// measured in them —
/// the first that only a forced break ends for the widest line, the
/// first piece for the widest piece — and the rest in `items`.
pub(crate) fn run_intrinsic_widths(
    items: &[InlineItem<'_>],
    first: Option<&[InlineItem<'_>]>,
    fonts: &FontCollection,
) -> (f32, f32) {
    let first = first.filter(|first| is_prefix(first, items));
    let mut min = 0.0f32;
    let mut max = 0.0f32;
    // The line being measured, and the unbreakable piece at its end.
    let mut line = 0.0f32;
    let mut piece = 0.0f32;
    let mut line_empty = true;
    let mut after_atomic = false;
    // Whether the line, and the piece, are the first: measured in the
    // first-line items.
    let mut first_line = first.is_some();
    let mut first_piece = first.is_some();
    for (i, item) in items.iter().enumerate() {
        // The item as the line and as the piece measure it: the first
        // line's items go as far as its first forced break.
        let version = |on_first: bool| match first {
            Some(first) if on_first => {
                debug_assert!(i < first.len(), "the first line reached past its items");
                first.get(i).unwrap_or(item)
            }
            _ => item,
        };
        if matches!(item, InlineItem::ForcedBreak(_)) {
            first_line = false;
            first_piece = false;
        }
        // A piece after the first: what starts one is measured as itself.
        let starts_piece = match item {
            InlineItem::Word {
                no_wrap,
                gap_before,
                ..
            } => !no_wrap && !line_empty && (*gap_before || after_atomic),
            InlineItem::Open {
                style, gap_before, ..
            } => {
                style.white_space != WhiteSpace::Pre && !line_empty && (*gap_before || after_atomic)
            }
            InlineItem::Atomic { .. } => !line_empty,
            _ => false,
        };
        if starts_piece {
            first_piece = false;
        }
        let (in_line, in_piece) = (version(first_line), version(first_piece));
        match (in_line, in_piece) {
            (
                InlineItem::Word {
                    width,
                    style,
                    gap_before,
                    ..
                },
                InlineItem::Word {
                    width: piece_width,
                    style: piece_style,
                    ..
                },
            ) => {
                let gap = |style| {
                    if line_empty || !gap_before {
                        0.0
                    } else {
                        space_width(style, fonts)
                    }
                };
                piece = if starts_piece {
                    *piece_width
                } else {
                    piece + gap(piece_style) + piece_width
                };
                line += gap(style) + width;
                line_empty = false;
                after_atomic = false;
            }
            // An atomic inline keeps its style on the first line.
            (InlineItem::Atomic { b, gap_before }, _) => {
                let (piece_min, piece_max) = crate::intrinsic::contribution(b, fonts);
                let gap = if line_empty || !gap_before {
                    0.0
                } else {
                    space_width(&b.style, fonts)
                };
                piece = piece_min;
                line += gap + piece_max;
                line_empty = false;
                after_atomic = true;
            }
            (InlineItem::Float(b), _) => {
                let (float_min, float_max) = crate::intrinsic::contribution(b, fonts);
                min = min.max(float_min);
                line += float_max;
            }
            // An inline element's edges (M11.6) are measured against a
            // zero containing block, as a percentage padding or margin
            // contributes nothing to an intrinsic size (css-sizing-3
            // §5.2.1). The start edge is where a word would break and
            // glues what follows; the end edge glues to what precedes.
            (
                InlineItem::Open {
                    style, gap_before, ..
                },
                InlineItem::Open {
                    style: piece_style, ..
                },
            ) => {
                let gap = |style| {
                    if line_empty || !gap_before {
                        0.0
                    } else {
                        space_width(style, fonts)
                    }
                };
                let (edge, _) = edge_widths(piece_style, 0.0);
                piece = if starts_piece {
                    edge
                } else {
                    piece + gap(piece_style) + edge
                };
                let (edge, _) = edge_widths(style, 0.0);
                line += gap(style) + edge;
                after_atomic = false;
            }
            (InlineItem::Close(style), InlineItem::Close(piece_style)) => {
                piece += edge_widths(piece_style, 0.0).1;
                line += edge_widths(style, 0.0).1;
            }
            (InlineItem::ForcedBreak(_), _) => {
                line = 0.0;
                piece = 0.0;
                line_empty = true;
                after_atomic = false;
            }
            // An absolutely positioned box takes no room; and the two
            // versions of an item are of one kind.
            _ => {}
        }
        min = min.max(piece);
        max = max.max(line);
    }
    (min, max)
}

/// css-text-3 §4.1's collapsible white space: space, tab, line feed,
/// carriage return, form feed — not Unicode's `White_Space`, which
/// would make a no-break space breakable and collapsible.
pub(crate) fn is_css_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

/// `style::computed::Direction` → `text::Direction` (M8.3): `text` sits
/// below `style` in the layer stack (see `docs/architecture.md`) and so
/// can't name `style`'s type itself — see that enum's own doc in
/// `text::lib` for the other half of this note. This is the boundary
/// conversion.
fn to_text_direction(direction: Direction) -> text::Direction {
    match direction {
        Direction::Ltr => text::Direction::Ltr,
        Direction::Rtl => text::Direction::Rtl,
    }
}

/// `text-transform` (M8.4), applied before any whitespace/word
/// processing, to a text that starts a word when `at_word_start`.
/// Borrows (no allocation) for the common `None` case.
fn apply_text_transform(
    text: &str,
    transform: TextTransform,
    mut at_word_start: bool,
) -> Cow<'_, str> {
    match transform {
        TextTransform::None => Cow::Borrowed(text),
        TextTransform::Uppercase => Cow::Owned(text.to_uppercase()),
        TextTransform::Lowercase => Cow::Owned(text.to_lowercase()),
        TextTransform::Capitalize => {
            let mut out = String::with_capacity(text.len());
            for c in text.chars() {
                if c.is_whitespace() {
                    at_word_start = true;
                    out.push(c);
                } else if at_word_start {
                    out.extend(c.to_uppercase());
                    at_word_start = false;
                } else {
                    out.push(c);
                }
            }
            Cow::Owned(out)
        }
    }
}

/// A space's width plus `word-spacing` (M8.4) — the gap between two
/// words on the same line.
pub(crate) fn space_width(style: &ComputedStyle, fonts: &FontCollection) -> f32 {
    fonts.measure(" ", style.font_size_px) + style.word_spacing
}

/// `word`'s width, including `letter-spacing`'s contribution (M8.4).
/// When `letter_spacing` is non-zero, this measures character-by-
/// character rather than shaping the whole word at once — matching
/// [`emit_letter_spaced`]'s rendering exactly (shaping the word as one
/// unit would apply kerning/ligatures between letters that `letter-
/// spacing` is, by its nature, about to space apart anyway; measuring
/// per-character here keeps the width used for line-breaking decisions
/// consistent with what actually gets drawn, instead of drifting apart
/// by whatever a given font's kerning happens to be).
fn measure_word(word: &str, style: &ComputedStyle, fonts: &FontCollection) -> f32 {
    if style.letter_spacing == 0.0 {
        return fonts.measure(word, style.font_size_px);
    }
    let mut buf = [0u8; 4];
    let mut total = 0.0;
    let mut count = 0u32;
    for ch in word.chars() {
        total += fonts.measure(ch.encode_utf8(&mut buf), style.font_size_px);
        count += 1;
    }
    total + style.letter_spacing * count as f32
}

/// Positions one line's items (M7.4: baseline-aligned — the shared
/// baseline sits at the tallest ascent on the line, an inline-block's
/// ascent being the distance from its margin-box top to its own
/// baseline, M7.7; M7.1: the line runs from the tallest ascent above
/// the baseline to the deepest extent below it, a text's descent plus
/// its line gap or an inline-block's rest of its margin box, the strut
/// counted among them) and pushes their fragments, honoring
/// `text_align` (M7.5) for the line's starting x within the line box,
/// which starts `line_left` from the content box's left edge and is
/// `available_width` wide (shortened beside floats, M11). The items of
/// an inline element go into its [`FragmentKind::InlineBox`] fragment
/// for the line (M11.6); `open` holds the inline elements open when the
/// line starts, outermost first, and is left holding those open when
/// it ends. `cb` is the containing block's width and definite height,
/// for the inline elements' edges and offsets. Returns the line's
/// baseline, or `None` for a line that is no line at all: nothing on
/// it, or no content and no inline box edge of any width (CSS2
/// §9.4.2), which takes no room — an absolutely positioned box met on
/// it still gets its static position there. Clears `line` for the next
/// line.
///
/// `first_line` is the `::first-line`s of the run's first formatted line
/// when this is that line (M16): those with a background get an inline
/// box fragment around the line's content, outermost first, and the
/// inline elements continued from before the line take their styles on
/// it.
#[allow(clippy::too_many_arguments)]
fn flush_line<'a>(
    line: &mut Vec<Placed<'a, '_>>,
    line_width: f32,
    line_left: f32,
    available_width: f32,
    text_align: TextAlign,
    strut: &FontMetrics,
    fonts: &FontCollection,
    cb: (f32, Option<f32>),
    y: &mut f32,
    fragments: &mut Vec<Fragment>,
    absolutes: &mut Vec<Deferred<'a>>,
    open: &mut Vec<&'a LayoutBox>,
    first_line: Option<&'a [LayoutBox]>,
) -> Option<f32> {
    if line.is_empty() {
        return None;
    }
    let makes_line = line_width > 0.0 || line.iter().any(|p| p.item.is_content());

    // Above the baseline: the tallest ascent. Below it: the deepest
    // extent — a text's descent and its line gap under it (the gap is
    // all below the baseline in this engine's `line-height: normal`,
    // not split as half-leading), an inline-block's margin box past its
    // baseline — which is where an inline-block's own last line gap
    // already is, so it is not added again under it. An inline box's
    // edges add nothing: its vertical padding and border are drawn
    // without changing the line's height (CSS2 §10.8).
    let mut max_ascent = strut.ascent;
    let mut max_descent = strut.descent;
    let mut max_below = strut.descent + strut.line_gap;
    for placed in line.iter() {
        match &placed.item {
            LineItem::Word(word) => {
                // Per-word, not just per-size (M8.1): a word's own
                // script may resolve to a different fallback font than
                // another word on the same line, with its own vertical
                // metrics — folding in `metrics_for_text` (keyed off
                // this word's actual text, not just its `font_size_px`)
                // extends the existing "shared baseline sits at the
                // tallest metrics present" rule (M7.4) to cover that,
                // not just mixed sizes.
                let m = fonts.metrics_for_text(word.text, word.style.font_size_px);
                max_ascent = max_ascent.max(m.ascent);
                max_descent = max_descent.max(m.descent);
                max_below = max_below.max(m.descent + m.line_gap);
            }
            LineItem::Atomic(a) => {
                max_ascent = max_ascent.max(a.ascent);
                max_below = max_below.max(a.height - a.ascent);
            }
            LineItem::Open(..) | LineItem::Close(..) | LineItem::Absolute(_) => {}
        }
    }
    let baseline_y = *y + max_ascent;

    // css-text-3 §7.1: contents too long for the line box are
    // start-aligned, never pushed left past its edge.
    let slack = (available_width - line_width).max(0.0);
    let mut x = line_left
        + match text_align {
            TextAlign::Left => 0.0,
            TextAlign::Right => slack,
            TextAlign::Center => slack / 2.0,
        };
    if !makes_line {
        // No line: the inline elements' edges are kept track of, an
        // absolutely positioned box gets its static position at the
        // line's start, and nothing else comes of it.
        for placed in line.drain(..) {
            match placed.item {
                LineItem::Open(b, ..) => open.push(b),
                LineItem::Close(..) => {
                    open.pop();
                }
                LineItem::Absolute(b) => absolutes.push(Deferred { b, x, y: *y }),
                LineItem::Word(_) | LineItem::Atomic(_) => {}
            }
        }
        return None;
    }
    // The inline elements open on the line, outermost first, each with
    // the fragments of its content on the line so far; the ones open
    // when the line started continue from the line's start, sliced. On
    // the first line, inside the `::first-line`s that paint (M16), which
    // the line's content is all in.
    let chain = first_line
        .unwrap_or_default()
        .iter()
        .filter(|b| b.style.background_color.a > 0.0)
        .map(|b| OpenInline {
            b,
            style: &b.style,
            x,
            starts: true,
            children: Vec::new(),
            absolutes_from: absolutes.len(),
        });
    let continued = open.iter().map(|b| OpenInline {
        b,
        style: match &b.first_line {
            Some(line) if first_line.is_some() => &line.style,
            _ => &b.style,
        },
        x,
        starts: false,
        children: Vec::new(),
        absolutes_from: absolutes.len(),
    });
    let mut stack: Vec<OpenInline<'a>> = chain.collect();
    // Below this, the `::first-line`s: never closed by an element's end.
    let floor = stack.len();
    stack.extend(continued);
    let mut next_open = open.clone();
    for (i, placed) in line.drain(..).enumerate() {
        if i > 0 && placed.gap_before {
            x += space_width(placed.item.style(), fonts);
        }
        match placed.item {
            LineItem::Word(word) => {
                let into = innermost(&mut stack, fragments);
                if word.style.letter_spacing != 0.0 {
                    // Per-character fragments (M8.4) — see
                    // `measure_word`'s doc for why this has to shape
                    // character-by-character, same as the width this
                    // word was measured with.
                    emit_letter_spaced(
                        into,
                        &word,
                        &mut x,
                        baseline_y,
                        max_ascent,
                        max_descent,
                        fonts,
                    );
                    continue;
                }
                into.push(text_fragment(x, baseline_y, &word, max_ascent, max_descent));
                x += word.width;
            }
            LineItem::Atomic(a) => {
                // The margin box's top sits `ascent` above the baseline;
                // the border box is a margin inside it.
                let top = baseline_y - a.ascent;
                let width = a.width;
                // The border box is a margin inside the margin box, and
                // a relatively positioned box is moved from there (M11);
                // the absolutely positioned boxes it carries go with it.
                let (dx, dy) = a.relative;
                let (bx, by) = (x + a.margin_left + dx, top + a.margin_top + dy);
                let a = *a;
                innermost(&mut stack, fragments).push(a.root.translated(bx, by));
                absolutes.extend(carried(a.absolutes, bx, by));
                x += width;
            }
            LineItem::Open(b, style, edge) => {
                next_open.push(b);
                stack.push(OpenInline {
                    b,
                    style,
                    x,
                    starts: true,
                    children: Vec::new(),
                    absolutes_from: absolutes.len(),
                });
                x += edge;
            }
            LineItem::Close(_, edge) => {
                x += edge;
                next_open.pop();
                // The collector opens and closes in pairs; a close
                // with nothing of its own open is a hand-built tree's,
                // and is nothing — the `::first-line`s below stay open.
                if stack.len() == floor {
                    continue;
                }
                let inline = stack.pop().expect("an element open above the floor");
                let fragment = close_inline(inline, x, true, baseline_y, fonts, cb, absolutes);
                innermost(&mut stack, fragments).push(fragment);
            }
            LineItem::Absolute(b) => absolutes.push(Deferred { b, x, y: *y }),
        }
    }
    // The inline elements still open are sliced at the line's end: no
    // end edge on this line. (The `::first-line`s, which end there, have
    // none: neither margins nor borders apply to them.)
    while let Some(inline) = stack.pop() {
        let fragment = close_inline(inline, x, false, baseline_y, fonts, cb, absolutes);
        innermost(&mut stack, fragments).push(fragment);
    }
    *open = next_open;
    *y += max_ascent + max_below;
    Some(baseline_y)
}

/// An inline element open on the line being flushed, with the
/// fragments of its content on the line so far.
struct OpenInline<'a> {
    b: &'a LayoutBox,
    /// Its style on the line: on the first, its style there (M16).
    style: &'a ComputedStyle,
    /// Where it starts on the line: before its start edge when it
    /// starts here, else the line's start.
    x: f32,
    /// Whether the element starts on this line, with its start edge;
    /// else it continues from the line before, sliced.
    starts: bool,
    children: Vec<Fragment>,
    /// The absolutely positioned boxes met inside it on this line
    /// begin here in the run's list.
    absolutes_from: usize,
}

/// Where the next fragment of the line goes: into the innermost open
/// inline element's box, else the line's own list.
fn innermost<'s>(
    stack: &'s mut [OpenInline<'_>],
    fragments: &'s mut Vec<Fragment>,
) -> &'s mut Vec<Fragment> {
    match stack.last_mut() {
        Some(inline) => &mut inline.children,
        None => fragments,
    }
}

/// The [`FragmentKind::InlineBox`] fragment of `inline` on the line
/// being flushed, ending at `x`: after its end edge when `ends`, else
/// sliced at the line's end (CSS2 §10.8; css-break-4's
/// `box-decoration-break: slice`). Its content area runs from the
/// line's baseline up by its font's ascent and down by its descent
/// (CSS2 §10.6.1), its padding and border around that, the vertical
/// ones without changing the line's height; a side sliced off has no
/// border in the fragment's style, so none is painted there. Its
/// children are made relative to it, as [`layout_document`]'s final
/// pass expects, and a relatively positioned inline element is moved
/// as a whole (CSS2 §9.4.3), the absolutely positioned boxes met inside
/// it on this line with it.
///
/// [`layout_document`]: crate::layout_document
fn close_inline<'a>(
    inline: OpenInline<'a>,
    x: f32,
    ends: bool,
    baseline_y: f32,
    fonts: &FontCollection,
    (cb_width, cb_height): (f32, Option<f32>),
    absolutes: &mut [Deferred<'a>],
) -> Fragment {
    let (b, own) = (inline.b, inline.style);
    let edges = Edges::of(own, cb_width);
    let margin = |side: usize| own.margin[side].resolve(cb_width, 0.0);
    let m = fonts.metrics_for_text(" ", own.font_size_px);
    let left = if inline.starts {
        inline.x + margin(LEFT)
    } else {
        inline.x
    };
    let right = if ends { x - margin(RIGHT) } else { x };
    let top = baseline_y - m.ascent - edges.padding[TOP] - edges.border[TOP];
    let height = edges.border[TOP]
        + edges.padding[TOP]
        + m.ascent
        + m.descent
        + edges.padding[BOTTOM]
        + edges.border[BOTTOM];
    let mut style = own.clone();
    if !inline.starts {
        style.border_style[LEFT] = BorderStyle::None;
    }
    if !ends {
        style.border_style[RIGHT] = BorderStyle::None;
    }
    let (dx, dy) = relative_offset(own, cb_width, cb_height);
    let mut children = inline.children;
    for c in &mut children {
        c.x -= left;
        c.y -= top;
    }
    for d in &mut absolutes[inline.absolutes_from..] {
        d.x += dx;
        d.y += dy;
    }
    Fragment {
        x: left + dx,
        y: top + dy,
        w: (right - left).max(0.0),
        h: height,
        style,
        kind: FragmentKind::InlineBox,
        node: b.node,
        generated: b.generated,
        ascent: 0.0,
        scroll_height: 0.0,
        children,
    }
}
/// A text fragment for `word` at `x` on a line whose baseline is
/// `baseline_y` and whose text rises `ascent` above it and `descent`
/// below: the fragment's `y` is the baseline (`block`'s and `paint`'s
/// convention) and its box is the line's, see [`Fragment::ascent`].
fn text_fragment(
    x: f32,
    baseline_y: f32,
    word: &LineWord<'_>,
    ascent: f32,
    descent: f32,
) -> Fragment {
    Fragment {
        x,
        y: baseline_y,
        w: word.width,
        h: ascent + descent,
        style: word.style.clone(),
        kind: FragmentKind::Text(word.text.to_string()),
        node: word.node,
        generated: word.generated,
        ascent,
        scroll_height: 0.0,
        children: Vec::new(),
    }
}

/// Pushes one `Text` fragment per character of `word`, each advanced by
/// `letter-spacing` past the previous one (M8.4) — the common
/// `letter_spacing == 0.0` case never calls this, so it costs nothing
/// when the property isn't used.
fn emit_letter_spaced(
    fragments: &mut Vec<Fragment>,
    word: &LineWord<'_>,
    x: &mut f32,
    baseline_y: f32,
    ascent: f32,
    descent: f32,
    fonts: &FontCollection,
) {
    let mut buf = [0u8; 4];
    for ch in word.text.chars() {
        let ch_str = ch.encode_utf8(&mut buf);
        let w = fonts.measure(ch_str, word.style.font_size_px);
        let one = LineWord {
            text: ch_str,
            width: w,
            style: word.style,
            node: word.node,
            generated: word.generated,
        };
        fragments.push(text_fragment(*x, baseline_y, &one, ascent, descent));
        *x += w + word.style.letter_spacing;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundled test font, so that every machine measures the same.
    fn test_fonts() -> FontCollection {
        FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
    }

    /// [`layout_inline`] under a default-styled container (a 16px
    /// strut) aligned per `align`, as the pre-M10 tests called it.
    fn lay(
        items: &[LayoutBox],
        available_width: f32,
        align: TextAlign,
        fonts: &FontCollection,
    ) -> (Vec<Fragment>, f32) {
        let container = ComputedStyle {
            text_align: align,
            ..ComputedStyle::default()
        };
        let mut floats = FloatContext::default();
        let l = layout_inline(
            items,
            available_width,
            None,
            &container,
            None,
            &mut floats,
            At::root(),
            &Pass::new(fonts),
        );
        (l.fragments, l.height)
    }

    fn text_box(text: &str, font_size_px: f32) -> LayoutBox {
        let style = ComputedStyle {
            font_size_px,
            ..ComputedStyle::default()
        };
        LayoutBox {
            style,
            kind: BoxKind::Text(text.to_string()),
            children: Vec::new(),
            node: crate::testing::no_node(),
            anonymous: true,
            generated: None,
            quote_depth: 0,
            styled_by_ancestor: false,
            first_line: None,
        }
    }

    fn inline_box(children: Vec<LayoutBox>) -> LayoutBox {
        LayoutBox {
            style: ComputedStyle::default(),
            kind: BoxKind::Inline,
            children,
            node: crate::testing::no_node(),
            anonymous: false,
            generated: None,
            quote_depth: 0,
            styled_by_ancestor: false,
            first_line: None,
        }
    }

    fn texts_of(fragments: &[Fragment]) -> Vec<&str> {
        fragments
            .iter()
            .filter_map(|f| match &f.kind {
                FragmentKind::Text(t) => Some(t.as_str()),
                FragmentKind::Box | FragmentKind::InlineBox => None,
            })
            .collect()
    }

    /// The text fragments under `fragments` in tree order, each with
    /// the coordinates it has in the run: an inline element's text is
    /// inside its box fragment (M11.6), relative to it.
    fn texts(fragments: &[Fragment]) -> Vec<Fragment> {
        fn walk(fragments: &[Fragment], dx: f32, dy: f32, out: &mut Vec<Fragment>) {
            for f in fragments {
                match f.kind {
                    FragmentKind::Text(_) => out.push(f.clone().translated(dx, dy)),
                    FragmentKind::InlineBox => walk(&f.children, dx + f.x, dy + f.y, out),
                    FragmentKind::Box => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(fragments, 0.0, 0.0, &mut out);
        out
    }

    #[test]
    fn single_word_fits_on_one_line() {
        let fonts = test_fonts();
        let (fragments, height) = lay(&[text_box("hello", 16.0)], 1000.0, TextAlign::Left, &fonts);
        assert_eq!(fragments.len(), 1);
        assert_eq!(height, fonts.primary().metrics(16.0).line_height());
    }

    #[test]
    fn narrow_width_wraps_each_word_onto_its_own_line() {
        let fonts = test_fonts();
        let word_width = fonts.measure("aaaa", 16.0);
        assert!(word_width > 0.0);
        let items = [text_box("aaaa bbbb cccc", 16.0)];
        // Just over one word wide: a second word never fits alongside it.
        let (fragments, height) = lay(&items, word_width + 1.0, TextAlign::Left, &fonts);
        assert_eq!(texts_of(&fragments), vec!["aaaa", "bbbb", "cccc"]);
        let line_height = fonts.primary().metrics(16.0).line_height();
        assert_eq!(height, line_height * 3.0);
        assert_eq!(fragments[1].y - fragments[0].y, line_height);
        assert_eq!(fragments[2].y - fragments[1].y, line_height);
    }

    #[test]
    fn wide_enough_width_keeps_words_on_one_line() {
        let fonts = test_fonts();
        let items = [text_box("aaaa bbbb cccc", 16.0)];
        let (fragments, _height) = lay(&items, 10_000.0, TextAlign::Left, &fonts);
        assert_eq!(texts_of(&fragments), vec!["aaaa", "bbbb", "cccc"]);
        assert_eq!(fragments[0].y, fragments[1].y);
        assert_eq!(fragments[1].y, fragments[2].y);
    }

    #[test]
    fn mixed_font_sizes_on_one_line_share_a_baseline() {
        let fonts = test_fonts();
        let items = [text_box("small", 10.0), text_box("BIG", 40.0)];
        let (fragments, height) = lay(&items, 10_000.0, TextAlign::Left, &fonts);
        assert_eq!(fragments.len(), 2);
        assert_eq!(
            fragments[0].y, fragments[1].y,
            "mixed sizes on one line must share a baseline"
        );

        let small = fonts.primary().metrics(10.0);
        let big = fonts.primary().metrics(40.0);
        // M7.4: the shared baseline sits at the *taller* word's ascent.
        assert_eq!(fragments[0].y, big.ascent.max(small.ascent));
        // M7.1: line height is the max ascent + max descent + max line-gap.
        assert_eq!(
            height,
            big.ascent.max(small.ascent)
                + big.descent.max(small.descent)
                + big.line_gap.max(small.line_gap)
        );
    }

    #[test]
    fn a_line_is_at_least_as_tall_as_the_strut() {
        // Small text in a 16px container: the line is the container's
        // font's height (css-inline-3's root inline box), its baseline
        // at the container's ascent, not the small text's.
        let fonts = test_fonts();
        let (fragments, height) = lay(&[text_box("tiny", 8.0)], 10_000.0, TextAlign::Left, &fonts);
        let strut = fonts.primary().metrics(16.0);
        assert_eq!(height, strut.line_height());
        assert_eq!(fragments[0].y, strut.ascent);
        assert_eq!(fragments[0].h, strut.ascent + strut.descent);
    }

    #[test]
    fn text_align_left_right_and_center() {
        let fonts = test_fonts();
        let width = fonts.measure("hi", 16.0);
        assert!(width > 0.0);
        let available = 1000.0;

        let (left, _) = lay(&[text_box("hi", 16.0)], available, TextAlign::Left, &fonts);
        assert_eq!(left[0].x, 0.0);

        let (right, _) = lay(&[text_box("hi", 16.0)], available, TextAlign::Right, &fonts);
        assert_eq!(right[0].x, available - width);

        let (center, _) = lay(
            &[text_box("hi", 16.0)],
            available,
            TextAlign::Center,
            &fonts,
        );
        assert_eq!(center[0].x, (available - width) / 2.0);
    }

    #[test]
    fn nested_inlines_flow_onto_one_line() {
        let fonts = test_fonts();
        let run = inline_box(vec![
            text_box("before", 16.0),
            inline_box(vec![text_box("inner", 16.0)]),
            text_box("after", 16.0),
        ]);
        let (fragments, height) = lay(&[run], 10_000.0, TextAlign::Left, &fonts);
        let fragments = texts(&fragments);
        assert_eq!(texts_of(&fragments), vec!["before", "inner", "after"]);
        assert!(fragments.iter().all(|f| f.y == fragments[0].y));
        assert_eq!(height, fonts.primary().metrics(16.0).line_height());
    }

    #[test]
    fn words_without_white_space_between_them_are_set_flush_and_never_broken() {
        // `a<b>b</b>` is one word in two fragments: no gap between them,
        // and no wrap opportunity, however narrow the line; ` c` after
        // it gets its gap and may wrap.
        let fonts = test_fonts();
        let run = [
            text_box("a", 16.0),
            inline_box(vec![text_box("b", 16.0)]),
            text_box(" c", 16.0),
        ];
        let (fragments, _) = lay(&run, 10_000.0, TextAlign::Left, &fonts);
        let fragments = texts(&fragments);
        assert_eq!(texts_of(&fragments), vec!["a", "b", "c"]);
        assert_eq!(fragments[1].x, fragments[0].x + fragments[0].w, "flush");
        assert_eq!(
            fragments[2].x,
            fragments[1].x + fragments[1].w + fonts.measure(" ", 16.0),
            "one space"
        );
        // Narrow: "ab" stays together on the first line, "c" wraps.
        let (fragments, height) = lay(&run, 1.0, TextAlign::Left, &fonts);
        let fragments = texts(&fragments);
        assert_eq!(fragments[0].y, fragments[1].y);
        assert_eq!(
            fragments[2].y,
            fragments[1].y + fonts.primary().metrics(16.0).line_height()
        );
        assert_eq!(height, 2.0 * fonts.primary().metrics(16.0).line_height());
        // White space in a box of its own is a gap too.
        let run = [
            text_box("a", 16.0),
            text_box(" ", 16.0),
            inline_box(vec![text_box("b", 16.0)]),
        ];
        let (fragments, _) = lay(&run, 10_000.0, TextAlign::Left, &fonts);
        assert_eq!(fragments[1].x, fragments[0].w + fonts.measure(" ", 16.0));
    }

    #[test]
    fn empty_input_produces_no_fragments_and_zero_height() {
        let fonts = test_fonts();
        let (fragments, height) = lay(&[], 100.0, TextAlign::Left, &fonts);
        assert!(fragments.is_empty());
        assert_eq!(height, 0.0);
        // White space alone is no line either.
        let (fragments, height) = lay(&[text_box(" \n ", 16.0)], 100.0, TextAlign::Left, &fonts);
        assert!(fragments.is_empty());
        assert_eq!(height, 0.0);
    }

    fn styled_text_box(text: &str, style: ComputedStyle) -> LayoutBox {
        LayoutBox {
            style,
            kind: BoxKind::Text(text.to_string()),
            children: Vec::new(),
            node: crate::testing::no_node(),
            anonymous: true,
            generated: None,
            quote_depth: 0,
            styled_by_ancestor: false,
            first_line: None,
        }
    }

    #[test]
    fn text_transform_uppercase_and_capitalize() {
        let fonts = test_fonts();
        let style = |t| ComputedStyle {
            text_transform: t,
            ..ComputedStyle::default()
        };

        let (up, _) = lay(
            &[styled_text_box(
                "hello world",
                style(TextTransform::Uppercase),
            )],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(texts_of(&up), vec!["HELLO", "WORLD"]);

        let (cap, _) = lay(
            &[styled_text_box(
                "hello world",
                style(TextTransform::Capitalize),
            )],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(texts_of(&cap), vec!["Hello", "World"]);

        // A word split across boxes is one word: only its start is
        // capitalized, and white space or an atomic inline ends it.
        let (cap, _) = lay(
            &[
                styled_text_box("t", style(TextTransform::Capitalize)),
                styled_text_box("he end ", style(TextTransform::Capitalize)),
                styled_text_box("now", style(TextTransform::Capitalize)),
            ],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(texts_of(&cap), vec!["T", "he", "End", "Now"]);
        let atomic = LayoutBox {
            style: ComputedStyle {
                display: style::computed::Display::InlineBlock,
                ..ComputedStyle::default()
            },
            kind: BoxKind::InlineBlock,
            ..inline_box(Vec::new())
        };
        let (cap, _) = lay(
            &[
                styled_text_box("a", style(TextTransform::Capitalize)),
                atomic,
                styled_text_box("b", style(TextTransform::Capitalize)),
            ],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(texts_of(&cap), vec!["A", "B"]);
    }

    #[test]
    fn white_space_pre_line_preserves_newlines_as_forced_breaks() {
        let fonts = test_fonts();
        let style = ComputedStyle {
            white_space: WhiteSpace::PreLine,
            ..ComputedStyle::default()
        };
        let (fragments, _) = lay(
            &[styled_text_box("a b\nc d", style)],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(texts_of(&fragments), vec!["a", "b", "c", "d"]);
        // "a b" and "c d" land on different lines (the newline forces a
        // break) even though `available_width` is huge — unlike
        // `normal`, where this input would all fit on one line.
        let line_of = |t: &str| {
            fragments
                .iter()
                .find(|f| matches!(&f.kind, FragmentKind::Text(x) if x == t))
                .unwrap()
                .y
        };
        assert_eq!(line_of("a"), line_of("b"));
        assert_ne!(line_of("b"), line_of("c"));
        assert_eq!(line_of("c"), line_of("d"));
    }

    #[test]
    fn white_space_normal_does_not_break_on_newlines() {
        let fonts = test_fonts();
        // The default (`Normal`) — a literal newline is just ordinary
        // collapsible whitespace, not a forced break.
        let (fragments, _) = lay(&[text_box("a\nb", 16.0)], 10_000.0, TextAlign::Left, &fonts);
        assert_eq!(texts_of(&fragments), vec!["a", "b"]);
        assert_eq!(fragments[0].y, fragments[1].y);
        assert_eq!(
            fragments[1].x,
            fragments[0].w + fonts.measure(" ", 16.0),
            "a newline is a gap"
        );
    }

    #[test]
    fn white_space_pre_never_wraps_even_when_too_narrow() {
        let fonts = test_fonts();
        let word_width = fonts.measure("aaaa", 16.0);
        assert!(word_width > 0.0);
        let style = ComputedStyle {
            white_space: WhiteSpace::Pre,
            font_size_px: 16.0,
            ..ComputedStyle::default()
        };
        // Same narrow width that forces 3 separate lines under `normal`
        // (see `narrow_width_wraps_each_word_onto_its_own_line`) — under
        // `pre`, it must still be exactly one line.
        let (fragments, _) = lay(
            &[styled_text_box("aaaa bbbb cccc", style)],
            word_width + 1.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(texts_of(&fragments), vec!["aaaa", "bbbb", "cccc"]);
        assert_eq!(fragments[0].y, fragments[1].y);
        assert_eq!(fragments[1].y, fragments[2].y);
    }

    #[test]
    fn word_spacing_adds_extra_gap_between_words_not_within_them() {
        let fonts = test_fonts();
        let normal = ComputedStyle {
            font_size_px: 16.0,
            ..ComputedStyle::default()
        };
        let spaced = ComputedStyle {
            font_size_px: 16.0,
            word_spacing: 20.0,
            ..ComputedStyle::default()
        };

        let (base, _) = lay(
            &[styled_text_box("aa bb", normal.clone())],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        let (wide, _) = lay(
            &[styled_text_box("aa bb", spaced)],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );

        let gap = |frags: &[Fragment]| frags[1].x - (frags[0].x + frags[0].w);
        // The extra gap between words is exactly `word-spacing` more
        // than without it; each word's own *width* is unaffected.
        assert_eq!(gap(&wide) - gap(&base), 20.0);
        assert_eq!(base[0].w, wide[0].w);
    }

    #[test]
    fn letter_spacing_adds_gaps_between_characters_and_splits_fragments() {
        let fonts = test_fonts();
        let style = ComputedStyle {
            font_size_px: 16.0,
            letter_spacing: 5.0,
            ..ComputedStyle::default()
        };
        let (fragments, _) = lay(
            &[styled_text_box("ab", style)],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        // One fragment per character, not one for the whole word.
        assert_eq!(texts_of(&fragments), vec!["a", "b"]);
        let gap = fragments[1].x - (fragments[0].x + fragments[0].w);
        assert_eq!(gap, 5.0);
    }

    #[test]
    fn letter_spacing_zero_keeps_one_fragment_per_word() {
        let fonts = test_fonts();
        // The common case: no per-character splitting when the property
        // isn't used at all.
        let (fragments, _) = lay(
            &[text_box("hello", 16.0)],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(texts_of(&fragments), vec!["hello"]);
    }

    #[test]
    fn bidi_reorders_mixed_direction_text_before_layout() {
        let fonts = test_fonts();
        // M8.6 acceptance: "a mixed LTR/RTL paragraph with an embedded
        // number and punctuation orders correctly" — Hebrew with an
        // embedded English word, a number, and a comma, under an
        // explicit `direction: rtl` (as `<html dir="rtl">` would
        // compute down to every descendant via inheritance).
        let text = "שלום world, 123 עולם";
        let style = ComputedStyle {
            direction: Direction::Rtl,
            font_size_px: 16.0,
            ..ComputedStyle::default()
        };
        let (fragments, _) = lay(
            &[styled_text_box(text, style)],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        let words = texts_of(&fragments);

        // Derive the expected word sequence from the same `bidi_reorder`
        // the integration calls (M8.3), rather than hand-predicting a
        // bidi outcome — exactly the kind of thing that's fragile to
        // get right by hand, per this project's own testing notes
        // elsewhere. This still exercises real wiring, not a tautology:
        // it's the only way `direction: Rtl` on a `ComputedStyle` can
        // reach `bidi_reorder` at all, through `to_text_direction` and
        // `collect_inline_items` — and it confirms nothing downstream
        // (greedy breaking, `flush_line`'s placement) disturbs the
        // order `bidi_reorder` produced.
        let reordered = text::bidi_reorder(text, Some(text::Direction::Rtl));
        let expected: Vec<&str> = reordered.split_whitespace().collect();
        assert_eq!(words, expected);

        // The integration actually ran — this reordering must differ
        // from plain logical-order splitting, or this test wouldn't be
        // exercising bidi at all.
        assert_ne!(words, text.split_whitespace().collect::<Vec<_>>());

        // The embedded number's digits must stay in forward order
        // (UAX#9 weak-type resolution) wherever they land.
        let joined = words.join(" ");
        let pos = |c: char| joined.find(c);
        let (p1, p2, p3) = (pos('1'), pos('2'), pos('3'));
        assert!(
            p1.is_some() && p2.is_some() && p3.is_some(),
            "all digits present in {joined:?}"
        );
        assert!(
            p1 < p2 && p2 < p3,
            "digits must stay in forward order within {joined:?}"
        );
    }

    #[test]
    fn an_overflowing_line_is_start_aligned_whatever_text_align_says() {
        // css-text-3 §7.1: too long to fit means start-aligned, not
        // pushed to a negative x.
        let fonts = test_fonts();
        for align in [TextAlign::Right, TextAlign::Center] {
            let (frags, _) = lay(
                &[text_box("supercalifragilistic", 16.0)],
                20.0,
                align,
                &fonts,
            );
            assert_eq!(frags[0].x, 0.0, "{align:?}");
        }
    }

    #[test]
    fn blank_lines_in_preformatted_text_take_a_line_each() {
        let fonts = test_fonts();
        let line_height = fonts.primary().metrics(16.0).line_height();
        for ws in [WhiteSpace::Pre, WhiteSpace::PreWrap, WhiteSpace::PreLine] {
            let style = ComputedStyle {
                white_space: ws,
                font_size_px: 16.0,
                ..ComputedStyle::default()
            };
            let (frags, height) = lay(
                &[styled_text_box("a\n\nb", style)],
                10_000.0,
                TextAlign::Left,
                &fonts,
            );
            assert_eq!(texts_of(&frags), vec!["a", "b"]);
            assert_eq!(
                frags[1].y - frags[0].y,
                2.0 * line_height,
                "{ws:?}: one blank line between"
            );
            assert_eq!(height, 3.0 * line_height, "{ws:?}");
        }
        // A trailing newline adds no line: `<pre>a\n</pre>` is one line.
        let style = ComputedStyle {
            white_space: WhiteSpace::Pre,
            font_size_px: 16.0,
            ..ComputedStyle::default()
        };
        let (_, height) = lay(
            &[styled_text_box("a\n", style)],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(height, line_height);
    }

    #[test]
    fn a_no_break_space_is_part_of_its_word() {
        let fonts = test_fonts();
        // "a\u{a0}b" is one word: never split, never collapsed.
        let a = fonts.measure("a", 16.0);
        let (frags, _) = lay(
            &[text_box("a\u{a0}b", 16.0)],
            a + 1.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(texts_of(&frags), vec!["a\u{a0}b"]);
        assert_eq!(frags[0].w, fonts.measure("a\u{a0}b", 16.0));
        // Around ordinary spaces it stays a word of its own.
        let (frags, _) = lay(
            &[text_box("a \u{a0} b", 16.0)],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(texts_of(&frags), vec!["a", "\u{a0}", "b"]);
        // A spacer paragraph of one no-break space is a line tall.
        let (frags, height) = lay(
            &[text_box("\u{a0}", 16.0)],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(frags.len(), 1);
        assert_eq!(height, fonts.primary().metrics(16.0).line_height());
    }

    #[test]
    fn intrinsic_widths_of_a_run_are_the_widest_word_and_the_widest_line() {
        let fonts = test_fonts();
        let w = |s: &str| fonts.measure(s, 16.0);
        let space = fonts.measure(" ", 16.0);
        let run = [text_box("aaa bb cccc", 16.0)];
        let items = collect_inline_items(&run, &fonts);
        let (min, max) = run_intrinsic_widths(&items, None, &fonts);
        assert_eq!(min, w("cccc"));
        assert_eq!(max, w("aaa") + space + w("bb") + space + w("cccc"));
        // Words set flush are one unbreakable piece.
        let run = [
            text_box("aaa", 16.0),
            inline_box(vec![text_box("bb", 16.0)]),
        ];
        let items = collect_inline_items(&run, &fonts);
        let (min, max) = run_intrinsic_widths(&items, None, &fonts);
        assert_eq!(min, w("aaa") + w("bb"));
        assert_eq!(max, min);
        // Under `pre` a line is one piece, and a newline ends it.
        let pre = ComputedStyle {
            white_space: WhiteSpace::Pre,
            ..ComputedStyle::default()
        };
        let run = [styled_text_box("a b\ncccc", pre)];
        let items = collect_inline_items(&run, &fonts);
        let (min, max) = run_intrinsic_widths(&items, None, &fonts);
        assert_eq!(min, w("cccc").max(w("a") + space + w("b")));
        assert_eq!(max, min);
    }

    #[test]
    fn intrinsic_widths_count_the_first_line_in_its_styles() {
        // "aa bb" in 16px, "aa" in 32px on the first line: the widest
        // line is the first, the widest piece its first.
        let fonts = test_fonts();
        let on_first_line = |text: &str, white_space| {
            let style = |font_size_px| ComputedStyle {
                font_size_px,
                white_space,
                ..ComputedStyle::default()
            };
            LayoutBox {
                first_line: Some(Box::new(FirstLine {
                    style: style(32.0),
                    chain: Vec::new(),
                })),
                ..styled_text_box(text, style(16.0))
            }
        };
        let w = |text, size| fonts.measure(text, size);
        let boxes = [on_first_line("aa bb", WhiteSpace::Normal)];
        let items = collect_inline_items(&boxes, &fonts);
        let first = collect_first_line_items(&boxes, &fonts, None);
        assert_eq!(
            run_intrinsic_widths(&items, Some(&first), &fonts),
            (
                w("aa", 32.0).max(w("bb", 16.0)),
                w("aa", 32.0) + w(" ", 32.0) + w("bb", 32.0)
            )
        );
        // After a forced break, the next line is the run's own.
        let boxes = [on_first_line("aa\nbbbbbb", WhiteSpace::Pre)];
        let items = collect_inline_items(&boxes, &fonts);
        let first = collect_first_line_items(&boxes, &fonts, None);
        let (min, max) = run_intrinsic_widths(&items, Some(&first), &fonts);
        assert_eq!(min, max);
        assert_eq!(max, w("aa", 32.0).max(w("bbbbbb", 16.0)));
    }

    /// `b` with `style` on the first formatted line.
    fn with_first_line(b: LayoutBox, style: ComputedStyle) -> LayoutBox {
        LayoutBox {
            first_line: Some(Box::new(FirstLine {
                style,
                chain: Vec::new(),
            })),
            ..b
        }
    }

    /// A 32px style, `white-space: pre` or not.
    fn big(white_space: WhiteSpace) -> ComputedStyle {
        ComputedStyle {
            font_size_px: 32.0,
            white_space,
            ..ComputedStyle::default()
        }
    }

    /// The items in short: a word as its text and font size, an inline
    /// element's edges as `<` and `>`, a forced break as `/`, an atomic
    /// inline as `[]`.
    fn shapes(items: &[InlineItem<'_>]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                InlineItem::Word { text, style, .. } => format!("{text}@{}", style.font_size_px),
                InlineItem::Open { .. } => "<".to_string(),
                InlineItem::Close(_) => ">".to_string(),
                InlineItem::ForcedBreak(_) => "/".to_string(),
                InlineItem::Atomic { .. } => "[]".to_string(),
                InlineItem::Float(_) => "float".to_string(),
                InlineItem::Absolute(_) => "absolute".to_string(),
            })
            .collect()
    }

    #[test]
    fn the_first_lines_items_end_at_the_first_break_past_the_room() {
        // Once the words so far are wider than the room, the line has
        // ended at the next break opportunity: the items end with it.
        let fonts = test_fonts();
        let w = |t| fonts.measure(t, 32.0);
        let normal = || big(WhiteSpace::Normal);
        let run = [with_first_line(text_box("aa bb cc dd ee", 16.0), normal())];
        let first = |room| shapes(&collect_first_line_items(&run, &fonts, room));
        assert_eq!(first(None), ["aa@32", "bb@32", "cc@32", "dd@32", "ee@32"]);
        assert_eq!(
            first(Some(w("aa") + w("bb") - 1.0)),
            ["aa@32", "bb@32", "cc@32"]
        );
        // As wide as the room is not past it.
        assert_eq!(
            first(Some(w("aa") + w("bb"))),
            ["aa@32", "bb@32", "cc@32", "dd@32"]
        );
        // A room below zero is none: the line ends after its first item.
        let run = [with_first_line(text_box(" aa bb cc", 16.0), normal())];
        assert_eq!(
            shapes(&collect_first_line_items(&run, &fonts, Some(-5.0))),
            ["aa@32", "bb@32"]
        );
        // Nothing after the end is collected: not the next box, text or
        // an inline element, nor a forced break after the word the line
        // ended at.
        let bold = with_first_line(
            inline_box(vec![with_first_line(text_box("cc", 16.0), normal())]),
            normal(),
        );
        for next in [with_first_line(text_box(" cc", 16.0), normal()), bold] {
            let run = [with_first_line(text_box("aa bb", 16.0), normal()), next];
            assert_eq!(
                shapes(&collect_first_line_items(&run, &fonts, Some(0.0))),
                ["aa@32", "bb@32"]
            );
        }
        let pre_line = ComputedStyle {
            white_space: WhiteSpace::PreLine,
            ..ComputedStyle::default()
        };
        let run = [with_first_line(
            styled_text_box("aa bb\ncc", pre_line),
            big(WhiteSpace::PreLine),
        )];
        assert_eq!(
            shapes(&collect_first_line_items(&run, &fonts, Some(0.0))),
            ["aa@32", "bb@32"]
        );
    }

    #[test]
    fn the_first_lines_items_end_only_where_the_line_can_break() {
        // With no room, the line ends at the first break opportunity
        // after its first item: not between words set flush, nor before
        // an inline element's start edge with no gap before it.
        let fonts = test_fonts();
        let normal = || big(WhiteSpace::Normal);
        let bold = |text: &str| {
            with_first_line(
                inline_box(vec![with_first_line(text_box(text, 16.0), normal())]),
                normal(),
            )
        };
        let first = |run: &[LayoutBox]| shapes(&collect_first_line_items(run, &fonts, Some(0.0)));
        let run = [
            with_first_line(text_box("aa", 16.0), normal()),
            bold("bb"),
            with_first_line(text_box(" cc dd", 16.0), normal()),
        ];
        assert_eq!(first(&run), ["aa@32", "<", "bb@32", ">", "cc@32"]);
        // A start edge with a gap before it is one, and a word inside
        // the element: the items end there, the element left open.
        let run = [with_first_line(text_box("aa ", 16.0), normal()), bold("bb")];
        assert_eq!(first(&run), ["aa@32", "<"]);
        let run = [
            with_first_line(text_box("aa", 16.0), normal()),
            bold(" bb cc"),
        ];
        assert_eq!(first(&run), ["aa@32", "<", "bb@32"]);
        // Nor anywhere under `pre` but at a forced break, where the line
        // ends whatever the room.
        let pre = ComputedStyle {
            white_space: WhiteSpace::Pre,
            ..ComputedStyle::default()
        };
        let run = [
            with_first_line(styled_text_box("aa bb ", pre.clone()), big(WhiteSpace::Pre)),
            with_first_line(
                inline_box(vec![with_first_line(
                    styled_text_box("cc", pre.clone()),
                    big(WhiteSpace::Pre),
                )]),
                big(WhiteSpace::Pre),
            ),
            with_first_line(styled_text_box("\ndd", pre), big(WhiteSpace::Pre)),
        ];
        for room in [Some(0.0), None] {
            assert_eq!(
                shapes(&collect_first_line_items(&run, &fonts, room)),
                ["aa@32", "bb@32", "<", "cc@32", ">", "/"]
            );
        }
    }

    #[test]
    fn what_takes_room_back_lets_the_first_lines_items_run_on() {
        // With no room the items would end at the second word; but an
        // atomic inline before it, whose width is not known, or a
        // negative word or letter spacing or margin, which lets more on
        // the line, turns the bound off.
        let fonts = test_fonts();
        let bounded_is_all = |run: &[LayoutBox]| {
            shapes(&collect_first_line_items(run, &fonts, Some(0.0)))
                == shapes(&collect_first_line_items(run, &fonts, None))
        };
        let normal = || big(WhiteSpace::Normal);
        let words = |text: &str, style: ComputedStyle| with_first_line(text_box(text, 16.0), style);
        assert!(!bounded_is_all(&[words("aa bb cc", normal())]));
        let atomic = LayoutBox {
            kind: BoxKind::InlineBlock,
            ..inline_box(Vec::new())
        };
        assert!(bounded_is_all(&[atomic, words(" aa bb cc", normal())]));
        let spaced = ComputedStyle {
            word_spacing: -1.0,
            ..normal()
        };
        assert!(bounded_is_all(&[words("aa bb cc", spaced)]));
        // Letter spacing that makes "iii" narrower than nothing and
        // leaves "mm" wider: the word that takes room back is where the
        // line would end, and it is let on first.
        let (i, m) = (fonts.measure("i", 32.0), fonts.measure("m", 32.0));
        assert!(i < m);
        let spaced = ComputedStyle {
            letter_spacing: -(i + m) / 2.0,
            ..normal()
        };
        assert!(bounded_is_all(&[words("mm iii mm mm", spaced)]));
        // So is a start edge that takes room back; and an end edge that
        // does turns the bound off for what follows it.
        let edged = |side: usize| {
            let mut style = normal();
            style.margin[side] = style::computed::LengthPercentageAuto::Px(-1000.0);
            with_first_line(
                inline_box(vec![with_first_line(text_box("bb", 16.0), normal())]),
                style,
            )
        };
        let run = [
            words("aa ", normal()),
            edged(LEFT),
            words(" cc dd", normal()),
        ];
        assert!(bounded_is_all(&run));
        let run = [
            words("aa", normal()),
            edged(RIGHT),
            words(" cc dd", normal()),
        ];
        assert!(bounded_is_all(&run));
    }

    #[test]
    fn a_run_is_restyled_on_the_first_line_where_a_box_in_it_is() {
        let restyled = || big(WhiteSpace::Normal);
        assert!(!restyled_on_first_line(&[
            text_box("aa", 16.0),
            inline_box(vec![text_box("bb", 16.0)]),
        ]));
        assert!(restyled_on_first_line(&[
            text_box("aa", 16.0),
            with_first_line(text_box("bb", 16.0), restyled()),
        ]));
        assert!(restyled_on_first_line(&[with_first_line(
            inline_box(Vec::new()),
            restyled()
        )]));
        assert!(restyled_on_first_line(&[inline_box(vec![
            with_first_line(text_box("bb", 16.0), restyled())
        ])]));
        // The boxes in an atomic inline are not the run's.
        let atomic = LayoutBox {
            kind: BoxKind::InlineBlock,
            ..inline_box(vec![with_first_line(text_box("bb", 16.0), restyled())])
        };
        assert!(!restyled_on_first_line(&[atomic]));
    }
}
