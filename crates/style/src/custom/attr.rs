//! `attr()` (M21.5): its first argument, an attribute's name and the type
//! its value is read as (Values 5 §8.7), and the `<syntax>` of `type()`
//! (Properties and Values API §5.1), as a value is matched against one.

use css::Token;

use super::is_custom_ident;
use crate::calc;
use crate::cascade::{components, matches_type};

/// What an `attr()`'s first argument says: `<attr-name> <attr-type>?`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Spec {
    /// The attribute's name, ASCII-lowercased, as an attribute selector
    /// matches one on an HTML element; `None` when a namespace prefix
    /// names a namespace, which no attribute of an HTML element is in.
    /// `|name`, the null namespace, is the name.
    pub(super) name: Option<String>,
    pub(super) kind: Kind,
}

/// How an attribute's value is read.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Kind {
    /// No type, or `raw-string`: the value as a string, not parsed.
    String,
    /// `number`, or an `<attr-unit>`: the value as one number token, a
    /// dimension in the unit or a percentage for `%` when one is given.
    /// A unit is any `<custom-ident>` here; one this engine does not know
    /// fails when the value is read.
    Number(Option<String>),
    /// `type(<syntax>)`: the value parsed as CSS, its own substitution
    /// functions replaced, matching the syntax.
    Typed(Syntax),
}

impl Spec {
    /// `<attr-name> <attr-type>?` from `tokens`, a first argument,
    /// wrapped in `{}` or not; `None` when it is not one. No white space
    /// may come between the parts of the name.
    pub(super) fn parse(tokens: &[Token]) -> Option<Spec> {
        let tokens = unwrap(trim(tokens));
        let (name, rest) = match tokens {
            [Token::Delim('|'), Token::Ident(name), rest @ ..] => {
                (Some(name.to_ascii_lowercase()), rest)
            }
            [
                Token::Ident(_),
                Token::Delim('|'),
                Token::Ident(_),
                rest @ ..,
            ] => (None, rest),
            [Token::Ident(name), rest @ ..] => (Some(name.to_ascii_lowercase()), rest),
            _ => return None,
        };
        let kind = match trim(rest) {
            [] => Kind::String,
            [Token::Ident(word)] if word.eq_ignore_ascii_case("raw-string") => Kind::String,
            [Token::Ident(word)] if word.eq_ignore_ascii_case("number") => Kind::Number(None),
            [Token::Ident(unit)] if is_custom_ident(unit) => Kind::Number(Some(unit.clone())),
            [Token::Delim('%')] => Kind::Number(Some("%".into())),
            [Token::Function(function), inside @ ..] if function.eq_ignore_ascii_case("type") => {
                // Closed at the end of the value when its `)` is missing.
                let inside = match inside {
                    [inside @ .., Token::CloseParen] => inside,
                    inside => inside,
                };
                Kind::Typed(Syntax::parse(inside)?)
            }
            _ => return None,
        };
        Some(Spec { name, kind })
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

/// `tokens`, or what is inside them when they are a `{}` block, the
/// wrapper of a free-form argument (Values 5 §2.4.1).
fn unwrap(tokens: &[Token]) -> &[Token] {
    match tokens {
        [Token::OpenCurly, inside @ .., Token::CloseCurly] => trim(inside),
        [Token::OpenCurly, inside @ ..] => trim(inside),
        tokens => tokens,
    }
}

/// The attribute value `value` read as one number, Values 5 §8.7 says:
/// with the white space at its ends stripped, one number token, kept as
/// a number, or made a dimension in `unit` or a percentage for `%`.
/// `None` when it is not one number, or `unit` is no unit this engine
/// knows (`fr` among them).
pub(super) fn number(value: &str, unit: Option<&str>) -> Option<Token> {
    let value = value.trim_matches(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C'));
    let (value, is_integer) = match css::tokenize(value).as_slice() {
        [Token::Number { value, is_integer }] => (*value, *is_integer),
        _ => return None,
    };
    Some(match unit {
        None => Token::Number { value, is_integer },
        Some("%") => Token::Percentage { value },
        Some(unit) if calc::measure(unit).is_some() || unit.eq_ignore_ascii_case("fr") => {
            Token::Dimension {
                value,
                unit: unit.to_owned(),
            }
        }
        Some(_) => return None,
    })
}

/// A `<syntax>` (Properties and Values API §5.1), as `type()` takes one:
/// `*`, any value, or components joined by `|`, a value matching the
/// syntax when it matches one of them.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Syntax {
    Universal,
    Components(Vec<Component>),
}

/// One component of a [`Syntax`]: a data type or a keyword, with how many
/// of it are taken.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Component {
    name: Name,
    multiplier: Option<Multiplier>,
}

#[derive(Debug, Clone, PartialEq)]
enum Name {
    /// `<data-type>`.
    Type(DataType),
    /// An identifier, which matches itself, case-sensitively.
    Keyword(String),
}

/// `+` or `#` after a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Multiplier {
    /// `+`: one or more, white space between them.
    Space,
    /// `#`: one or more, commas between them.
    Comma,
}

