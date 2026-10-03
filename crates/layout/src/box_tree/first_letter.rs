//! `::first-letter` (M16, docs/design/selectors-pseudo.md): once the
//! box tree is built, the text box holding a block container's first
//! letter is split, the first-letter text going into a box of its own.
//!
//! The first letter is in the inline content at the start of the
//! container's first formatted line: its own, or that of its first
//! in-flow block-level child that has a line, recursively — not inside
//! an inline-block or a table, whose lines are their own, and not after
//! any of them, which is other content first. Collapsible white space
//! at the start is passed over, and so is white space kept as it is;
//! a forced line break first means the line is empty, and there is no
//! first letter. The text is Pseudo-Elements 4's pattern (§2.2.1)
//! within the one text box the first letter is in: punctuation is
//! taken with the letter only from the same text.
//!
//! The box is an inline box, or a float when the `::first-letter`
//! style floats, as every browser honours for a drop cap. It names the
//! element the text is in, as its text does, and its style inherits from
//! that element's (Pseudo-Elements 4 §2.2.3), which the cascade gives
//! through [`Recascade`]. When several containers whose first formatted
//! line this is have a `::first-letter`, the outermost's box is made
//! first and the next one's inside it, so they nest outermost first.

use style::computed::{Float, WhiteSpace};
use style::selector_matching::PseudoElement;
use style::{ComputedStyle, Recascade};
use unicode_properties::{GeneralCategory, GeneralCategoryGroup, UnicodeGeneralCategory};

use super::{BoxKind, LayoutBox, Styled};

/// Splits off the first letter of each block container in `boxes` and
/// their descendants that has a `::first-letter`.
pub(super) fn apply(styled: &Styled<'_>, boxes: &mut [LayoutBox]) {
    for b in boxes {
        visit(styled, b);
    }
}

fn visit(styled: &Styled<'_>, b: &mut LayoutBox) {
    if originates(styled, b) {
        let mut path = Vec::new();
        if let Found::Text { start, end } = in_blocks(&b.children, &mut path) {
            split(styled.recascade, b, &path, start, end);
        }
    }
    for child in &mut b.children {
        visit(styled, child);
    }
}

/// Whether `b` is an element's box with a `::first-letter` style. Not
/// an anonymous box, which a generated one is too, though it names the
/// element. Whether it is a block container — a block, an
/// inline-block, a float or an absolutely positioned box that is not a
/// table, a cell or a caption — the walk tells: only a block
/// container's children are blocks, and an inline element's are on
/// its line, a table's its parts.
fn originates(styled: &Styled<'_>, b: &LayoutBox) -> bool {
    // Nor a flex container (css-flexbox-1 §3, M17), whose children are
    // blocks but not its lines.
    !b.anonymous
        && !b.is_flex_container()
        && styled
            .styles
            .pseudo(b.node, PseudoElement::FirstLetter)
            .is_some()
}

/// What a walk of the start of a first formatted line found.
enum Found {
    /// The first-letter text: the byte range in the text box the path
    /// leads to.
    Text { start: usize, end: usize },
    /// Content that is not a first letter came first: none here.
    Blocked,
    /// Nothing in flow: the line is further on.
    Empty,
}

/// The first letter among the block-level `children` of a block
/// container: in the first that has a line. Any other box's children
/// are found to be no such line: text, inline boxes and table parts
/// block it. `path` is the way down so far, by index, and on
/// [`Found::Text`] the way to the text box.
fn in_blocks(children: &[LayoutBox], path: &mut Vec<usize>) -> Found {
    for (i, child) in children.iter().enumerate() {
        path.push(i);
        let found = match child.kind {
            BoxKind::AnonymousBlock => in_line(&child.children, path),
            BoxKind::Block => in_blocks(&child.children, path),
            // Out of flow. (An ancestor's floated first letter is in a
            // line, where it was a text's.)
            BoxKind::Float | BoxKind::Absolute => Found::Empty,
            // A table's lines are its cells', a flex container's its
            // items' (M17).
            _ => Found::Blocked,
        };
        match found {
            Found::Empty => {
                path.pop();
            }
            found => return found,
        }
    }
    Found::Empty
}

/// The first letter among the inline-level `items` of a line.
fn in_line(items: &[LayoutBox], path: &mut Vec<usize>) -> Found {
    for (i, item) in items.iter().enumerate() {
        path.push(i);
        let found = match &item.kind {
            BoxKind::Text(text) => in_text(text, item.style.white_space),
            BoxKind::Inline => in_line(&item.children, path),
            // An ancestor's first letter, floated: the letter is in it.
            BoxKind::Float if item.generated == Some(PseudoElement::FirstLetter) => {
                in_blocks(&item.children, path)
            }
            BoxKind::Float | BoxKind::Absolute => Found::Empty,
            // An inline-block, an inline table or an inline flex
            // container: an atomic inline first.
            _ => Found::Blocked,
        };
        match found {
            Found::Empty => {
                path.pop();
            }
            found => return found,
        }
    }
    Found::Empty
}

