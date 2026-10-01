//! Inline layout: line boxes and greedy line breaking (roadmap M7).
//!
//! Replaces M6's "every text run is its own line" stand-in with real
//! text flow: words measured through `text::FontCollection` (a
//! per-script fallback font, M8.1 — not just the one primary font
//! every word used before it), packed onto lines greedily at
//! whitespace break opportunities (css-text-3 §3's
//! `white-space: normal` collapsing — `str::split_whitespace` already
//! implements exactly that collapsing), baseline-aligned across mixed
//! font sizes on the same line (the line's shared baseline sits at the
//! tallest ascent present), and aligned per `text-align`. `white-space`
//! (M8.4) selects a different tokenization; `text-transform` (M8.4) runs
//! before any of it; `letter-spacing`/`word-spacing` (M8.4) adjust the
//! measured/rendered widths. Bidirectional text (M8.3) reorders each
//! resulting line into visual order — via `text::bidi_reorder`, keyed off
//! the computed `direction` — before word-splitting, so wrapping then
//! just packs words left to right same as any other text.
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
//! * Block-in-inline (M7.6: a block box nested inside inline content,
//!   e.g. `<span>a<div>b</div>c</span>`) doesn't restructure the box
//!   tree into real anonymous boxes the way browsers do — it ends the
//!   current line, lays the block out on its own at the full available
//!   width (via `block::layout_block`, not collapsing margins with
//!   anything outside it), and resumes a fresh line afterward. Same
//!   observable "splits the run" effect, simpler implementation.
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
//! * `line-height` and vertical-align of replaced content are M8/M42
//!   territory.

use std::borrow::Cow;

use style::computed::{ComputedStyle, Direction, TextAlign, TextTransform, WhiteSpace};
use text::FontCollection;

use crate::block::{Fragment, FragmentKind, layout_block};
use crate::box_tree::{BoxKind, LayoutBox};

/// One flattened piece of inline content, in document order.
enum InlineItem<'a> {
    Word {
        text: String,
        /// Already includes `letter-spacing`'s contribution (measured
        /// consistently with how [`emit_letter_spaced`] renders it —
        /// see that function's doc for why that's the same code path).
        width: f32,
        style: &'a ComputedStyle,
        /// `white-space: pre`: this word's own trailing gap is never a
        /// line-break opportunity (text flows without wrapping).
        no_wrap: bool,
        /// The element the text belongs to (M9.4), carried into its
        /// fragment.
        node: usize,
    },
    /// A literal newline under `white-space: pre`/`pre-wrap`/`pre-line`
    /// — ends the current line unconditionally, however much of
    /// `available_width` is left. On an empty line it *is* the line: a
    /// blank line of the text's own font height.
    ForcedBreak(&'a ComputedStyle),
    /// See the module doc's block-in-inline note.
    Block(&'a LayoutBox),
}

/// One word placed on the line being built, before [`flush_line`]
/// positions it.
struct LineWord<'a> {
    text: &'a str,
    width: f32,
    style: &'a ComputedStyle,
    node: usize,
}

