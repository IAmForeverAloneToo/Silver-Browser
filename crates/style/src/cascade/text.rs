//! The text properties of [`crate::computed::TextProps`] (CSS Text 3 and
//! 4, gap batch D's text gaps). Each parser takes a declaration's value
//! as the cascade has it and says `None` when the value is not one the
//! property takes, so that the declaration is dropped.

use std::sync::Arc;

use css::Token;

use super::{LengthBases, Range, Want, components, integer, length, length_percentage, math};
use super::{single_ident, single_token};
use crate::calc;
use crate::computed::{
    AutospaceMode, HangingEnd, HangingPunctuation, Hyphens, LengthPercentage, LineBreak, MathStyle,
    RubyAlign, RubyPosition, TabSize, TextAutospace, TextFit, TextFitKind, TextFitMethod,
    TextGroupAlign, TextIndent, TextSpacingTrim, TextTransform, TextWrapStyle, TransformExtra,
    WordSpaceTransform,
};

/// The keyword `tokens` are, lowercased.
fn keyword(tokens: &[Token]) -> Option<String> {
    single_ident(tokens).map(str::to_ascii_lowercase)
}

/// `text-indent`: a length or percentage, with `hanging` and
/// `each-line` each at most once, in any order.
pub(super) fn parse_text_indent(tokens: &[Token], bases: &LengthBases<'_>) -> Option<TextIndent> {
    let mut indent = TextIndent::default();
    let mut length = None;
    for part in components(tokens) {
        match keyword(part).as_deref() {
            Some("hanging") if !indent.hanging => indent.hanging = true,
            Some("each-line") if !indent.each_line => indent.each_line = true,
            _ if length.is_none() => length = Some(length_percentage(part, bases, Range::All)?),
            _ => return None,
        }
    }
    indent.length = length?;
    Some(indent)
}

/// `tab-size`: a number of spaces or a length, neither below zero.
pub(super) fn parse_tab_size(tokens: &[Token], bases: &LengthBases<'_>) -> Option<TabSize> {
    if let Some(Token::Number { value, .. }) = single_token(tokens) {
        return (*value >= 0.0 && value.is_finite()).then_some(TabSize::Spaces(*value as f32));
    }
    if let Some(calc::Computed::Value(v)) = math(tokens, Want::Number, bases, None) {
        return Some(TabSize::Spaces(v.clamp(0.0, f64::from(f32::MAX)) as f32));
    }
    length(tokens, bases, Range::NonNegative).map(TabSize::Length)
}

/// `line-break`.
pub(super) fn parse_line_break(tokens: &[Token]) -> Option<LineBreak> {
    Some(match keyword(tokens)?.as_str() {
        "auto" => LineBreak::Auto,
        "loose" => LineBreak::Loose,
        "normal" => LineBreak::Normal,
        "strict" => LineBreak::Strict,
        "anywhere" => LineBreak::Anywhere,
        _ => return None,
    })
}

/// `hyphens`.
pub(super) fn parse_hyphens(tokens: &[Token]) -> Option<Hyphens> {
    Some(match keyword(tokens)?.as_str() {
        "none" => Hyphens::None,
        "manual" => Hyphens::Manual,
        "auto" => Hyphens::Auto,
        _ => return None,
    })
}

/// `hyphenate-character`: `auto` (`Some(None)`) or a string.
pub(super) fn parse_hyphenate_character(tokens: &[Token]) -> Option<Option<Arc<str>>> {
    match single_token(tokens)? {
        Token::Ident(word) if word.eq_ignore_ascii_case("auto") => Some(None),
        Token::String { value, bad: false } => Some(Some(Arc::from(value.as_str()))),
        _ => None,
    }
}

