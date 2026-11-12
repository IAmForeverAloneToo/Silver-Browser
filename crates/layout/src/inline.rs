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
//! `vertical-align` (CSS2 section 10.8.1, gap batch H) moves an atomic
//! inline's or an inline element's baseline from its parent's: `sub`,
//! `super`, `text-top`, `text-bottom`, `middle`, a length or a
//! percentage of its line height, the element's content moved with it;
//! `top` and `bottom` align the subtree it roots with the line's top or
//! bottom once the rest of the line is laid out, the line grown to it
//! when it is taller, as the reference browser grows it.
//!
//! White space (css-text-3 §4.1.1): collapsible white space between
//! two words, within a text node or across the boxes of a run, is one
//! gap of a space's width and a soft wrap opportunity; where there was
//! none, as in `a<b>b</b>`, the words are set flush and never broken
//! apart: when the second does not fit, both move to the next line
//! together, back to the last wrap opportunity on the line, as one
//! unbreakable piece (CSS2 section 9.4.2). Where lines wrap, an atomic
//! inline is a wrap opportunity on both sides. Collapsible white space at
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
//! * Justification (M21.7) only widens a line's gaps, and with
//!   `text-justify: inter-character` the places between a word's
//!   characters: it never compresses a line, and knows nothing of the
//!   scripts whose text the engine cannot yet break into lines (G10).
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
//! * An inline element's own `line-height` does not make its line
//!   taller: its text's does, and the strut's.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use dom::NodeId;
use style::computed::{
    BOTTOM, BorderStyle, ComputedStyle, Direction, LEFT, RIGHT, TOP, TextAlign, TextAlignLast,
    TextJustify, TextTransform,
};
use style::selector_matching::PseudoElement;
use text::{FontCollection, FontMetrics};

use crate::block::{
    AtomicLayout, Deferred, Edges, Fragment, FragmentKind, carried, layout_float,
    layout_inline_block, pending_float, relative_offset,
};
use crate::box_tree::{BoxKind, FirstLine, LayoutBox};
use crate::decoration::{self, Decorated, Decorating, DrawnLine};
use crate::float::{At, FloatContext, PendingFloat};
use crate::font;
use crate::marker::Marker;
use crate::pass::Pass;

