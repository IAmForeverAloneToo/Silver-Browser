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
//! apart. An atomic inline is a wrap opportunity on both sides
//! regardless. Collapsible white space at the start or end of a line
//! is discarded.
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

use style::computed::{ComputedStyle, Direction, TextAlign, TextTransform, WhiteSpace};
use text::{FontCollection, FontMetrics};

use crate::block::{
    AtomicLayout, Fragment, FragmentKind, layout_float, layout_inline_block, pending_float,
};
use crate::box_tree::{BoxKind, LayoutBox};
use crate::float::{At, FloatContext, PendingFloat};

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
        node: usize,
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
}

/// One word placed on the line being built, before [`flush_line`]
/// positions it.
struct LineWord<'a> {
    text: &'a str,
    width: f32,
    style: &'a ComputedStyle,
    node: usize,
}

/// One item on the line being built. The atomic's layout is boxed: it
/// carries a fragment subtree, many times a word's size.
enum LineItem<'a> {
    Word(LineWord<'a>),
    Atomic(Box<AtomicLayout>),
}

impl LineItem<'_> {
    fn width(&self) -> f32 {
        match self {
            LineItem::Word(w) => w.width,
            LineItem::Atomic(a) => a.width,
        }
    }

    /// The style whose font measures the gap before this item.
    fn style(&self) -> &ComputedStyle {
        match self {
            LineItem::Word(w) => w.style,
            LineItem::Atomic(a) => &a.root.style,
        }
    }
}

/// An item's place on the line as it is being built: whether a gap
/// precedes it.
struct Placed<'a> {
    item: LineItem<'a>,
    gap_before: bool,
}

/// The lines of one inline formatting context.
pub(crate) struct InlineLayout {
    /// Relative to the run's container's content-box top-left `(0, 0)`.
    pub(crate) fragments: Vec<Fragment>,
    /// The height of the lines together.
    pub(crate) height: f32,
    /// The baseline of the last line, from the top of the run (CSS2
    /// §10.8.1: what an inline-block's own baseline is), `None` when no
    /// line was made.
    pub(crate) last_baseline: Option<f32>,
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
pub(crate) fn layout_inline(
    items: &[LayoutBox],
    cb_width: f32,
    cb_height: Option<f32>,
    container: &ComputedStyle,
    floats: &mut FloatContext,
    at: At,
    fonts: &FontCollection,
) -> InlineLayout {
    let flat = collect_inline_items(items, fonts);
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
        strut: fonts.metrics_for_text(" ", container.font_size_px),
        fonts,
        line: Vec::new(),
        line_width: 0.0,
        y: 0.0,
        fragments: Vec::new(),
        last_baseline: None,
        deferred: Vec::new(),
    };
    // The item before was an atomic inline: a wrap opportunity follows
    // it whether or not white space does.
    let mut after_atomic = false;