/// Lays a run of inline-level boxes out into line boxes within
/// `available_width`, returning fragments relative to the run's
/// container's content-box top-left `(0, 0)` and the total height
/// consumed — `block::layout_children`'s inline-run handling translates
/// both into the container's own coordinate space, same as any block
/// child's.
pub(crate) fn layout_inline(
    items: &[LayoutBox],
    available_width: f32,
    text_align: TextAlign,
    fonts: &FontCollection,
) -> (Vec<Fragment>, f32) {
    let mut flat = Vec::new();
    collect_inline_items(items, fonts, &mut flat);

    let mut fragments = Vec::new();
    let mut y = 0.0f32;
    let mut line: Vec<LineWord<'_>> = Vec::new();
    let mut line_width = 0.0f32;

    for item in &flat {
        match item {
            InlineItem::Word {
                text,
                width,
                style,
                no_wrap,
                node,
            } => {
                let gap = if line.is_empty() {
                    0.0
                } else {
                    space_width(style, fonts)
                };
                // Greedy line breaking at a soft-wrap opportunity
                // (M7.3): if this word doesn't fit, start a new line —
                // unless the line is still empty (a solo word wider
                // than `available_width` just overflows; see the
                // module doc's overflow-wrap note) or this word's own
                // `white-space: pre` forbids wrapping before it (M8.4).
                if !no_wrap && !line.is_empty() && line_width + gap + width > available_width {
                    flush_line(
                        &mut line,
                        line_width,
                        available_width,
                        text_align,
                        fonts,
                        &mut y,
                        &mut fragments,
                    );
                    line_width = *width;
                } else {
                    line_width += gap + width;
                }
                line.push(LineWord {
                    text: text.as_str(),
                    width: *width,
                    style,
                    node: *node,
                });
            }
            InlineItem::ForcedBreak(style) => {
                // `white-space: pre`/`pre-wrap`/`pre-line` (M8.4): a
                // literal newline ends the line unconditionally. A
                // break on an empty line is a blank line, as tall as
                // the strut of the text it is in (css-text-3 §4.1.3) —
                // a code block's blank lines are not nothing.
                if line.is_empty() {
                    y += fonts
                        .metrics_for_text(" ", style.font_size_px)
                        .line_height();
                } else {
                    flush_line(
                        &mut line,
                        line_width,
                        available_width,
                        text_align,
                        fonts,
                        &mut y,
                        &mut fragments,
                    );
                }
                line_width = 0.0;
            }
            InlineItem::Block(b) => {
                flush_line(
                    &mut line,
                    line_width,
                    available_width,
                    text_align,
                    fonts,
                    &mut y,
                    &mut fragments,
                );
                line_width = 0.0;

                let block = layout_block(b, available_width, fonts);
                // A self-collapsing block's one margin set counts once;
                // nothing adjoins it here, so it is simply spacing.
                let (above, below) = if block.self_collapsing {
                    (block.margin_top.resolve(), 0.0)
                } else {
                    (block.margin_top.resolve(), block.margin_bottom.resolve())
                };
                y += above;
                let margin_left = block.margin_left;
                fragments.push(block.root.translated(margin_left, y));
                y += block.height + below;
            }
        }
    }
    flush_line(
        &mut line,
        line_width,
        available_width,
        text_align,
        fonts,
        &mut y,
        &mut fragments,
    );

    (fragments, y)
}

/// Recursively flattens an inline/text subtree into [`InlineItem`]s,
/// applying `text-transform` and `white-space`-driven tokenization
/// (M8.4) and measuring each word (with `letter-spacing`, M8.4) as it
/// goes. A nested `Block` box (M7.6) is kept as its own item rather than
/// recursed into — it isn't inline content to flow onto a line, it ends
/// one.
fn collect_inline_items<'a>(
    boxes: &'a [LayoutBox],
    fonts: &FontCollection,
    out: &mut Vec<InlineItem<'a>>,
) {
    for b in boxes {
        match &b.kind {
            BoxKind::Text(text) => {
                let transformed = apply_text_transform(text, b.style.text_transform);
                let no_wrap = b.style.white_space == WhiteSpace::Pre;
                // `pre`/`pre-wrap`/`pre-line` all keep literal newlines
                // as forced breaks; only `normal` collapses them away
                // like any other whitespace (see the module doc for
                // what's *not* preserved: multi-space run width).
                let preserves_newlines = b.style.white_space != WhiteSpace::Normal;
                // Always split on literal newlines, even under `normal`
                // (where they're not a forced break — just whitespace
                // like any other, collapsed by `split_whitespace` below
                // same as before). This is also what keeps `\n` from
                // ever reaching `bidi_reorder`: `unicode-bidi` treats it
                // as its own paragraph separator and `bidi_reorder` only
                // returns the first paragraph (see its doc's "level 1"
                // scope cut), so a line containing an embedded `\n`
                // would silently lose everything after it.
                let lines: Vec<&str> = transformed.split('\n').collect();
                let base_direction = to_text_direction(b.style.direction);
                for (i, line) in lines.into_iter().enumerate() {
                    if i > 0 && preserves_newlines {
                        out.push(InlineItem::ForcedBreak(&b.style));
                    }
                    // Bidi reordering (M8.3), one paragraph per literal
                    // line. `direction` is always a definite Ltr/Rtl by
                    // the time it's computed (there's no `dir="auto"`
                    // yet — see `cascade`'s note), so the base direction
                    // passed here is always explicit, matching real UAs:
                    // an RTL run shapes right-to-left via UAX#9
                    // embedding regardless, but the *paragraph's* own
                    // direction only follows content when nothing set
                    // it, which this engine doesn't yet do.
                    let reordered = text::bidi_reorder(line, Some(base_direction));
                    for word in split_css_spaces(&reordered) {
                        out.push(InlineItem::Word {
                            text: word.to_string(),
                            width: measure_word(word, &b.style, fonts),
                            style: &b.style,
                            no_wrap,
                            node: b.node,
                        });
                    }
                }
            }
            BoxKind::Inline => collect_inline_items(&b.children, fonts, out),
            BoxKind::Block => out.push(InlineItem::Block(b)),
        }
    }
}

