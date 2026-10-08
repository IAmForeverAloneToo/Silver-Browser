//! What a stylesheet's `@import` rules and its element's `media` say
//! (docs/design/media.md, "`@import` and the `media` attribute"): the
//! imports at a sheet's start with their layers and conditions, and the
//! blocks that wrap an imported sheet's rules, or an element's, in
//! them, so that the cascade treats them as rules written inside those
//! blocks.

use css::{AtRule, Block, Rule, Stylesheet, Token};
use net::Url;
use style::{Media, is_layer_name, parse_media_query_list, supports_condition};

/// An `@import` rule at the start of a sheet: its URL as written, and
/// what wraps the rules of the sheet it imports.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Import {
    /// The rule's index among the sheet's.
    pub(crate) rule: usize,
    pub(crate) url: String,
    /// The layer: none, the anonymous one (no tokens), or a name.
    layer: Option<Vec<Token>>,
    /// The `supports()` condition, a declaration put in parentheses.
    supports: Option<Vec<Token>>,
    /// The media query list; empty when there is none.
    media: Vec<Token>,
}

/// Makes the URLs in the declarations of `rules`, a sheet fetched from
/// `base`, absolute against it (M23.3, `docs/design/images.md`): each
/// `url()`, and each string of an `image-set()`, which is a URL there;
/// in nested rules and custom properties' values too, a URL in one being
/// its sheet's (Values 4 section 4.5.4: parsed as UTF-8). One that does
/// not parse becomes empty, which shows nothing. What the style crate
/// then gets is absolute, and the engine resolves what is not, a
/// `style` attribute's or element's, against the document's base.
pub(crate) fn absolute_urls(rules: &mut [Rule], base: &Url) {
    for rule in rules {
        match rule {
            Rule::Qualified(rule) => {
                for declaration in &mut rule.declarations {
                    absolute_in(&mut declaration.value, base);
                }
            }
            Rule::At(AtRule {
                block: Some(Block::Declarations(declarations)),
                ..
            }) => {
                for declaration in declarations {
                    absolute_in(&mut declaration.value, base);
                }
            }
            Rule::At(AtRule {
                block: Some(Block::Rules(rules)),
                ..
            }) => absolute_urls(rules, base),
            Rule::At(_) => {}
        }
    }
}

/// [`absolute_urls`] for one value's tokens.
fn absolute_in(tokens: &mut [Token], base: &Url) {
    // An empty URL stays empty: it names nothing (Values 4 section 4.5.1).
    let absolute = |value: &str| {
        if value.is_empty() {
            return String::new();
        }
        base.join(value)
            .map_or_else(|_| String::new(), |url| url.as_str().to_owned())
    };
    // How deep the value is in `image-set()`s, and in other blocks inside
    // the innermost: a string right inside one is a URL.
    let mut sets: Vec<usize> = Vec::new();
    for token in tokens {
        match token {
            Token::Url { value, bad: false } => *value = absolute(value),
            Token::String { value, bad: false } if sets.last() == Some(&0) => {
                *value = absolute(value)
            }
            Token::Function(name)
                if name.eq_ignore_ascii_case("image-set")
                    || name.eq_ignore_ascii_case("-webkit-image-set") =>
            {
                sets.push(0)
            }
            Token::Function(_) | Token::OpenParen | Token::OpenSquare | Token::OpenCurly => {
                if let Some(depth) = sets.last_mut() {
                    *depth += 1;
                }
            }
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => match sets.last_mut() {
                Some(0) => {
                    sets.pop();
                }
                Some(depth) => *depth -= 1,
                None => {}
            },
            _ => {}
        }
    }
}