/// `auto` (`Some(None)`) or an integer, which the reference browser
/// takes below zero too.
fn auto_or_count(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Option<i32>> {
    if keyword(tokens).as_deref() == Some("auto") {
        return Some(None);
    }
    Some(Some(integer(tokens, bases)?))
}

/// `hyphenate-limit-chars`: one to three values, the second `auto`
/// when left out, the third the second.
pub(super) fn parse_hyphenate_limit_chars(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<[Option<i32>; 3]> {
    let parts = components(tokens);
    let values: Vec<Option<i32>> = parts
        .iter()
        .map(|part| auto_or_count(part, bases))
        .collect::<Option<_>>()?;
    match values.as_slice() {
        [word] => Some([*word, None, None]),
        [word, before] => Some([*word, *before, *before]),
        [word, before, after] => Some([*word, *before, *after]),
        _ => None,
    }
}

/// `hyphenate-limit-lines`: `no-limit` (`Some(None)`) or an integer not
/// below zero.
pub(super) fn parse_hyphenate_limit_lines(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Option<i32>> {
    if keyword(tokens).as_deref() == Some("no-limit") {
        return Some(None);
    }
    Some(Some(integer(tokens, bases).filter(|n| *n >= 0)?))
}

/// `text-autospace`: `normal`, `auto`, `no-autospace`, or any of
/// `ideograph-alpha`, `ideograph-numeric` and `punctuation` with
/// `insert` or `replace`, each at most once, in any order.
pub(super) fn parse_text_autospace(tokens: &[Token]) -> Option<TextAutospace> {
    let parts = components(tokens);
    if let [one] = parts.as_slice() {
        match keyword(one)?.as_str() {
            "normal" => return Some(TextAutospace::Normal),
            "auto" => return Some(TextAutospace::Auto),
            "no-autospace" => return Some(TextAutospace::NoAutospace),
            _ => {}
        }
    }
    let (mut alpha, mut numeric, mut punctuation, mut mode) =
        (false, false, false, AutospaceMode::Unset);
    for part in &parts {
        let flag = match keyword(part)?.as_str() {
            "ideograph-alpha" => &mut alpha,
            "ideograph-numeric" => &mut numeric,
            "punctuation" => &mut punctuation,
            "insert" | "replace" if mode == AutospaceMode::Unset => {
                mode = if keyword(part)? == "insert" {
                    AutospaceMode::Insert
                } else {
                    AutospaceMode::Replace
                };
                continue;
            }
            _ => return None,
        };
        if std::mem::replace(flag, true) {
            return None;
        }
    }
    (alpha || numeric || punctuation).then_some(TextAutospace::Spaces {
        ideograph_alpha: alpha,
        ideograph_numeric: numeric,
        punctuation,
        mode,
    })
}

/// `text-spacing-trim`.
pub(super) fn parse_text_spacing_trim(tokens: &[Token]) -> Option<TextSpacingTrim> {
    Some(match keyword(tokens)?.as_str() {
        "normal" => TextSpacingTrim::Normal,
        "space-all" => TextSpacingTrim::SpaceAll,
        "space-first" => TextSpacingTrim::SpaceFirst,
        "trim-start" => TextSpacingTrim::TrimStart,
        "trim-both" => TextSpacingTrim::TrimBoth,
        "trim-all" => TextSpacingTrim::TrimAll,
        "auto" => TextSpacingTrim::Auto,
        _ => return None,
    })
}

/// `ruby-position` (CSS Ruby 1 section 3.1, D77): `over` or `under`,
/// the values the reference browser takes.
pub(super) fn parse_ruby_position(tokens: &[Token]) -> Option<RubyPosition> {
    Some(match keyword(tokens)?.as_str() {
        "over" => RubyPosition::Over,
        "under" => RubyPosition::Under,
        _ => return None,
    })
}

/// `math-style` (MathML Core section 2.1.1, D77).
pub(super) fn parse_math_style(tokens: &[Token]) -> Option<MathStyle> {
    Some(match keyword(tokens)?.as_str() {
        "normal" => MathStyle::Normal,
        "compact" => MathStyle::Compact,
        _ => return None,
    })
}

/// `ruby-align` (CSS Ruby 1 section 4.1, D77).
pub(super) fn parse_ruby_align(tokens: &[Token]) -> Option<RubyAlign> {
    Some(match keyword(tokens)?.as_str() {
        "space-around" => RubyAlign::SpaceAround,
        "space-between" => RubyAlign::SpaceBetween,
        "start" => RubyAlign::Start,
        "center" => RubyAlign::Center,
        _ => return None,
    })
}

/// `text-spacing`, the shorthand of `text-spacing-trim` and
/// `text-autospace`: `none`, `auto`, or a value of either or both, in
/// either order, `normal` standing for either.
pub(super) fn parse_text_spacing(tokens: &[Token]) -> Option<(TextSpacingTrim, TextAutospace)> {
    let parts = components(tokens);
    if let [one] = parts.as_slice() {
        match keyword(one)?.as_str() {
            "none" => return Some((TextSpacingTrim::SpaceAll, TextAutospace::NoAutospace)),
            "auto" => return Some((TextSpacingTrim::Auto, TextAutospace::Auto)),
            _ => {}
        }
    }
    let (mut trim, mut autospace) = (None, None);
    for part in &parts {
        if keyword(part).as_deref() == Some("normal") {
            if trim.is_none() {
                trim = Some(TextSpacingTrim::Normal);
            } else if autospace.is_none() {
                autospace = Some(TextAutospace::Normal);
            } else {
                return None;
            }
            continue;
        }
        if let Some(t) = parse_text_spacing_trim(part).filter(|t| *t != TextSpacingTrim::Auto) {
            if trim.is_some_and(|old| old != TextSpacingTrim::Normal) {
                return None;
            }
            if trim == Some(TextSpacingTrim::Normal) && autospace.is_none() {
                autospace = Some(TextAutospace::Normal);
            }
            trim = Some(t);
        } else if keyword(part).as_deref() == Some("no-autospace") && autospace.is_none() {
            autospace = Some(TextAutospace::NoAutospace);
        } else {
            return None;
        }
    }
    (!parts.is_empty()).then(|| {
        (
            trim.unwrap_or_default(),
            autospace.unwrap_or(TextAutospace::Normal),
        )
    })
}

/// `hanging-punctuation`: `none`, or any of `first`, `last` and one of
/// `force-end` and `allow-end`, each at most once, in any order.
pub(super) fn parse_hanging_punctuation(tokens: &[Token]) -> Option<HangingPunctuation> {
    let parts = components(tokens);
    let mut value = HangingPunctuation::default();
    if let [one] = parts.as_slice()
        && keyword(one).as_deref() == Some("none")
    {
        return Some(value);
    }
    for part in &parts {
        match keyword(part)?.as_str() {
            "first" if !value.first => value.first = true,
            "last" if !value.last => value.last = true,
            "force-end" if value.end == HangingEnd::None => value.end = HangingEnd::ForceEnd,
            "allow-end" if value.end == HangingEnd::None => value.end = HangingEnd::AllowEnd,
            _ => return None,
        }
    }
    (!parts.is_empty()).then_some(value)
}

/// `text-wrap-style`.
pub(super) fn parse_text_wrap_style(tokens: &[Token]) -> Option<TextWrapStyle> {
    Some(match keyword(tokens)?.as_str() {
        "auto" => TextWrapStyle::Auto,
        "balance" => TextWrapStyle::Balance,
        "stable" => TextWrapStyle::Stable,
        "pretty" => TextWrapStyle::Pretty,
        _ => return None,
    })
}

/// `text-wrap`, the shorthand of `text-wrap-mode` and `text-wrap-style`:
/// a value of either or both, in either order, the one left out
/// initial; whether lines wrap, and how.
pub(super) fn parse_text_wrap(tokens: &[Token]) -> Option<(bool, TextWrapStyle)> {
    let parts = components(tokens);
    let (mut wrap, mut wrap_style) = (None, None);
    for part in &parts {
        match keyword(part)?.as_str() {
            "wrap" if wrap.is_none() => wrap = Some(true),
            "nowrap" if wrap.is_none() => wrap = Some(false),
            _ if wrap_style.is_none() => wrap_style = Some(parse_text_wrap_style(part)?),
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| (wrap.unwrap_or(true), wrap_style.unwrap_or_default()))
}

/// `word-space-transform`: `none`, or `space` or `ideographic-space`
/// with `auto-phrase` in either order.
pub(super) fn parse_word_space_transform(tokens: &[Token]) -> Option<WordSpaceTransform> {
    let parts = components(tokens);
    if let [one] = parts.as_slice()
        && keyword(one).as_deref() == Some("none")
    {
        return Some(WordSpaceTransform::None);
    }
    let (mut space, mut auto_phrase) = (None, false);
    for part in &parts {
        match keyword(part)?.as_str() {
            "space" if space.is_none() => space = Some(false),
            "ideographic-space" if space.is_none() => space = Some(true),
            "auto-phrase" if !auto_phrase => auto_phrase = true,
            _ => return None,
        }
    }
    Some(WordSpaceTransform::Space {
        ideographic: space?,
        auto_phrase,
    })
}

/// `text-fit`: `none`, or `grow` or `shrink`, then a method, then a
/// limit, a percentage, either left out.
pub(super) fn parse_text_fit(tokens: &[Token]) -> Option<TextFit> {
    let parts = components(tokens);
    let (first, rest) = parts.split_first()?;
    let kind = match keyword(first)?.as_str() {
        "none" if rest.is_empty() => return Some(TextFit::default()),
        "grow" => TextFitKind::Grow,
        "shrink" => TextFitKind::Shrink,
        _ => return None,
    };
    let mut fit = TextFit {
        kind,
        ..TextFit::default()
    };
    let mut rest = rest;
    if let Some((part, after)) = rest.split_first()
        && let Some(word) = keyword(part)
    {
        fit.method = match word.as_str() {
            "consistent" => TextFitMethod::Consistent,
            "per-line" => TextFitMethod::PerLine,
            "per-line-all" => TextFitMethod::PerLineAll,
            _ => return None,
        };
        rest = after;
    }
    match rest {
        [] => {}
        [part] => match single_token(part)? {
            Token::Percentage { value } if *value >= 0.0 && value.is_finite() => {
                fit.limit = Some(*value as f32);
            }
            _ => return None,
        },
        _ => return None,
    }
    Some(fit)
}

/// `text-group-align`.
pub(super) fn parse_text_group_align(tokens: &[Token]) -> Option<TextGroupAlign> {
    Some(match keyword(tokens)?.as_str() {
        "none" => TextGroupAlign::None,
        "start" => TextGroupAlign::Start,
        "end" => TextGroupAlign::End,
        "left" => TextGroupAlign::Left,
        "right" => TextGroupAlign::Right,
        "center" => TextGroupAlign::Center,
        _ => return None,
    })
}

/// `text-transform`: `none`, `math-auto`, or a case with `full-width`
/// and `full-size-kana`, each at most once, in any order.
pub(super) fn parse_text_transform(tokens: &[Token]) -> Option<(TextTransform, TransformExtra)> {
    let parts = components(tokens);
    let mut extra = TransformExtra::default();
    if let [one] = parts.as_slice() {
        match keyword(one)?.as_str() {
            "none" => return Some((TextTransform::None, extra)),
            "math-auto" => {
                extra.math_auto = true;
                return Some((TextTransform::None, extra));
            }
            _ => {}
        }
    }
    let mut case = None;
    for part in &parts {
        match keyword(part)?.as_str() {
            "uppercase" if case.is_none() => case = Some(TextTransform::Uppercase),
            "lowercase" if case.is_none() => case = Some(TextTransform::Lowercase),
            "capitalize" if case.is_none() => case = Some(TextTransform::Capitalize),
            "full-width" if !extra.full_width => extra.full_width = true,
            "full-size-kana" if !extra.full_size_kana => extra.full_size_kana = true,
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| (case.unwrap_or(TextTransform::None), extra))
}

/// `letter-spacing` and `word-spacing`: `normal` (zero) or a length or
/// percentage.
pub(super) fn parse_spacing(tokens: &[Token], bases: &LengthBases<'_>) -> Option<LengthPercentage> {
    if keyword(tokens).as_deref() == Some("normal") {
        return Some(LengthPercentage::Px(0.0));
    }
    length_percentage(tokens, bases, Range::All)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(text: &str) -> Vec<Token> {
        css::tokenize(text)
    }

    fn with<R>(f: impl FnOnce(&LengthBases<'_>) -> R) -> R {
        super::super::with_default_bases(f)
    }

    #[test]
    fn text_indent_takes_its_keywords_in_any_order_once() {
        let indent = with(|b| parse_text_indent(&tokens("each-line 10px hanging"), b)).unwrap();
        assert_eq!(indent.length, LengthPercentage::Px(10.0));
        assert!(indent.hanging && indent.each_line);
        assert!(with(|b| parse_text_indent(&tokens("hanging"), b)).is_none());
        assert!(with(|b| parse_text_indent(&tokens("1px hanging hanging"), b)).is_none());
        assert!(with(|b| parse_text_indent(&tokens("1px 2px"), b)).is_none());
    }

    #[test]
    fn tab_size_is_a_number_or_a_length_never_negative() {
        assert_eq!(
            with(|b| parse_tab_size(&tokens("2.5"), b)),
            Some(TabSize::Spaces(2.5))
        );
        assert_eq!(
            with(|b| parse_tab_size(&tokens("1em"), b)),
            Some(TabSize::Length(16.0))
        );
        assert_eq!(
            with(|b| parse_tab_size(&tokens("calc(10px - 1em)"), b)),
            Some(TabSize::Length(0.0))
        );
        assert!(with(|b| parse_tab_size(&tokens("-1"), b)).is_none());
        assert!(with(|b| parse_tab_size(&tokens("-1px"), b)).is_none());
    }

    #[test]
    fn hyphenate_limit_chars_fills_what_is_left_out() {
        let of = |text: &str| with(|b| parse_hyphenate_limit_chars(&tokens(text), b));
        assert_eq!(of("5"), Some([Some(5), None, None]));
        assert_eq!(of("5 2"), Some([Some(5), Some(2), Some(2)]));
        assert_eq!(of("auto 2 auto"), Some([None, Some(2), None]));
        assert_eq!(of("1 2 3 4"), None);
        assert_eq!(of("-1"), Some([Some(-1), None, None]));
    }

    #[test]
    fn text_autospace_and_text_spacing() {
        assert_eq!(
            parse_text_autospace(&tokens("insert punctuation ideograph-alpha")),
            Some(TextAutospace::Spaces {
                ideograph_alpha: true,
                ideograph_numeric: false,
                punctuation: true,
                mode: AutospaceMode::Insert
            })
        );
        assert!(parse_text_autospace(&tokens("insert")).is_none());
        assert!(parse_text_autospace(&tokens("punctuation punctuation")).is_none());
        assert_eq!(
            parse_text_spacing(&tokens("no-autospace space-all")),
            Some((TextSpacingTrim::SpaceAll, TextAutospace::NoAutospace))
        );
        assert_eq!(
            parse_text_spacing(&tokens("normal trim-start")),
            Some((TextSpacingTrim::TrimStart, TextAutospace::Normal))
        );
        assert!(parse_text_spacing(&tokens("trim-start space-all")).is_none());
    }

    #[test]
    fn text_wrap_takes_a_mode_and_a_style() {
        assert_eq!(
            parse_text_wrap(&tokens("balance nowrap")),
            Some((false, TextWrapStyle::Balance))
        );
        assert_eq!(
            parse_text_wrap(&tokens("auto")),
            Some((true, TextWrapStyle::Auto))
        );
        assert!(parse_text_wrap(&tokens("wrap nowrap")).is_none());
    }

    #[test]
    fn text_transform_takes_a_case_and_the_widths() {
        let (case, extra) =
            parse_text_transform(&tokens("full-size-kana uppercase full-width")).unwrap();
        assert_eq!(case, TextTransform::Uppercase);
        assert!(extra.full_width && extra.full_size_kana && !extra.math_auto);
        assert!(parse_text_transform(&tokens("uppercase lowercase")).is_none());
        assert!(parse_text_transform(&tokens("none uppercase")).is_none());
        assert!(
            parse_text_transform(&tokens("math-auto")).is_some_and(|(_, extra)| extra.math_auto)
        );
    }

    #[test]
    fn the_other_keywords() {
        assert_eq!(
            parse_word_space_transform(&tokens("auto-phrase ideographic-space")),
            Some(WordSpaceTransform::Space {
                ideographic: true,
                auto_phrase: true
            })
        );
        assert!(parse_word_space_transform(&tokens("auto-phrase")).is_none());
        assert_eq!(
            parse_text_fit(&tokens("shrink per-line 40%")),
            Some(TextFit {
                kind: TextFitKind::Shrink,
                method: TextFitMethod::PerLine,
                limit: Some(40.0)
            })
        );
        assert!(parse_text_fit(&tokens("grow 150% consistent")).is_none());
        assert_eq!(
            parse_hanging_punctuation(&tokens("last allow-end first")),
            Some(HangingPunctuation {
                first: true,
                end: HangingEnd::AllowEnd,
                last: true
            })
        );
        assert!(parse_hanging_punctuation(&tokens("force-end allow-end")).is_none());
        assert_eq!(
            parse_hyphenate_character(&tokens("\"-\"")),
            Some(Some(Arc::from("-")))
        );
    }
}