/// The first-letter text in `text`, after the white space at its start:
/// [`Found::Empty`] when there is nothing else, [`Found::Blocked`] when
/// what comes first is not the pattern — a forced line break among
/// them, which is not passed over.
fn in_text(text: &str, white_space: WhiteSpace) -> Found {
    let breaks_kept = matches!(
        white_space,
        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine
    );
    let Some((start, _)) = text
        .char_indices()
        .find(|&(_, c)| !crate::inline::is_css_space(c) || (c == '\n' && breaks_kept))
    else {
        return Found::Empty;
    };
    match pattern_len(&text[start..]) {
        Some(len) => Found::Text {
            start,
            end: start + len,
        },
        None => Found::Blocked,
    }
}

/// The length in bytes of Pseudo-Elements 4's first-letter text at the
/// start of `s`, `([P] [Zs P]*)? [L N S] [M]* ([Zs P--[Ps Pd]]*
/// [P--[Ps Pd]])?` (§2.2.1): punctuation with any spaces between, the
/// letter, number or symbol with the combining marks after it, then
/// any punctuation but opening punctuation and dashes, with spaces
/// between that are not word separators. The typographic character
/// unit is a scalar value and its combining marks; the language
/// tailorings (Dutch "ij") wait for M42. `None` when `s` does not start
/// with the pattern.
fn pattern_len(s: &str) -> Option<usize> {
    let mut chars = s.char_indices().peekable();
    // Preceding punctuation, with spaces between.
    if chars.next_if(|&(_, c)| is_punctuation(c)).is_some() {
        while chars
            .next_if(|&(_, c)| is_punctuation(c) || is_space(c))
            .is_some()
        {}
    }
    // The letter, and its combining marks.
    chars.next_if(|&(_, c)| {
        matches!(
            c.general_category_group(),
            GeneralCategoryGroup::Letter
                | GeneralCategoryGroup::Number
                | GeneralCategoryGroup::Symbol
        )
    })?;
    while chars
        .next_if(|&(_, c)| c.general_category_group() == GeneralCategoryGroup::Mark)
        .is_some()
    {}
    let mut end = chars.peek().map_or(s.len(), |&(i, _)| i);
    // Following punctuation, with spaces between that are no word
    // separators; spaces after the last are not taken.
    while let Some((i, c)) = chars.next_if(|&(_, c)| {
        (is_punctuation(c)
            && !matches!(
                c.general_category(),
                GeneralCategory::OpenPunctuation | GeneralCategory::DashPunctuation
            ))
            || (is_space(c) && !is_word_separator(c))
    }) {
        if is_punctuation(c) {
            end = i + c.len_utf8();
        }
    }
    Some(end)
}

fn is_punctuation(c: char) -> bool {
    c.general_category_group() == GeneralCategoryGroup::Punctuation
}

/// A typographic space: Zs other than U+3000 IDEOGRAPHIC SPACE.
fn is_space(c: char) -> bool {
    c.general_category() == GeneralCategory::SpaceSeparator && c != '\u{3000}'
}

/// CSS Text 3's word-separator characters.
fn is_word_separator(c: char) -> bool {
    matches!(
        c,
        ' ' | '\u{a0}' | '\u{1361}' | '\u{10100}' | '\u{10101}' | '\u{1039f}' | '\u{1091f}'
    )
}