/// The `@import` rules at the start of `sheet`, in order. Cascade 5
/// §6.1 has them first: after `@charset`, and after `@layer` statements
/// before the first of them, and any other rule ends them. An `@import`
/// with a block, one whose prelude does not begin with a URL, and one
/// whose layer or `supports()` does not parse are invalid, and passed
/// over.
pub(crate) fn imports(sheet: &Stylesheet) -> Vec<Import> {
    let mut found = Vec::new();
    for (index, rule) in sheet.rules.iter().enumerate() {
        let Rule::At(at) = rule else { break };
        let name = at.name.to_ascii_lowercase();
        match name.as_str() {
            "charset" => continue,
            "layer" if at.block.is_none() && found.is_empty() => continue,
            "import" => {}
            _ => break,
        }
        if at.block.is_some() {
            continue;
        }
        if let Some(import) = import(index, &at.prelude) {
            found.push(import);
        }
    }
    found
}

/// The `@import` whose prelude is `prelude`, at `index`: `[ <url> |
/// <string> ] [ layer | layer(<layer-name>) ]? [ supports( [
/// <supports-condition> | <declaration> ] ) ]? <media-query-list>?`.
fn import(index: usize, prelude: &[Token]) -> Option<Import> {
    let (url, rest) = import_url(prelude)?;
    let mut rest = trim_start(rest);
    let mut layer = None;
    match rest.first() {
        Some(Token::Ident(word)) if word.eq_ignore_ascii_case("layer") => {
            layer = Some(Vec::new());
            rest = trim_start(&rest[1..]);
        }
        Some(Token::Function(word)) if word.eq_ignore_ascii_case("layer") => {
            let (name, after) = function_argument(rest);
            let name = trim(name);
            if !is_layer_name(name) {
                return None;
            }
            layer = Some(name.to_vec());
            rest = trim_start(after);
        }
        _ => {}
    }
    let mut supports = None;
    if let Some(Token::Function(word)) = rest.first()
        && word.eq_ignore_ascii_case("supports")
    {
        let (argument, after) = function_argument(rest);
        let argument = trim(argument);
        let condition = if supports_condition(argument).is_some() {
            argument.to_vec()
        } else if let [Token::Ident(_), more @ ..] = argument
            && trim_start(more).first() == Some(&Token::Colon)
        {
            // A declaration, which a condition holds in parentheses.
            let mut condition = vec![Token::OpenParen];
            condition.extend_from_slice(argument);
            condition.push(Token::CloseParen);
            condition
        } else {
            return None;
        };
        supports = Some(condition);
        rest = trim_start(after);
    }
    Some(Import {
        rule: index,
        url,
        layer,
        supports,
        media: trim(rest).to_vec(),
    })
}

impl Import {
    /// `rules`, an imported sheet's, in the blocks this import puts
    /// them in: its media list outermost, then its `supports()`
    /// condition, then its layer, so that the layer is named only where
    /// the conditions hold (Cascade 5 §6.4.3). An import whose sheet is
    /// not there gives the blocks empty, its layer named all the same.
    pub(crate) fn wrap(&self, rules: Vec<Rule>) -> Vec<Rule> {
        let mut rules = rules;
        if let Some(layer) = &self.layer {
            rules = vec![block("layer", layer.clone(), rules)];
        }
        if let Some(condition) = &self.supports {
            rules = vec![block("supports", condition.clone(), rules)];
        }
        if !self.media.is_empty() {
            rules = vec![block("media", self.media.clone(), rules)];
        }
        rules
    }
}

/// `rules`, an element's sheet's, inside an `@media` block of its
/// `media` attribute; as they are for an absent or empty one, which
/// holds whatever the medium (HTML).
pub(crate) fn wrap_in_media(media: Option<&str>, rules: Vec<Rule>) -> Vec<Rule> {
    let list = media.map(css::tokenize).unwrap_or_default();
    if trim(&list).is_empty() {
        return rules;
    }
    vec![block("media", trim(&list).to_vec(), rules)]
}

/// Whether an element's `media` attribute holds for `media`; an absent
/// or empty one does.
pub(crate) fn media_attribute_holds(value: Option<&str>, media: &Media) -> bool {
    value.is_none_or(|value| parse_media_query_list(&css::tokenize(value)).evaluate(media))
}

