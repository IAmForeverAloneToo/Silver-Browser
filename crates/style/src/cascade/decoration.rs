//! The text decoration properties (Text Decoration 3 and 4, M21.7).
//! Each parser takes a declaration's value as the cascade has it and
//! says `None` when the value is not one the property takes, so that the
//! declaration is dropped.

use css::Token;

use super::{LengthBases, Range, components, length_percentage, single_ident};
use crate::computed::{
    Color, DecorationLine, DecorationStyle, DecorationThickness, LengthPercentage,
    UnderlinePosition,
};

/// The keyword `word`, any case, alone in `tokens`.
fn is_word(tokens: &[Token], word: &str) -> bool {
    single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case(word))
}

/// The line keyword `part` is, set in `line` when `line` lacks it:
/// `Some(false)` when it has it already, `None` when `part` is none.
fn add_line(line: &mut DecorationLine, part: &[Token]) -> Option<bool> {
    let flag = match single_ident(part)?.to_ascii_lowercase().as_str() {
        "underline" => &mut line.underline,
        "overline" => &mut line.overline,
        "line-through" => &mut line.line_through,
        "blink" => &mut line.blink,
        _ => return None,
    };
    Some(!std::mem::replace(flag, true))
}

/// `text-decoration-line`: `none`, or any of `underline`, `overline`,
/// `line-through` and `blink`, each once, in any order.
pub(super) fn parse_line(tokens: &[Token]) -> Option<DecorationLine> {
    if is_word(tokens, "none") {
        return Some(DecorationLine::default());
    }
    let mut line = DecorationLine::default();
    let parts = components(tokens);
    for part in &parts {
        add_line(&mut line, part)?.then_some(())?;
    }
    (!parts.is_empty()).then_some(line)
}

/// `text-decoration-style`.
pub(super) fn parse_style(tokens: &[Token]) -> Option<DecorationStyle> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "solid" => Some(DecorationStyle::Solid),
        "double" => Some(DecorationStyle::Double),
        "dotted" => Some(DecorationStyle::Dotted),
        "dashed" => Some(DecorationStyle::Dashed),
        "wavy" => Some(DecorationStyle::Wavy),
        _ => None,
    }
}

/// `text-decoration-color`: a colour, or `currentcolor` (`Some(None)`).
pub(super) fn parse_color_or_current(tokens: &[Token], current: Color) -> Option<Option<Color>> {
    if is_word(tokens, "currentcolor") {
        return Some(None);
    }
    super::color_value(tokens, current).map(Some)
}

/// `text-decoration-thickness`: `auto`, `from-font`, or a length or
/// percentage of the font size, not negative.
pub(super) fn parse_thickness(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<DecorationThickness> {
    if is_word(tokens, "auto") {
        return Some(DecorationThickness::Auto);
    }
    if is_word(tokens, "from-font") {
        return Some(DecorationThickness::FromFont);
    }
    length_percentage(tokens, bases, Range::NonNegative).map(DecorationThickness::Length)
}

/// `text-underline-offset`: `auto` (`Some(None)`), or a length or
/// percentage of the font size, which may be negative.
pub(super) fn parse_underline_offset(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Option<LengthPercentage>> {
    if is_word(tokens, "auto") {
        return Some(None);
    }
    length_percentage(tokens, bases, Range::All).map(Some)
}

/// `text-underline-position`: `auto`, or `from-font` or `under`, `left`
/// or `right`, or one of each of the two in either order; `left` and
/// `right` are `auto` in horizontal text.
pub(super) fn parse_underline_position(tokens: &[Token]) -> Option<UnderlinePosition> {
    if is_word(tokens, "auto") {
        return Some(UnderlinePosition::Auto);
    }
    let mut position = None;
    let mut side = false;
    let parts = components(tokens);
    for part in &parts {
        match single_ident(part)?.to_ascii_lowercase().as_str() {
            "under" if position.is_none() => position = Some(UnderlinePosition::Under),
            "from-font" if position.is_none() => position = Some(UnderlinePosition::FromFont),
            "left" | "right" if !side => side = true,
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| position.unwrap_or(UnderlinePosition::Auto))
}

/// What the `text-decoration` shorthand sets: the line, its style,
/// colour and thickness.
pub(super) type Shorthand = (
    DecorationLine,
    DecorationStyle,
    Option<Color>,
    DecorationThickness,
);

/// The `text-decoration` shorthand: a line, a style, a colour and a
/// thickness, in any order, each at most once and the line's keywords
/// together, each the initial value where it is left out.
pub(super) fn parse_shorthand(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Shorthand> {
    let mut line: Option<DecorationLine> = None;
    // The line's keywords have been met and something else after them.
    let mut line_done = false;
    let mut style = None;
    let mut color = None;
    let mut thickness = None;
    let parts = components(tokens);
    for part in &parts {
        if is_word(part, "none") {
            if line.is_some() {
                return None;
            }
            line = Some(DecorationLine::default());
            line_done = true;
            continue;
        }
        let mut next = line.unwrap_or_default();
        if !line_done && let Some(added) = add_line(&mut next, part) {
            added.then_some(())?;
            line = Some(next);
            continue;
        }
        if line.is_some() {
            line_done = true;
        }
        let first = if let Some(value) = parse_style(part) {
            style.replace(value).is_none()
        } else if let Some(value) = parse_color_or_current(part, bases.color) {
            color.replace(value).is_none()
        } else if let Some(value) = parse_thickness(part, bases) {
            thickness.replace(value).is_none()
        } else {
            false
        };
        if !first {
            return None;
        }
    }
    (!parts.is_empty()).then(|| {
        (
            line.unwrap_or_default(),
            style.unwrap_or_default(),
            color.unwrap_or_default(),
            thickness.unwrap_or_default(),
        )
    })
}
