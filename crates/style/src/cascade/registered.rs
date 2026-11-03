//! A registered custom property's computed value (Properties and Values
//! API section 2.4, D73): its value, matching its syntax, with each of
//! the data type it matches computed as a property of that type computes
//! it, in the form `getComputedStyle` writes, which `var()` then
//! substitutes. A length is in pixels; an angle in degrees, a time in
//! seconds, a resolution in dots per pixel; a number at its value and
//! an integer rounded; a `<length-percentage>` with a percentage in it a
//! `calc()` of the two; a colour `rgb()`, `currentcolor` the element's; a
//! transform function's lengths and angles so. A keyword, an identifier,
//! a string, a URL and an image are as written.

use css::Token;

use crate::calc::{self, Want};
use crate::custom::{DataType, Matched, Multiplier, Syntax};

use super::{
    LengthBases, Range, color_with, components, length, length_percentage, single_token,
    with_default_bases,
};

/// `tokens`, a value of a property registered with `syntax`, computed
/// against `bases`; `None` when they do not match the syntax.
pub(crate) fn compute(
    syntax: &Syntax,
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Vec<Token>> {
    let tokens = trim(tokens);
    let (data, multiplier) = match syntax.matched(tokens)? {
        Matched::AsIs => return Some(tokens.to_vec()),
        Matched::Type(data, multiplier) => (data, multiplier),
    };
    let parts = components(tokens);
    let (items, separator): (Vec<&[Token]>, &str) = match multiplier {
        // A transform list is one value, its functions its parts.
        None if data == DataType::TransformList => (parts, " "),
        None => (vec![tokens], ""),
        Some(Multiplier::Space) => (parts, " "),
        Some(Multiplier::Comma) => (
            parts
                .into_iter()
                .filter(|part| !matches!(part, [Token::Comma]))
                .collect(),
            ", ",
        ),
    };
    let data = match data {
        DataType::TransformList => DataType::TransformFunction,
        other => other,
    };
    let mut written = Vec::with_capacity(items.len());
    for item in items {
        written.push(one(data, item, bases)?);
    }
    // A URL kept as `url()` of a string, which is written quoted.
    let mut computed = Vec::new();
    for token in css::tokenize(&written.join(separator)) {
        match token {
            Token::Url { value, bad: false } => computed.extend([
                Token::Function("url".into()),
                Token::String { value, bad: false },
                Token::CloseParen,
            ]),
            other => computed.push(other),
        }
    }
    Some(computed)
}

/// [`compute`] with no element's bases: a registration's initial value,
/// which is computationally independent.
pub(crate) fn compute_alone(syntax: &Syntax, tokens: &[Token]) -> Option<Vec<Token>> {
    with_default_bases(|bases| compute(syntax, tokens, bases))
}

/// Whether `value`, of a property registered with `syntax`, is a colour
/// that reads `currentcolor`, which is computed once the element's
/// colour is.
pub(crate) fn reads_current_color(syntax: &Syntax, value: &[Token]) -> bool {
    matches!(
        syntax.matched(trim(value)),
        Some(Matched::Type(DataType::Color, _))
    ) && value
        .iter()
        .any(|t| matches!(t, Token::Ident(word) if word.eq_ignore_ascii_case("currentcolor")))
}

/// One value of the data type `data`, computed and written.
fn one(data: DataType, tokens: &[Token], bases: &LengthBases<'_>) -> Option<String> {
    let tokens = trim(tokens);
    let measure = |want: Want, unit: &str| -> Option<String> {
        let computed = calc::parse(&as_math(tokens), want)?.compute(bases, None);
        match computed {
            calc::Computed::Value(v) => Some(format!("{}{unit}", number(v))),
            _ => None,
        }
    };
    Some(match data {
        DataType::Length => crate::serialize::px(length(tokens, bases, Range::All)?),
        DataType::LengthPercentage => {
            crate::serialize::length_percentage(&length_percentage(tokens, bases, Range::All)?)?
        }
        DataType::Number => measure(Want::Number, "")?,
        DataType::Integer => {
            let computed = calc::parse(&as_math(tokens), Want::Number)?.compute(bases, None);
            match computed {
                calc::Computed::Value(v) => number(v.round()),
                _ => return None,
            }
        }
        DataType::Percentage => {
            let computed = calc::parse(&as_math(tokens), Want::Percentage)?.compute(bases, None);
            match computed {
                calc::Computed::Percent(p) | calc::Computed::Value(p) => format!("{}%", number(p)),
                calc::Computed::Expression(_) => return None,
            }
        }
        DataType::Angle => measure(Want::Angle, "deg")?,
        DataType::Time => measure(Want::Time, "s")?,
        DataType::Resolution => measure(Want::Resolution, "dppx")?,
        // `currentcolor` the element's colour, the colour it is used at,
        // in a mix too, and `light-dark()` by the element's scheme.
        DataType::Color => {
            crate::serialize::color(color_with(tokens, Some(bases.color), bases.dark)?.used())
        }
        DataType::TransformFunction | DataType::TransformList => function(tokens, bases)?,
        // A URL is written quoted, as `getComputedStyle` writes one.
        DataType::Url | DataType::Image => match url_of(tokens) {
            Some(url) => format!(
                "url(\"{}\")",
                url.replace('\\', "\\\\").replace('"', "\\\"")
            ),
            None => crate::serialize::tokens(tokens),
        },
        DataType::CustomIdent | DataType::String => crate::serialize::tokens(tokens),
    })
}

/// The address `tokens` give when they are one `url()`.
fn url_of(tokens: &[Token]) -> Option<&str> {
    match tokens {
        [Token::Url { value, bad: false }] => Some(value),
        [
            Token::Function(name),
            Token::String { value, bad: false },
            Token::CloseParen,
        ] if name.eq_ignore_ascii_case("url") => Some(value),
        _ => None,
    }
}

/// A transform function computed: each argument that is a length or a
/// percentage, or a math function of them, computed as a
/// `<length-percentage>` is; each angle in degrees; the rest as written.
fn function(tokens: &[Token], bases: &LengthBases<'_>) -> Option<String> {
    let (name, arguments) = match tokens {
        [Token::Function(name), inside @ .., Token::CloseParen] => (name, inside),
        _ => return None,
    };
    let mut written = Vec::new();
    for argument in arguments
        .split(|t| matches!(t, Token::Comma))
        .map(trim)
        .filter(|a| !a.is_empty())
    {
        let angle = calc::parse(&as_math(argument), Want::Angle).map(|p| p.compute(bases, None));
        let plain = calc::parse(&as_math(argument), Want::Number).map(|p| p.compute(bases, None));
        let text = match (angle, plain) {
            (Some(calc::Computed::Value(degrees)), _) => format!("{}deg", number(degrees)),
            (_, Some(calc::Computed::Value(value))) => number(value),
            _ => match single_token(argument) {
                Some(Token::Number { .. }) => crate::serialize::tokens(argument),
                _ => match length_percentage(argument, bases, Range::All) {
                    Some(value) => crate::serialize::length_percentage(&value)?,
                    None => crate::serialize::tokens(argument),
                },
            },
        };
        written.push(text);
    }
    Some(format!("{name}({})", written.join(", ")))
}

/// `tokens`, one value, as a math function: itself when it is one, else
/// `calc()` of it, which the math functions' parser computes in the
/// canonical unit.
fn as_math(tokens: &[Token]) -> Vec<Token> {
    if calc::is_math_function(tokens) {
        return tokens.to_vec();
    }
    let mut math = vec![Token::Function("calc".into())];
    math.extend_from_slice(tokens);
    math.push(Token::CloseParen);
    math
}

/// A number at its shortest, as CSSOM writes one: six significant places
/// at most, `-0` as `0`.
pub(crate) fn number(value: f64) -> String {
    let rounded = (value * 1e6).round() / 1e6;
    if rounded == 0.0 {
        "0".to_owned()
    } else {
        format!("{}", rounded as f32)
    }
}

/// `tokens` without the white space at their ends.
fn trim(mut tokens: &[Token]) -> &[Token] {
    while let [Token::Whitespace, rest @ ..] = tokens {
        tokens = rest;
    }
    while let [rest @ .., Token::Whitespace] = tokens {
        tokens = rest;
    }
    tokens
}