/// Splits the text box `path` leads to from among `container`'s
/// children: the first-letter text, `start..end` of it, goes into a
/// `::first-letter` box of `container`'s element, between what comes
/// before it, white space, and the rest. The blocks on the way down
/// are marked as styled by an ancestor: their layout reads the
/// container's `::first-letter` style, which is not theirs.
fn split(
    recascade: &dyn Recascade,
    container: &mut LayoutBox,
    path: &[usize],
    start: usize,
    end: usize,
) {
    let element = container.node;
    let (&last, way) = path.split_last().expect("a path to a text box");
    let mut holder = container;
    for &i in way {
        holder = &mut holder.children[i];
        if holder.kind == BoxKind::Block {
            holder.styled_by_ancestor = true;
        }
    }
    let text_box = &holder.children[last];
    let BoxKind::Text(text) = &text_box.kind else {
        unreachable!("the path leads to a text box");
    };
    // The box inherits from what the text is in — its element, or the
    // pseudo-element whose box it is in — as that is on the first line,
    // where the letter is: a `::first-line` styles it too. `display`
    // does not apply to it, so its style's is `inline`, or a float's
    // block.
    let parent = text_box
        .first_line
        .as_ref()
        .map_or(&text_box.style, |line| &line.style);
    let Some(style) = recascade.pseudo_element(element, PseudoElement::FirstLetter, parent) else {
        return; // no rule styles it after all
    };
    // The pieces of the text left as they were, and the boxes of the
    // first letter, which are marked as its.
    let piece = |text: &str| LayoutBox {
        style: text_box.style.clone(),
        kind: BoxKind::Text(text.to_string()),
        children: Vec::new(),
        node: text_box.node,
        anonymous: text_box.anonymous,
        generated: text_box.generated,
        quote_depth: text_box.quote_depth,
        styled_by_ancestor: false,
        first_line: text_box.first_line.clone(),
        tree_order: 0,
    };
    let generated = |kind, style: ComputedStyle, children| LayoutBox {
        style,
        kind,
        children,
        node: text_box.node,
        anonymous: true,
        generated: Some(PseudoElement::FirstLetter),
        quote_depth: text_box.quote_depth,
        styled_by_ancestor: false,
        first_line: None,
        tree_order: 0,
    };
    let letter = generated(
        BoxKind::Text(text[start..end].to_string()),
        style.clone(),
        Vec::new(),
    );
    let first_letter = if style.float == Float::None {
        generated(BoxKind::Inline, style, vec![letter])
    } else {
        // A block container inside, its line in an anonymous block.
        let line = generated(
            BoxKind::AnonymousBlock,
            ComputedStyle::inherited_from(&style),
            vec![letter],
        );
        generated(BoxKind::Float, style, vec![line])
    };
    let mut replacement = Vec::with_capacity(3);
    if start > 0 {
        replacement.push(piece(&text[..start]));
    }
    replacement.push(first_letter);
    if end < text.len() {
        replacement.push(piece(&text[end..]));
    }
    holder.children.splice(last..=last, replacement);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first-letter text at the start of `s`, if any.
    fn first(s: &str) -> Option<&str> {
        pattern_len(s).map(|len| &s[..len])
    }

    #[test]
    fn the_pattern_is_pseudo_elements_4s() {
        for (text, expected) in [
            ("The", Some("T")),
            ("67 million", Some("6")),
            ("$5", Some("$")),
            // Punctuation before, any kind, with spaces between.
            ("\u{201C}The", Some("\u{201C}T")),
            ("\u{201C} The", Some("\u{201C} T")),
            ("(T)he", Some("(T)")),
            ("-T", Some("-T")),
            ("_x", Some("_x")),
            ("\u{bf}\u{a1}Qu\u{e9}", Some("\u{bf}\u{a1}Q")),
            // The letter's combining marks are its.
            ("e\u{301}t\u{e9}", Some("e\u{301}")),
            // Punctuation after, but opening ones and dashes, with
            // spaces between that are no word separators.
            ("T. Next", Some("T.")),
            ("T\u{2019}s", Some("T\u{2019}")),
            ("T\u{2009}.", Some("T\u{2009}.")),
            ("T .", Some("T")),
            ("T\u{a0}.", Some("T")),
            ("T-he", Some("T")),
            ("T(he", Some("T")),
            ("T.\u{2009}", Some("T.")),
            // Nothing that is a letter, number or symbol where one must be.
            ("", None),
            ("...", None),
            ("\u{201C}", None),
            ("\u{3000}T", None),
            ("\u{201C}\u{3000}T", None),
            ("\u{a0}T", None),
            ("\u{301}T", None),
        ] {
            assert_eq!(first(text), expected, "{text:?}");
        }
    }

    #[test]
    fn white_space_first_is_passed_over_but_a_kept_line_break_is_an_empty_line() {
        let found = |text: &str, white_space| match in_text(text, white_space) {
            Found::Text { start, end } => Some((start, end)),
            Found::Blocked => None,
            Found::Empty => Some((0, 0)),
        };
        assert_eq!(found("  \tThe", WhiteSpace::Normal), Some((3, 4)));
        assert_eq!(found("\nThe", WhiteSpace::Normal), Some((1, 2)));
        assert_eq!(found("  The", WhiteSpace::Pre), Some((2, 3)));
        for white_space in [WhiteSpace::Pre, WhiteSpace::PreWrap, WhiteSpace::PreLine] {
            assert_eq!(found(" \nThe", white_space), None, "{white_space:?}");
        }
        // Only white space: nothing here, the letter is further on.
        assert_eq!(found(" \n ", WhiteSpace::Normal), Some((0, 0)));
        // A no-break space is no white space to pass over.
        assert_eq!(found("\u{a0}The", WhiteSpace::Normal), None);
        // Nor is the pattern's absence.
        assert_eq!(found(" ...", WhiteSpace::Normal), None);
    }
}