/// One flattened piece of inline content, in document order.
pub(crate) enum InlineItem<'a> {
    Word {
        text: String,
        /// Already includes `letter-spacing`'s contribution (measured
        /// consistently with how [`emit_letter_spaced`] renders it —
        /// see that function's doc for why that's the same code path).
        width: f32,
        style: &'a Arc<ComputedStyle>,
        /// `white-space: pre`: the gap before this word is never a
        /// line-break opportunity (text flows without wrapping).
        no_wrap: bool,
        /// Collapsible white space preceded this word (in its text, or
        /// in the boxes before it): a gap of a space's width, and a
        /// soft wrap opportunity unless `no_wrap`.
        gap_before: bool,
        /// A soft wrap opportunity before it with no white space: where
        /// UAX #14 breaks between the text before it, in another box,
        /// and its own, `foo<b>\u{6587}</b>` say.
        break_before: bool,
        /// The soft wrap opportunities within it (D55), as byte offsets
        /// into `text`: UAX #14's, after a hyphen, between ideographs,
        /// after preserved spaces. It is cut at one only where a line
        /// breaks there.
        breaks: Vec<usize>,
        /// How much of `width` is preserved spaces at its end, which hang
        /// past the line's end rather than wrap it (CSS Text 3 section
        /// 4.1.3); none under `break-spaces`.
        hang: f32,
        /// A preserved tab, its text `\t`: as wide as takes the line to
        /// its next tab stop, which only its place on the line says.
        tab: bool,
        /// The space `text-autospace` puts before it, after an ideograph
        /// or before one: a gap but where it starts a line, and no
        /// opportunity to break.
        lead: f32,
        /// The element the text belongs to (M9.4), carried into its
        /// fragment.
        node: NodeId,
        /// Whether the text is a generated box's (M16), also carried
        /// into its fragment.
        generated: Option<PseudoElement>,
        /// The text decorations in force over the word (M21.7).
        decorations: Option<&'a Arc<[Decorating]>>,
        /// Those over the white space whose gap is before it: the
        /// element's the space was first met in.
        gap_decorations: Option<&'a Arc<[Decorating]>>,
        /// That white space's style, whose space the gap is as wide as:
        /// the text's the space was first met in, not the word's.
        gap_style: Option<&'a Arc<ComputedStyle>>,
    },
    /// A literal newline under `white-space: pre`/`pre-wrap`/`pre-line`
    /// — ends the current line unconditionally, however much of
    /// `available_width` is left. On an empty line it *is* the line: a
    /// blank line of the text's own font height.
    ForcedBreak(&'a Arc<ComputedStyle>),
    /// An atomic inline-level box (M10.4: an inline-block), one item on
    /// the line, as wide as its margin box.
    Atomic {
        b: &'a LayoutBox,
        /// As for a word; a wrap opportunity exists before an atomic
        /// inline either way.
        gap_before: bool,
        /// As for a word (M21.7).
        gap_decorations: Option<&'a Arc<[Decorating]>>,
        /// As for a word.
        gap_style: Option<&'a Arc<ComputedStyle>>,
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
        style: &'a Arc<ComputedStyle>,
        gap_before: bool,
        /// As for a word: the opportunity of the text it starts with.
        break_before: bool,
        /// As for a word (M21.7).
        gap_decorations: Option<&'a Arc<[Decorating]>>,
        /// As for a word.
        gap_style: Option<&'a Arc<ComputedStyle>>,
    },
    /// The end of an inline element: its right padding, border and
    /// margin, on the line it ends on, glued to what precedes it; with
    /// the element's style where the items go.
    Close(&'a Arc<ComputedStyle>),
    /// A list item's marker (M21.6, [`BoxKind::Marker`]): outside, as
    /// wide as nothing and glued to what follows it, the list item
    /// placing it once laid out; inside, a shape as wide as itself and
    /// the room after it. Either way its font is on the line.
    Marker {
        b: &'a LayoutBox,
        style: &'a Arc<ComputedStyle>,
    },
}

/// One word placed on the line being built, before [`flush_line`]
/// positions it. It borrows the run's [`InlineItem`]s (`'i`), which
/// live for the layout of the run only.
struct LineWord<'i> {
    text: &'i str,
    width: f32,
    /// The preserved spaces at its end, which may hang past the line.
    hang: f32,
    /// A preserved tab (CSS Text 3 section 4.2), as wide as its place
    /// makes it.
    tab: bool,
    /// The line breaks after it at a hyphenation opportunity (CSS Text 3
    /// section 5.4): it ends with the hyphen, which `width` counts.
    hyphen: bool,
    /// The opportunities of the item's text (D55), and where in that
    /// text this word, all of it or a piece of it, begins.
    breaks: &'i [usize],
    offset: usize,
    style: &'i Arc<ComputedStyle>,
    node: NodeId,
    generated: Option<PseudoElement>,
    decorations: Option<&'i Arc<[Decorating]>>,
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
    Open(&'a LayoutBox, &'a Arc<ComputedStyle>, f32),
    /// An inline element's end edge, as wide as its right padding,
    /// border and margin, with the element's style on the line.
    Close(&'a Arc<ComputedStyle>, f32),
    /// An absolutely positioned box met here (M11): no width, its
    /// static position taken where the line puts it.
    Absolute(&'a LayoutBox),
    /// A list item's marker (M21.6): as wide as it moves the line on.
    Marker(Marker<'a>),
}

impl LineItem<'_, '_> {
    /// How much of its width may hang past the line's end.
    fn hang(&self) -> f32 {
        match self {
            LineItem::Word(w) => w.hang,
            _ => 0.0,
        }
    }

    fn width(&self) -> f32 {
        match self {
            LineItem::Word(w) => w.width,
            LineItem::Atomic(a) => a.width,
            LineItem::Open(_, _, edge) | LineItem::Close(_, edge) => *edge,
            LineItem::Absolute(_) => 0.0,
            LineItem::Marker(m) => m.advance,
        }
    }

    /// The style whose font measures the gap before this item.
    fn style(&self) -> &ComputedStyle {
        match self {
            LineItem::Word(w) => w.style,
            LineItem::Atomic(a) => &a.root.style,
            LineItem::Open(_, style, _) | LineItem::Close(style, _) => style,
            LineItem::Absolute(b) => &b.style,
            LineItem::Marker(m) => m.style,
        }
    }

    /// Whether the item is content: a word or an atomic inline, which
    /// a gap can stand before and which makes a line box a line (CSS2
    /// §9.4.2), or an inside marker drawn as a shape. An inline box's
    /// edge is not, nor is an absolutely positioned box, nor an outside
    /// marker, which is as wide as nothing.
    fn is_content(&self) -> bool {
        match self {
            LineItem::Word(_) | LineItem::Atomic(_) => true,
            LineItem::Marker(m) => !m.is_outside(),
            LineItem::Open(..) | LineItem::Close(..) | LineItem::Absolute(_) => false,
        }
    }
}

/// An item's place on the line as it is being built: whether a gap
/// precedes it, and whether the line may break before it; `index` is
/// its [`InlineItem`]'s in the run.
struct Placed<'a, 'i> {
    item: LineItem<'a, 'i>,
    index: usize,
    gap_before: bool,
    /// The style of the white space a gap before it is, which its width
    /// is a space of; the item's own when it has none.
    gap_style: Option<&'a Arc<ComputedStyle>>,
    can_break: bool,
    /// The autospace taken before it.
    lead: f32,
}

impl Placed<'_, '_> {
    /// The style of the white space before it, which its gap is as wide
    /// as a space of.
    fn gap_of(&self) -> &ComputedStyle {
        self.gap_style.map_or(self.item.style(), |style| &**style)
    }

    /// The width of the gap before it, when there is one.
    fn gap(&self, fonts: &FontCollection) -> f32 {
        space_width(self.gap_of(), fonts)
    }
}

/// What a line's words of one style share, kept for the next word as
/// a line is placed: the font, metrics and space width a word asks for
/// again and again. Only a word of ASCII text in a font with no chain
/// takes the font and metrics kept, a text that could fall back to
/// another face or a ranged one never; a space's width is its style's
/// whatever the word.
#[derive(Default)]
struct LineMemo {
    metrics: Option<(*const ComputedStyle, FontMetrics)>,
    font: Option<(*const ComputedStyle, text::Font)>,
    gap: Option<(*const ComputedStyle, f32)>,
}

impl LineMemo {
    /// The line metrics of `text` in `style`, as [`font::line`] has them.
    fn metrics(
        &mut self,
        fonts: &FontCollection,
        text: &str,
        style: &ComputedStyle,
    ) -> FontMetrics {
        let key = style as *const ComputedStyle;
        if text.is_ascii()
            && let Some((kept, m)) = self.metrics
            && kept == key
        {
            return m;
        }
        let m = font::line(fonts, text, style);
        if text.is_ascii() && self.font(fonts, text, style).chain == 0 {
            self.metrics = Some((key, m));
        }
        m
    }

    /// The font `text` is in in `style`, as [`font::font_for`] has it.
    fn font(&mut self, fonts: &FontCollection, text: &str, style: &ComputedStyle) -> text::Font {
        let key = style as *const ComputedStyle;
        if text.is_ascii()
            && let Some((kept, font)) = self.font
            && kept == key
        {
            return font;
        }
        let font = font::font_for(fonts, style, text);
        if text.is_ascii() {
            self.font = Some((key, font));
        }
        font
    }

    /// The width of `placed`'s gap, as [`Placed::gap`] has it.
    fn gap(&mut self, fonts: &FontCollection, placed: &Placed<'_, '_>) -> f32 {
        let style = placed.gap_of();
        let key = style as *const ComputedStyle;
        if let Some((kept, width)) = self.gap
            && kept == key
        {
            return width;
        }
        let width = space_width(style, fonts);
        self.gap = Some((key, width));
        width
    }
}

impl<'a> InlineItem<'a> {
    /// The space `text-autospace` puts before it.
    fn lead(&self) -> f32 {
        match self {
            InlineItem::Word { lead, .. } => *lead,
            _ => 0.0,
        }
    }

    /// The style of the white space whose gap is before it, if one is.
    fn gap_style(&self) -> Option<&'a Arc<ComputedStyle>> {
        match self {
            InlineItem::Word { gap_style, .. }
            | InlineItem::Atomic { gap_style, .. }
            | InlineItem::Open { gap_style, .. } => *gap_style,
            _ => None,
        }
    }
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
    /// The first line's top, from the top of the run, and its height:
    /// where a list item's outside marker on it looks for floats
    /// (M21.6).
    pub(crate) first_line: Option<(f32, f32)>,
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
///
/// `indented` says the run's first line is its block container's first
/// formatted line, which `text-indent` indents (CSS Text 3 section 8.1),
/// or, with `hanging`, the one line it does not.
///
/// Under `text-wrap: balance` (CSS Text 4 section 6.2) the lines of each
/// paragraph, the run between forced breaks, of at most
/// [`MAX_BALANCED_LINES`] lines, are made as alike in length as they can
/// be without making more of them: each is broken as if the line were
/// the narrowest that keeps the paragraph's count of lines, found by
/// trying, and aligned in its own room. Under `pretty` (section 6.1) a
/// paragraph whose last line is a word alone has the last word of the
/// line before join it, where that keeps the count, the reference
/// browser's avoidance of a short last line, approximated.
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
    indented: bool,
) -> InlineLayout<'a> {
    dom::hook!("layout::inline::layout_inline");
    let run = |caps: &[Cap], floats: &mut FloatContext| {
        layout_run(
            items, cb_width, cb_height, container, first_line, floats, at, pass, indented, caps,
        )
    };
    use style::computed::TextWrapStyle;
    let wrap_style = container.text().text_wrap_style;
    if !container.white_space.wrap
        || !matches!(wrap_style, TextWrapStyle::Balance | TextWrapStyle::Pretty)
    {
        return run(&[], floats).0;
    }
    let (unbalanced, paragraphs) = run(&[], &mut floats.clone());
    let height = unbalanced.height;
    let mut caps: Vec<Cap> = vec![(f32::INFINITY, 0, false); paragraphs.len()];
    if wrap_style == TextWrapStyle::Pretty {
        // A paragraph whose last line is one piece, a word alone, takes
        // the last piece of the line before with it, where that keeps its
        // count of lines: the lines before those two are as they were.
        let mut changed = false;
        for (n, paragraph) in paragraphs.iter().enumerate() {
            if paragraph.lines < 2 || paragraph.last_pieces != 1 || paragraph.before_pieces < 2 {
                continue;
            }
            caps[n] = (paragraph.before - 0.5, paragraph.lines - 2, false);
            let (_, tried) = run(&caps, &mut floats.clone());
            if tried.get(n).map(|p| p.lines) == Some(paragraph.lines) {
                changed = true;
            } else {
                caps[n] = (f32::INFINITY, 0, false);
            }
        }
        return if changed {
            run(&caps, floats).0
        } else {
            run(&[], floats).0
        };
    }
    for (paragraph, count) in paragraphs.iter().map(|p| p.lines).enumerate() {
        if !(2..=MAX_BALANCED_LINES).contains(&count) {
            continue;
        }
        // The narrowest room, to half a pixel, that keeps the count.
        let (mut low, mut high) = (0.0f32, cb_width);
        while high - low > 0.5 {
            let middle = (low + high) / 2.0;
            caps[paragraph] = (middle, 0, true);
            let (laid, tried) = run(&caps, &mut floats.clone());
            // Its lines as many, and none of them pushed down past a
            // float for want of room.
            if tried.get(paragraph).map(|p| p.lines) == Some(count) && laid.height <= height + 0.5 {
                high = middle;
            } else {
                low = middle;
            }
        }
        caps[paragraph] = (high, 0, true);
    }
    run(&caps, floats).0
}

/// The most lines `text-wrap: balance` balances a paragraph of, as the
/// reference browser has it: a longer one wraps as `wrap` does.
const MAX_BALANCED_LINES: usize = 6;

/// The room for a paragraph's lines from its line of that number on;
/// with `true`, the room each line has beside the floats less what the
/// containing block's width is past it, so that a line beside a float
/// is balanced in its own room (as the reference browser does).
type Cap = (f32, usize, bool);

/// What `text-wrap: balance` and `pretty` read of a paragraph's lines:
/// how many there are, the width of the line before the last, and the
/// pieces (words, or what no wrap opportunity parts) of the last two.
#[derive(Debug, Clone, Copy, Default)]
struct Paragraph {
    lines: usize,
    before: f32,
    before_pieces: usize,
    last: f32,
    last_pieces: usize,
}

/// [`layout_inline`]'s lines, each paragraph's broken in at most its
/// cap in `caps` from its line there where it has one; with what each
/// paragraph's lines
/// were.
#[allow(clippy::too_many_arguments)]
fn layout_run<'a>(
    items: &'a [LayoutBox],
    cb_width: f32,
    cb_height: Option<f32>,
    container: &ComputedStyle,
    first_line: Option<&'a FirstLine>,
    floats: &mut FloatContext,
    at: At,
    pass: &Pass<'a>,
    indented: bool,
    caps: &[Cap],
) -> (InlineLayout<'a>, Vec<Paragraph>) {
    dom::hook!("layout::inline::layout_run");
    let flat = collect_inline_items(items, pass.fonts);
    let strut = font::line(pass.fonts, " ", container);
    // What the root inline box aligns its content by: measured only for
    // a line that aligns something by it (G247).
    let root = container;
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
            root_after: root,
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
        indent: Indent::of(container, cb_width),
        indent_next: indented,
        line_indent: 0.0,
        caps,
        paragraph: 0,
        counts: vec![Paragraph::default()],
        alignment: Alignment::of(container),
        strut: match (&first, first_line) {
            (Some(_), Some(line)) => font::line(pass.fonts, " ", &line.style),
            _ => strut,
        },
        root: match (&first, first_line) {
            (Some(_), Some(line)) => &line.style,
            _ => root,
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
        first_line: None,
        deferred: Vec::new(),
        absolutes: Vec::new(),
    };
    // The item before was an atomic inline: a wrap opportunity follows
    // it whether or not white space does.
    let mut after_atomic = false;
    // An outside marker is on the line and no content yet: nothing may
    // break the line before the content it is glued to (M21.6).
    let mut after_marker = false;

    for index in 0..flat.len() {
        match lines.item(index) {
            InlineItem::Word {
                text,
                width,
                style,
                no_wrap,
                gap_before,
                break_before,
                breaks,
                hang,
                tab,
                node,
                generated,
                decorations,
                gap_style,
                ..
            } => {
                let word = LineWord {
                    text: text.as_str(),
                    width: *width,
                    hang: *hang,
                    tab: *tab,
                    hyphen: false,
                    breaks,
                    offset: 0,
                    style,
                    node: *node,
                    generated: *generated,
                    decorations: *decorations,
                };
                let can_break = breaks_before(
                    *no_wrap,
                    *gap_before,
                    *gap_style,
                    *break_before || after_atomic,
                ) && !after_marker;
                lines.place(LineItem::Word(word), index, *gap_before, can_break);
                after_atomic = false;
                after_marker = false;
            }
            InlineItem::Atomic { b, gap_before, .. } => {
                let atomic = layout_inline_block(b, cb_width, cb_height, pass);
                // Under `nowrap` not even beside an atomic inline (CSS Text 3
                // section 5.1): its own style is the one it inherits there.
                let can_break = !after_marker && b.style.white_space.wrap;
                lines.place(
                    LineItem::Atomic(Box::new(atomic)),
                    index,
                    *gap_before,
                    can_break,
                );
                after_atomic = true;
                after_marker = false;
            }
            InlineItem::Marker { b, style } => {
                let marker = Marker::of(b, style, pass.fonts);
                let outside = marker.is_outside();
                lines.place(LineItem::Marker(marker), index, false, false);
                after_marker |= outside;
                after_atomic = !outside;
            }
            InlineItem::Float(b) => lines.float(b),
            InlineItem::Absolute(b) => lines.place(LineItem::Absolute(b), index, false, false),
            InlineItem::Open {
                b,
                style,
                gap_before,
                break_before,
                ..
            } => {
                let (start, _) = edge_widths(style, cb_width);
                let can_break = style.white_space.wrap
                    && (*gap_before || *break_before || after_atomic)
                    && !after_marker;
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
                if !lines.flush(true) {
                    lines.start_line();
                    let m = join(lines.strut, font::line(pass.fonts, " ", style));
                    lines.last_baseline = Some(lines.y + m.ascent);
                    lines.first_baseline = lines.first_baseline.or(lines.last_baseline);
                    lines.first_line = lines.first_line.or(Some((lines.y, m.line_height())));
                    lines.y += m.line_height();
                    lines.end_first_line();
                }
                // `text-indent: each-line` indents the line after a forced
                // break as the first; a paragraph begins after it.
                lines.indent_next = lines.indent.each_line;
                lines.paragraph += 1;
                lines.counts.push(Paragraph::default());
                after_atomic = false;
            }
        }
    }
    lines.flush(true);

    let layout = InlineLayout {
        fragments: lines.fragments,
        height: lines.y,
        last_baseline: lines.last_baseline,
        first_baseline: lines.first_baseline,
        first_line: lines.first_line,
        absolutes: lines.absolutes,
    };
    (layout, lines.counts)
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
    /// What the root inline box aligns its content by after the first
    /// line: the run's own style.
    root_after: &'i ComputedStyle,
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
    /// `text-indent`, whether the next line started takes it as a first
    /// line, and the line in progress's indent.
    indent: Indent,
    indent_next: bool,
    line_indent: f32,
    /// `text-wrap: balance`'s and `pretty`'s room for each paragraph's
    /// lines, the paragraph in progress, and the lines of each so far.
    caps: &'c [Cap],
    paragraph: usize,
    counts: Vec<Paragraph>,
    /// How the lines are aligned (M7.5, M21.7).
    alignment: Alignment,
    /// The containing block's own font metrics, the least any line is:
    /// on the first line, the innermost `::first-line`'s (M16).
    strut: FontMetrics,
    /// The style the root inline box aligns its content by (CSS2 section
    /// 10.8.1): the containing block's font, as the strut is.
    root: &'i ComputedStyle,
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
    /// The first line's top and height.
    first_line: Option<(f32, f32)>,
    /// Floats met on the line in progress that did not fit beside it,
    /// placed below it once its height is known, each with the
    /// absolutely positioned boxes it carries, from its border box.
    deferred: Vec<(PendingFloat, Vec<Deferred<'a>>, (f32, f32))>,
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
                    hang,
                    tab,
                    breaks,
                    style,
                    node,
                    generated,
                    decorations,
                    ..
                },
                LineItem::Word(piece),
            ) if piece.offset == 0 || piece.offset + piece.text.len() == text.len() => {
                let range = piece.offset..piece.offset + piece.text.len();
                let whole = range == (0..text.len());
                let text = &text[range];
                LineItem::Word(LineWord {
                    text,
                    width: if whole {
                        *width
                    } else {
                        measure_word(text, style, self.pass.fonts)
                    },
                    hang: if whole {
                        *hang
                    } else {
                        hang_of(text, style, self.pass.fonts)
                    },
                    tab: *tab,
                    hyphen: false,
                    breaks,
                    offset: piece.offset,
                    style,
                    node: *node,
                    generated: *generated,
                    decorations: *decorations,
                })
            }
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
            self.root = first.root_after;
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
        self.line_indent = self.indent.of_line(self.indent_next);
        self.measure_room();
    }

    /// How wide the line in progress may be for its breaks: its room,
    /// or less where `text-wrap: balance` or `pretty` says.
    fn room(&self) -> f32 {
        let line = self.counts.get(self.paragraph).map_or(0, |p| p.lines);
        match self.caps.get(self.paragraph) {
            Some(&(cap, from, true)) if line >= from => {
                (self.available_width - (self.cb_width - cap).max(0.0)).max(0.0)
            }
            Some(&(cap, from, false)) if line >= from => self.available_width.min(cap),
            _ => self.available_width,
        }
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
        // The indent at the line's start: its left, or its right where
        // the text runs right to left.
        if self.alignment.direction == Direction::Ltr {
            self.line_left += self.line_indent;
        }
        self.available_width = (self.available_width - self.line_indent).max(0.0);
    }

    /// Puts `item`, the run's `index`th, on the line in progress, or on
    /// a new one when it does not fit and a break before it is allowed.
    fn place(
        &mut self,
        mut item: LineItem<'a, 'i>,
        index: usize,
        gap_before: bool,
        can_break: bool,
    ) {
        if self.line.is_empty() {
            self.start_line();
            // Nothing fits beside the floats: move the line down to
            // where a float ends and try again, until the item fits
            // or no float is beside it, where it overflows as a line
            // too narrow for a word always has (CSS2 §9.5).
            while item.width() > self.room() && self.available_width < self.cb_width {
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
        // first on the line, and an inline box's edge is not content, so
        // that a space after an empty inline at the line's start goes as
        // one at its start does (css-text-3 §4.1.2). What is placed keeps
        // only a gap taken, for the line's placement to agree.
        // Its width is a space's of the white space's own style, the
        // text's the space was in, not the item's after it.
        let gap_before = gap_before && self.has_content;
        let gap_style = self.item(index).gap_style();
        let gap = if gap_before {
            space_width(
                gap_style.map_or(item.style(), |style| &**style),
                self.pass.fonts,
            )
        } else {
            0.0
        };
        // `text-autospace`'s space, but at the line's start.
        let lead = if self.has_content {
            self.item(index).lead()
        } else {
            0.0
        };
        let gap = gap + lead;
        // A tab reaches the next tab stop from the line's start.
        if let LineItem::Word(word) = &mut item
            && word.tab
        {
            let x = self.line_indent + self.line_width + gap;
            word.width = tab_advance(word.style, x, self.pass.fonts);
            word.hang = if hangs_spaces(word.style) {
                word.width
            } else {
                0.0
            };
        }
        // A word too wide for the room left is cut at its last
        // opportunity that fits (D55), or, alone on its line, where
        // `overflow-wrap` lets it or at its first: the head ends the
        // line, the rest goes on.
        let room = self.room() - self.line_width - gap;
        // White space that hangs whole, other space separators after
        // collapsible spaces say, goes on the line however full: at its
        // end it hangs, the spaces before it with it. So does an end edge
        // of no width after it.
        let hangs_whole = item.width() - item.hang() <= 0.0
            && (item.hang() > 0.0 || matches!(item, LineItem::Close(..)));
        // The word is alone where it starts the line, or is glued to
        // what is before it on a line with no opportunity to break: the
        // piece they make is the line's (`12345<span>67890</span>`).
        let alone =
            self.line.is_empty() || (!can_break && !self.line.iter().skip(1).any(|p| p.can_break));
        if let LineItem::Word(word) = &item
            && !hangs_whole
            && item.width() - item.hang() > room
            && word.style.white_space.wrap
            && let Some((head, tail)) = cut_word(word, room, alone, self.pass.fonts)
        {
            self.place(LineItem::Word(head), index, gap_before, can_break);
            let was_first = self.first.is_some();
            self.flush(false);
            let tail = self.settled(LineItem::Word(tail), index, was_first);
            self.place(tail, index, false, true);
            return;
        }
        // Greedy line breaking at a soft-wrap opportunity (M7.3): if
        // this item doesn't fit, start a new line — unless the line is
        // still empty (a solo word wider than `available_width` just
        // overflows; see the module doc's overflow-wrap note).
        if !self.line.is_empty()
            && !hangs_whole
            && self.line_width + gap + item.width() - item.hang() > self.room()
        {
            let was_first = self.first.is_some();
            if can_break {
                self.flush(false);
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
                self.flush(false);
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
            gap_style,
            can_break,
            lead,
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
        // Where the float's border box is in its margin box, which is
        // what the context places: the absolutely positioned boxes it
        // carries are from the border box.
        let (dx, dy) = atomic.relative;
        let border = (atomic.margin_left + dx, atomic.margin_top + dy);
        let absolutes = atomic.absolutes;
        if self.line.is_empty() {
            if self.resolved {
                let at = self.floats.place(float, self.line_top());
                self.carry(absolutes, at, border);
            } else {
                // Taken to be at the containing block's left on this line
                // until the flow position resolves and it is placed. Only a
                // float with nothing before it in the run waits here, which
                // in practice is a floated `::first-letter`, carrying none.
                let key = (float.node, float.generated);
                self.floats.defer(float);
                let from = self.absolutes.len();
                self.absolutes
                    .extend(carried(absolutes, border.0, self.y + border.1));
                for d in &mut self.absolutes[from..] {
                    d.float = Some(key);
                }
            }
        } else if self.line_width + float.w <= self.available_width {
            // Beside the line, which is measured again: no narrower
            // than the content on it, by the check above; or lower,
            // past other floats, where the room is as it was.
            let at = self.floats.place(float, self.line_top());
            self.carry(absolutes, at, border);
            self.measure_room();
        } else {
            self.deferred.push((float, absolutes, border));
        }
    }

    /// Takes in the absolutely positioned boxes a float carries, its
    /// margin box placed at `(x, y)` in the context's coordinates and its
    /// border box `border` into it.
    fn carry(&mut self, absolutes: Vec<Deferred<'a>>, (x, y): (f32, f32), border: (f32, f32)) {
        self.absolutes.extend(carried(
            absolutes,
            x - self.cb_left + border.0,
            y - self.top + border.1,
        ));
    }

    /// Places the floats that waited for the line just ended, at its
    /// bottom.
    fn place_deferred(&mut self) {
        let top = self.line_top();
        for (float, absolutes, border) in std::mem::take(&mut self.deferred) {
            let at = self.floats.place(float, top);
            self.carry(absolutes, at, border);
        }
    }

    /// Ends the line in progress, if there is one: whether a line was
    /// made of it. `ends` says the line is the run's last or ends at a
    /// forced break, which `text-align-last` aligns.
    fn flush(&mut self, ends: bool) -> bool {
        let first_line = self.first.as_ref().map(|first| first.chain);
        let top = self.y;
        let (width, pieces) = (
            self.line_width,
            1 + self
                .line
                .iter()
                .skip(1)
                .filter(|p| p.can_break && p.item.is_content())
                .count(),
        );
        let placement =
            self.alignment
                .place(&self.line, self.available_width - self.line_width, ends);
        let reach = decoration_reach(
            &self.line,
            |index| match self.item(index) {
                InlineItem::Word {
                    gap_decorations, ..
                }
                | InlineItem::Open {
                    gap_decorations, ..
                }
                | InlineItem::Atomic {
                    gap_decorations, ..
                } => *gap_decorations,
                _ => None,
            },
            placement.gap,
            self.pass.fonts,
        );
        let baseline = flush_line(
            &mut self.line,
            self.line_width,
            self.line_left,
            placement,
            (&self.strut, self.root),
            self.pass.fonts,
            (self.cb_width, self.cb_height),
            &mut self.y,
            &mut self.fragments,
            &mut self.absolutes,
            &mut self.open,
            first_line,
            &reach,
        );
        self.last_baseline = baseline.or(self.last_baseline);
        self.first_baseline = self.first_baseline.or(baseline);
        if baseline.is_some() {
            self.first_line = self.first_line.or(Some((top, self.y - top)));
        }
        self.line_width = 0.0;
        self.has_content = false;
        if baseline.is_some() {
            self.end_first_line();
            self.indent_next = false;
            if let Some(p) = self.counts.get_mut(self.paragraph) {
                *p = Paragraph {
                    lines: p.lines + 1,
                    before: p.last,
                    before_pieces: p.last_pieces,
                    last: width,
                    last_pieces: pieces,
                };
            }
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
            width += p.gap(fonts);
        }
        if has_content {
            width += p.lead;
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

/// `text-indent` as a run's lines take it (CSS Text 3 section 8.1).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Indent {
    /// How far, a percentage of the containing block's width.
    by: f32,
    hanging: bool,
    each_line: bool,
}

impl Indent {
    /// `style`'s `text-indent` in a containing block `basis` wide.
    pub(crate) fn of(style: &ComputedStyle, basis: f32) -> Indent {
        let indent = &style.text().text_indent;
        Indent {
            by: indent.length.resolve(basis),
            hanging: indent.hanging,
            each_line: indent.each_line,
        }
    }

    /// The indent of a line, `first` saying it is the first formatted
    /// line, or one after a forced break under `each-line`: those lines'
    /// alone, or with `hanging` every other's.
    fn of_line(self, first: bool) -> f32 {
        if first != self.hanging { self.by } else { 0.0 }
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
    dom::hook!("layout::inline::collect");
    let mut collector = Collector {
        fonts,
        first_line,
        done: false,
        out: Vec::new(),
        space_pending: false,
        space_decorations: None,
        space_style: None,
        gap_placed: false,
        at_word_start: true,
        glued: String::new(),
        glued_style: None,
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
    /// The text decorations over that white space (M21.7).
    space_decorations: Option<&'a Arc<[Decorating]>>,
    /// And its style, which the gap is a space of.
    space_style: Option<&'a Arc<ComputedStyle>>,
    /// An inline element's start edge took a gap and no content has
    /// followed it: white space seen now collapses into that gap
    /// (css-text-3 §4.1.1, across the boundary of the inline).
    gap_placed: bool,
    /// The next text starts a word: nothing came before it in the run,
    /// or white space or an atomic inline did. A word split across
    /// boxes — by an inline element, or a `::first-letter` (M16) — is
    /// one word to `text-transform: capitalize`.
    at_word_start: bool,
    /// The end of the last word's text, while nothing but the edges of
    /// inline elements came after it: what the next word's text is
    /// glued to, where UAX #14 may still break between the two; and
    /// that text's style.
    glued: String,
    glued_style: Option<&'a Arc<ComputedStyle>>,
}

impl<'a> Collector<'a, '_> {
    /// `b`'s style where the items go: on the first line, or its own.
    fn style_of(&self, b: &'a LayoutBox) -> &'a Arc<ComputedStyle> {
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
                    break_before,
                    ..
                } => reach.take(*width, style, !no_wrap && (*gap_before || *break_before)),
                InlineItem::Open {
                    style,
                    gap_before,
                    break_before,
                    ..
                } => reach.room.is_some_and(|room| {
                    let breaks = style.white_space.wrap && (*gap_before || *break_before);
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
                // Its width only its font finds, as an atomic's only its
                // layout does.
                InlineItem::Marker { .. } => {
                    reach.room = None;
                    false
                }
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
                    if gap_before {
                        self.glued.clear();
                    }
                    self.push(InlineItem::Open {
                        b,
                        style,
                        gap_before,
                        break_before: false,
                        gap_decorations: self.space_decorations.filter(|_| gap_before),
                        gap_style: self.space_style.filter(|_| gap_before),
                    });
                    // Collects nothing when the line ended at the start
                    // edge, and the end edge is past the end as well.
                    self.collect(&b.children);
                    if self.done {
                        return;
                    }
                    self.push(InlineItem::Close(style));
                }
                // An inline-block, or a table, a flex container or a grid
                // container in a run, which is an inline table (M12), an
                // inline flex container (M17) or an inline grid container
                // (M18): atomic on the line.
                BoxKind::InlineBlock | BoxKind::Table | BoxKind::Flex | BoxKind::Grid => {
                    let gap_before = std::mem::take(&mut self.space_pending);
                    self.gap_placed = false;
                    self.at_word_start = true;
                    self.glued.clear();
                    self.push(InlineItem::Atomic {
                        b,
                        gap_before,
                        gap_decorations: self.space_decorations.filter(|_| gap_before),
                        gap_style: self.space_style.filter(|_| gap_before),
                    });
                }
                // A float or an absolutely positioned box is invisible
                // to the white space around it: a gap pending before it
                // is still pending after it.
                BoxKind::Float => self.push(InlineItem::Float(b)),
                BoxKind::Absolute => self.push(InlineItem::Absolute(b)),
                // A list item's marker (M21.6): white space on either side
                // of it is as it would be without it, the marker being at
                // the start of the run.
                BoxKind::Marker { .. } => {
                    let style = self.style_of(b);
                    self.push(InlineItem::Marker { b, style });
                }
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
                | BoxKind::Caption
                | BoxKind::Legend => {}
            }
        }
    }

    fn text(&mut self, b: &'a LayoutBox, text: &str) {
        let style = self.style_of(b);
        let how = text_transform(style);
        let transformed = if how.is_none() {
            self.at_word_start = text::ends_word(text, self.at_word_start);
            Cow::Borrowed(text)
        } else {
            Cow::Owned(text::transform(text, how, &mut self.at_word_start))
        };
        // `white-space` is the box's own: `::first-line` does not apply it
        // (css-pseudo-4 section 2.3.1), and a `br`'s newline stays a break.
        let white_space = b.style.white_space;
        let no_wrap = !white_space.wrap;
        // Where newlines are kept they are forced breaks; elsewhere they
        // collapse away like any other white space.
        let preserves_newlines = white_space.preserves_breaks();
        // Where spaces are kept they are text, each a space wide (D55).
        let preserves_spaces = white_space.preserves_spaces();
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
                    self.glued.clear();
                    self.push(InlineItem::ForcedBreak(style));
                    self.space_pending = false;
                    self.gap_placed = false;
                    if self.done {
                        return;
                    }
                } else {
                    self.space(b);
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
            // Kept tabs are items of their own, each as wide as takes the
            // line to its next tab stop (CSS Text 3 section 4.2).
            if preserves_spaces && line.contains('\t') {
                self.tabbed(b, style, line, no_wrap);
                continue;
            }
            let reordered = text::bidi_reorder(line, Some(base_direction));
            // The line's soft wrap opportunities (UAX #14, D55), within
            // what is unbroken by collapsible white space.
            let mut breaks = if no_wrap {
                Vec::new()
            } else {
                text::line_breaks(&reordered, break_rules(style))
            };
            hyphenation(&reordered, style, &mut breaks);
            // Under `break-spaces` after every space too, and every other
            // space separator (CSS Text 3 section 5.1).
            if !no_wrap
                && style.white_space.collapse == style::computed::WhiteSpaceCollapse::BreakSpaces
            {
                breaks.extend(
                    reordered
                        .char_indices()
                        .filter(|&(i, c)| {
                            (c == ' ' || is_other_space(c)) && i + c.len_utf8() < reordered.len()
                        })
                        .map(|(i, c)| i + c.len_utf8()),
                );
                breaks.sort_unstable();
                breaks.dedup();
            }
            if preserves_spaces {
                // Spaces are text: the line is one word, spaces and all,
                // with its opportunities.
                let gap_before = std::mem::take(&mut self.space_pending);
                self.word(b, style, &reordered, no_wrap, gap_before, breaks);
                continue;
            }
            // A word runs from a non-space to the next space; it takes
            // the gap pending when it starts, and every space seen,
            // before, between or after, leaves one pending. It takes the
            // opportunities within it.
            let mut word: Option<(usize, bool)> = None;
            for (i, c) in reordered.char_indices() {
                if is_css_space(c) {
                    if let Some((start, gap_before)) = word.take() {
                        self.words(
                            b,
                            style,
                            (&reordered, start..i),
                            &breaks,
                            no_wrap,
                            gap_before,
                        );
                        if self.done {
                            return;
                        }
                    }
                    self.space(b);
                } else if word.is_none() && !is_bidi_control(c) {
                    // A bidi formatting character starts no word: the
                    // white space around it collapses as if it were not
                    // there (CSS Text 3 section 4.1.1).
                    word = Some((i, std::mem::take(&mut self.space_pending)));
                }
            }
            if let Some((start, gap_before)) = word {
                let end = reordered.len();
                self.words(
                    b,
                    style,
                    (&reordered, start..end),
                    &breaks,
                    no_wrap,
                    gap_before,
                );
            }
        }
    }

    /// `line`, a line of text whose spaces are kept, with the tabs in it:
    /// the text between them words, each tab an item, a soft wrap
    /// opportunity after it where lines wrap.
    fn tabbed(
        &mut self,
        b: &'a LayoutBox,
        style: &'a Arc<ComputedStyle>,
        line: &str,
        no_wrap: bool,
    ) {
        let base_direction = to_text_direction(style.direction);
        for (i, part) in line.split('\t').enumerate() {
            if i > 0 {
                let gap_before = std::mem::take(&mut self.space_pending);
                self.gap_placed = false;
                self.glued.clear();
                self.push(InlineItem::Word {
                    text: "\t".to_owned(),
                    width: 0.0,
                    hang: 0.0,
                    tab: true,
                    lead: 0.0,
                    style,
                    no_wrap,
                    gap_before,
                    break_before: false,
                    breaks: Vec::new(),
                    node: b.node,
                    generated: b.generated,
                    decorations: b.decorations.as_ref(),
                    gap_decorations: self.space_decorations.filter(|_| gap_before),
                    gap_style: self.space_style.filter(|_| gap_before),
                });
                if self.done {
                    return;
                }
            }
            if part.is_empty() {
                continue;
            }
            let reordered = text::bidi_reorder(part, Some(base_direction));
            let mut breaks = if no_wrap {
                Vec::new()
            } else {
                text::line_breaks(&reordered, break_rules(style))
            };
            hyphenation(&reordered, style, &mut breaks);
            let gap_before = std::mem::take(&mut self.space_pending);
            let after_tab = i > 0 && !no_wrap;
            self.word(b, style, &reordered, no_wrap, gap_before, breaks);
            if after_tab && let Some(InlineItem::Word { break_before, .. }) = self.out.last_mut() {
                *break_before = true;
            }
            if self.done {
                return;
            }
        }
    }

    /// The word `range` of `line`, with the opportunities in `breaks`
    /// within it.
    fn words(
        &mut self,
        b: &'a LayoutBox,
        style: &'a Arc<ComputedStyle>,
        (line, range): (&str, std::ops::Range<usize>),
        breaks: &[usize],
        no_wrap: bool,
        gap_before: bool,
    ) {
        let within = breaks
            .iter()
            .filter(|&&at| at > range.start && at < range.end)
            .map(|at| at - range.start)
            .collect();
        self.word(b, style, &line[range], no_wrap, gap_before, within);
    }

    /// The word `text`, with the opportunities `breaks` in it: in pieces
    /// where `text-autospace` puts space between an ideograph and a
    /// letter or digit (CSS Text 4 section 8.1), each but the first led
    /// by an eighth of an em; the first too where the text it is glued
    /// to, in another element, ends with a character that the space
    /// goes after, element boundaries being no boundaries to it
    /// (section 1.5), where both elements' `text-autospace` puts it.
    fn word(
        &mut self,
        b: &'a LayoutBox,
        style: &'a Arc<ComputedStyle>,
        text: &str,
        no_wrap: bool,
        gap_before: bool,
        breaks: Vec<usize>,
    ) {
        let spaced = autospaced(text, style);
        let boundary = match (self.glued.chars().next_back(), text.chars().next()) {
            (Some(before), Some(after)) => self.glued_style.is_some_and(|glued| {
                autospaced_pair(before, after, glued) && autospaced_pair(before, after, style)
            }),
            _ => false,
        };
        if spaced.is_empty() && !boundary {
            self.word_piece(b, style, text, no_wrap, gap_before, breaks, 0.0);
            return;
        }
        let space = style.font_size_px / 8.0;
        let mut start = 0;
        for (n, end) in spaced.iter().copied().chain([text.len()]).enumerate() {
            let within = breaks
                .iter()
                .filter(|&&at| at > start && at < end)
                .map(|at| at - start)
                .collect();
            let lead = if n > 0 || boundary { space } else { 0.0 };
            let gap = gap_before && n == 0;
            self.word_piece(b, style, &text[start..end], no_wrap, gap, within, lead);
            if self.done {
                return;
            }
            start = end;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn word_piece(
        &mut self,
        b: &'a LayoutBox,
        style: &'a Arc<ComputedStyle>,
        text: &str,
        no_wrap: bool,
        gap_before: bool,
        breaks: Vec<usize>,
        lead: f32,
    ) {
        self.gap_placed = false;
        let break_before = !gap_before && !no_wrap && self.breaks_after_glued(text, style);
        self.glued.clear();
        let tail = text.char_indices().rev().nth(7).map_or(0, |(i, _)| i);
        self.glued.push_str(&text[tail..]);
        self.glued_style = Some(style);
        self.push(InlineItem::Word {
            text: text.to_string(),
            width: measure_word(text, style, self.fonts),
            hang: hang_of(text, style, self.fonts),
            tab: false,
            lead,
            style,
            no_wrap,
            gap_before,
            break_before,
            breaks,
            node: b.node,
            generated: b.generated,
            decorations: b.decorations.as_ref(),
            gap_decorations: self.space_decorations.filter(|_| gap_before),
            gap_style: self.space_style.filter(|_| gap_before),
        });
    }

    /// Whether a line may break between the text the next word is glued
    /// to, before it in another box, and `text`, the word's, by UAX #14
    /// under `style`'s rules, or the glued text's (a `break-all` box's
    /// last letter breaks from what follows it, as the reference browser
    /// has it). Where it may, the inline elements opened
    /// between the two take the opportunity, before their start edges,
    /// and the word is glued to them.
    fn breaks_after_glued(&mut self, text: &str, style: &ComputedStyle) -> bool {
        if self.glued.is_empty() || text.is_empty() {
            return false;
        }
        let head = text.char_indices().nth(8).map_or(text.len(), |(i, _)| i);
        let joined = format!("{}{}", self.glued, &text[..head]);
        // Under `break-spaces` after any preserved space too.
        let after_space = style.white_space.collapse
            == style::computed::WhiteSpaceCollapse::BreakSpaces
            && self.glued.ends_with(|c| c == ' ' || is_other_space(c));
        let at = self.glued.len();
        let breaks = |rules| text::line_breaks(&joined, rules).contains(&at);
        let rules = break_rules(style);
        let glued_rules = self
            .glued_style
            .map(|glued| break_rules(glued))
            .filter(|glued| *glued != rules);
        if !after_space && !breaks(rules) && !glued_rules.is_some_and(breaks) {
            return false;
        }
        let opened = self
            .out
            .iter()
            .rev()
            .take_while(|item| matches!(item, InlineItem::Open { .. }))
            .count();
        if opened == 0 {
            return true;
        }
        let first = self.out.len() - opened;
        if let InlineItem::Open { break_before, .. } = &mut self.out[first] {
            *break_before = true;
        }
        false
    }

    /// White space met in the text box `b`: a gap pending unless one is
    /// placed, the first of a collapsed run's spaces being the one kept
    /// and so the one whose element's decorations (M21.7) and style it
    /// has.
    fn space(&mut self, b: &'a LayoutBox) {
        self.glued.clear();
        let pending = !self.gap_placed;
        if pending && !self.space_pending {
            self.space_decorations = b.decorations.as_ref();
            self.space_style = Some(self.style_of(b));
        }
        self.space_pending = pending;
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
pub(crate) fn run_intrinsic_widths<'a>(
    items: &[InlineItem<'a>],
    first: Option<&[InlineItem<'a>]>,
    pass: &Pass<'a>,
) -> (f32, f32) {
    run_intrinsic_widths_indented(items, first, pass, Indent::default())
}

/// [`run_intrinsic_widths`] of a run whose first line is indented by
/// `indent`, or whose other lines are (`text-indent`): the line and the
/// piece it starts with are that much wider.
pub(crate) fn run_intrinsic_widths_indented<'a>(
    items: &[InlineItem<'a>],
    first: Option<&[InlineItem<'a>]>,
    pass: &Pass<'a>,
    indent: Indent,
) -> (f32, f32) {
    let fonts = pass.fonts;
    let first = first.filter(|first| is_prefix(first, items));
    let mut min = 0.0f32;
    let mut max = 0.0f32;
    // The line being measured, and the unbreakable piece at its end.
    let mut line = indent.of_line(true);
    let mut piece = line;
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
                gap_style,
                break_before,
                ..
            } => {
                !line_empty
                    && breaks_before(
                        *no_wrap,
                        *gap_before,
                        *gap_style,
                        *break_before || after_atomic,
                    )
            }
            InlineItem::Open {
                style,
                gap_before,
                break_before,
                ..
            } => {
                style.white_space.wrap
                    && !line_empty
                    && (*gap_before || *break_before || after_atomic)
            }
            InlineItem::Atomic { b, .. } => !line_empty && b.style.white_space.wrap,
            InlineItem::Marker { b, .. } => {
                !line_empty && matches!(b.kind, BoxKind::Marker { outside: false, .. })
            }
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
                    text: piece_text,
                    width: piece_width,
                    style: piece_style,
                    breaks: piece_breaks,
                    tab,
                    ..
                },
            ) => {
                // A tab is as wide as it takes the line to its next stop.
                let tab_width = tab.then(|| tab_advance(piece_style, line, fonts));
                let lead = if line_empty { 0.0 } else { in_line.lead() };
                let piece_width = &tab_width.unwrap_or(*piece_width);
                let width = &tab_width.unwrap_or(*width);
                // A space of the white space's own style.
                let gap = |item: &InlineItem<'_>, style| {
                    if line_empty || !gap_before {
                        0.0
                    } else {
                        space_width(item.gap_style().unwrap_or(style), fonts)
                    }
                };
                // `overflow-wrap: anywhere` breaks a word at any letter
                // for the min-content width too (CSS Text 3 section 5.5):
                // its widest typographic character unit is the piece.
                if min_content_anywhere(piece_style) && piece_style.white_space.wrap {
                    let ends = text::grapheme_boundaries(piece_text);
                    let mut start = 0;
                    let widest = ends
                        .into_iter()
                        .chain([piece_text.len()])
                        .map(|end| {
                            let unit = &piece_text[start..end];
                            start = end;
                            measure_word(unit, piece_style, fonts)
                        })
                        .fold(0.0f32, f32::max);
                    min = min.max(piece);
                    piece = widest;
                } else if !piece_breaks.is_empty() && piece_style.white_space.wrap {
                    // Its opportunities (D55): the first segment ends the
                    // piece before it, each one after is a piece, the last
                    // goes on; spaces hanging at a segment's end count
                    // for none.
                    let ends = piece_breaks.iter().copied().chain([piece_text.len()]);
                    let mut start = 0;
                    for (n, end) in ends.enumerate() {
                        let segment = &piece_text[start..end];
                        let hyphen = if end < piece_text.len()
                            && hyphenates_at(piece_text, end, piece_style)
                        {
                            measure_word(hyphen_of(piece_style), piece_style, fonts)
                        } else {
                            0.0
                        };
                        let width = measure_word(segment, piece_style, fonts) + hyphen
                            - hang_of(segment, piece_style, fonts);
                        if n == 0 && !starts_piece {
                            piece += gap(in_piece, piece_style) + lead + width;
                        } else {
                            piece = width;
                        }
                        min = min.max(piece);
                        start = end;
                    }
                } else {
                    piece = if starts_piece {
                        *piece_width
                    } else {
                        piece + gap(in_piece, piece_style) + lead + piece_width
                    };
                }
                line += gap(in_line, style) + lead + width;
                line_empty = false;
                after_atomic = false;
            }
            // An atomic inline keeps its style on the first line.
            (InlineItem::Atomic { b, gap_before, .. }, _) => {
                let (piece_min, piece_max) = crate::intrinsic::contribution(b, pass);
                let gap = if line_empty || !gap_before {
                    0.0
                } else {
                    space_width(in_line.gap_style().unwrap_or(&b.style), fonts)
                };
                // Where no line may break before it (`nowrap`), it goes on
                // the piece before.
                piece = if starts_piece || line_empty {
                    piece_min
                } else {
                    piece + gap + piece_min
                };
                line += gap + piece_max;
                line_empty = false;
                after_atomic = true;
            }
            (InlineItem::Float(b), _) => {
                let (float_min, float_max) = crate::intrinsic::contribution(b, pass);
                min = min.max(float_min);
                line += float_max;
            }
            // A list item's marker (M21.6): an outside one takes no room,
            // an inside shape its own and that after it, as an atomic.
            (InlineItem::Marker { b, style }, _) => {
                let marker = Marker::of(b, style, fonts);
                if !marker.is_outside() {
                    piece = marker.advance;
                    line += marker.advance;
                    line_empty = false;
                    after_atomic = true;
                }
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
                let gap = |item: &InlineItem<'_>, style| {
                    if line_empty || !gap_before {
                        0.0
                    } else {
                        space_width(item.gap_style().unwrap_or(style), fonts)
                    }
                };
                let (edge, _) = edge_widths(piece_style, 0.0);
                piece = if starts_piece {
                    edge
                } else {
                    piece + gap(in_piece, piece_style) + edge
                };
                let (edge, _) = edge_widths(style, 0.0);
                line += gap(in_line, style) + edge;
                after_atomic = false;
            }
            (InlineItem::Close(style), InlineItem::Close(piece_style)) => {
                piece += edge_widths(piece_style, 0.0).1;
                line += edge_widths(style, 0.0).1;
            }
            (InlineItem::ForcedBreak(_), _) => {
                line = indent.of_line(indent.each_line);
                piece = line;
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

/// The byte offsets in `text` where `style`'s `text-autospace` puts space
/// (CSS Text 4 section 8.1): between an ideograph (Han, kana, bopomofo)
/// and a letter of another script or a digit, either way round, as
/// `normal`, the initial value, has it, which the reference browser
/// ships; none for ASCII text, the common case.
fn autospaced(text: &str, style: &ComputedStyle) -> Vec<usize> {
    if text.is_ascii() || autospace_classes(style).is_none() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut previous: Option<char> = None;
    for (i, c) in text.char_indices() {
        if let Some(p) = previous
            && autospaced_pair(p, c, style)
        {
            out.push(i);
        }
        previous = Some(c);
    }
    out
}

/// Whether `style`'s `text-autospace` puts space between ideographs and
/// letters, and between ideographs and digits; `None` for neither.
fn autospace_classes(style: &ComputedStyle) -> Option<(bool, bool)> {
    dom::hook!("layout::inline::autospace_classes");
    use style::computed::TextAutospace;
    match style.text().text_autospace {
        TextAutospace::Normal => Some((true, true)),
        TextAutospace::NoAutospace => None,
        TextAutospace::Spaces {
            ideograph_alpha,
            ideograph_numeric,
            ..
        } => (ideograph_alpha || ideograph_numeric).then_some((ideograph_alpha, ideograph_numeric)),
    }
}

/// Whether `style`'s `text-autospace` puts space between `before` and
/// `after`: an ideograph and a letter or a digit of no ideographic
/// script, and not fullwidth, either way round.
fn autospaced_pair(before: char, after: char, style: &ComputedStyle) -> bool {
    if before.is_ascii() && after.is_ascii() {
        return false;
    }
    let Some((alpha, numeric)) = autospace_classes(style) else {
        return false;
    };
    let other = |c: char| {
        !is_ideograph(c)
            && !matches!(c, '\u{1100}'..='\u{11ff}' | '\u{ac00}'..='\u{d7af}' | '\u{ff00}'..='\u{ffef}')
            && ((alpha && c.is_alphabetic()) || (numeric && c.is_numeric()))
    };
    (is_ideograph(before) && other(after)) || (other(before) && is_ideograph(after))
}

/// Whether a line may break before a word: at the white space before
/// it, as that white space's own `white-space` says (CSS Text 3 section
/// 5.1), else at an `opportunity` without white space where the word's
/// lines wrap (not `no_wrap`).
fn breaks_before(
    no_wrap: bool,
    gap_before: bool,
    gap_style: Option<&Arc<ComputedStyle>>,
    opportunity: bool,
) -> bool {
    (gap_before && gap_style.map_or(!no_wrap, |gap| gap.white_space.wrap))
        || (!no_wrap && opportunity)
}

/// Whether `c` is an ideograph, a kana or a bopomofo letter: the
/// characters of the scripts that set no spaces between words.
fn is_ideograph(c: char) -> bool {
    matches!(c,
        '\u{2e80}'..='\u{2fdf}'
        | '\u{3005}'..='\u{3007}'
        | '\u{3021}'..='\u{3029}'
        | '\u{3031}'..='\u{3035}'
        | '\u{3038}'..='\u{303c}'
        | '\u{3040}'..='\u{30ff}'
        | '\u{3100}'..='\u{312f}'
        | '\u{31a0}'..='\u{31ff}'
        | '\u{3400}'..='\u{4dbf}'
        | '\u{4e00}'..='\u{9fff}'
        | '\u{f900}'..='\u{faff}'
        | '\u{20000}'..='\u{3ffff}')
}

/// Whether `text` is ideographs, format characters aside: a word that
/// `text-justify: auto` spreads between its characters.
fn of_ideographs(text: &str) -> bool {
    text.chars()
        .all(|c| is_ideograph(c) || is_zero_width(c) || is_bidi_control(c))
}

/// Whether `c` is a bidi formatting character: the marks, embeddings,
/// overrides and isolates, which take no room.
fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{200e}' | '\u{200f}' | '\u{61c}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

/// css-text-3 §4.1's collapsible white space: space, tab, line feed,
/// carriage return; not Unicode's `White_Space`, which would make a
/// no-break space breakable and collapsible. A form feed is a control
/// character, drawn as one, as the reference browser has it.
pub(crate) fn is_css_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// `style::computed::Direction` → `text::Direction` (M8.3): `text` sits
/// below `style` in the layer stack (see `docs/architecture.md`) and so
/// can't name `style`'s type itself — see that enum's own doc in
/// `text::lib` for the other half of this note. This is the boundary
/// conversion.
pub(crate) fn to_text_direction(direction: Direction) -> text::Direction {
    match direction {
        Direction::Ltr => text::Direction::Ltr,
        Direction::Rtl => text::Direction::Rtl,
    }
}

/// `style`'s `text-transform` (M8.4, gap batch D), tailored by its
/// content language.
fn text_transform(style: &ComputedStyle) -> text::Transform<'_> {
    let props = style.text();
    text::Transform {
        case: match style.text_transform {
            TextTransform::None => text::Case::None,
            TextTransform::Uppercase => text::Case::Upper,
            TextTransform::Lowercase => text::Case::Lower,
            TextTransform::Capitalize => text::Case::Capitalize,
        },
        full_width: props.transform_extra.full_width,
        full_size_kana: props.transform_extra.full_size_kana,
        language: props.lang.as_deref().unwrap_or_default(),
    }
}

/// The width of a tab `x` from the line's start (CSS Text 3 section
/// 4.2): to the next tab stop, `tab-size` apart, a number of them as
/// many spaces with their `letter-spacing` and `word-spacing`; to the one
/// after where the next is nearer than half a space. A tab size of zero
/// makes a tab of none.
fn tab_advance(style: &ComputedStyle, x: f32, fonts: &FontCollection) -> f32 {
    let space = font::measure(fonts, " ", style);
    let stop = match style.text().tab_size {
        style::computed::TabSize::Spaces(n) => {
            n * (space + style.letter_spacing + style.word_spacing)
        }
        style::computed::TabSize::Length(px) => px,
    };
    if stop.is_nan() || stop <= 0.0 {
        return 0.0;
    }
    let mut next = ((x / stop).floor() + 1.0) * stop;
    if next - x < space / 2.0 {
        next += stop;
    }
    next - x
}

/// Whether preserved white space at a line's end hangs past it there:
/// where spaces are kept but under `break-spaces`.
fn hangs_spaces(style: &ComputedStyle) -> bool {
    style.white_space.preserves_spaces()
        && style.white_space.collapse != style::computed::WhiteSpaceCollapse::BreakSpaces
}

/// The opportunities `style`'s `word-break` and `line-break` give
/// (D55), the latter tailored to Chinese and Japanese by the content
/// language.
fn break_rules(style: &ComputedStyle) -> text::BreakRules {
    use style::computed::LineBreak;
    let props = style.text();
    let words = match style.word_break {
        style::computed::WordBreak::BreakAll => text::WordBreaking::BreakAll,
        style::computed::WordBreak::KeepAll => text::WordBreaking::KeepAll,
        _ => text::WordBreaking::Normal,
    };
    let strictness = match props.line_break {
        LineBreak::Auto | LineBreak::Strict => text::Strictness::Strict,
        LineBreak::Normal => text::Strictness::Normal,
        LineBreak::Loose => text::Strictness::Loose,
        LineBreak::Anywhere => text::Strictness::Anywhere,
    };
    let language = props.lang.as_deref().unwrap_or_default();
    let chinese_or_japanese = ["ja", "zh"].iter().any(|code| {
        language
            .strip_prefix(code)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
    });
    text::BreakRules {
        words,
        strictness,
        chinese_or_japanese,
    }
}

/// Whether `style` breaks a word anywhere for the min-content width too:
/// `overflow-wrap: anywhere`, or `word-break: break-word`, which is
/// `normal` with it (CSS Text 3 section 5.2).
fn min_content_anywhere(style: &ComputedStyle) -> bool {
    style.overflow_wrap == style::computed::OverflowWrap::Anywhere
        || style.word_break == style::computed::WordBreak::BreakWord
}

/// Whether `style` lets a word too long for its line break anywhere:
/// `overflow-wrap: anywhere` or `break-word`, or `word-break:
/// break-word`.
fn breaks_anywhere(style: &ComputedStyle) -> bool {
    style.overflow_wrap != style::computed::OverflowWrap::Normal
        || style.word_break == style::computed::WordBreak::BreakWord
}

/// `text`'s soft wrap opportunities `breaks` as `hyphens` has them (CSS
/// Text 3 section 5.4): none after a soft hyphen under `none`; under
/// `auto`, in a language with patterns, the hyphenation opportunities of
/// its words too.
fn hyphenation(text: &str, style: &ComputedStyle, breaks: &mut Vec<usize>) {
    use style::computed::Hyphens;
    let props = style.text();
    match props.hyphens {
        Hyphens::None if text.contains('\u{ad}') => {
            breaks.retain(|&at| !text[..at].ends_with('\u{ad}'));
        }
        Hyphens::Auto if style.white_space.wrap => {
            let language = props.lang.as_deref().unwrap_or_default();
            if !text::hyphenates(language) {
                return;
            }
            let [word, before, after] = props.hyphenate_limit_chars;
            let limit = |v: Option<i32>| v.map(|n| n.max(0) as u32);
            let limits = text::HyphenLimits {
                word: limit(word),
                before: limit(before),
                after: limit(after),
            };
            let mut start = None;
            for (i, c) in text.char_indices().chain([(text.len(), ' ')]) {
                match (is_hyphenable(c), start) {
                    (true, None) => start = Some(i),
                    (false, Some(from)) => {
                        let points = text::hyphenation_points(&text[from..i], language, limits);
                        breaks.extend(points.into_iter().map(|at| from + at));
                        start = None;
                    }
                    _ => {}
                }
            }
            breaks.sort_unstable();
            breaks.dedup();
        }
        _ => {}
    }
}

/// Whether `c` is a letter of a word `hyphens: auto` may hyphenate: a
/// letter of an alphabet, not an ideograph or a kana.
fn is_hyphenable(c: char) -> bool {
    c.is_alphabetic() && (c as u32) < 0x2e80
}

/// Whether a line broken at byte `at` of `text`, in `style`, ends with a
/// hyphen: after a soft hyphen, or between two letters where only
/// `hyphens: auto` breaks.
fn hyphenates_at(text: &str, at: usize, style: &ComputedStyle) -> bool {
    use style::computed::Hyphens;
    let hyphens = style.text().hyphens;
    let before = text[..at].chars().next_back();
    if before == Some('\u{ad}') {
        return hyphens != Hyphens::None;
    }
    hyphens == Hyphens::Auto
        && style.word_break == style::computed::WordBreak::Normal
        && style.text().line_break != style::computed::LineBreak::Anywhere
        && before.is_some_and(is_hyphenable)
        && text[at..].chars().next().is_some_and(is_hyphenable)
}

/// The hyphen a hyphenated line ends with: `hyphenate-character`'s, or
/// for `auto` the hyphen, U+2010.
fn hyphen_of(style: &ComputedStyle) -> &str {
    style
        .text()
        .hyphenate_character
        .as_deref()
        .unwrap_or("\u{2010}")
}

/// How much of `text`'s width is white space at its end, which hangs
/// past a line's end (CSS Text 3 section 4.1.3): preserved spaces and
/// tabs, and the other space separators whether preserved or not; none
/// under `break-spaces`, where they wrap.
fn hang_of(text: &str, style: &ComputedStyle, fonts: &FontCollection) -> f32 {
    if style.white_space.collapse == style::computed::WhiteSpaceCollapse::BreakSpaces {
        return 0.0;
    }
    let kept = if style.white_space.preserves_spaces() {
        text.trim_end_matches(|c| c == ' ' || c == '\t' || is_other_space(c))
    } else {
        text.trim_end_matches(is_other_space)
    };
    if kept.len() == text.len() {
        return 0.0;
    }
    measure_word(text, style, fonts) - measure_word(kept, style, fonts)
}

/// Whether `c` is one of the other space separators a line may break
/// after (CSS Text 3 section 4.1.3; UAX #14's class BA): the ogham space
/// mark, the en and em spaces and their kin, and the ideographic space.
/// Unlike the space they never collapse; at a line's end they hang.
fn is_other_space(c: char) -> bool {
    matches!(
        c,
        '\u{1680}' | '\u{2000}'..='\u{2006}' | '\u{2008}'..='\u{200a}' | '\u{205f}' | '\u{3000}'
    )
}

/// `word` cut for a line with `room` left (D55): at its last opportunity
/// whose head fits; alone on its line (`alone`), where `overflow-wrap`
/// lets it, else at its first opportunity. `None` where it is not cut.
fn cut_word<'i>(
    word: &LineWord<'i>,
    room: f32,
    alone: bool,
    fonts: &FontCollection,
) -> Option<(LineWord<'i>, LineWord<'i>)> {
    let end = word.offset + word.text.len();
    let within: Vec<usize> = word
        .breaks
        .iter()
        .copied()
        .filter(|&at| at > word.offset && at < end)
        .map(|at| at - word.offset)
        .collect();
    let fits = within.iter().rev().copied().find(|&at| {
        let head = &word.text[..at];
        let hyphen = if hyphenates_at(word.text, at, word.style) {
            measure_word(hyphen_of(word.style), word.style, fonts)
        } else {
            0.0
        };
        // A head that hangs whole fits a line however full.
        let width =
            measure_word(head, word.style, fonts) + hyphen - hang_of(head, word.style, fonts);
        width <= room || width <= 0.0
    });
    let at = match fits {
        Some(at) => at,
        None if !alone => return None,
        None if breaks_anywhere(word.style) => return split_to_fit(word, room, fonts),
        None => *within.first()?,
    };
    Some(split_at(word, at, fonts))
}

/// `word` in two at byte `at` of its text.
fn split_at<'i>(
    word: &LineWord<'i>,
    at: usize,
    fonts: &FontCollection,
) -> (LineWord<'i>, LineWord<'i>) {
    let (head, tail) = word.text.split_at(at);
    let piece = |text: &'i str, offset: usize| LineWord {
        text,
        width: measure_word(text, word.style, fonts),
        hang: hang_of(text, word.style, fonts),
        offset,
        hyphen: false,
        ..*word
    };
    let mut head = piece(head, word.offset);
    if hyphenates_at(word.text, at, word.style) {
        head.hyphen = true;
        head.width += measure_word(hyphen_of(word.style), word.style, fonts);
    }
    (head, piece(tail, word.offset + at))
}

/// `word` cut where it reaches `room`: the typographic character units
/// (grapheme clusters) that fit, at least one, and the rest; `None` for
/// a word of one.
fn split_to_fit<'i>(
    word: &LineWord<'i>,
    room: f32,
    fonts: &FontCollection,
) -> Option<(LineWord<'i>, LineWord<'i>)> {
    let ends = text::grapheme_boundaries(word.text);
    let first = *ends.first()?;
    let mut cut = None;
    for end in ends {
        if measure_word(&word.text[..end], word.style, fonts) > room {
            break;
        }
        cut = Some(end);
    }
    Some(split_at(word, cut.unwrap_or(first), fonts))
}

/// A space's width plus `word-spacing` (M8.4), the gap between two
/// words on the same line, and `letter-spacing`, which the space takes
/// as any character does.
pub(crate) fn space_width(style: &ComputedStyle, fonts: &FontCollection) -> f32 {
    font::measure(fonts, " ", style) + style.word_spacing + style.letter_spacing
}

/// The units `letter-spacing` spaces in `text` (CSS Text 3 section 8.2),
/// as the reference browser has them: its grapheme clusters, the letters
/// of a cursive script kept joined in one, each with whether spacing
/// follows it: none after a cursive script's letters, nor after a
/// zero-width format character.
fn letter_units(text: &str) -> Vec<(std::ops::Range<usize>, bool)> {
    if text.is_ascii() {
        return (0..text.len()).map(|i| (i..i + 1, true)).collect();
    }
    let mut units: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
    let mut start = 0;
    for end in text::grapheme_boundaries(text)
        .into_iter()
        .chain([text.len()])
    {
        let unit = &text[start..end];
        let first = unit.chars().next().unwrap_or(' ');
        let cursive = is_cursive(first);
        match units.last_mut() {
            // Joined to the cursive letter before it.
            Some((range, false))
                if cursive && text[range.clone()].chars().next().is_some_and(is_cursive) =>
            {
                range.end = end;
            }
            _ => units.push((start..end, !cursive && !unit.chars().all(is_zero_width))),
        }
        start = end;
    }
    units
}

/// Whether `c` is a letter of a script whose letters join (Arabic,
/// Syriac, N'Ko, Mandaic, Mongolian, Phags-pa, Adlam and their kin),
/// which `letter-spacing` leaves joined.
fn is_cursive(c: char) -> bool {
    matches!(c,
        '\u{600}'..='\u{8ff}'
        | '\u{1800}'..='\u{18af}'
        | '\u{a840}'..='\u{a87f}'
        | '\u{fb50}'..='\u{fdff}'
        | '\u{fe70}'..='\u{fefe}'
        | '\u{10d00}'..='\u{10d3f}'
        | '\u{10f30}'..='\u{10fff}'
        | '\u{1e900}'..='\u{1e95f}')
}

/// Whether `c` is a format character of no width, which takes no
/// `letter-spacing`: the zero-width space, joiners, marks and the like.
fn is_zero_width(c: char) -> bool {
    matches!(
        c,
        '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}'
    )
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
    dom::hook!("layout::inline::measure_word");
    let font = font::font_for(fonts, style, word);
    if style.letter_spacing == 0.0 {
        return fonts.measure_in(font, word, style.font_size_px);
    }
    letter_units(word)
        .into_iter()
        .map(|(range, spaced)| {
            fonts.measure_in(font, &word[range], style.font_size_px)
                + if spaced { style.letter_spacing } else { 0.0 }
        })
        .sum()
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
    placement: Placement,
    (strut, root): (&FontMetrics, &ComputedStyle),
    fonts: &FontCollection,
    cb: (f32, Option<f32>),
    y: &mut f32,
    fragments: &mut Vec<Fragment>,
    absolutes: &mut Vec<Deferred<'a>>,
    open: &mut Vec<&'a LayoutBox>,
    first_line: Option<&'a [LayoutBox]>,
    reach: &[Spread],
) -> Option<f32> {
    dom::hook!("layout::inline::flush_line");
    if line.is_empty() {
        return None;
    }
    let makes_line = line_width > 0.0 || line.iter().any(|p| p.item.is_content());

    // Above the baseline: the tallest ascent. Below it: the deepest
    // extent: a text's descent and its line gap under it (the gap is
    // all below the baseline in this engine's `line-height: normal`,
    // not split as half-leading), an inline-block's margin box past its
    // baseline, which is where an inline-block's own last line gap
    // already is, so it is not added again under it. An inline box's
    // edges add nothing: its vertical padding and border are drawn
    // without changing the line's height (CSS2 section 10.8). Each item
    // is where `vertical-align` puts it (section 10.8.1): its baseline
    // shifted from its parent inline box's, the root inline box's or,
    // in a subtree aligned to the line's top or bottom, that subtree's
    // root's, which is placed once the line's height is known.
    let mut memo = LineMemo::default();
    let mut max_descent = strut.descent;
    let mut extents = Extents {
        root: (strut.ascent, strut.descent + strut.line_gap),
        groups: Vec::new(),
    };
    // A line no inline box or atomic inline is on, as most are, has
    // every item at the root inline box's baseline and no box decorating
    // its words: nothing is aligned by the root, nor kept for each item
    // (G247), measuring the root's font and the allocations costing more
    // than the rest of the line.
    let plain = open.is_empty()
        && !line
            .iter()
            .any(|p| matches!(p.item, LineItem::Open(..) | LineItem::Atomic(_)));
    // The root inline box's frame, and those of the inline boxes open
    // in it, innermost last: none on a plain line, nothing allocated.
    let root_frame = Aligned {
        at: (None, 0.0),
        parent: if plain {
            Parent::default()
        } else {
            Parent::of(fonts, root)
        },
    };
    let mut frames: Vec<Aligned> = Vec::new();
    let top = |frames: &[Aligned]| frames.last().copied().unwrap_or(root_frame);
    // The places of the open inline elements that decorate their text,
    // outermost first, and whether each open one is among them: a text's
    // lines are drawn where its decorating boxes are (CSS Text Decoration
    // 3 section 2.1), not where `vertical-align` moved the text.
    let mut decorating = Decorators::default();
    // The inline elements continued from the line before, as they are
    // placed again on this one.
    let mut continued = Vec::with_capacity(open.len());
    for b in open.iter() {
        let style = match &b.first_line {
            Some(line) if first_line.is_some() => &line.style,
            _ => &b.style,
        };
        let frame = extents.enter(style, top(&frames), fonts);
        continued.push(frame.at);
        frames.push(frame);
        decorating.open(style, frame.at);
    }
    // Each item's place: its subtree and its baseline's shift in it; and
    // a word's decorating boxes' places, none kept for a plain line.
    let kept = if plain { 0 } else { line.len() };
    let mut places = Vec::with_capacity(kept);
    let mut decorated_at = Vec::with_capacity(kept);
    for placed in line.iter() {
        let frame = top(&frames);
        if !plain {
            decorated_at.push(match &placed.item {
                LineItem::Word(_) => decorating.places(),
                _ => None,
            });
        }
        let at = match &placed.item {
            LineItem::Word(word) => {
                // Per-word, not just per-size (M8.1): a word's own
                // script may resolve to a different fallback font than
                // another word on the same line, with its own vertical
                // metrics; folding in `metrics_for_text` (keyed off
                // this word's actual text, not just its `font_size_px`)
                // extends the existing "shared baseline sits at the
                // tallest metrics present" rule (M7.4) to cover that,
                // not just mixed sizes.
                let m = memo.metrics(fonts, word.text, word.style);
                max_descent = max_descent.max(m.descent);
                extents.extend(frame.at, m.ascent, m.descent + m.line_gap);
                frame.at
            }
            LineItem::Atomic(a) => {
                let (above, below) = (a.ascent, a.height - a.ascent);
                let at = extents.align(&a.root.style, above, below, frame, fonts);
                extents.extend(at, above, below);
                at
            }
            // A marker is on the line as an inline box of its font is.
            LineItem::Marker(m) => {
                max_descent = max_descent.max(m.metrics.descent);
                extents.extend(
                    frame.at,
                    m.metrics.ascent,
                    m.metrics.descent + m.metrics.line_gap,
                );
                frame.at
            }
            LineItem::Open(_, style, _) => {
                let inner = extents.enter(style, frame, fonts);
                frames.push(inner);
                decorating.open(style, inner.at);
                inner.at
            }
            LineItem::Close(..) => {
                if frames.pop().is_some() {
                    decorating.close();
                }
                frame.at
            }
            LineItem::Absolute(_) => frame.at,
        };
        if !plain {
            places.push(at);
        }
    }
    let (max_ascent, height, tops) = extents.settle();
    let baseline_y = *y + max_ascent;
    let line_top = *y;
    let baseline_of = |(group, shift): Place| -> f32 {
        let base = match group {
            None => baseline_y,
            Some(g) => line_top + tops[g],
        };
        base + shift
    };

    let mut x = line_left + placement.offset;
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
                LineItem::Absolute(b) => {
                    absolutes.push(through_inlines(Deferred::at(b, x, *y), open));
                }
                LineItem::Word(_) | LineItem::Atomic(_) | LineItem::Marker(_) => {}
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
            baseline: baseline_y,
            starts: true,
            children: Default::default(),
            absolutes_from: absolutes.len(),
        });
    let continued = open.iter().zip(continued).map(|(b, at)| OpenInline {
        b,
        style: match &b.first_line {
            Some(line) if first_line.is_some() => &line.style,
            _ => &b.style,
        },
        x,
        baseline: baseline_of(at),
        starts: false,
        children: Default::default(),
        absolutes_from: absolutes.len(),
    });
    let mut stack: Vec<OpenInline<'a>> = chain.collect();
    // Below this, the `::first-line`s: never closed by an element's end.
    let floor = stack.len();
    stack.extend(continued);
    let mut next_open = open.clone();
    let mut drawn = DrawnLines::default();
    let last_content = line.iter().rposition(|p| p.item.is_content());
    for (i, placed) in line.drain(..).enumerate() {
        if i > 0 && placed.gap_before {
            x += memo.gap(fonts, &placed) + placement.gap;
        }
        if i > 0 {
            x += placed.lead;
        }
        let baseline = baseline_of(places.get(i).copied().unwrap_or((None, 0.0)));
        match placed.item {
            LineItem::Word(word) if word.tab => x += word.width,
            LineItem::Word(word) => {
                let lines =
                    word.decorations
                        .map(|d| match decorated_at.get(i).and_then(Option::as_ref) {
                            Some(boxes) => {
                                let at = |place: Place| baseline_of(place) - baseline;
                                drawn.moved(d, boxes, at, fonts)
                            }
                            None => drawn.of(d, fonts),
                        });
                let into = innermost(&mut stack, fragments);
                let between = if placement.ideographs_only && !of_ideographs(word.text) {
                    0.0
                } else {
                    placement.between
                };
                if word.style.letter_spacing != 0.0
                    || (between > 0.0 && word.text.chars().nth(1).is_some())
                {
                    // Per-character fragments (M8.4) — see
                    // `measure_word`'s doc for why this has to shape
                    // character-by-character, same as the width this
                    // word was measured with; or a word justified
                    // between its characters (M21.7).
                    emit_letter_spaced(
                        into,
                        &word,
                        &mut x,
                        (baseline, max_ascent, max_descent),
                        between,
                        lines.map(|lines| (lines, reach[i])),
                        fonts,
                    );
                    continue;
                }
                let spaced = kept_spaces(word.text, Some(i) == last_content);
                if placement.gap > 0.0 && spaced > 0 {
                    // Kept spaces justified (CSS Text 3 section 7.3): the
                    // word in pieces, each after a space widened.
                    emit_justified(
                        into,
                        &word,
                        &mut x,
                        (baseline, max_ascent, max_descent),
                        (placement.gap, spaced),
                        lines.map(|lines| (lines, reach[i])),
                        fonts,
                    );
                    continue;
                }
                let decorated = lines.map(|lines| {
                    Box::new(Decorated {
                        lines,
                        left: reach[i].0,
                        right: reach[i].1,
                    })
                });
                into.push(text_fragment(
                    x,
                    baseline,
                    &word,
                    max_ascent,
                    max_descent,
                    decorated,
                    memo.font(fonts, word.text, word.style),
                ));
                x += word.width;
            }
            LineItem::Atomic(a) => {
                // The margin box's top sits `ascent` above the baseline;
                // the border box is a margin inside it.
                let top = baseline - a.ascent;
                let width = a.width;
                // The border box is a margin inside the margin box, and
                // a relatively positioned box is moved from there (M11);
                // the absolutely positioned boxes it carries go with it.
                let (dx, dy) = a.relative;
                let (bx, by) = (x + a.margin_left + dx, top + a.margin_top + dy);
                let a = *a;
                innermost(&mut stack, fragments).push(a.root.translated(bx, by));
                absolutes.extend(
                    carried(a.absolutes, bx, by)
                        .into_iter()
                        .map(|d| through_inlines(d, &next_open)),
                );
                x += width;
            }
            LineItem::Open(b, style, edge) => {
                next_open.push(b);
                stack.push(OpenInline {
                    b,
                    style,
                    x,
                    baseline,
                    starts: true,
                    children: Default::default(),
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
                let fragment = close_inline(inline, x, true, fonts, cb, absolutes);
                innermost(&mut stack, fragments).push(fragment);
            }
            LineItem::Absolute(b) => {
                absolutes.push(through_inlines(Deferred::at(b, x, *y), &next_open));
            }
            LineItem::Marker(m) => {
                let into = innermost(&mut stack, fragments);
                into.extend(m.fragments(x, baseline, max_ascent, max_descent, fonts));
                x += m.advance;
            }
        }
    }
    // The inline elements still open are sliced at the line's end: no
    // end edge on this line. (The `::first-line`s, which end there, have
    // none: neither margins nor borders apply to them.)
    while let Some(inline) = stack.pop() {
        let fragment = close_inline(inline, x, false, fonts, cb, absolutes);
        innermost(&mut stack, fragments).push(fragment);
    }
    *open = next_open;
    *y += height;
    Some(baseline_y)
}

/// What an inline box aligns its content by (CSS2 section 10.8.1,
/// `vertical-align`): its font's size, its content area above and
/// below its baseline, and its x-height.
#[derive(Debug, Clone, Copy, Default)]
struct Parent {
    font_size: f32,
    ascent: f32,
    descent: f32,
    x_height: f32,
}

impl Parent {
    fn of(fonts: &FontCollection, style: &ComputedStyle) -> Parent {
        dom::hook!("layout::inline::parent_of");
        let m = font::content(fonts, " ", style);
        let face = fonts.face(font::font_of(fonts, style).face);
        Parent {
            font_size: style.font_size_px,
            ascent: m.ascent,
            descent: m.descent,
            x_height: face.x_height(style.font_size_px),
        }
    }
}

/// Where a box's baseline is on a line: in the subtree aligned to the
/// line's top or bottom it is in, `None` for the root inline box's, and
/// how far below that subtree's root's baseline.
type Place = (Option<usize>, f32);

/// An inline box on the line as its content is aligned in it: where its
/// baseline is, and what it aligns its children by.
#[derive(Debug, Clone, Copy)]
struct Aligned {
    at: Place,
    parent: Parent,
}

/// A line's extents above and below the root inline box's baseline,
/// and those of each subtree `vertical-align: top` or `bottom` roots
/// (CSS2 section 10.8.1), with whether it is aligned to the top.
struct Extents {
    root: (f32, f32),
    groups: Vec<(bool, f32, f32)>,
}

impl Extents {
    /// An item `above` and `below` its baseline at `at` reaches that far.
    fn extend(&mut self, (group, shift): Place, above: f32, below: f32) {
        let (a, b) = match group {
            None => (&mut self.root.0, &mut self.root.1),
            Some(g) => {
                let (_, a, b) = &mut self.groups[g];
                (a, b)
            }
        };
        *a = a.max(above - shift);
        *b = b.max(below + shift);
    }

    /// Where a box styled `style`, `above` and `below` its baseline, is
    /// put in `parent` by its `vertical-align`: a subtree of its own at
    /// the line's top or bottom, or its parent's baseline shifted.
    fn align(
        &mut self,
        style: &ComputedStyle,
        above: f32,
        below: f32,
        parent: Aligned,
        fonts: &FontCollection,
    ) -> Place {
        use style::computed::VerticalAlign;
        match style.vertical_align {
            VerticalAlign::Top | VerticalAlign::Bottom => {
                let top = style.vertical_align == VerticalAlign::Top;
                self.groups.push((top, above, below));
                (Some(self.groups.len() - 1), 0.0)
            }
            _ => {
                let (group, shift) = parent.at;
                let own = baseline_shift(style, above, below, &parent.parent, fonts);
                (group, shift + own)
            }
        }
    }

    /// The inline box styled `style` opened in `parent`: aligned by its
    /// line-height box, its font's metrics with its leading.
    fn enter(&mut self, style: &ComputedStyle, parent: Aligned, fonts: &FontCollection) -> Aligned {
        dom::hook!("layout::inline::enter");
        let m = font::line(fonts, " ", style);
        Aligned {
            at: self.align(style, m.ascent, m.descent + m.line_gap, parent, fonts),
            parent: Parent::of(fonts, style),
        }
    }

    /// The line's ascent above the root inline box's baseline, its
    /// height, and the top of each subtree's root's baseline below the
    /// line's top: the line as tall as its tallest subtree, a taller
    /// top-aligned one hanging below the rest, a taller bottom-aligned
    /// one standing above it, as the reference browser grows the line.
    fn settle(&self) -> (f32, f32, Vec<f32>) {
        dom::hook!("layout::inline::settle");
        let (mut ascent, mut below) = self.root;
        for &(top, a, b) in &self.groups {
            let short = a + b - (ascent + below);
            if short > 0.0 {
                if top {
                    below += short;
                } else {
                    ascent += short;
                }
            }
        }
        let height = ascent + below;
        let tops = self
            .groups
            .iter()
            .map(|&(top, a, b)| if top { a } else { height - b })
            .collect();
        (ascent, height, tops)
    }
}

/// How far below its parent's baseline `vertical-align` puts the
/// baseline of a box styled `style`, `above` and `below` it (CSS2
/// section 10.8.1), by the reference browser's figures for `sub` and
/// `super` (a fifth of the parent's font size and a pixel down, a third
/// and a pixel up); a percentage is of the box's own line height.
fn baseline_shift(
    style: &ComputedStyle,
    above: f32,
    below: f32,
    parent: &Parent,
    fonts: &FontCollection,
) -> f32 {
    dom::hook!("layout::inline::baseline_shift");
    use style::computed::VerticalAlign;
    let line_height = || font::line(fonts, " ", style).line_height();
    match &style.vertical_align {
        VerticalAlign::Baseline | VerticalAlign::Top | VerticalAlign::Bottom => 0.0,
        VerticalAlign::Sub => parent.font_size / 5.0 + 1.0,
        VerticalAlign::Super => -(parent.font_size / 3.0 + 1.0),
        VerticalAlign::TextTop => above - parent.ascent,
        VerticalAlign::TextBottom => parent.descent - below,
        VerticalAlign::Middle => (above - below - parent.x_height) / 2.0,
        VerticalAlign::BaselineMiddle => (above - below) / 2.0,
        VerticalAlign::Length(px) => -px,
        VerticalAlign::Percent(p) => -p / 100.0 * line_height(),
        VerticalAlign::Calc(calc) => -calc.resolve(line_height()),
    }
}

/// An absolutely positioned box met inside the inline elements `open`,
/// innermost last, carried up through them (M19): an inline box is no
/// containing block here, but it can be the stacking context the box
/// belongs to, a faded `span` say.
fn through_inlines<'a>(mut d: Deferred<'a>, open: &[&'a LayoutBox]) -> Deferred<'a> {
    for inline in open.iter().rev() {
        d.carried_through(inline);
    }
    d
}

/// An inline element open on the line being flushed, with the
/// fragments of its content on the line so far.
struct OpenInline<'a> {
    b: &'a LayoutBox,
    /// Its style on the line: on the first, its style there (M16).
    style: &'a Arc<ComputedStyle>,
    /// Where it starts on the line: before its start edge when it
    /// starts here, else the line's start.
    x: f32,
    /// Its baseline, where `vertical-align` puts it on the line.
    baseline: f32,
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
/// children are made relative to it, as every fragment's are (M35.9),
/// and a relatively positioned inline element is moved
/// as a whole (CSS2 §9.4.3), the absolutely positioned boxes met inside
/// it on this line with it.
///
/// [`layout_document`]: crate::layout_document
fn close_inline<'a>(
    inline: OpenInline<'a>,
    x: f32,
    ends: bool,
    fonts: &FontCollection,
    (cb_width, cb_height): (f32, Option<f32>),
    absolutes: &mut [Deferred<'a>],
) -> Fragment {
    dom::hook!("layout::inline::close_inline");
    let (b, own) = (inline.b, inline.style);
    let edges = Edges::of(own, cb_width);
    let margin = |side: usize| own.margin[side].resolve(cb_width, 0.0);
    let m = font::content(fonts, " ", own);
    let left = if inline.starts {
        inline.x + margin(LEFT)
    } else {
        inline.x
    };
    let right = if ends { x - margin(RIGHT) } else { x };
    let top = inline.baseline - m.ascent - edges.padding[TOP] - edges.border[TOP];
    let height = edges.border[TOP]
        + edges.padding[TOP]
        + m.ascent
        + m.descent
        + edges.padding[BOTTOM]
        + edges.border[BOTTOM];
    // The element's style shared, but where a line break slices it: no
    // border on that side, and its corners square, as the reference
    // browser draws them.
    let mut style = own.clone();
    let square = |s: &mut ComputedStyle, corners: [usize; 2]| {
        if s.radii.is_some() {
            let zero = style::computed::LengthPercentage::Px(0.0);
            for corner in corners {
                s.radii_mut().0[corner] = [zero.clone(), zero.clone()];
            }
        }
    };
    if !inline.starts {
        let style = Arc::make_mut(&mut style);
        style.border_style[LEFT] = BorderStyle::None;
        square(style, [0, 3]);
    }
    if !ends {
        let style = Arc::make_mut(&mut style);
        style.border_style[RIGHT] = BorderStyle::None;
        square(style, [1, 2]);
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
        children: children.into(),
        tree_order: b.tree_order,
        stacking_parent: None,
        parent: b.parent,
        in_3d: b.in_3d,
        legend: false,
        decorated: None,
        font: Default::default(),
    }
}

/// How a run's lines are aligned (css-text-3 §7, M7.5, M21.7): its
/// container's `text-align-all`, `text-align-last` and `text-justify`,
/// and its direction, which `start` and `end` are by.
#[derive(Debug, Clone, Copy)]
struct Alignment {
    all: TextAlign,
    last: TextAlignLast,
    justify: TextJustify,
    direction: Direction,
}

/// Where a line's content goes along it: how far after the line's
/// start, and how much a justified line widens each gap, and each place
/// between two characters of a word.
#[derive(Debug, Clone, Copy, Default)]
struct Placement {
    offset: f32,
    gap: f32,
    between: f32,
    /// `between` is only between the characters of words of ideographs.
    ideographs_only: bool,
}

impl Alignment {
    fn of(style: &ComputedStyle) -> Self {
        Alignment {
            all: style.text_align,
            last: style.text_align_last,
            justify: style.text_justify,
            direction: style.direction,
        }
    }

    /// How `line`, `room` short of its room (less than nothing when it
    /// overflows), is placed: the last line, or one ending at a forced
    /// break, when `ends`, by `text-align-last`
    /// — `auto` being `text-align-all`'s value, or `start` where that is
    /// `justify` — and any other by `text-align-all`; `start` and `end`
    /// by the direction. A justified line's slack is shared equally among
    /// its justification opportunities: the gaps between its items, the
    /// spaces kept in its words but those that hang at its end (whose
    /// width is slack then), and with `inter-character` the places
    /// between two characters of a
    /// word as well. A line with none, with `text-justify: none`, or with
    /// no slack is aligned to its start; one too long for its room is
    /// too, whatever the alignment, never pushed back past its start
    /// (css-text-3 §7.1).
    fn place(&self, line: &[Placed<'_, '_>], room: f32, ends: bool) -> Placement {
        let slack = room.max(0.0);
        let align = match (ends, self.last) {
            (true, TextAlignLast::Align(align)) => align,
            (true, TextAlignLast::Auto) if self.all == TextAlign::Justify => TextAlign::Start,
            _ => self.all,
        };
        let at = |align: TextAlign| Placement {
            offset: match align.matched(self.direction) {
                TextAlign::Right => slack,
                TextAlign::Center => slack / 2.0,
                _ => 0.0,
            },
            ..Placement::default()
        };
        if align != TextAlign::Justify {
            return at(align);
        }
        let last_content = line.iter().rposition(|p| p.item.is_content());
        // Kept spaces between words are opportunities too, but those
        // that hang at the line's end, which the line is not justified
        // over.
        let kept: usize = line
            .iter()
            .enumerate()
            .map(|(i, p)| match &p.item {
                LineItem::Word(word) if !word.tab => {
                    kept_spaces(word.text, Some(i) == last_content)
                }
                _ => 0,
            })
            .sum();
        let slack = if kept > 0 {
            (room + last_content.map_or(0.0, |i| line[i].item.hang())).max(0.0)
        } else {
            slack
        };
        let gaps = kept
            + line
                .iter()
                .enumerate()
                .filter(|(i, p)| *i > 0 && p.gap_before)
                .count();
        // Between the characters of words, every word's with
        // `inter-character`; with `auto` those of words of ideographs, as
        // the reference browser spreads them.
        let between = |only_ideographs: bool| -> usize {
            line.iter()
                .map(|p| match &p.item {
                    LineItem::Word(word) if !only_ideographs || of_ideographs(word.text) => {
                        letter_units(word.text)
                            .iter()
                            .filter(|(_, spaced)| *spaced)
                            .count()
                            .saturating_sub(1)
                    }
                    _ => 0,
                })
                .sum()
        };
        let (opportunities, spread) = match self.justify {
            TextJustify::None => (0, false),
            TextJustify::InterCharacter => (gaps + between(false), true),
            TextJustify::Auto => {
                let between = between(true);
                (gaps + between, between > 0)
            }
            TextJustify::InterWord => (gaps, false),
        };
        if slack <= 0.0 || opportunities == 0 {
            return at(TextAlign::Start);
        }
        let share = slack / opportunities as f32;
        Placement {
            offset: 0.0,
            gap: share,
            between: if spread { share } else { 0.0 },
            ideographs_only: self.justify == TextJustify::Auto,
        }
    }
}

/// How far past each word on `line` its text decorations reach (M21.7):
/// over the gap before an item, which `owner` says the decorations of by
/// the item's place in the run, back from the next word, across the
/// inline boxes' edges between, when that word's are those, else on
/// from the word before it the same way; over no gap otherwise, and over
/// none at a line's ends, which has none.
fn decoration_reach<'a>(
    line: &[Placed<'_, '_>],
    owner: impl Fn(usize) -> Option<&'a Arc<[Decorating]>>,
    justified: f32,
    fonts: &FontCollection,
) -> Vec<Spread> {
    let mut reach = vec![(0.0, 0.0); line.len()];
    let same =
        |a: Option<&Arc<[Decorating]>>, b: &Arc<[Decorating]>| a.is_some_and(|a| Arc::ptr_eq(a, b));
    for i in 1..line.len() {
        let Some(owner) = owner(line[i].index).filter(|_| line[i].gap_before) else {
            continue;
        };
        let gap = line[i].gap(fonts) + justified;
        // The word after, past the edges of the inline boxes the space
        // ended or began, its own start edge among them.
        let mut edges = 0.0;
        let mut found = false;
        for j in i..line.len() {
            match &line[j].item {
                LineItem::Open(.., edge) | LineItem::Close(_, edge) => edges += edge,
                LineItem::Word(after) => {
                    if same(after.decorations, owner) {
                        reach[j].0 = gap + edges;
                        found = true;
                    }
                    break;
                }
                _ => break,
            }
        }
        if found {
            continue;
        }
        let mut edges = 0.0;
        for j in (0..i).rev() {
            match &line[j].item {
                LineItem::Open(.., edge) | LineItem::Close(_, edge) => edges += edge,
                LineItem::Word(before) => {
                    if same(before.decorations, owner) {
                        reach[j].1 = edges + gap;
                    }
                    break;
                }
                _ => break,
            }
        }
    }
    reach
}

/// How far a word's text decorations reach past its left edge and its
/// right ([`decoration_reach`]).
type Spread = (f32, f32);

/// The lines each list of decorations on a line draws, found once.
#[derive(Default)]
struct DrawnLines(HashMap<*const Decorating, Arc<[DrawnLine]>>);

impl DrawnLines {
    fn of(&mut self, decorations: &Arc<[Decorating]>, fonts: &FontCollection) -> Arc<[DrawnLine]> {
        self.0
            .entry(decorations.as_ptr())
            .or_insert_with(|| Arc::from(decoration::lines(decorations, fonts)))
            .clone()
    }

    /// The lines of `decorations` for a text whose innermost decorating
    /// boxes on the line are at `boxes`, outermost first, each line
    /// moved by `by` of its box's place (how far below the text's
    /// baseline that box's is); the decorations before them are the
    /// block's, at the root inline box's baseline.
    fn moved(
        &mut self,
        decorations: &Arc<[Decorating]>,
        boxes: &[Place],
        by: impl Fn(Place) -> f32,
        fonts: &FontCollection,
    ) -> Arc<[DrawnLine]> {
        let from = decorations.len().saturating_sub(boxes.len());
        let skipped = boxes.len().saturating_sub(decorations.len());
        let moves: Vec<f32> = (0..decorations.len())
            .map(|n| match n.checked_sub(from) {
                Some(k) => by(boxes[skipped + k]),
                None => by((None, 0.0)),
            })
            .collect();
        if moves.iter().all(|&dy| dy == 0.0) {
            return self.of(decorations, fonts);
        }
        let mut out = Vec::new();
        for (decoration, dy) in decorations.iter().zip(moves) {
            for mut line in decoration::lines(std::slice::from_ref(decoration), fonts) {
                line.top += dy;
                line.second = line.second.map(|second| second + dy);
                out.push(line);
            }
        }
        Arc::from(out)
    }
}

/// The open inline elements that decorate their text, as a line's items
/// open and close them: each one's place, and for every open element
/// whether it is one of them.
#[derive(Default)]
struct Decorators {
    places: Vec<Place>,
    open: Vec<bool>,
    /// The places as the words take them, made once while no element
    /// opens or closes.
    shared: Option<Arc<[Place]>>,
}

impl Decorators {
    fn open(&mut self, style: &ComputedStyle, at: Place) {
        let decorates = Decorating::of(style).is_some();
        self.open.push(decorates);
        if decorates {
            self.places.push(at);
            self.shared = None;
        }
    }

    fn close(&mut self) {
        if self.open.pop() == Some(true) {
            self.places.pop();
            self.shared = None;
        }
    }

    /// The places a word in the innermost open element takes; `None`
    /// when no decorating element is open.
    fn places(&mut self) -> Option<Arc<[Place]>> {
        if self.places.is_empty() {
            return None;
        }
        Some(
            self.shared
                .get_or_insert_with(|| Arc::from(self.places.as_slice()))
                .clone(),
        )
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
    decorated: Option<Box<Decorated>>,
    font: text::Font,
) -> Fragment {
    dom::hook!("layout::inline::text_fragment");
    let shown = if word.hyphen {
        format!("{}{}", word.text, hyphen_of(word.style))
    } else {
        word.text.to_string()
    };
    Fragment {
        x,
        y: baseline_y,
        w: word.width,
        h: ascent + descent,
        style: word.style.clone(),
        kind: FragmentKind::Text(shown),
        node: word.node,
        generated: word.generated,
        ascent,
        scroll_height: 0.0,
        children: Default::default(),
        tree_order: 0,
        stacking_parent: None,
        parent: Some(word.node),
        in_3d: false,
        legend: false,
        decorated,
        font,
    }
}

/// Pushes one `Text` fragment per character of `word`, each advanced by
/// `letter-spacing` past the previous one (M8.4) — the common
/// `letter_spacing == 0.0` case never calls this, so it costs nothing
/// when the property isn't used. On a line whose baseline is at `y` and
/// whose text rises and falls `ascent` and `descent`; `between` more
/// between two characters, which justification gives (M21.7). The
/// word's text decorations (M21.7) reach over the space after each
/// character, and the first's and the last's over the gaps `reach`
/// says.
fn emit_letter_spaced(
    fragments: &mut Vec<Fragment>,
    word: &LineWord<'_>,
    x: &mut f32,
    (baseline_y, ascent, descent): (f32, f32, f32),
    between: f32,
    decorations: Option<(Arc<[DrawnLine]>, Spread)>,
    fonts: &FontCollection,
) {
    let shown = if word.hyphen {
        Cow::Owned(format!("{}{}", word.text, hyphen_of(word.style)))
    } else {
        Cow::Borrowed(word.text)
    };
    let units = letter_units(&shown);
    let last = units.len().saturating_sub(1);
    // Justification widens the places after the spaced units but the
    // last: none after a format character or within a cursive word.
    let last_spaced = units.iter().rposition(|(_, spaced)| *spaced);
    let font = font::font_of(fonts, word.style);
    for (i, (range, spaced)) in units.into_iter().enumerate() {
        let ch_str = &shown[range];
        let w = fonts.measure_in(font, ch_str, word.style.font_size_px);
        let one = LineWord {
            text: ch_str,
            width: w,
            hang: 0.0,
            tab: false,
            hyphen: false,
            breaks: &[],
            offset: 0,
            style: word.style,
            node: word.node,
            generated: word.generated,
            decorations: word.decorations,
        };
        let letter = if spaced {
            word.style.letter_spacing
        } else {
            0.0
        };
        let widened = spaced && Some(i) != last_spaced;
        let spacing = letter + if widened { between } else { 0.0 };
        let decorated = decorations.as_ref().map(|(lines, (left, right))| {
            Box::new(Decorated {
                lines: Arc::clone(lines),
                left: if i == 0 { *left } else { 0.0 },
                right: spacing + if i == last { *right } else { 0.0 },
            })
        });
        fragments.push(text_fragment(
            *x, baseline_y, &one, ascent, descent, decorated, font,
        ));
        *x += w + spacing;
    }
}

/// How many kept spaces in `text` a justified line widens: every space
/// and no-break space, as the reference browser has it, but the spaces
/// at its end when it ends the line (`last`), which hang.
fn kept_spaces(text: &str, last: bool) -> usize {
    let text = if last {
        text.trim_end_matches(' ')
    } else {
        text
    };
    text.chars().filter(|&c| is_justified_space(c)).count()
}

/// Whether justification widens `c` where it is kept in a word: a space
/// or a no-break space.
fn is_justified_space(c: char) -> bool {
    c == ' ' || c == '\u{a0}'
}

/// Pushes `word` in pieces, each ending after one of its first `spaced`
/// spaces, which justification widens by `gap` (M21.7): the kept spaces
/// of `white-space: pre-wrap` text are its opportunities. As
/// [`emit_letter_spaced`] does otherwise.
fn emit_justified(
    fragments: &mut Vec<Fragment>,
    word: &LineWord<'_>,
    x: &mut f32,
    (baseline_y, ascent, descent): (f32, f32, f32),
    (gap, spaced): (f32, usize),
    decorations: Option<(Arc<[DrawnLine]>, Spread)>,
    fonts: &FontCollection,
) {
    let font = font::font_of(fonts, word.style);
    let mut cuts: Vec<usize> = word
        .text
        .char_indices()
        .filter(|&(_, c)| is_justified_space(c))
        .take(spaced)
        .map(|(at, c)| at + c.len_utf8())
        .collect();
    if cuts.last() != Some(&word.text.len()) {
        cuts.push(word.text.len());
    }
    let pieces = cuts.len();
    let mut start = 0;
    for (n, end) in cuts.into_iter().enumerate() {
        let text = &word.text[start..end];
        let last = n + 1 == pieces;
        let piece = LineWord {
            text,
            width: measure_word(text, word.style, fonts)
                + if word.hyphen && last {
                    measure_word(hyphen_of(word.style), word.style, fonts)
                } else {
                    0.0
                },
            hang: 0.0,
            offset: word.offset + start,
            hyphen: word.hyphen && last,
            ..*word
        };
        let widened = if n < spaced { gap } else { 0.0 };
        let decorated = decorations.as_ref().map(|(lines, (left, right))| {
            Box::new(Decorated {
                lines: Arc::clone(lines),
                left: if n == 0 { *left } else { 0.0 },
                right: widened + if last { *right } else { 0.0 },
            })
        });
        fragments.push(text_fragment(
            *x, baseline_y, &piece, ascent, descent, decorated, font,
        ));
        *x += piece.width + widened;
        start = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use style::computed::WhiteSpace;

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
            true,
        );
        (l.fragments, l.height)
    }

    fn text_box(text: &str, font_size_px: f32) -> LayoutBox {
        let style = ComputedStyle {
            font_size_px,
            ..ComputedStyle::default()
        };
        LayoutBox {
            style: style.into(),
            kind: BoxKind::Text(text.to_string()),
            children: Default::default(),
            node: crate::testing::no_node(),
            anonymous: true,
            generated: None,
            generated_text: 0,
            decorations: None,
            styled_by_ancestor: false,
            first_line: None,
            tree_order: 0,
            item: false,
            parent: None,
            in_3d: false,
            replaced: None,
            control_size: None,
        }
    }

    #[test]
    fn a_lines_words_keep_their_own_gaps_and_faces_whatever_came_before() {
        // A line is placed with what its words of one style share kept
        // (`LineMemo`): a gap is still its own white space's, and a word
        // of private-use code points is not given the face an ASCII word
        // of its style was. Ahem is the monospace face, so that its line
        // is not the primary face's.
        let fonts = FontCollection::from_bytes_with_monospace(
            text::TEST_FONT.to_vec(),
            0,
            include_bytes!("../../engine/tests/wpt/fonts/Ahem.ttf").to_vec(),
        )
        .expect("parses");
        let mut mono = text_box("ab \u{F000}", 16.0);
        let mut style = (*mono.style).clone();
        style.font_mut().family = "monospace, \"Not Had\"".into();
        mono.style = style.into();
        let (fragments, _) = lay(
            &[text_box("a b", 16.0), text_box(" c", 32.0), mono],
            2000.0,
            TextAlign::Left,
            &fonts,
        );
        let text: Vec<(&str, f32, f32, text::Font)> = fragments
            .iter()
            .filter_map(|f| match &f.kind {
                FragmentKind::Text(t) => Some((t.as_str(), f.x, f.w, f.font)),
                _ => None,
            })
            .collect();
        let at = |word: &str| text.iter().find(|t| t.0 == word).expect(word);
        let space = |size: f32| {
            space_width(
                &ComputedStyle {
                    font_size_px: size,
                    ..ComputedStyle::default()
                },
                &fonts,
            )
        };
        let (b, c) = (at("b"), at("c"));
        assert_eq!(c.1 - (b.1 + b.2), space(32.0), "the gap is the 32px space");
        let (ab, private) = (at("ab"), at("\u{F000}"));
        assert_ne!(ab.3, text::Font::default(), "the monospace face");
        assert_eq!(private.3, text::Font::default(), "no named family had");
        // And the private-use word's line metrics are its own face's: the
        // line is as tall as the taller of the two words' lines.
        let mono_style = |b: &LayoutBox| {
            let mut style = (*b.style).clone();
            style.font_mut().family = "monospace, \"Not Had\"".into();
            Arc::new(style)
        };
        let line = |text: &str| {
            let mut b = text_box(text, 40.0);
            b.style = mono_style(&b);
            lay(&[b], 2000.0, TextAlign::Left, &fonts).1
        };
        let (ascii, pua) = (line("ab"), line("\u{F000}"));
        assert_ne!(ascii, pua, "the faces' lines differ");
        assert_eq!(line("ab \u{F000}"), ascii.max(pua));
    }

    #[test]
    fn a_word_of_a_partial_face_keeps_no_line_metrics_for_the_next() {
        // A face for only "a" (`unicode-range`) ahead of the primary
        // face: "b" is in the primary face, and its line is its own,
        // not the line "a" had before it.
        let faces = Arc::new(text::DocumentFaces::new(vec![text::FaceRule {
            family: "Web".into(),
            ranges: vec![(0x61, 0x61)],
            ..text::FaceRule::default()
        }]));
        let fonts = test_fonts().with_document(faces);
        fonts
            .face_arrived(
                0,
                include_bytes!("../../engine/tests/wpt/fonts/Ahem.ttf").to_vec(),
            )
            .expect("a font");
        let line = |text: &str| {
            let mut b = text_box(text, 40.0);
            let mut style = (*b.style).clone();
            style.font_mut().family = "Web, sans-serif".into();
            b.style = Arc::new(style);
            lay(&[b], 2000.0, TextAlign::Left, &fonts).1
        };
        assert!(line("a") < line("b"), "Ahem's line is the shorter");
        assert_eq!(line("a b"), line("b"));
    }

    fn inline_box(children: Vec<LayoutBox>) -> LayoutBox {
        LayoutBox {
            style: ComputedStyle::default().into(),
            kind: BoxKind::Inline,
            children,
            node: crate::testing::no_node(),
            anonymous: false,
            generated: None,
            generated_text: 0,
            decorations: None,
            styled_by_ancestor: false,
            first_line: None,
            tree_order: 0,
            item: false,
            parent: None,
            in_3d: false,
            replaced: None,
            control_size: None,
        }
    }

    fn texts_of(fragments: &[Fragment]) -> Vec<&str> {
        fragments
            .iter()
            .filter_map(|f| match &f.kind {
                FragmentKind::Text(t) => Some(t.as_str()),
                FragmentKind::Box
                | FragmentKind::InlineBox
                | FragmentKind::Marker(_)
                | FragmentKind::Replaced(_) => None,
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
                    FragmentKind::Box | FragmentKind::Marker(_) | FragmentKind::Replaced(_) => {}
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

    /// An empty inline-block `height` pixels tall, aligned by `align`.
    fn aligned_block(height: f32, align: style::computed::VerticalAlign) -> LayoutBox {
        LayoutBox {
            style: ComputedStyle {
                display: style::computed::Display::InlineBlock,
                width: style::computed::Size::Px(10.0),
                height: style::computed::Size::Px(height),
                vertical_align: align,
                ..ComputedStyle::default()
            }
            .into(),
            kind: BoxKind::InlineBlock,
            ..inline_box(Vec::new())
        }
    }

    /// The y of each box fragment's top among `fragments`.
    fn box_tops(fragments: &[Fragment]) -> Vec<f32> {
        fragments
            .iter()
            .filter(|f| matches!(f.kind, FragmentKind::Box))
            .map(|f| f.y)
            .collect()
    }

    #[test]
    fn vertical_align_top_and_bottom_place_a_box_at_the_lines_edges() {
        // WPT's background-origin references: inline-blocks aligned to
        // the line's bottom make it as tall as they are, the strut's
        // descent no longer under them.
        use style::computed::VerticalAlign;
        let fonts = test_fonts();
        let strut = fonts.primary().metrics(16.0);
        let (fragments, height) = lay(
            &[
                aligned_block(100.0, VerticalAlign::Bottom),
                aligned_block(40.0, VerticalAlign::Bottom),
            ],
            1000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(height, 100.0);
        assert_eq!(box_tops(&fragments), [0.0, 60.0]);
        let (baseline, _) = lay(
            &[aligned_block(100.0, VerticalAlign::Baseline)],
            1000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(baseline[0].y, 0.0);
        // A top-aligned box hangs from the line's top, below the text;
        // the text's baseline is where the strut puts it.
        let (fragments, height) = lay(
            &[
                text_box("a", 16.0),
                aligned_block(100.0, VerticalAlign::Top),
                aligned_block(10.0, VerticalAlign::Bottom),
            ],
            1000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(height, 100.0);
        assert_eq!(box_tops(&fragments), [0.0, 90.0]);
        assert_eq!(texts(&fragments)[0].y, strut.ascent);
        // A bottom-aligned one taller than the line stands above it:
        // the text is at the line's bottom.
        let (fragments, height) = lay(
            &[
                text_box("a", 16.0),
                aligned_block(100.0, VerticalAlign::Bottom),
            ],
            1000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(height, 100.0);
        assert_eq!(
            texts(&fragments)[0].y,
            100.0 - strut.descent - strut.line_gap
        );
    }

    #[test]
    fn vertical_align_shifts_a_box_from_its_parents_baseline() {
        use style::computed::VerticalAlign;
        let fonts = test_fonts();
        let strut = fonts.primary().metrics(16.0);
        let x_height = fonts.x_height(16.0);
        let top_of = |align: VerticalAlign| {
            let (fragments, _) = lay(
                &[text_box("a", 16.0), aligned_block(10.0, align)],
                1000.0,
                TextAlign::Left,
                &fonts,
            );
            let baseline = texts(&fragments)[0].y;
            box_tops(&fragments)[0] - baseline
        };
        let near = |got: f32, want: f32| assert!((got - want).abs() < 1e-3, "{got} for {want}");
        // From the text's baseline to the box's top: a box on the
        // baseline is all above it.
        near(top_of(VerticalAlign::Baseline), -10.0);
        near(top_of(VerticalAlign::Length(4.0)), -14.0);
        near(top_of(VerticalAlign::Length(-4.0)), -6.0);
        // A fifth of the parent's font size and a pixel down; a third
        // and a pixel up.
        near(top_of(VerticalAlign::Sub), -10.0 + 16.0 / 5.0 + 1.0);
        near(top_of(VerticalAlign::Super), -10.0 - 16.0 / 3.0 - 1.0);
        near(top_of(VerticalAlign::Middle), -x_height / 2.0 - 5.0);
        // The box's middle on the baseline.
        near(top_of(VerticalAlign::BaselineMiddle), -5.0);
        near(top_of(VerticalAlign::TextTop), -strut.ascent);
        near(top_of(VerticalAlign::TextBottom), strut.descent - 10.0);
        let line = strut.line_height();
        near(top_of(VerticalAlign::Percent(50.0)), -10.0 - line / 2.0);
        // A `sub` element's text is lowered with it, and an element in
        // it from there.
        let sub = ComputedStyle {
            vertical_align: VerticalAlign::Sub,
            ..ComputedStyle::default()
        };
        let lowered = LayoutBox {
            style: sub.clone().into(),
            ..inline_box(vec![
                text_box("b", 16.0),
                LayoutBox {
                    style: sub.into(),
                    ..inline_box(vec![text_box("c", 16.0)])
                },
            ])
        };
        let (fragments, _) = lay(
            &[text_box("a", 16.0), lowered],
            1000.0,
            TextAlign::Left,
            &fonts,
        );
        let ys: Vec<f32> = texts(&fragments).iter().map(|f| f.y).collect();
        let step = 16.0 / 5.0 + 1.0;
        assert_eq!(ys.len(), 3);
        near(ys[1] - ys[0], step);
        near(ys[2] - ys[0], 2.0 * step);
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

    /// [`layout_inline`] of one text box saying `text`, 16 pixels, in
    /// `width` under a container styled as `container` says.
    fn lay_styled(text: &str, width: f32, container: ComputedStyle) -> Vec<Fragment> {
        let fonts = test_fonts();
        let mut floats = FloatContext::default();
        let items = [text_box(text, 16.0)];
        let l = layout_inline(
            &items,
            width,
            None,
            &container,
            None,
            &mut floats,
            At::root(),
            &Pass::new(&fonts),
            true,
        );
        texts(&l.fragments)
    }

    #[test]
    fn text_indent_indents_the_first_line_or_with_hanging_the_others() {
        let indented = |length, hanging, each_line| {
            let mut style = ComputedStyle::default();
            style.text_mut().text_indent = style::computed::TextIndent {
                length,
                hanging,
                each_line,
            };
            style
        };
        let fonts = test_fonts();
        let one_word = fonts.measure("aaaa", 16.0) + 25.0;
        // Lines of one word each, their x.
        let xs = |container: ComputedStyle| {
            lay_styled("aaaa aaaa aaaa", one_word, container)
                .iter()
                .map(|f| f.x)
                .collect::<Vec<_>>()
        };
        let px = |v| style::computed::LengthPercentage::Px(v);
        assert_eq!(xs(indented(px(20.0), false, false)), [20.0, 0.0, 0.0]);
        assert_eq!(xs(indented(px(20.0), true, false)), [0.0, 20.0, 20.0]);
        let pct = style::computed::LengthPercentage::Percent(10.0);
        assert_eq!(xs(indented(pct, false, false)), [one_word / 10.0, 0.0, 0.0]);
        // `each-line`: after a forced break too.
        let mut container = indented(px(20.0), false, true);
        container.white_space = WhiteSpace::PRE_LINE;
        let run = [styled_text_box(
            "a a\na",
            ComputedStyle {
                white_space: WhiteSpace::PRE_LINE,
                ..container.clone()
            },
        )];
        let mut floats = FloatContext::default();
        let l = layout_inline(
            &run,
            fonts.measure("a", 16.0) + 25.0,
            None,
            &container,
            None,
            &mut floats,
            At::root(),
            &Pass::new(&fonts),
            true,
        );
        let xs: Vec<f32> = texts(&l.fragments).iter().map(|f| f.x).collect();
        assert_eq!(xs, [20.0, 0.0, 20.0]);
        // The max-content width counts the first line's indent.
        let run = [styled_text_box("aaaa", ComputedStyle::default())];
        let items = collect_inline_items(&run, &fonts);
        let indent = Indent::of(&indented(px(20.0), false, false), 0.0);
        let (min, max) = run_intrinsic_widths_indented(&items, None, &Pass::new(&fonts), indent);
        assert_eq!(
            (min, max),
            (
                20.0 + fonts.measure("aaaa", 16.0),
                20.0 + fonts.measure("aaaa", 16.0)
            )
        );
    }

    #[test]
    fn text_autospace_puts_an_eighth_of_an_em_between_ideographs_and_letters() {
        let fonts = test_fonts();
        let normal = || {
            let mut style = ComputedStyle {
                font_size_px: 16.0,
                ..ComputedStyle::default()
            };
            style.text_mut().text_autospace = style::computed::TextAutospace::Normal;
            style
        };
        let lay_normal = |text: &str| {
            let run = [styled_text_box(text, normal())];
            lay(&run, 400.0, TextAlign::Left, &fonts).0
        };
        let spaced = lay_normal("\u{65e5}a1\u{672c}");
        let xs: Vec<(String, f32)> = spaced
            .iter()
            .map(|f| match &f.kind {
                FragmentKind::Text(t) => (t.clone(), f.x),
                _ => unreachable!(),
            })
            .collect();
        let ideograph = fonts.measure("\u{65e5}", 16.0);
        assert_eq!(
            xs,
            [
                ("\u{65e5}".to_string(), 0.0),
                ("a1".to_string(), ideograph + 2.0),
                (
                    "\u{672c}".to_string(),
                    ideograph + 2.0 + fonts.measure("a1", 16.0) + 2.0
                ),
            ]
        );
        let mut none = ComputedStyle {
            font_size_px: 16.0,
            ..ComputedStyle::default()
        };
        none.text_mut().text_autospace = style::computed::TextAutospace::NoAutospace;
        let run = [styled_text_box("\u{65e5}a1\u{672c}", none)];
        let (unspaced, _) = lay(&run, 400.0, TextAlign::Left, &fonts);
        assert_eq!(
            texts_of(&texts(&unspaced)),
            ["\u{65e5}a1\u{672c}"],
            "no space"
        );
        assert_eq!(texts(&lay_normal("abc")).len(), 1);
        // An iteration mark is an ideograph's: no space before it.
        assert_eq!(texts(&lay_normal("\u{65e5}\u{3005}")).len(), 1);
        // The initial value is the reference browser's, `no-autospace`.
        assert_eq!(
            lay_styled("\u{65e5}a1\u{672c}", 400.0, ComputedStyle::default()).len(),
            1,
            "no space by default"
        );
    }

    #[test]
    fn text_autospace_puts_its_space_across_element_boundaries() {
        // WPT's text-autospace-elements-005.html: an ideograph in one
        // element and a letter in the next, either way round, and in
        // elements nested deeper; none where either side's
        // `text-autospace` is `no-autospace`, nor after white space.
        let fonts = test_fonts();
        let style = || {
            let mut style = ComputedStyle {
                font_size_px: 16.0,
                ..ComputedStyle::default()
            };
            style.text_mut().text_autospace = style::computed::TextAutospace::Normal;
            style
        };
        let xs = |run: &[LayoutBox]| -> Vec<f32> {
            let (fragments, _) = lay(run, 400.0, TextAlign::Left, &fonts);
            texts(&fragments).iter().map(|f| f.x).collect()
        };
        let ideograph = fonts.measure("\u{6c38}", 16.0);
        let abc = fonts.measure("abc", 16.0);
        let run = [
            inline_box(vec![inline_box(vec![styled_text_box("\u{6c38}", style())])]),
            styled_text_box("abc", style()),
            inline_box(vec![styled_text_box("\u{6c38}", style())]),
        ];
        assert_eq!(
            xs(&run),
            [0.0, ideograph + 2.0, ideograph + 2.0 + abc + 2.0]
        );
        let mut off = style();
        off.text_mut().text_autospace = style::computed::TextAutospace::NoAutospace;
        let run = [
            styled_text_box("\u{6c38}", off.clone()),
            styled_text_box("abc", style()),
            styled_text_box("\u{6c38}", off),
        ];
        assert_eq!(xs(&run), [0.0, ideograph, ideograph + abc]);
        let run = [
            styled_text_box("\u{6c38} ", style()),
            styled_text_box("abc", style()),
        ];
        let space = fonts.measure(" ", 16.0);
        assert_eq!(xs(&run), [0.0, ideograph + space]);
    }

    #[test]
    fn text_wrap_balance_evens_the_lines_out_and_pretty_leaves_no_word_alone() {
        let fonts = test_fonts();
        let width = fonts.measure("aa aa aa aa", 16.0) + 2.0;
        let rows = |text: &str, wrap_style| {
            let mut container = ComputedStyle::default();
            container.text_mut().text_wrap_style = wrap_style;
            let fragments = lay_styled(text, width, container);
            let mut rows: Vec<usize> = Vec::new();
            let mut last_y = None;
            for f in &fragments {
                if last_y != Some(f.y) {
                    rows.push(0);
                    last_y = Some(f.y);
                }
                *rows.last_mut().unwrap() += 1;
            }
            rows
        };
        use style::computed::TextWrapStyle;
        let five = "aa aa aa aa aa";
        assert_eq!(rows(five, TextWrapStyle::Auto), [4, 1]);
        assert_eq!(rows(five, TextWrapStyle::Balance), [3, 2]);
        assert_eq!(rows(five, TextWrapStyle::Pretty), [3, 2]);
        // `pretty` moves one word down; `balance` evens every line out.
        let nine = "aa aa aa aa aa aa aa aa aa";
        assert_eq!(rows(nine, TextWrapStyle::Auto), [4, 4, 1]);
        assert_eq!(rows(nine, TextWrapStyle::Balance), [3, 3, 3]);
        assert_eq!(rows(nine, TextWrapStyle::Pretty), [4, 3, 2]);
    }

    #[test]
    fn start_and_end_follow_the_direction() {
        // css-text-3 §7.1 (M21.7): `start`, the initial value, is the
        // right in right-to-left text, and `end` the left.
        let fonts = test_fonts();
        let width = fonts.measure("hi", 16.0);
        let at = |align, direction| {
            lay_styled(
                "hi",
                100.0,
                ComputedStyle {
                    text_align: align,
                    direction,
                    ..ComputedStyle::default()
                },
            )[0]
            .x
        };
        assert_eq!(at(TextAlign::Start, Direction::Ltr), 0.0);
        assert_eq!(at(TextAlign::Start, Direction::Rtl), 100.0 - width);
        assert_eq!(at(TextAlign::End, Direction::Ltr), 100.0 - width);
        assert_eq!(at(TextAlign::End, Direction::Rtl), 0.0);
    }

    #[test]
    fn auto_justification_spreads_ideographs_but_not_letters() {
        let container = ComputedStyle {
            text_align: TextAlign::Justify,
            text_align_last: TextAlignLast::Align(TextAlign::Justify),
            ..ComputedStyle::default()
        };
        let xs = |text: &str| -> Vec<(String, f32)> {
            lay_styled(text, 200.0, container.clone())
                .iter()
                .map(|f| match &f.kind {
                    FragmentKind::Text(t) => (t.clone(), f.x + f.w),
                    _ => unreachable!("texts"),
                })
                .collect()
        };
        // Two ideographs, a bidi isolate between them taking no share.
        let wide = xs("\u{6771}\u{2066}\u{4eac}\u{2069}");
        let last = wide.iter().rfind(|(t, _)| t == "\u{4eac}").expect("drawn");
        assert!((last.1 - 200.0).abs() < 0.01, "{wide:?}");
        // A word of letters stays one fragment, at the start.
        assert_eq!(xs("ab").len(), 1);
    }

    #[test]
    fn kept_spaces_are_justified_but_those_that_hang() {
        let fonts = test_fonts();
        let w = |s: &str| fonts.measure(s, 16.0);
        let pre_wrap = ComputedStyle {
            font_size_px: 16.0,
            white_space: WhiteSpace::PRE_WRAP,
            ..ComputedStyle::default()
        };
        let container = ComputedStyle {
            text_align: TextAlign::Justify,
            ..ComputedStyle::default()
        };
        let width = w("aa bb c");
        let items = [styled_text_box("aa bb cc", pre_wrap)];
        let mut floats = FloatContext::default();
        let l = layout_inline(
            &items,
            width,
            None,
            &container,
            None,
            &mut floats,
            At::root(),
            &Pass::new(&fonts),
            true,
        );
        let shown: Vec<(String, f32)> = texts(&l.fragments)
            .iter()
            .map(|f| match &f.kind {
                FragmentKind::Text(t) => (t.clone(), f.x),
                _ => unreachable!("texts"),
            })
            .collect();
        // The line's one kept space between words takes the slack and the
        // hanging space's width: "bb" ends at the line's end.
        assert_eq!(shown[0], ("aa ".to_owned(), 0.0));
        assert_eq!(shown[1].0, "bb ");
        assert!((shown[1].1 + w("bb") - width).abs() < 0.01, "{shown:?}");
        assert_eq!(shown[2], ("cc".to_owned(), 0.0));
    }

    #[test]
    fn a_justified_line_shares_its_slack_among_its_gaps() {
        let fonts = test_fonts();
        let space = fonts.measure(" ", 16.0);
        let text = "aaa bb cccc dd eee ffff gg hhh iii";
        let justified = |container: ComputedStyle| -> Vec<(String, f32, f32)> {
            lay_styled(text, 200.0, container)
                .iter()
                .map(|f| match &f.kind {
                    FragmentKind::Text(t) => (t.clone(), f.x, f.y),
                    _ => unreachable!("texts"),
                })
                .collect()
        };
        let style = |all, last, justify| ComputedStyle {
            text_align: all,
            text_align_last: last,
            text_justify: justify,
            ..ComputedStyle::default()
        };
        let words = justified(style(
            TextAlign::Justify,
            TextAlignLast::Auto,
            TextJustify::Auto,
        ));
        let first = words[0].2;
        let line: Vec<&(String, f32, f32)> = words.iter().filter(|w| w.2 == first).collect();
        // The first line runs from edge to edge, each gap as wide as the
        // others: a space and an equal share of the slack.
        assert_eq!(line[0].1, 0.0);
        let last = line.last().expect("a word");
        assert!((last.1 + fonts.measure(&last.0, 16.0) - 200.0).abs() < 0.01);
        let gaps: Vec<f32> = line
            .windows(2)
            .map(|w| w[1].1 - (w[0].1 + fonts.measure(&w[0].0, 16.0)))
            .collect();
        assert!(
            gaps.iter()
                .all(|g| (g - gaps[0]).abs() < 0.01 && *g > space)
        );
        // The last line is aligned to its start, or as
        // `text-align-last` says.
        let rest: Vec<&(String, f32, f32)> = words.iter().filter(|w| w.2 != first).collect();
        assert_eq!(rest[0].1, 0.0);
        assert_eq!(
            rest[1].1 - (rest[0].1 + fonts.measure(&rest[0].0, 16.0)),
            space
        );
        let centred = justified(style(
            TextAlign::Justify,
            TextAlignLast::Align(TextAlign::Center),
            TextJustify::Auto,
        ));
        assert!(centred.iter().find(|w| w.2 != first).unwrap().1 > 0.0);
        let all = justified(style(
            TextAlign::Justify,
            TextAlignLast::Align(TextAlign::Justify),
            TextJustify::Auto,
        ));
        let last = all.last().expect("a word");
        assert!((last.1 + fonts.measure(&last.0, 16.0) - 200.0).abs() < 0.01);
        // `none`: no justification, the line at its start.
        let none = justified(style(
            TextAlign::Justify,
            TextAlignLast::Auto,
            TextJustify::None,
        ));
        assert_eq!(none[1].1, fonts.measure("aaa", 16.0) + space);
        // A line with no gap is at its start, as is one too long.
        let one = lay_styled(
            "word",
            200.0,
            style(
                TextAlign::Justify,
                TextAlignLast::Align(TextAlign::Justify),
                TextJustify::Auto,
            ),
        );
        assert_eq!(one[0].x, 0.0);
    }

    #[test]
    fn inter_character_justification_spaces_the_characters_too() {
        let fonts = test_fonts();
        let fragments = lay_styled(
            "ab cd",
            100.0,
            ComputedStyle {
                text_align: TextAlign::Justify,
                text_align_last: TextAlignLast::Align(TextAlign::Justify),
                text_justify: TextJustify::InterCharacter,
                ..ComputedStyle::default()
            },
        );
        // One fragment a character, the slack shared among the gap and
        // the two places between characters: three shares.
        let xs: Vec<f32> = fragments.iter().map(|f| f.x).collect();
        assert_eq!(xs.len(), 4);
        let slack = 100.0
            - fonts.measure("ab", 16.0)
            - fonts.measure(" ", 16.0)
            - fonts.measure("cd", 16.0);
        let share = slack / 3.0;
        assert!((xs[1] - (fonts.measure("a", 16.0) + share)).abs() < 0.01);
        let end = xs[3] + fonts.measure("d", 16.0);
        assert!((end - 100.0).abs() < 0.01);
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
            style: style.into(),
            kind: BoxKind::Text(text.to_string()),
            children: Default::default(),
            node: crate::testing::no_node(),
            anonymous: true,
            generated: None,
            generated_text: 0,
            decorations: None,
            styled_by_ancestor: false,
            first_line: None,
            tree_order: 0,
            item: false,
            parent: None,
            in_3d: false,
            replaced: None,
            control_size: None,
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
            }
            .into(),
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
    fn overflow_wrap_cuts_a_word_glued_to_the_text_before_it() {
        let fonts = test_fonts();
        let anywhere = ComputedStyle {
            overflow_wrap: style::computed::OverflowWrap::Anywhere,
            ..ComputedStyle::default()
        };
        let (fragments, _) = lay(
            &[
                text_box("12345", 16.0),
                inline_box(vec![styled_text_box("67890", anywhere)]),
            ],
            fonts.measure("12345678", 16.0) + 0.5,
            TextAlign::Left,
            &fonts,
        );
        let shown: Vec<(String, f32)> = texts(&fragments)
            .iter()
            .map(|f| match &f.kind {
                FragmentKind::Text(t) => (t.clone(), f.y),
                _ => unreachable!("texts"),
            })
            .collect();
        let texts: Vec<&str> = shown.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(texts, ["12345", "678", "90"]);
        assert_eq!(shown[0].1, shown[1].1);
        assert!(shown[2].1 > shown[1].1);
    }

    #[test]
    fn nowrap_text_does_not_break_beside_an_atomic_inline() {
        let fonts = test_fonts();
        let nowrap = ComputedStyle {
            white_space: WhiteSpace::NOWRAP,
            ..ComputedStyle::default()
        };
        let atomic = |white_space| LayoutBox {
            style: ComputedStyle {
                display: style::computed::Display::InlineBlock,
                width: style::computed::Size::Px(30.0),
                height: style::computed::Size::Px(10.0),
                white_space,
                ..ComputedStyle::default()
            }
            .into(),
            kind: BoxKind::InlineBlock,
            ..inline_box(Vec::new())
        };
        let ys = |white_space, text: ComputedStyle| {
            let (fragments, _) = lay(
                &[
                    styled_text_box("aaa", text.clone()),
                    atomic(white_space),
                    styled_text_box("\u{a0}bbb", text),
                ],
                1.0,
                TextAlign::Left,
                &fonts,
            );
            let mut ys: Vec<f32> = texts(&fragments).iter().map(|f| f.y).collect();
            ys.dedup();
            ys.len()
        };
        assert_eq!(ys(WhiteSpace::NOWRAP, nowrap), 1, "one line");
        assert_eq!(ys(WhiteSpace::NORMAL, ComputedStyle::default()), 2);
        // Two atomics with no break between them are one piece of the
        // min-content width.
        let min = |white_space| {
            let run = [atomic(white_space), atomic(white_space)];
            let items = collect_inline_items(&run, &fonts);
            run_intrinsic_widths(&items, None, &Pass::new(&fonts)).0
        };
        assert_eq!(min(WhiteSpace::NOWRAP), 60.0);
        assert_eq!(min(WhiteSpace::NORMAL), 30.0);
    }

    #[test]
    fn white_space_pre_line_preserves_newlines_as_forced_breaks() {
        let fonts = test_fonts();
        let style = ComputedStyle {
            white_space: WhiteSpace::PRE_LINE,
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
    fn a_space_after_an_empty_inline_at_a_lines_start_goes() {
        // css-text-3 §4.1.2: the space after `<span></span>` at a line's
        // start is at its start still, an inline box's edge being no
        // content, as in the reference browser; after a word it stays.
        let fonts = test_fonts();
        let items = [inline_box(vec![]), text_box(" a", 16.0)];
        let (fragments, _) = lay(&items, 800.0, TextAlign::Left, &fonts);
        assert_eq!(texts(&fragments)[0].x, 0.0);
        let items = [
            text_box("a", 16.0),
            inline_box(vec![]),
            text_box(" b", 16.0),
        ];
        let (fragments, _) = lay(&items, 800.0, TextAlign::Left, &fonts);
        let gap = space_width(&ComputedStyle::default(), &fonts);
        assert_eq!(texts(&fragments)[1].x, fonts.measure("a", 16.0) + gap);
    }

    #[test]
    fn a_gap_is_a_space_of_the_white_spaces_own_font() {
        // `a <span>b</span>`: the space is the 16px text's, whatever the
        // span's font; `<span>a </span>b`: the span's (the reference
        // browser's).
        let fonts = test_fonts();
        let big = |children: Vec<LayoutBox>| {
            let mut span = inline_box(children);
            Arc::make_mut(&mut span.style).font_size_px = 32.0;
            span
        };
        let items = [text_box("a ", 16.0), big(vec![text_box("b", 32.0)])];
        let (fragments, _) = lay(&items, 800.0, TextAlign::Left, &fonts);
        let small_gap = fonts.measure(" ", 16.0);
        assert_eq!(texts(&fragments)[1].x, fonts.measure("a", 16.0) + small_gap);
        let items = [big(vec![text_box("a ", 32.0)]), text_box("b", 16.0)];
        let (fragments, _) = lay(&items, 800.0, TextAlign::Left, &fonts);
        assert_eq!(
            texts(&fragments)[1].x,
            fonts.measure("a", 32.0) + fonts.measure(" ", 32.0)
        );
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

    /// The lines of `text` in `style` at 16px laid out `width` wide: each
    /// line's texts, joined.
    fn lines_of(text: &str, style: ComputedStyle, width: f32) -> Vec<String> {
        let fonts = test_fonts();
        let style = ComputedStyle {
            font_size_px: 16.0,
            ..style
        };
        let (fragments, _) = lay(
            &[styled_text_box(text, style)],
            width,
            TextAlign::Left,
            &fonts,
        );
        let mut lines: Vec<(f32, String)> = Vec::new();
        for f in &fragments {
            let FragmentKind::Text(t) = &f.kind else {
                continue;
            };
            match lines.last_mut() {
                Some((y, line)) if *y == f.y => {
                    line.push('|');
                    line.push_str(t);
                }
                _ => lines.push((f.y, t.clone())),
            }
        }
        lines.into_iter().map(|(_, line)| line).collect()
    }

    #[test]
    fn lines_break_where_uax_14_lets_them_and_where_white_space_says() {
        // D55.
        let fonts = test_fonts();
        let w = |s: &str| fonts.measure(s, 16.0);
        let normal = ComputedStyle::default();
        // After a hyphen, within a word.
        assert_eq!(
            lines_of("well-known", normal.clone(), w("known") + 1.0),
            ["well-", "known"]
        );
        // `nowrap` keeps a line whole; collapsible spaces still collapse.
        let nowrap = ComputedStyle {
            white_space: WhiteSpace::NOWRAP,
            ..ComputedStyle::default()
        };
        assert_eq!(lines_of("aa   bb", nowrap, 1.0), ["aa|bb"]);
        // `pre-wrap` keeps spaces, which hang at a line's end; under
        // `break-spaces` they take room and wrap.
        let wrap = |white_space| ComputedStyle {
            white_space,
            ..ComputedStyle::default()
        };
        let room = w("aaaa") + 1.0;
        assert_eq!(
            lines_of("aaaa    bbbb", wrap(WhiteSpace::PRE_WRAP), room),
            ["aaaa    ", "bbbb"]
        );
        assert_eq!(
            lines_of(
                "aaaa  bbbb",
                wrap(WhiteSpace::BREAK_SPACES),
                w("aaaa ") + 1.0
            ),
            ["aaaa ", " bbbb"]
        );
        // `word-break: break-all` breaks between letters.
        let break_all = ComputedStyle {
            word_break: style::computed::WordBreak::BreakAll,
            ..ComputedStyle::default()
        };
        // A word cut only where a line breaks: each line one fragment.
        assert_eq!(lines_of("aaaa", break_all, w("aa") + 0.5), ["aa", "aa"]);
        // A word too long for its line overflows, unless `overflow-wrap`
        // breaks it where the line ends.
        assert_eq!(lines_of("aaaaa", normal, w("aa") + 0.5), ["aaaaa"]);
        for overflow_wrap in [
            style::computed::OverflowWrap::Anywhere,
            style::computed::OverflowWrap::BreakWord,
        ] {
            let style = ComputedStyle {
                overflow_wrap,
                ..ComputedStyle::default()
            };
            assert_eq!(lines_of("aaaaa", style, w("aa") + 0.5), ["aa", "aa", "a"]);
        }
        let style = ComputedStyle {
            word_break: style::computed::WordBreak::BreakWord,
            ..ComputedStyle::default()
        };
        assert_eq!(
            lines_of("aaa", style, 1.0),
            ["a", "a", "a"],
            "a letter at least"
        );
    }

    #[test]
    fn white_space_keeps_newlines_wraps_and_hangs_as_its_longhands_say() {
        let fonts = test_fonts();
        let w = |s: &str| fonts.measure(s, 16.0);
        // `nowrap` collapses newlines as `normal` does.
        let nowrap = ComputedStyle {
            white_space: WhiteSpace::NOWRAP,
            ..ComputedStyle::default()
        };
        assert_eq!(lines_of("aa\nbb", nowrap, 400.0), ["aa|bb"]);
        // `keep-all` keeps ideographs together; normally each may break.
        let cjk = "\u{65e5}\u{672c}\u{8a9e}";
        let keep_all = ComputedStyle {
            word_break: style::computed::WordBreak::KeepAll,
            ..ComputedStyle::default()
        };
        assert_eq!(lines_of(cjk, ComputedStyle::default(), 1.0).len(), 3);
        assert_eq!(lines_of(cjk, keep_all, 1.0).len(), 1);
        // Spaces kept at the end of a line hang after what is on it.
        let normal = ComputedStyle {
            font_size_px: 16.0,
            ..ComputedStyle::default()
        };
        let pre_wrap = ComputedStyle {
            white_space: WhiteSpace::PRE_WRAP,
            ..normal.clone()
        };
        let (fragments, _) = lay(
            &[
                styled_text_box("x ", normal),
                styled_text_box("aaaa    ", pre_wrap),
            ],
            w("x aaaa") + 1.0,
            TextAlign::Left,
            &fonts,
        );
        let ys: Vec<f32> = fragments.iter().map(|f| f.y).collect();
        assert!(ys.iter().all(|&y| y == ys[0]), "one line: {ys:?}");
    }

    #[test]
    fn a_form_feed_is_a_character_not_white_space() {
        assert_eq!(
            lines_of("a\u{c}b c", ComputedStyle::default(), 400.0),
            ["a\u{c}b|c"]
        );
    }

    #[test]
    fn other_space_separators_hang_or_under_break_spaces_wrap() {
        let fonts = test_fonts();
        let w = |s: &str| fonts.measure(s, 16.0);
        let space = "\u{3000}";
        // Ideographic spaces at a line's end hang past it, collapsible
        // spaces between them with them, an inline's end edge after.
        let text = format!("x ab{space}{space}cd");
        assert_eq!(
            lines_of(&text, ComputedStyle::default(), w("x ab") + 1.0),
            [format!("x|ab{space}{space}"), "cd".to_owned()]
        );
        let boxes = [
            text_box("ab", 16.0),
            inline_box(vec![text_box(&format!("{space} {space}"), 16.0)]),
            text_box("cd", 16.0),
        ];
        let (fragments, _) = lay(&boxes, w("ab") + 1.0, TextAlign::Left, &fonts);
        let all = texts(&fragments);
        let y_of = |t: &str| {
            all.iter()
                .find(|f| matches!(&f.kind, FragmentKind::Text(s) if s == t))
                .map(|f| f.y)
        };
        let spaces: Vec<f32> = all
            .iter()
            .filter(|f| matches!(&f.kind, FragmentKind::Text(s) if s == space))
            .map(|f| f.y)
            .collect();
        assert_eq!(spaces.len(), 2);
        assert!(spaces.iter().all(|&y| Some(y) == y_of("ab")), "one line");
        assert!(y_of("cd") > y_of("ab"));
        // Under `break-spaces` a line breaks after each, which do not hang.
        let break_spaces = ComputedStyle {
            white_space: WhiteSpace::BREAK_SPACES,
            ..ComputedStyle::default()
        };
        let text = format!("ab{space}{space}{space}cd");
        let width = w(&format!("ab{space}")) + 1.0;
        assert_eq!(
            lines_of(&text, break_spaces, width),
            [format!("ab{space}"), space.repeat(2), "cd".to_owned()]
        );
    }

    #[test]
    fn kept_tabs_go_to_the_next_tab_stop_from_the_lines_start() {
        let fonts = test_fonts();
        let space = fonts.measure(" ", 16.0);
        let pre = |tab_size| {
            let mut style = ComputedStyle {
                white_space: WhiteSpace::PRE,
                font_size_px: 16.0,
                ..ComputedStyle::default()
            };
            style.text_mut().tab_size = tab_size;
            style
        };
        use style::computed::TabSize;
        // `b` after a tab of eight spaces from the line's start.
        let x_of_b = |run: &[LayoutBox]| {
            let (fragments, _) = lay(run, 400.0, TextAlign::Left, &fonts);
            texts(&fragments)
                .iter()
                .find(|f| matches!(&f.kind, FragmentKind::Text(t) if t == "b"))
                .unwrap()
                .x
        };
        let eight = pre(TabSize::Spaces(8.0));
        assert_eq!(
            x_of_b(&[styled_text_box("a\tb", eight.clone())]),
            8.0 * space
        );
        // The stop counted from the line's start, not the text's.
        let run = [
            styled_text_box("aa", eight.clone()),
            styled_text_box("\tb", eight.clone()),
        ];
        assert_eq!(x_of_b(&run), 8.0 * space);
        // A length; and the next stop where this one is under half a
        // space away.
        let px = pre(TabSize::Length(30.0));
        assert_eq!(x_of_b(&[styled_text_box("\tb", px.clone())]), 30.0);
        // A tab alone in its text, an element after it.
        let run = [
            styled_text_box("\t", px.clone()),
            inline_box(vec![styled_text_box("b", px.clone())]),
        ];
        assert_eq!(x_of_b(&run), 30.0);
        let near = 30.0 - fonts.measure("m", 16.0);
        assert!(near > space / 2.0, "m is far enough from 30px");
        assert_eq!(x_of_b(&[styled_text_box("m\tb", px)]), 30.0);
        // A tab size of zero: no tab.
        let zero = pre(TabSize::Spaces(0.0));
        assert_eq!(x_of_b(&[styled_text_box("\tb", zero)]), 0.0);
        // The max-content width counts it.
        let run = [styled_text_box("\tb", eight)];
        let items = collect_inline_items(&run, &fonts);
        let (_, max) = run_intrinsic_widths(&items, None, &Pass::new(&fonts));
        assert_eq!(max, 8.0 * space + fonts.measure("b", 16.0));
    }

    #[test]
    fn a_words_first_segment_joins_what_it_is_glued_to() {
        // D55: "xx" and "yy-zz" set flush; the min-content piece is
        // "xxyy-", where the line may first break.
        let fonts = test_fonts();
        let w = |s: &str| fonts.measure(s, 16.0);
        let run = [text_box("xx", 16.0), text_box("yy-zz", 16.0)];
        let items = collect_inline_items(&run, &fonts);
        let (min, _) = run_intrinsic_widths(&items, None, &Pass::new(&fonts));
        assert_eq!(min, w("xx") + w("yy-"));
        // After a gap it starts a piece of its own.
        let run = [text_box("a bb-cc", 16.0)];
        let items = collect_inline_items(&run, &fonts);
        let (min, _) = run_intrinsic_widths(&items, None, &Pass::new(&fonts));
        assert_eq!(min, w("bb-"));
    }

    #[test]
    fn overflow_wrap_anywhere_counts_toward_the_min_content_width() {
        let fonts = test_fonts();
        let w = |s: &str| fonts.measure(s, 16.0);
        let min = |overflow_wrap| {
            let style = ComputedStyle {
                overflow_wrap,
                font_size_px: 16.0,
                ..ComputedStyle::default()
            };
            let run = [styled_text_box("mmm ii", style)];
            let items = collect_inline_items(&run, &fonts);
            run_intrinsic_widths(&items, None, &Pass::new(&fonts)).0
        };
        assert_eq!(min(style::computed::OverflowWrap::Normal), w("mmm"));
        assert_eq!(min(style::computed::OverflowWrap::BreakWord), w("mmm"));
        assert_eq!(min(style::computed::OverflowWrap::Anywhere), w("m"));
    }

    #[test]
    fn word_break_break_word_breaks_grapheme_clusters_for_the_min_content_width() {
        let fonts = test_fonts();
        let min = |word_break, text: &str| {
            let style = ComputedStyle {
                word_break,
                font_size_px: 16.0,
                ..ComputedStyle::default()
            };
            let run = [styled_text_box(text, style)];
            let items = collect_inline_items(&run, &fonts);
            run_intrinsic_widths(&items, None, &Pass::new(&fonts)).0
        };
        let w = |s: &str| fonts.measure(s, 16.0);
        assert_eq!(min(style::computed::WordBreak::Normal, "mmm"), w("mmm"));
        assert_eq!(
            min(style::computed::WordBreak::BreakWord, "mmm"),
            w("mmm") / 3.0
        );
        // A letter and its combining mark are one unit.
        assert_eq!(
            min(style::computed::WordBreak::BreakWord, "ie\u{301}i"),
            w("e\u{301}")
        );
    }

    #[test]
    fn hyphens_break_words_at_soft_hyphens_and_by_patterns() {
        use style::computed::Hyphens;
        let style = |hyphens, lang: &str, character: Option<&str>| {
            let mut style = ComputedStyle::default();
            style.text_mut().hyphens = hyphens;
            style.text_mut().lang = Some(Arc::from(lang));
            style.text_mut().hyphenate_character = character.map(Arc::from);
            style
        };
        let shy = "a\u{ad}b";
        assert_eq!(
            lines_of(shy, style(Hyphens::Manual, "en", None), 1.0),
            ["a\u{ad}\u{2010}", "b"]
        );
        assert_eq!(lines_of(shy, style(Hyphens::None, "en", None), 1.0), [shy]);
        assert_eq!(
            lines_of("extensive", style(Hyphens::Auto, "en", Some("=")), 1.0),
            ["ex=", "ten=", "sive"]
        );
        // Not without a language, nor under `manual`.
        assert_eq!(
            lines_of("extensive", style(Hyphens::Auto, "", None), 1.0),
            ["extensive"]
        );
        assert_eq!(
            lines_of("extensive", style(Hyphens::Manual, "en", None), 1.0),
            ["extensive"]
        );
        // Where the line has room, no hyphen.
        assert_eq!(
            lines_of("extensive", style(Hyphens::Auto, "en", None), 400.0),
            ["extensive"]
        );
        // The min-content width is the widest part with its hyphen.
        let fonts = test_fonts();
        let run = [styled_text_box(
            "extensive",
            ComputedStyle {
                font_size_px: 16.0,
                ..style(Hyphens::Auto, "en", Some("="))
            },
        )];
        let items = collect_inline_items(&run, &fonts);
        let (min, _) = run_intrinsic_widths(&items, None, &Pass::new(&fonts));
        assert_eq!(min, fonts.measure("ten=", 16.0));
    }

    #[test]
    fn line_break_says_how_strictly_lines_break() {
        let style = |line_break, lang: &str| {
            let mut style = ComputedStyle::default();
            style.text_mut().line_break = line_break;
            style.text_mut().lang = Some(Arc::from(lang));
            style
        };
        use style::computed::LineBreak;
        let kana = "\u{6587}\u{3041}";
        assert_eq!(lines_of(kana, style(LineBreak::Strict, "ja"), 1.0), [kana]);
        assert_eq!(lines_of(kana, style(LineBreak::Auto, "ja"), 1.0), [kana]);
        assert_eq!(
            lines_of(kana, style(LineBreak::Normal, "ja"), 1.0),
            ["\u{6587}", "\u{3041}"]
        );
        // The wave dash starts a line under `normal` in Japanese alone.
        let dash = "\u{6587}\u{301c}";
        assert_eq!(lines_of(dash, style(LineBreak::Normal, "en"), 1.0), [dash]);
        assert_eq!(
            lines_of(dash, style(LineBreak::Normal, "ja-JP"), 1.0),
            ["\u{6587}", "\u{301c}"]
        );
        // `anywhere` between letters.
        assert_eq!(
            lines_of("ab", style(LineBreak::Anywhere, "en"), 1.0),
            ["a", "b"]
        );
    }

    #[test]
    fn glued_text_in_two_boxes_breaks_where_uax_14_says() {
        let fonts = test_fonts();
        // Two ideographs: an opportunity between them, the inline box's
        // start edge going with what it holds.
        let run = [
            text_box("\u{6587}", 16.0),
            inline_box(vec![text_box("\u{5b57}", 16.0)]),
        ];
        let (fragments, _) = lay(&run, 1.0, TextAlign::Left, &fonts);
        let words = texts(&fragments);
        assert_eq!(texts_of(&words), ["\u{6587}", "\u{5b57}"]);
        assert!(words[1].y > words[0].y, "the second on a line of its own");
        let inline = fragments
            .iter()
            .find(|f| matches!(f.kind, FragmentKind::InlineBox))
            .unwrap();
        assert!(inline.y >= words[1].y - words[1].ascent, "with its box");
        // Two letters: none.
        let run = [text_box("a", 16.0), inline_box(vec![text_box("b", 16.0)])];
        let (fragments, _) = lay(&run, 1.0, TextAlign::Left, &fonts);
        let words = texts(&fragments);
        assert_eq!(words[0].y, words[1].y);
        // A hyphen ends a piece: `well-<b>known</b>`.
        let run = [
            text_box("well-", 16.0),
            inline_box(vec![text_box("known", 16.0)]),
        ];
        let (fragments, _) = lay(&run, 1.0, TextAlign::Left, &fonts);
        let words = texts(&fragments);
        assert!(words[1].y > words[0].y);
    }

    #[test]
    fn white_space_pre_never_wraps_even_when_too_narrow() {
        let fonts = test_fonts();
        let word_width = fonts.measure("aaaa", 16.0);
        assert!(word_width > 0.0);
        let style = ComputedStyle {
            white_space: WhiteSpace::PRE,
            font_size_px: 16.0,
            ..ComputedStyle::default()
        };
        // Same narrow width that forces 3 separate lines under `normal`
        // (see `narrow_width_wraps_each_word_onto_its_own_line`): under
        // `pre`, one line, its spaces kept as text (D55), each a space
        // wide, a run of them too.
        let (fragments, _) = lay(
            &[styled_text_box("aaaa bbbb  cccc", style)],
            word_width + 1.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(texts_of(&fragments), vec!["aaaa bbbb  cccc"]);
        assert_eq!(fragments[0].w, fonts.measure("aaaa bbbb  cccc", 16.0));
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
    fn letter_spacing_spaces_clusters_and_the_space_but_not_joined_letters() {
        // A grapheme cluster is one unit; a cursive script's letters stay
        // joined and take none, nor does a zero-width format character.
        fn units(text: &str) -> Vec<(&str, bool)> {
            letter_units(text)
                .into_iter()
                .map(|(range, spaced)| (&text[range], spaced))
                .collect()
        }
        assert_eq!(units("ab"), [("a", true), ("b", true)]);
        assert_eq!(units("e\u{301}x"), [("e\u{301}", true), ("x", true)]);
        assert_eq!(
            units("a\u{200b}b"),
            [("a", true), ("\u{200b}", false), ("b", true)]
        );
        assert_eq!(
            units("\u{645}\u{631}\u{62d}a"),
            [("\u{645}\u{631}\u{62d}", false), ("a", true)]
        );
        // The gap between two words takes it as a space would.
        let fonts = test_fonts();
        let style = ComputedStyle {
            font_size_px: 16.0,
            letter_spacing: 5.0,
            ..ComputedStyle::default()
        };
        let (fragments, _) = lay(
            &[styled_text_box("a b", style)],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        let space = font::measure(&fonts, " ", &ComputedStyle::default());
        let gap = fragments[1].x - (fragments[0].x + fragments[0].w);
        assert!((gap - (5.0 + space + 5.0)).abs() < 1e-3, "{gap}");
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
        for ws in [WhiteSpace::PRE, WhiteSpace::PRE_WRAP, WhiteSpace::PRE_LINE] {
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
            white_space: WhiteSpace::PRE,
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
        let (min, max) = run_intrinsic_widths(&items, None, &Pass::new(&fonts));
        assert_eq!(min, w("cccc"));
        assert_eq!(max, w("aaa") + space + w("bb") + space + w("cccc"));
        // Words set flush are one unbreakable piece.
        let run = [
            text_box("aaa", 16.0),
            inline_box(vec![text_box("bb", 16.0)]),
        ];
        let items = collect_inline_items(&run, &fonts);
        let (min, max) = run_intrinsic_widths(&items, None, &Pass::new(&fonts));
        assert_eq!(min, w("aaa") + w("bb"));
        assert_eq!(max, min);
        // Under `pre` a line is one piece, and a newline ends it.
        let pre = ComputedStyle {
            white_space: WhiteSpace::PRE,
            ..ComputedStyle::default()
        };
        let run = [styled_text_box("a b\ncccc", pre)];
        let items = collect_inline_items(&run, &fonts);
        let (min, max) = run_intrinsic_widths(&items, None, &Pass::new(&fonts));
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
                    style: style(32.0).into(),
                    chain: Vec::new(),
                })),
                ..styled_text_box(text, style(16.0))
            }
        };
        let w = |text, size| fonts.measure(text, size);
        let boxes = [on_first_line("aa bb", WhiteSpace::NORMAL)];
        let items = collect_inline_items(&boxes, &fonts);
        let first = collect_first_line_items(&boxes, &fonts, None);
        assert_eq!(
            run_intrinsic_widths(&items, Some(&first), &Pass::new(&fonts)),
            (
                w("aa", 32.0).max(w("bb", 16.0)),
                w("aa", 32.0) + w(" ", 32.0) + w("bb", 32.0)
            )
        );
        // After a forced break, the next line is the run's own.
        let boxes = [on_first_line("aa\nbbbbbb", WhiteSpace::PRE)];
        let items = collect_inline_items(&boxes, &fonts);
        let first = collect_first_line_items(&boxes, &fonts, None);
        let (min, max) = run_intrinsic_widths(&items, Some(&first), &Pass::new(&fonts));
        assert_eq!(min, max);
        assert_eq!(max, w("aa", 32.0).max(w("bbbbbb", 16.0)));
    }

    /// `b` with `style` on the first formatted line.
    fn with_first_line(b: LayoutBox, style: ComputedStyle) -> LayoutBox {
        LayoutBox {
            first_line: Some(Box::new(FirstLine {
                style: style.into(),
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
                InlineItem::Marker { .. } => "marker".to_string(),
            })
            .collect()
    }

    #[test]
    fn the_first_lines_items_end_at_the_first_break_past_the_room() {
        // Once the words so far are wider than the room, the line has
        // ended at the next break opportunity: the items end with it.
        let fonts = test_fonts();
        let w = |t| fonts.measure(t, 32.0);
        let normal = || big(WhiteSpace::NORMAL);
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
            white_space: WhiteSpace::PRE_LINE,
            ..ComputedStyle::default()
        };
        let run = [with_first_line(
            styled_text_box("aa bb\ncc", pre_line),
            big(WhiteSpace::PRE_LINE),
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
        let normal = || big(WhiteSpace::NORMAL);
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
            white_space: WhiteSpace::PRE,
            ..ComputedStyle::default()
        };
        let run = [
            with_first_line(styled_text_box("aa bb ", pre.clone()), big(WhiteSpace::PRE)),
            with_first_line(
                inline_box(vec![with_first_line(
                    styled_text_box("cc", pre.clone()),
                    big(WhiteSpace::PRE),
                )]),
                big(WhiteSpace::PRE),
            ),
            with_first_line(styled_text_box("\ndd", pre), big(WhiteSpace::PRE)),
        ];
        for room in [Some(0.0), None] {
            assert_eq!(
                shapes(&collect_first_line_items(&run, &fonts, room)),
                ["aa bb @32", "<", "cc@32", ">", "/"]
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
        let normal = || big(WhiteSpace::NORMAL);
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
        let restyled = || big(WhiteSpace::NORMAL);
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
