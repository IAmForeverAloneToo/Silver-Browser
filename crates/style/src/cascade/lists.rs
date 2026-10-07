//! The counter properties and the counter functions of `content`
//! (Lists 3 §4, CSS 2.2 §12.2; M21.4). Each parser takes a declaration's
//! value as the cascade has it and says `None` when the value is not one
//! the property takes, so that the declaration is dropped.

use std::sync::Arc;

use css::Token;

use super::{LengthBases, components, function_arguments, integer, single_ident};
use crate::computed::{ContentItem, CounterChange, CounterReset};
use crate::counter_style;
use crate::custom::is_custom_ident;

/// A `<counter-name>`: a `<custom-ident>` other than `none`, its case
/// kept, names being case-sensitive.
fn counter_name(tokens: &[Token]) -> Option<Arc<str>> {
    match tokens {
        [Token::Ident(name)] if is_custom_ident(name) && !name.eq_ignore_ascii_case("none") => {
            Some(Arc::from(name.as_str()))
        }
        _ => None,
    }
}

/// An integer of a counter property: a number token that is one, or a
/// math function's number rounded, either clamped to 32 bits.
fn counter_integer(tokens: &[Token], bases: &LengthBases<'_>) -> Option<i32> {
    integer(tokens, bases)
}

/// `counter-reset`: `none`, or names and `reversed(<name>)`s, each with
/// an integer or not. A name written twice counts once, its last.
pub(super) fn parse_counter_reset(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Vec<CounterReset>> {
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("none")) {
        return Some(Vec::new());
    }
    let parts = components(tokens);
    let mut out: Vec<CounterReset> = Vec::new();
    let mut at = 0;
    while at < parts.len() {
        let (name, reversed) = match parts[at] {
            [Token::Function(function), inside @ .., Token::CloseParen]
                if function.eq_ignore_ascii_case("reversed") =>
            {
                (counter_name(super::trim(inside))?, true)
            }
            part => (counter_name(part)?, false),
        };
        at += 1;
        let value = match parts.get(at).and_then(|p| counter_integer(p, bases)) {
            Some(value) => {
                at += 1;
                Some(value)
            }
            None if reversed => None,
            None => Some(0),
        };
        out.retain(|reset| reset.name != name);
        out.push(CounterReset {
            name,
            reversed,
            value,
        });
    }
    (!out.is_empty()).then_some(out)
}

/// `counter-increment` and `counter-set`: `none`, or names each with an
/// integer or not, `default` when not (1 to increment, 0 to set). A name
/// written twice counts twice, in order.
pub(super) fn parse_counter_changes(
    tokens: &[Token],
    bases: &LengthBases<'_>,
    default: i32,
) -> Option<Vec<CounterChange>> {
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("none")) {
        return Some(Vec::new());
    }
    let parts = components(tokens);
    let mut out = Vec::new();
    let mut at = 0;
    while at < parts.len() {
        let name = counter_name(parts[at])?;
        at += 1;
        let value = match parts.get(at).and_then(|p| counter_integer(p, bases)) {
            Some(value) => {
                at += 1;
                value
            }
            None => default,
        };
        out.push(CounterChange { name, value });
    }
    (!out.is_empty()).then_some(out)
}

/// A `<counter-style>` where `counter()` and `counters()` take one: a
/// name, which a predefined style's ASCII-lowercased (Counter Styles 3
/// §3), `none` among them. `symbols()` is not taken (section 2 of
/// `docs/design/values.md`).
fn counter_style_name(tokens: &[Token]) -> Option<Arc<str>> {
    match tokens {
        [Token::Ident(name)] if is_custom_ident(name) => {
            Some(Arc::from(counter_style::canonical_name(name).as_ref()))
        }
        _ => None,
    }
}

/// `counter(<counter-name>, <counter-style>?)` or
/// `counters(<counter-name>, <string>, <counter-style>?)`, `component`
/// being one; the style `decimal` when none is given.
pub(super) fn counter_item(component: &[Token]) -> Option<ContentItem> {
    let (function, arguments) = function_arguments(component)?;
    let arguments: Vec<&[Token]> = arguments.into_iter().map(super::trim).collect();
    let decimal = || Arc::from("decimal");
    if function.eq_ignore_ascii_case("counter") {
        let (name, style) = match arguments.as_slice() {
            [name] => (counter_name(name)?, decimal()),
            [name, style] => (counter_name(name)?, counter_style_name(style)?),
            _ => return None,
        };
        return Some(ContentItem::Counter { name, style });
    }
    if function.eq_ignore_ascii_case("counters") {
        let separator = |tokens: &[Token]| match tokens {
            [Token::String { value, bad: false }] => Some(value.clone()),
            _ => None,
        };
        let (name, separator, style) = match arguments.as_slice() {
            [name, separator_tokens] => {
                (counter_name(name)?, separator(separator_tokens)?, decimal())
            }
            [name, separator_tokens, style] => (
                counter_name(name)?,
                separator(separator_tokens)?,
                counter_style_name(style)?,
            ),
            _ => return None,
        };
        return Some(ContentItem::Counters {
            name,
            separator,
            style,
        });
    }
    None
}