/// css-text-3 §4.1's collapsible white space: space, tab, line feed,
/// carriage return, form feed — not Unicode's `White_Space`, which
/// would make a no-break space breakable and collapsible.
pub(crate) fn is_css_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

/// The words of `text`, split at css-text-3 collapsible white space.
fn split_css_spaces(text: &str) -> impl Iterator<Item = &str> {
    text.split(is_css_space).filter(|w| !w.is_empty())
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
fn space_width(style: &ComputedStyle, fonts: &FontCollection) -> f32 {
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

/// Positions one line's words (M7.4: baseline-aligned — the shared
/// baseline sits at the tallest ascent on the line; M7.1: the line's
/// height is the tallest ascent+descent+line-gap on it) and pushes
/// their `Text` fragments, honoring `text_align` (M7.5) for the line's
/// starting x. A no-op (and returns `0.0`) for an empty line — callers
/// always check `is_empty` themselves first, but this stays safe either
/// way. Clears `line`/resets `line_width` for the next line.
fn flush_line(
    line: &mut Vec<LineWord<'_>>,
    line_width: f32,
    available_width: f32,
    text_align: TextAlign,
    fonts: &FontCollection,
    y: &mut f32,
    fragments: &mut Vec<Fragment>,
) {
    if line.is_empty() {
        return;
    }

    let mut max_ascent = 0.0f32;
    let mut max_descent = 0.0f32;
    let mut max_line_gap = 0.0f32;
    for word in line.iter() {
        // Per-word, not just per-size (M8.1): a word's own script may
        // resolve to a different fallback font than another word on
        // the same line, with its own vertical metrics — folding in
        // `metrics_for_text` (keyed off this word's actual text, not
        // just its `font_size_px`) extends the existing "shared
        // baseline sits at the tallest metrics present" rule (M7.4) to
        // cover that, not just mixed sizes.
        let m = fonts.metrics_for_text(word.text, word.style.font_size_px);
        max_ascent = max_ascent.max(m.ascent);
        max_descent = max_descent.max(m.descent);
        max_line_gap = max_line_gap.max(m.line_gap);
    }
    let baseline_y = *y + max_ascent;

    // css-text-3 §7.1: contents too long for the line box are
    // start-aligned, never pushed left past its edge.
    let slack = (available_width - line_width).max(0.0);
    let mut x = match text_align {
        TextAlign::Left => 0.0,
        TextAlign::Right => slack,
        TextAlign::Center => slack / 2.0,
    };
    for (i, word) in line.iter().enumerate() {
        if i > 0 {
            x += space_width(word.style, fonts);
        }
        if word.style.letter_spacing != 0.0 {
            // Per-character fragments (M8.4) — see `measure_word`'s doc
            // for why this has to shape character-by-character, same as
            // the width this word was measured with.
            emit_letter_spaced(
                fragments,
                word,
                &mut x,
                baseline_y,
                max_ascent,
                max_descent,
                fonts,
            );
            continue;
        }
        fragments.push(text_fragment(x, baseline_y, word, max_ascent, max_descent));
        x += word.width;
    }

    *y += max_ascent + max_descent + max_line_gap;
    line.clear();
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
        let (fragments, height) =
            layout_inline(&[text_box("hello", 16.0)], 1000.0, TextAlign::Left, &fonts);
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
        let (fragments, height) = layout_inline(&items, word_width + 1.0, TextAlign::Left, &fonts);
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
        let (fragments, _height) = layout_inline(&items, 10_000.0, TextAlign::Left, &fonts);
        assert_eq!(texts_of(&fragments), vec!["aaaa", "bbbb", "cccc"]);
        assert_eq!(fragments[0].y, fragments[1].y);
        assert_eq!(fragments[1].y, fragments[2].y);
    }

    #[test]
    fn mixed_font_sizes_on_one_line_share_a_baseline() {
        let fonts = test_fonts();
        let items = [text_box("small", 10.0), text_box("BIG", 40.0)];
        let (fragments, height) = layout_inline(&items, 10_000.0, TextAlign::Left, &fonts);
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
    fn text_align_left_right_and_center() {
        let fonts = test_fonts();
        let width = fonts.measure("hi", 16.0);
        assert!(width > 0.0);
        let available = 1000.0;

        let (left, _) = layout_inline(&[text_box("hi", 16.0)], available, TextAlign::Left, &fonts);
        assert_eq!(left[0].x, 0.0);

        let (right, _) =
            layout_inline(&[text_box("hi", 16.0)], available, TextAlign::Right, &fonts);
        assert_eq!(right[0].x, available - width);

        let (center, _) = layout_inline(
            &[text_box("hi", 16.0)],
            available,
            TextAlign::Center,
            &fonts,
        );
        assert_eq!(center[0].x, (available - width) / 2.0);
    }

    #[test]
    fn block_in_inline_splits_the_run_without_panicking() {
        let fonts = test_fonts();
        let block = LayoutBox {
            style: ComputedStyle::default(),
            kind: BoxKind::Block,
            children: Vec::new(),
            node: 0,
        };
        let run = inline_box(vec![
            text_box("before", 16.0),
            block,
            text_box("after", 16.0),
        ]);
        let (fragments, _height) = layout_inline(&[run], 10_000.0, TextAlign::Left, &fonts);

        assert_eq!(texts_of(&fragments), vec!["before", "after"]);
        // The embedded block contributes its own Box fragment too (M7.6).
        assert!(
            fragments
                .iter()
                .any(|f| matches!(f.kind, FragmentKind::Box))
        );
        let y_of = |t: &str| {
            fragments
                .iter()
                .find(|f| matches!(&f.kind, FragmentKind::Text(x) if x == t))
                .unwrap()
                .y
        };
        assert!(
            y_of("after") > y_of("before"),
            "the block must visually separate the two text runs"
        );
    }

    #[test]
    fn empty_input_produces_no_fragments_and_zero_height() {
        let fonts = test_fonts();
        let (fragments, height) = layout_inline(&[], 100.0, TextAlign::Left, &fonts);
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

        let (up, _) = layout_inline(
            &[styled_text_box(
                "hello world",
                style(TextTransform::Uppercase),
            )],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(texts_of(&up), vec!["HELLO", "WORLD"]);

        let (cap, _) = layout_inline(
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
        let (fragments, _) = layout_inline(
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
        let (fragments, _) =
            layout_inline(&[text_box("a\nb", 16.0)], 10_000.0, TextAlign::Left, &fonts);
        assert_eq!(texts_of(&fragments), vec!["a", "b"]);
        assert_eq!(fragments[0].y, fragments[1].y);
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
        let (fragments, _) = layout_inline(
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

        let (base, _) = layout_inline(
            &[styled_text_box("aa bb", normal.clone())],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        let (wide, _) = layout_inline(
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
        let (fragments, _) = layout_inline(
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
        let (fragments, _) = layout_inline(
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
        let (fragments, _) = layout_inline(
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
            let (frags, _) = layout_inline(
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
            let (frags, height) = layout_inline(
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
        let (_, height) = layout_inline(
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
        let (frags, _) = layout_inline(
            &[text_box("a\u{a0}b", 16.0)],
            a + 1.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(texts_of(&frags), vec!["a\u{a0}b"]);
        assert_eq!(frags[0].w, fonts.measure("a\u{a0}b", 16.0));
        // Around ordinary spaces it stays a word of its own.
        let (frags, _) = layout_inline(
            &[text_box("a \u{a0} b", 16.0)],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(texts_of(&frags), vec!["a", "\u{a0}", "b"]);
        // A spacer paragraph of one no-break space is a line tall.
        let (frags, height) = layout_inline(
            &[text_box("\u{a0}", 16.0)],
            10_000.0,
            TextAlign::Left,
            &fonts,
        );
        assert_eq!(frags.len(), 1);
        assert_eq!(height, fonts.primary().metrics(16.0).line_height());
    }
}
