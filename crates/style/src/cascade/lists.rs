//! The counter properties and the counter functions of `content`
//! (Lists 3 §4, CSS 2.2 §12.2; M21.4), and the list properties (§3,
//! M21.6). Each parser takes a declaration's value as the cascade has it
//! and says `None` when the value is not one the property takes, so that
//! the declaration is dropped.

use std::sync::Arc;

use css::Token;

use super::{LengthBases, components, function_arguments, integer, single_ident};
use crate::computed::{
    ContentItem, CounterChange, CounterReset, Image, ListStylePosition, ListStyleType,
};
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

/// `list-style-type`: a `<counter-style>`, a string, or `none`.
pub(super) fn parse_list_style_type(tokens: &[Token]) -> Option<ListStyleType> {
    match super::trim(tokens) {
        [Token::Ident(word)] if word.eq_ignore_ascii_case("none") => Some(ListStyleType::None),
        [Token::String { value, bad: false }] => {
            Some(ListStyleType::String(Arc::from(value.as_str())))
        }
        part => counter_style_name(part).map(ListStyleType::Style),
    }
}

/// `list-style-position`: `inside` or `outside`.
pub(super) fn parse_list_style_position(tokens: &[Token]) -> Option<ListStylePosition> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "inside" => Some(ListStylePosition::Inside),
        "outside" => Some(ListStylePosition::Outside),
        _ => None,
    }
}

/// `list-style-image`: `none`, or an `<image>` (M23.3). `Some(None)` is
/// `none`, and so is an `image-set()` with no option this engine shows.
pub(super) fn parse_list_style_image(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Option<Image>> {
    match super::trim(tokens) {
        [Token::Ident(word)] if word.eq_ignore_ascii_case("none") => Some(None),
        part => super::backgrounds::image(part, bases),
    }
}

/// What the `list-style` shorthand sets: a position, an image and a
/// type, each the initial value where the shorthand leaves it out.
pub(super) type ListStyle = (ListStylePosition, Option<Image>, ListStyleType);

/// The `list-style` shorthand (Lists 3 §3.6): a position, an image and a
/// type in any order, each at most once. A `none` sets whichever of the
/// image and the type the rest leaves unset, both when the rest sets
/// neither.
pub(super) fn parse_list_style(tokens: &[Token], bases: &LengthBases<'_>) -> Option<ListStyle> {
    let mut position = None;
    let mut image = None;
    let mut kind = None;
    let mut nones = 0;
    for part in components(tokens) {
        if matches!(part, [Token::Ident(word)] if word.eq_ignore_ascii_case("none")) {
            nones += 1;
            continue;
        }
        // Each a first time, or the value is not the shorthand's.
        let first = if let Some(value) = parse_list_style_position(part) {
            position.replace(value).is_none()
        } else if let Some(value) = parse_list_style_image(part, bases) {
            image.replace(value).is_none()
        } else if let Some(value) = parse_list_style_type(part) {
            kind.replace(value).is_none()
        } else {
            false
        };
        if !first {
            return None;
        }
    }
    let unset = usize::from(image.is_none()) + usize::from(kind.is_none());
    match nones {
        0 => {}
        // One `none` and both unset: it is both.
        _ if nones <= unset => {
            image.get_or_insert(None);
            kind.get_or_insert(ListStyleType::None);
        }
        _ => return None,
    }
    if position.is_none() && image.is_none() && kind.is_none() && nones == 0 {
        return None;
    }
    Some((
        position.unwrap_or_default(),
        image.unwrap_or_default(),
        kind.unwrap_or_default(),
    ))
}

/// Whether the attribute `name` of an element named `tag` is one of the
/// list hints [`list_hints`] reads, whose change restyles the element.
pub(crate) fn is_list_hint(tag: &str, name: &str) -> bool {
    match tag {
        "ol" => matches!(name, "start" | "reversed" | "type"),
        "ul" => name == "type",
        "li" => matches!(name, "value" | "type"),
        _ => false,
    }
}

/// The declarations HTML's list attributes are presentational hints for
/// (HTML §15.3.8, M21.6), as the text of a declaration block: an `ol`'s
/// `start` resets `list-item` to one less, its `reversed` makes the
/// counter `reversed(list-item)`, starting at one more than `start`
/// when there is one; an `li`'s `value` sets `list-item`; and `type` on
/// an `ol`, a `ul` or an `li` gives `list-style-type`, `1`, `a`, `A`,
/// `i` and `I` matched as written and `none`, `disc`, `circle` and
/// `square` in any case. Empty when the element has none.
pub(super) fn list_hints(tag: &str, attr: impl Fn(&str) -> Option<String>) -> String {
    if !matches!(tag, "ol" | "ul" | "li") {
        return String::new();
    }
    let integer = |name: &str| attr(name).and_then(|value| dom::parse_integer(&value));
    let clamp = |n: i64| n.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    let mut out = String::new();
    match tag {
        "ol" => match (attr("reversed").is_some(), integer("start")) {
            (true, Some(start)) => {
                let start = clamp(start.saturating_add(1));
                out.push_str(&format!("counter-reset: reversed(list-item) {start};"));
            }
            (true, None) => out.push_str("counter-reset: reversed(list-item);"),
            (false, Some(start)) => {
                let start = clamp(start.saturating_sub(1));
                out.push_str(&format!("counter-reset: list-item {start};"));
            }
            (false, None) => {}
        },
        "li" => {
            if let Some(value) = integer("value") {
                let value = clamp(value);
                out.push_str(&format!("counter-set: list-item {value};"));
            }
        }
        _ => {}
    }
    let numbered = matches!(tag, "ol" | "li");
    let bulleted = matches!(tag, "ul" | "li");
    let style = attr("type").and_then(|kind| match kind.as_str() {
        "1" if numbered => Some("decimal"),
        "a" if numbered => Some("lower-alpha"),
        "A" if numbered => Some("upper-alpha"),
        "i" if numbered => Some("lower-roman"),
        "I" if numbered => Some("upper-roman"),
        _ if bulleted => ["none", "disc", "circle", "square"]
            .into_iter()
            .find(|name| kind.eq_ignore_ascii_case(name)),
        _ => None,
    });
    if let Some(style) = style {
        out.push_str(&format!("list-style-type: {style};"));
    }
    out
}