    for item in &flat {
        match item {
            InlineItem::Word {
                text,
                width,
                style,
                no_wrap,
                gap_before,
                node,
            } => {
                let word = LineWord {
                    text: text.as_str(),
                    width: *width,
                    style,
                    node: *node,
                };
                let can_break = !no_wrap && (*gap_before || after_atomic);
                lines.place(LineItem::Word(word), *gap_before, can_break);
                after_atomic = false;
            }
            InlineItem::Atomic { b, gap_before } => {
                let atomic = layout_inline_block(b, cb_width, cb_height, fonts);
                lines.place(LineItem::Atomic(Box::new(atomic)), *gap_before, true);
                after_atomic = true;
            }
            InlineItem::Float(b) => lines.float(b),
            InlineItem::ForcedBreak(style) => {
                // `white-space: pre`/`pre-wrap`/`pre-line` (M8.4): a
                // literal newline ends the line unconditionally. A
                // break on an empty line is a blank line, as tall as
                // the strut and the text it is in (css-text-3 §4.1.3) —
                // a code block's blank lines are not nothing.
                if lines.line.is_empty() {
                    lines.start_line();
                    let m = join(lines.strut, fonts.metrics_for_text(" ", style.font_size_px));
                    lines.last_baseline = Some(lines.y + m.ascent);
                    lines.y += m.line_height();
                } else {
                    lines.flush();
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
    }
}

/// The lines of one run as they are built: the line in progress and
/// the fragments of the lines before it.
struct Lines<'a, 'f, 'c> {
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
    /// The containing block's own font metrics, the least any line is.
    strut: FontMetrics,
    fonts: &'f FontCollection,
    line: Vec<Placed<'a>>,
    line_width: f32,
    /// The top of the line in progress.
    y: f32,
    fragments: Vec<Fragment>,
    last_baseline: Option<f32>,
    /// Floats met on the line in progress that did not fit beside it,
    /// placed below it once its height is known.
    deferred: Vec<PendingFloat>,
}

impl<'a> Lines<'a, '_, '_> {
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

    /// Puts `item` on the line in progress, or on a new one when it
    /// does not fit and a break before it is allowed.
    fn place(&mut self, item: LineItem<'a>, gap_before: bool, can_break: bool) {
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
        let gap = if self.line.is_empty() || !gap_before {
            0.0
        } else {
            space_width(item.style(), self.fonts)
        };
        // Greedy line breaking at a soft-wrap opportunity (M7.3): if
        // this item doesn't fit, start a new line — unless the line is
        // still empty (a solo word wider than `available_width` just
        // overflows; see the module doc's overflow-wrap note) or no
        // break is allowed before it (glued to the word before, or
        // `white-space: pre`, M8.4).
        if can_break
            && !self.line.is_empty()
            && self.line_width + gap + item.width() > self.available_width
        {
            self.flush();
            self.place(item, gap_before, can_break);
            return;
        }
        self.line_width += gap + item.width();
        self.line.push(Placed { item, gap_before });
    }

    /// A float met in the run (M11): placed at the line in progress's
    /// top when it fits in what is left of the line — the line is then
    /// shortened by it, its content moved beside it, unless the float
    /// context put it lower — or below the line once that ends; on an
    /// empty line, at the line's top, or held while the flow position
    /// is unresolved. Its fragment goes in its place, positioned by the
    /// root's `attach_floats`.
    fn float(&mut self, b: &'a LayoutBox) {
        let atomic = layout_float(b, self.cb_width, self.cb_height, self.fonts);
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

    /// Ends the line in progress, if there is one.
    fn flush(&mut self) {
        let baseline = flush_line(
            &mut self.line,
            self.line_width,
            self.line_left,
            self.available_width,
            self.text_align,
            &self.strut,
            self.fonts,
            &mut self.y,
            &mut self.fragments,
        );
        self.last_baseline = baseline.or(self.last_baseline);
        self.line_width = 0.0;
        self.place_deferred();
    }
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
    let mut collector = Collector {
        fonts,
        out: Vec::new(),
        space_pending: false,
    };
    collector.collect(boxes);
    collector.out
}

struct Collector<'a, 'f> {
    fonts: &'f FontCollection,
    out: Vec<InlineItem<'a>>,
    /// Collapsible white space has been seen since the last item: the
    /// next item gets a gap before it.
    space_pending: bool,
}

impl<'a> Collector<'a, '_> {
    fn collect(&mut self, boxes: &'a [LayoutBox]) {
        for b in boxes {
            match &b.kind {
                BoxKind::Text(text) => self.text(b, text),
                BoxKind::Inline => self.collect(&b.children),
                BoxKind::InlineBlock => {
                    let gap_before = std::mem::take(&mut self.space_pending);
                    self.out.push(InlineItem::Atomic { b, gap_before });
                }
                // A float is invisible to the white space around it: a
                // gap pending before it is still pending after it.
                BoxKind::Float => self.out.push(InlineItem::Float(b)),
                // The box tree builder hoists a block out of inline
                // content (CSS2 §9.2.1.1), so none reaches here from a
                // parsed document; see the module doc.
                BoxKind::Block | BoxKind::AnonymousBlock => {}
            }
        }
    }

    fn text(&mut self, b: &'a LayoutBox, text: &str) {
        let transformed = apply_text_transform(text, b.style.text_transform);
        let no_wrap = b.style.white_space == WhiteSpace::Pre;
        // `pre`/`pre-wrap`/`pre-line` all keep literal newlines as
        // forced breaks; only `normal` collapses them away like any
        // other whitespace (see the module doc for what's *not*
        // preserved: multi-space run width).
        let preserves_newlines = b.style.white_space != WhiteSpace::Normal;
        // Always split on literal newlines, even under `normal` (where
        // they're not a forced break — just whitespace like any other,
        // a gap). This is also what keeps `\n` from ever reaching
        // `bidi_reorder`: `unicode-bidi` treats it as its own paragraph
        // separator and `bidi_reorder` only returns the first paragraph
        // (see its doc's "level 1" scope cut), so a line containing an
        // embedded `\n` would silently lose everything after it.
        let base_direction = to_text_direction(b.style.direction);
        for (i, line) in transformed.split('\n').enumerate() {
            if i > 0 {
                if preserves_newlines {
                    self.out.push(InlineItem::ForcedBreak(&b.style));
                    self.space_pending = false;
                } else {
                    self.space_pending = true;
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
                        self.word(b, &reordered[start..i], no_wrap, gap_before);
                    }
                    self.space_pending = true;
                } else if word.is_none() {
                    word = Some((i, std::mem::take(&mut self.space_pending)));
                }
            }
            if let Some((start, gap_before)) = word {
                self.word(b, &reordered[start..], no_wrap, gap_before);
            }
        }
    }

    fn word(&mut self, b: &'a LayoutBox, text: &str, no_wrap: bool, gap_before: bool) {
        self.out.push(InlineItem::Word {
            text: text.to_string(),
            width: measure_word(text, &b.style, self.fonts),
            style: &b.style,
            no_wrap,
            gap_before,
            node: b.node,
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
pub(crate) fn run_intrinsic_widths(items: &[InlineItem<'_>], fonts: &FontCollection) -> (f32, f32) {
    let mut min = 0.0f32;
    let mut max = 0.0f32;
    // The line being measured, and the unbreakable piece at its end.
    let mut line = 0.0f32;
    let mut piece = 0.0f32;
    let mut line_empty = true;
    let mut after_atomic = false;
    for item in items {
        match item {
            InlineItem::Word {
                width,
                style,
                no_wrap,
                gap_before,
                ..
            } => {
                let gap = if line_empty || !gap_before {
                    0.0
                } else {
                    space_width(style, fonts)
                };
                let can_break = !no_wrap && !line_empty && (*gap_before || after_atomic);
                piece = if can_break {
                    *width
                } else {
                    piece + gap + width
                };
                line += gap + width;
                line_empty = false;
                after_atomic = false;
            }
            InlineItem::Atomic { b, gap_before } => {
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
            InlineItem::Float(b) => {
                let (float_min, float_max) = crate::intrinsic::contribution(b, fonts);
                min = min.max(float_min);
                line += float_max;
            }
            InlineItem::ForcedBreak(_) => {
                line = 0.0;
                piece = 0.0;
                line_empty = true;
                after_atomic = false;
            }
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
/// processing. Borrows (no allocation) for the common `None` case.
fn apply_text_transform(text: &str, transform: TextTransform) -> Cow<'_, str> {
    match transform {
        TextTransform::None => Cow::Borrowed(text),
        TextTransform::Uppercase => Cow::Owned(text.to_uppercase()),
        TextTransform::Lowercase => Cow::Owned(text.to_lowercase()),
        TextTransform::Capitalize => {
            let mut out = String::with_capacity(text.len());
            let mut at_word_start = true;
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
/// `available_width` wide (shortened beside floats, M11). Returns the
/// line's baseline, or `None` for an empty line, which is no line at
/// all. Clears `line` for the next line.
#[allow(clippy::too_many_arguments)]
fn flush_line(
    line: &mut Vec<Placed<'_>>,
    line_width: f32,
    line_left: f32,
    available_width: f32,
    text_align: TextAlign,
    strut: &FontMetrics,
    fonts: &FontCollection,
    y: &mut f32,
    fragments: &mut Vec<Fragment>,
) -> Option<f32> {
    if line.is_empty() {
        return None;
    }

    // Above the baseline: the tallest ascent. Below it: the deepest
    // extent — a text's descent and its line gap under it (the gap is
    // all below the baseline in this engine's `line-height: normal`,
    // not split as half-leading), an inline-block's margin box past its
    // baseline — which is where an inline-block's own last line gap
    // already is, so it is not added again under it.
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
    for (i, placed) in line.drain(..).enumerate() {
        if i > 0 && placed.gap_before {
            x += space_width(placed.item.style(), fonts);
        }
        match placed.item {
            LineItem::Word(word) => {
                if word.style.letter_spacing != 0.0 {
                    // Per-character fragments (M8.4) — see
                    // `measure_word`'s doc for why this has to shape
                    // character-by-character, same as the width this
                    // word was measured with.
                    emit_letter_spaced(
                        fragments,
                        &word,
                        &mut x,
                        baseline_y,
                        max_ascent,
                        max_descent,
                        fonts,
                    );
                    continue;
                }
                fragments.push(text_fragment(x, baseline_y, &word, max_ascent, max_descent));
                x += word.width;
            }
            LineItem::Atomic(a) => {
                // The margin box's top sits `ascent` above the baseline;
                // the border box is a margin inside it.
                let top = baseline_y - a.ascent;
                let width = a.width;
                // The border box is a margin inside the margin box, and
                // a relatively positioned box is moved from there (M11).
                let (dx, dy) = a.relative;
                fragments.push(
                    a.root
                        .translated(x + a.margin_left + dx, top + a.margin_top + dy),
                );
                x += width;
            }
        }
    }

    *y += max_ascent + max_below;
    Some(baseline_y)
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
            &mut floats,
            At::root(),
            fonts,
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
            node: 0,
        }
    }

    fn inline_box(children: Vec<LayoutBox>) -> LayoutBox {
        LayoutBox {
            style: ComputedStyle::default(),
            kind: BoxKind::Inline,
            children,
            node: 0,
        }
    }

    fn texts_of(fragments: &[Fragment]) -> Vec<&str> {
        fragments
            .iter()
            .filter_map(|f| match &f.kind {
                FragmentKind::Text(t) => Some(t.as_str()),
                FragmentKind::Box => None,
            })
            .collect()
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
        assert_eq!(texts_of(&fragments), vec!["a", "b", "c"]);
        assert_eq!(fragments[1].x, fragments[0].x + fragments[0].w, "flush");
        assert_eq!(
            fragments[2].x,
            fragments[1].x + fragments[1].w + fonts.measure(" ", 16.0),
            "one space"
        );
        // Narrow: "ab" stays together on the first line, "c" wraps.
        let (fragments, height) = lay(&run, 1.0, TextAlign::Left, &fonts);
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
            node: 0,
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
        let (min, max) = run_intrinsic_widths(&items, &fonts);
        assert_eq!(min, w("cccc"));
        assert_eq!(max, w("aaa") + space + w("bb") + space + w("cccc"));
        // Words set flush are one unbreakable piece.
        let run = [
            text_box("aaa", 16.0),
            inline_box(vec![text_box("bb", 16.0)]),
        ];
        let items = collect_inline_items(&run, &fonts);
        let (min, max) = run_intrinsic_widths(&items, &fonts);
        assert_eq!(min, w("aaa") + w("bb"));
        assert_eq!(max, min);
        // Under `pre` a line is one piece, and a newline ends it.
        let pre = ComputedStyle {
            white_space: WhiteSpace::Pre,
            ..ComputedStyle::default()
        };
        let run = [styled_text_box("a b\ncccc", pre)];
        let items = collect_inline_items(&run, &fonts);
        let (min, max) = run_intrinsic_widths(&items, &fonts);
        assert_eq!(min, w("cccc").max(w("a") + space + w("b")));
        assert_eq!(max, min);
    }
}
