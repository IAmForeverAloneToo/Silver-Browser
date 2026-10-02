//! What decides whether a stylesheet applies: a media query list, read
//! as far as this engine evaluates media (docs/design/net.md, "The
//! engine's loader"), and the `@import` rules at a sheet's start.

use css::{Rule, Stylesheet, Token};

/// Whether a media query list holds on a screen. An empty list holds,
/// and so does one with a query that is `all` or `screen`, with or
/// without `only` before it, in any case. A query with a condition, or
/// for another media type, is not evaluated until M20 evaluates media
/// queries, and does not hold: its sheet is left out, as the cascade
/// leaves out `@media` rules.
pub(crate) fn media_matches(tokens: &[Token]) -> bool {
    let mut queries = tokens.split(|token| *token == Token::Comma).peekable();
    // A list of nothing but whitespace is the empty list, which holds.
    if tokens.iter().all(|token| *token == Token::Whitespace) {
        return true;
    }
    queries.any(|query| {
        let words: Vec<&Token> = query
            .iter()
            .filter(|token| **token != Token::Whitespace)
            .collect();
        let media_type = match words.as_slice() {
            [Token::Ident(media_type)] => media_type,
            [Token::Ident(only), Token::Ident(media_type)] if only.eq_ignore_ascii_case("only") => {
                media_type
            }
            _ => return false,
        };
        media_type.eq_ignore_ascii_case("all") || media_type.eq_ignore_ascii_case("screen")
    })
}

/// Whether the value of a `media` attribute holds on a screen; an
/// absent attribute does.
pub(crate) fn media_attribute_matches(value: Option<&str>) -> bool {
    value.is_none_or(|value| media_matches(&css::tokenize(value)))
}

/// The URLs of the `@import` rules at the start of `sheet` that apply on
/// a screen, as written, in order. css-cascade-5 §6.1 has `@import`
/// rules first: after `@charset`, and after `@layer` statements before
/// the first of them, and any other rule ends them. An `@import` with a
/// block, or whose prelude does not begin with a URL, is invalid and
/// passed over. One into a cascade layer, or under a `supports()`
/// condition, is left out, as the cascade leaves out `@layer` and
/// `@supports` blocks; so is one whose media list does not hold.
pub(crate) fn imports(sheet: &Stylesheet) -> Vec<String> {
    let mut urls = Vec::new();
    let mut seen_import = false;
    for rule in &sheet.rules {
        let Rule::At(at) = rule else { break };
        let name = at.name.to_ascii_lowercase();
        match name.as_str() {
            "charset" => continue,
            "layer" if at.block.is_none() && !seen_import => continue,
            "import" => {}
            _ => break,
        }
        if at.block.is_some() {
            continue;
        }
        let Some((url, rest)) = import_url(&at.prelude) else {
            continue;
        };
        seen_import = true;
        let conditional = rest.iter().any(|token| match token {
            Token::Ident(name) => name.eq_ignore_ascii_case("layer"),
            Token::Function(name) => {
                name.eq_ignore_ascii_case("layer") || name.eq_ignore_ascii_case("supports")
            }
            _ => false,
        });
        if !conditional && media_matches(rest) {
            urls.push(url);
        }
    }
    urls
}

/// The URL an `@import` prelude begins with, a `url()` or a string, and
/// the tokens after it.
fn import_url(prelude: &[Token]) -> Option<(String, &[Token])> {
    let start = prelude
        .iter()
        .position(|token| *token != Token::Whitespace)?;
    match &prelude[start..] {
        [Token::Url { value, bad: false }, rest @ ..]
        | [Token::String { value, bad: false }, rest @ ..] => Some((value.clone(), rest)),
        // css-syntax's `url(` with a quoted string: a function token,
        // which this tokenizer folds into a url token, but a prelude
        // made otherwise may hold.
        [Token::Function(name), rest @ ..] if name.eq_ignore_ascii_case("url") => {
            let mut rest = rest
                .iter()
                .enumerate()
                .filter(|(_, t)| **t != Token::Whitespace);
            let (_, Token::String { value, bad: false }) = rest.next()? else {
                return None;
            };
            let (close, Token::CloseParen) = rest.next()? else {
                return None;
            };
            let after = start + 1 + close + 1;
            Some((value.clone(), &prelude[after..]))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holds(list: &str) -> bool {
        media_matches(&css::tokenize(list))
    }

    #[test]
    fn a_media_list_holds_for_all_and_screen_alone() {
        for list in [
            "",
            "  ",
            "all",
            "screen",
            "SCREEN",
            "only screen",
            "print, screen",
            " all ",
        ] {
            assert!(holds(list), "{list:?}");
        }
        for list in [
            "print",
            "speech",
            "not screen",
            "screen and (min-width: 1px)",
            "(min-width: 1px)",
            "only",
            "screen screen",
            ",",
            "print,",
            "only print",
        ] {
            assert!(!holds(list), "{list:?}");
        }
        assert!(media_attribute_matches(None));
        assert!(media_attribute_matches(Some("")));
        assert!(!media_attribute_matches(Some("print")));
    }

    fn imports_of(source: &str) -> Vec<String> {
        imports(&css::parse_stylesheet(source).expect("never fails"))
    }

    #[test]
    fn imports_are_read_from_the_start_of_a_sheet() {
        assert_eq!(
            imports_of(
                "@charset \"utf-8\"; @layer base; @import url(a.css); @import 'b.css' screen;\
                 @import \"c.css\" print; @import url(d.css) all, print; p {} @import 'late.css';"
            ),
            ["a.css", "b.css", "d.css"]
        );
    }

    #[test]
    fn a_rule_that_is_not_an_import_ends_them() {
        assert_eq!(imports_of("p {} @import 'a.css';"), Vec::<String>::new());
        assert_eq!(
            imports_of("@media screen {} @import 'a.css';"),
            Vec::<String>::new()
        );
        // A layer statement may come before the imports, not between.
        assert_eq!(
            imports_of("@import 'a.css'; @layer x; @import 'b.css';"),
            ["a.css"]
        );
    }

    #[test]
    fn an_invalid_or_conditional_import_is_passed_over() {
        assert_eq!(
            imports_of(
                "@import; @import 12px; @import url(a.css) {} @import 'layered.css' layer;\
                 @import 'named.css' layer(x); @import 'supported.css' supports(display: grid);\
                 @import 'layered2.css' layer, screen; @import 'named2.css' layer(x) all, screen;\
                 @import 'supported2.css' supports(display: grid) all, screen;\
                 @import url(b.css);"
            ),
            ["b.css"]
        );
    }

    #[test]
    fn a_url_function_with_a_string_is_an_import_url() {
        let prelude = [
            Token::Whitespace,
            Token::Function("url".into()),
            Token::Whitespace,
            Token::String {
                value: "a.css".into(),
                bad: false,
            },
            Token::CloseParen,
            Token::Whitespace,
            Token::Ident("print".into()),
        ];
        let (url, rest) = import_url(&prelude).expect("a URL");
        assert_eq!(url, "a.css");
        assert_eq!(rest, [Token::Whitespace, Token::Ident("print".into())]);
        assert_eq!(import_url(&prelude[..4]), None);
    }
}