/// A data type a `<syntax>` may name (Properties and Values API §5.1);
/// `<url>` may not be one in `attr()`'s `type()` (Values 5 §8.7), an
/// attribute's value never being a URL. `<image>` is taken and matches
/// nothing until the engine has images (M23).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DataType {
    Angle,
    Color,
    CustomIdent,
    Image,
    Integer,
    Length,
    LengthPercentage,
    Number,
    Percentage,
    Resolution,
    String,
    Time,
    TransformFunction,
    /// `<transform-list>`, which takes no multiplier.
    TransformList,
}

impl DataType {
    /// The data type of the name between `<` and `>`, case-sensitively.
    fn of(name: &str) -> Option<DataType> {
        Some(match name {
            "angle" => DataType::Angle,
            "color" => DataType::Color,
            "custom-ident" => DataType::CustomIdent,
            "image" => DataType::Image,
            "integer" => DataType::Integer,
            "length" => DataType::Length,
            "length-percentage" => DataType::LengthPercentage,
            "number" => DataType::Number,
            "percentage" => DataType::Percentage,
            "resolution" => DataType::Resolution,
            "string" => DataType::String,
            "time" => DataType::Time,
            "transform-function" => DataType::TransformFunction,
            "transform-list" => DataType::TransformList,
            _ => return None,
        })
    }
}

impl Syntax {
    /// The `<syntax>` `tokens` are, the inside of `type()`; `None` when
    /// they are none. White space may come around `|` and nowhere inside
    /// a component: `< length >` and `<length> +` are not syntax.
    fn parse(tokens: &[Token]) -> Option<Syntax> {
        let tokens = trim(tokens);
        if let [Token::Delim('*')] = tokens {
            return Some(Syntax::Universal);
        }
        let mut components = Vec::new();
        let mut at = 0;
        loop {
            while matches!(tokens.get(at), Some(Token::Whitespace)) {
                at += 1;
            }
            let name = match tokens.get(at..at + 3) {
                Some([Token::Delim('<'), Token::Ident(name), Token::Delim('>')]) => {
                    at += 3;
                    Name::Type(DataType::of(name)?)
                }
                _ => match tokens.get(at) {
                    Some(Token::Ident(word)) if is_custom_ident(word) => {
                        at += 1;
                        Name::Keyword(word.clone())
                    }
                    _ => return None,
                },
            };
            let multiplier = match tokens.get(at) {
                Some(Token::Delim('+')) => Some(Multiplier::Space),
                Some(Token::Delim('#')) => Some(Multiplier::Comma),
                _ => None,
            };
            if multiplier.is_some() {
                if name == Name::Type(DataType::TransformList) {
                    return None;
                }
                at += 1;
            }
            components.push(Component { name, multiplier });
            while matches!(tokens.get(at), Some(Token::Whitespace)) {
                at += 1;
            }
            match tokens.get(at) {
                None => break,
                Some(Token::Delim('|')) => at += 1,
                Some(_) => return None,
            }
        }
        Some(Syntax::Components(components))
    }