/// An at-rule `@name prelude { rules }`.
fn block(name: &str, prelude: Vec<Token>, rules: Vec<Rule>) -> Rule {
    Rule::At(AtRule {
        name: name.to_owned(),
        prelude,
        block: Some(Block::Rules(rules)),
    })
}

/// The tokens between the function token at the start of `tokens` and
/// the `)` that closes it, and the tokens after; a function the tokens
/// end inside is closed by their end.
fn function_argument(tokens: &[Token]) -> (&[Token], &[Token]) {
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Function(_) | Token::OpenParen | Token::OpenSquare | Token::OpenCurly => {
                depth += 1
            }
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return (&tokens[1..i], &tokens[i + 1..]);
                }
            }
            _ => {}
        }
    }
    (&tokens[1.min(tokens.len())..], &[])
}

fn trim_start(tokens: &[Token]) -> &[Token] {
    let start = tokens
        .iter()
        .position(|t| *t != Token::Whitespace)
        .unwrap_or(tokens.len());
    &tokens[start..]
}

fn trim(tokens: &[Token]) -> &[Token] {
    let tokens = trim_start(tokens);
    let end = tokens
        .iter()
        .rposition(|t| *t != Token::Whitespace)
        .map_or(0, |i| i + 1);
    &tokens[..end]
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

    #[test]
    fn a_fetched_sheets_urls_are_made_absolute_against_it() {
        let base = Url::parse("http://a.test/css/s.css").unwrap();
        let mut sheet = css::parse_stylesheet(
            "p { background: url(i.png), image-set(\"j.png\" 2x type(\"image/png\"), url(k.png));              content: \"x.png\"; --v: url(../v.png); list-style-image: url('') }
             @media screen { q { background-image: url(/m.png) } }",
        )
        .unwrap();
        absolute_urls(&mut sheet.rules, &base);
        let strings = |rule: &Rule| -> Vec<String> {
            let Rule::Qualified(rule) = rule else {
                panic!("a style rule")
            };
            rule.declarations
                .iter()
                .flat_map(|d| d.value.iter())
                .filter_map(|t| match t {
                    Token::Url { value, .. } | Token::String { value, .. } => Some(value.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            strings(&sheet.rules[0]),
            [
                "http://a.test/css/i.png",
                "http://a.test/css/j.png",
                "image/png",
                "http://a.test/css/k.png",
                "x.png",
                "http://a.test/v.png",
                "",
            ]
        );
        let Rule::At(AtRule {
            block: Some(Block::Rules(inner)),
            ..
        }) = &sheet.rules[1]
        else {
            panic!("a block of rules")
        };
        assert_eq!(strings(&inner[0]), ["http://a.test/m.png"]);
    }

    fn imports_of(source: &str) -> Vec<Import> {
        imports(&css::parse_stylesheet(source).expect("never fails"))
    }

    fn urls_of(source: &str) -> Vec<String> {
        imports_of(source).into_iter().map(|i| i.url).collect()
    }

    /// The rule an import wraps an empty sheet in, written back.
    fn wrapped(source: &str) -> String {
        let import = imports_of(source).pop().expect("an import");
        let mut out = String::new();
        let mut rules = import.wrap(Vec::new());
        while let Some(Rule::At(at)) = rules.pop() {
            out.push_str(&format!("@{} {}; ", at.name, written(&at.prelude)));
            rules = match at.block {
                Some(Block::Rules(inner)) => inner,
                _ => Vec::new(),
            };
        }
        out.trim_end().to_owned()
    }

    fn written(tokens: &[Token]) -> String {
        tokens
            .iter()
            .map(|token| match token {
                Token::Ident(name) => name.clone(),
                Token::Function(name) => format!("{name}("),
                Token::OpenParen => "(".into(),
                Token::CloseParen => ")".into(),
                Token::Colon => ":".into(),
                Token::Comma => ",".into(),
                Token::Whitespace => " ".into(),
                Token::Delim(c) => c.to_string(),
                Token::Dimension { value, unit } => format!("{value}{unit}"),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn imports_are_read_from_the_start_of_a_sheet() {
        assert_eq!(
            urls_of(
                "@charset \"utf-8\"; @layer base; @import url(a.css); @import 'b.css' screen;\
                 @import \"c.css\" print; @import url(d.css) all, print; p {} @import 'late.css';"
            ),
            ["a.css", "b.css", "c.css", "d.css"]
        );
        // The rule each import stands for.
        let rules: Vec<usize> = imports_of("@layer a; @import 'a.css'; @import 'b.css';")
            .iter()
            .map(|i| i.rule)
            .collect();
        assert_eq!(rules, [1, 2]);
    }

    #[test]
    fn a_rule_that_is_not_an_import_ends_them() {
        assert!(urls_of("p {} @import 'a.css';").is_empty());
        assert!(urls_of("@media screen {} @import 'a.css';").is_empty());
        // A layer statement may come before the imports, not between.
        assert_eq!(
            urls_of("@import 'a.css'; @layer x; @import 'b.css';"),
            ["a.css"]
        );
    }

    #[test]
    fn layers_conditions_and_media_are_read_in_their_order() {
        assert_eq!(
            urls_of(
                "@import 'layered.css' layer; @import 'named.css' layer(x.y);\
                 @import 'supported.css' supports(display: grid);\
                 @import 'both.css' layer(x) supports((display: grid) and (gap: 0)) screen;\
                 @import 'selector.css' supports(selector(a > b)) print, (min-width: 5px);"
            ),
            [
                "layered.css",
                "named.css",
                "supported.css",
                "both.css",
                "selector.css"
            ]
        );
        assert_eq!(wrapped("@import 'a.css';"), "");
        assert_eq!(wrapped("@import 'a.css' layer;"), "@layer ;");
        assert_eq!(wrapped("@import 'a.css' LAYER(x.y);"), "@layer x.y;");
        // A declaration is put in parentheses; a condition is kept.
        assert_eq!(
            wrapped("@import 'a.css' supports(display: grid);"),
            "@supports (display: grid);"
        );
        assert_eq!(
            wrapped("@import 'a.css' supports(not (display: grid));"),
            "@supports not (display: grid);"
        );
        // The media outermost, then the condition, then the layer.
        assert_eq!(
            wrapped("@import 'a.css' layer(x) supports(color: red) print and (min-width: 5px);"),
            "@media print and (min-width: 5px); @supports (color: red); @layer x;"
        );
        assert_eq!(
            wrapped("@import 'a.css' layer screen;"),
            "@media screen; @layer ;"
        );
    }

    #[test]
    fn an_invalid_import_is_passed_over() {
        assert_eq!(
            urls_of(
                "@import; @import 12px; @import url(a.css) {} @import 'bad.css' layer(1);\
                 @import 'bad.css' layer(initial); @import 'bad.css' layer(a b);\
                 @import 'bad.css' layer();\
                 @import 'bad.css' supports(nonsense); @import 'bad.css' supports();\
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

    #[test]
    fn an_elements_media_wraps_its_rules_unless_it_is_empty() {
        let rules = css::parse_stylesheet("p {}").unwrap().rules;
        assert_eq!(wrap_in_media(None, rules.clone()), rules);
        assert_eq!(wrap_in_media(Some(" "), rules.clone()), rules);
        let wrapped = wrap_in_media(Some(" print "), rules.clone());
        let [Rule::At(at)] = wrapped.as_slice() else {
            panic!("one block");
        };
        assert_eq!(at.name, "media");
        assert_eq!(at.prelude, [Token::Ident("print".into())]);
        assert_eq!(at.block, Some(Block::Rules(rules)));
        let screen = Media::screen(800.0, 600.0);
        assert!(media_attribute_holds(None, &screen));
        assert!(media_attribute_holds(Some(""), &screen));
        assert!(media_attribute_holds(
            Some("screen and (min-width: 500px)"),
            &screen
        ));
        assert!(!media_attribute_holds(Some("print"), &screen));
        assert!(!media_attribute_holds(Some("(min-width: 1000px)"), &screen));
    }
}