    /// Whether `tokens`, a value with its substitution functions
    /// replaced and its white space at the ends left out, match the
    /// syntax. `*` takes any value: [`super::check`] has found it a
    /// `<declaration-value>` before.
    pub(super) fn matches(&self, tokens: &[Token]) -> bool {
        match self {
            Syntax::Universal => true,
            Syntax::Components(choices) => choices.iter().any(|c| c.matches(tokens)),
        }
    }
}

impl Component {
    /// Whether `tokens`, a whole value, match this component.
    fn matches(&self, tokens: &[Token]) -> bool {
        let parts = components(tokens);
        match self.multiplier {
            None if self.name == Name::Type(DataType::TransformList) => {
                matches_type(DataType::TransformList, tokens)
            }
            None => matches!(parts.as_slice(), [one] if self.matches_one(one)),
            Some(Multiplier::Space) => {
                !parts.is_empty() && parts.iter().all(|p| self.matches_one(p))
            }
            Some(Multiplier::Comma) => {
                // One value between each two commas, and at least one.
                let mut items = parts.split(|p| matches!(p, [Token::Comma]));
                items.all(|item| matches!(item, [one] if self.matches_one(one)))
            }
        }
    }

    /// Whether `tokens`, one component value, are one of this component.
    fn matches_one(&self, tokens: &[Token]) -> bool {
        match &self.name {
            Name::Keyword(word) => matches!(tokens, [Token::Ident(w)] if w == word),
            Name::Type(data) => matches_type(*data, tokens),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(source: &str) -> Option<Spec> {
        Spec::parse(&css::tokenize(source))
    }

    fn syntax(source: &str) -> Syntax {
        match spec(&format!("x type({source})")).map(|s| s.kind) {
            Some(Kind::Typed(syntax)) => syntax,
            other => panic!("{source}: {other:?}"),
        }
    }

    #[test]
    fn the_first_argument_is_a_name_and_a_type() {
        let named = |name: &str, kind| {
            Some(Spec {
                name: Some(name.into()),
                kind,
            })
        };
        assert_eq!(spec("data-foo"), named("data-foo", Kind::String));
        assert_eq!(spec(" Data-Foo "), named("data-foo", Kind::String));
        assert_eq!(spec("{data-foo}"), named("data-foo", Kind::String));
        assert_eq!(spec("|data-foo"), named("data-foo", Kind::String));
        assert_eq!(spec("x raw-string"), named("x", Kind::String));
        assert_eq!(spec("x NUMBER"), named("x", Kind::Number(None)));
        assert_eq!(spec("x px"), named("x", Kind::Number(Some("px".into()))));
        assert_eq!(spec("x xx"), named("x", Kind::Number(Some("xx".into()))));
        assert_eq!(spec("x %"), named("x", Kind::Number(Some("%".into()))));
        assert_eq!(
            spec("svg|x"),
            Some(Spec {
                name: None,
                kind: Kind::String
            })
        );
        for bad in [
            "",
            "()",
            "!",
            "1px",
            "x &",
            "x inherit",
            "x <px>",
            "x px px",
            "x | y",
            "x |y",
            "x type(string)) y",
            "x type(<custom-ident>) <ident>",
            "x type(<color>) | <red>",
            "x type(< length>)",
            "x type(<length >)",
            "x type(<length> +)",
            "x type(<invalid>)",
            "x type(<url>)",
            "x type(invalid |)",
            "x type(<time>, <length>)",
            "x type(<transform-list>+)",
            "x type()",
            "x type(* | <length>)",
        ] {
            assert_eq!(spec(bad), None, "{bad:?}");
        }
        // A `type()` left open at the end of the value is closed there.
        assert!(spec("x type(<length>").is_some());
    }

    #[test]
    fn a_syntax_matches_its_values() {
        let value = |source: &str| css::tokenize(source);
        let matches = |syntax_source: &str, value_source: &str| {
            syntax(syntax_source).matches(&value(value_source))
        };
        for (syntax, value) in [
            ("*", "anything at all"),
            ("<length>", "10px"),
            ("<length>", "0"),
            ("<length>", "calc(1px + 3px)"),
            ("<length>", "2em"),
            ("<length-percentage>", "calc(10% - 1px)"),
            ("<number>", "1e3"),
            ("<number>", "calc(3 + 3)"),
            ("<integer>", "-4"),
            ("<percentage>", "10%"),
            ("<angle>", "10deg"),
            ("<angle>", "1turn"),
            ("<time>", "10ms"),
            ("<resolution>", "2x"),
            ("<color>", "red"),
            ("<color>", "#ff0099aa"),
            ("<custom-ident>", "anim"),
            ("<string>", "\"hi\""),
            ("<string>+", "\"hello\" \"hi\""),
            ("<length>#", "1px, 2px,3px"),
            ("<transform-function>", "rotate(10deg)"),
            ("<transform-list>", "rotate(10deg) scale(2)"),
            ("<number> | lighter | bold", "lighter"),
            ("<percentage> | <length>", "10px"),
            ("foo", "foo"),
        ] {
            assert!(matches(syntax, value), "{syntax}: {value}");
        }
        for (syntax, value) in [
            ("<length>", "10"),
            ("<length>", "10%"),
            ("<length>", "1px 2px"),
            ("<length>", "calc(10 + 20)"),
            ("<number>", "10px"),
            ("<integer>", "1.5"),
            ("<percentage>", "10% a"),
            ("<angle>", "10%"),
            ("<angle>", "calc(10px + 20px)"),
            ("<angle>", "30"),
            ("<time>", "10"),
            ("<time>", "10m"),
            ("<custom-ident>", "initial"),
            ("<custom-ident>", "\"anim\""),
            ("<custom-ident>", "default"),
            ("<color>", "10"),
            ("<color>", "1000px"),
            ("<string>", "abc"),
            ("<length>#", "1px,,2px"),
            ("<length>#", "1px 2px"),
            ("<length>+", ""),
            ("<transform-list>", "none"),
            ("<image>", "red"),
            ("foo", "Foo"),
            ("foo", "foo bar"),
        ] {
            assert!(!matches(syntax, value), "{syntax}: {value}");
        }
    }

    #[test]
    fn numbers_are_one_token_in_a_known_unit() {
        let number = |value: &str, unit: Option<&str>| number(value, unit);
        assert_eq!(
            number(" 10\n", None),
            Some(Token::Number {
                value: 10.0,
                is_integer: true
            })
        );
        assert_eq!(
            number("2.5", Some("px")),
            Some(Token::Dimension {
                value: 2.5,
                unit: "px".into()
            })
        );
        assert_eq!(
            number("3", Some("kHz")),
            Some(Token::Dimension {
                value: 3.0,
                unit: "kHz".into()
            })
        );
        assert_eq!(
            number("1", Some("fr")),
            Some(Token::Dimension {
                value: 1.0,
                unit: "fr".into()
            })
        );
        assert_eq!(
            number("10", Some("%")),
            Some(Token::Percentage { value: 10.0 })
        );
        for (value, unit) in [
            ("10px", Some("px")),
            ("abc", None),
            ("1 2", None),
            ("calc(1 + 3)", None),
            ("10", Some("xx")),
            ("", None),
            ("1.", None),
        ] {
            assert_eq!(number(value, unit), None, "{value:?} {unit:?}");
        }
    }
}
