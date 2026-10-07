//! `@supports` conditions (M20): CSS Conditional 3's
//! `<supports-condition>`, with Conditional 4's `selector()`, parsed
//! from the css crate's tokens and decided by what this engine does. See
//! `docs/design/media.md`.
//!
//! `not`, `and` and `or` join conditions in parentheses, and `and` and
//! `or` may not be mixed without them. A declaration in parentheses is
//! supported when the cascade accepts it ([`crate::supports`]), and
//! `selector()` when the selector parser takes the selector strictly
//! ([`crate::selector_matching::supports_selector`]). Anything else in
//! parentheses or a function is `<general-enclosed>`, which is false
//! here, unlike a media query's unknown: `not` of it is true. Like a
//! media query's, parentheses nested past [`crate::media::MAX_NESTING`]
//! levels are not looked into, and a block the tokens end inside is
//! closed by their end.

use css::Token;

use crate::media::{Cursor, MAX_NESTING, trim};

/// Decides a `<supports-condition>`: `None` when the tokens do not
/// match its grammar, which makes the rule invalid (Conditional 3
/// §2.2), and otherwise whether this engine supports what it asks.
pub fn supports_condition(tokens: &[Token]) -> Option<bool> {
    let mut c = Cursor::new(tokens);
    let supported = condition(&mut c, 0)?;
    c.skip_ws();
    c.at_end().then_some(supported)
}

/// A condition at the cursor: `not` and one term, or terms joined by
/// `and` alone or by `or` alone. The terms are parsed whatever the
/// answer, since a later one can make the condition invalid. A keyword
/// needs no whitespace around it, as in a media query.
fn condition(c: &mut Cursor<'_>, depth: usize) -> Option<bool> {
    c.skip_ws();
    if c.take_word("not") {
        c.skip_ws();
        return Some(!in_parens(c, depth)?);
    }
    let mut supported = in_parens(c, depth)?;
    let mut joiner: Option<bool> = None; // Some(true) for `and`
    loop {
        let before = c.pos;
        c.skip_ws();
        let is_and = if c.take_word("and") {
            true
        } else if c.take_word("or") {
            false
        } else {
            c.pos = before;
            break;
        };
        if joiner.is_some_and(|j| j != is_and) {
            return None;
        }
        joiner = Some(is_and);
        c.skip_ws();
        let next = in_parens(c, depth)?;
        supported = if is_and {
            supported && next
        } else {
            supported || next
        };
    }
    Some(supported)
}

/// A `<supports-in-parens>`: a condition in parentheses, a declaration
/// in them, `selector()`, or `<general-enclosed>`; `None` when the
/// cursor is at none of them. `depth` is how many parentheses are
/// around the cursor.
fn in_parens(c: &mut Cursor<'_>, depth: usize) -> Option<bool> {
    match c.peek()? {
        Token::Function(name) => {
            let selector = name.eq_ignore_ascii_case("selector");
            let argument = c.take_block();
            Some(selector && crate::selector_matching::supports_selector(argument))
        }
        Token::OpenParen => {
            let inner = trim(c.take_block());
            if depth < MAX_NESTING {
                let mut inside = Cursor::new(inner);
                if let Some(supported) = condition(&mut inside, depth + 1) {
                    inside.skip_ws();
                    if inside.at_end() {
                        return Some(supported);
                    }
                }
            }
            Some(declaration(inner))
        }
        _ => None,
    }
}

/// Whether the cascade accepts `name: value`, the tokens between a
/// declaration's parentheses, with `!important` or without it. A custom
/// property's name keeps its case and its value may be empty (M21).
fn declaration(tokens: &[Token]) -> bool {
    let [Token::Ident(name), rest @ ..] = tokens else {
        return false;
    };
    let [Token::Colon, value @ ..] = trim(rest) else {
        return false;
    };
    let value = without_important(trim(value));
    if crate::custom::is_custom_property_name(name) {
        return crate::supports(name, value);
    }
    !value.is_empty() && crate::supports(&name.to_ascii_lowercase(), value)
}

/// A declaration's value without a trailing `!important`, written in
/// any case and with whitespace anywhere in it, as the css crate takes
/// it from a declaration.
fn without_important(value: &[Token]) -> &[Token] {
    if let [rest @ .., Token::Ident(word)] = value
        && word.eq_ignore_ascii_case("important")
        && let [rest @ .., Token::Delim('!')] = trim(rest)
    {
        return trim(rest);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decide(source: &str) -> Option<bool> {
        supports_condition(&css::tokenize(source))
    }

    /// Each case against what the reference browser answers, by
    /// `CSS.supports` and by keeping or dropping the rule.
    fn check(cases: &[(&str, Option<bool>)]) {
        for (source, want) in cases {
            assert_eq!(decide(source), *want, "{source}");
        }
    }

    #[test]
    fn declarations() {
        check(&[
            ("(color: red)", Some(true)),
            ("(color:red)", Some(true)),
            ("( color : red )", Some(true)),
            ("  (color: red)  ", Some(true)),
            ("(COLOR: RED)", Some(true)),
            ("(color: red !important)", Some(true)),
            ("(color: red!important)", Some(true)),
            ("(display: flex !important )", Some(true)),
            ("(display: flex ! important)", Some(true)),
            ("(display: flex!IMPORTANT)", Some(true)),
            ("(display: grid)", Some(true)),
            ("(margin: 1px 2px)", Some(true)),
            ("(transform: rotate(10deg))", Some(true)),
            ("(color: initial)", Some(true)),
            ("(color: inherit)", Some(true)),
            ("(color: unset)", Some(true)),
            ("(-webkit-transform: none)", Some(true)),
            // What the cascade does not take.
            ("(color:)", Some(false)),
            ("(color: red;)", Some(false)),
            ("(color: !important)", Some(false)),
            ("(color: red !important !important)", Some(false)),
            ("(color: red !ie)", Some(false)),
            ("(width: 1px 2px)", Some(false)),
            ("(color: [x])", Some(false)),
            ("(nonsense: 1)", Some(false)),
            // Custom properties and substitution functions (M21), as the
            // web-platform-tests' variable-supports cases have them.
            ("(--x: 1)", Some(true)),
            ("(--a: )", Some(true)),
            ("(--a:)", Some(true)),
            ("(--A: a)", Some(true)),
            ("(--0: a)", Some(true)),
            ("(--a:var(--a))", Some(true)),
            ("(--a: x !important)", Some(true)),
            ("(--a: inherit)", Some(true)),
            ("(--: a)", Some(false)),
            ("(--a: !)", Some(false)),
            ("(--a: var(--b,!))", Some(false)),
            ("(--a: revert)", Some(false)),
            ("(VAR-a: a)", Some(false)),
            ("(color: var(--a))", Some(true)),
            ("(color:var(--a))", Some(true)),
            ("(color: VAR(--a))", Some(true)),
            ("(color: var(--a, ))", Some(true)),
            ("(color: var(--a,/**/))", Some(true)),
            ("(color: var(1px))", Some(true)),
            ("(color: var(--a) <!--)", Some(true)),
            ("(color: --> var(--a))", Some(true)),
            ("(color: { [ var(--a) ] })", Some(true)),
            ("(color: [;] var(--a))", Some(true)),
            ("(color: var(--a)var(--b))", Some(true)),
            ("(color: 1px var(--a))", Some(true)),
            ("(color: env(safe-area-inset-top))", Some(true)),
            ("(color: var(--a,!))", Some(false)),
            ("(color: var(--a,!important))", Some(false)),
            ("(color: var(--a) !important)", Some(true)),
            ("(color: var(--a) !important !important)", Some(false)),
            ("(color: var(--a) ! 1)", Some(false)),
            ("(color: (!) var(--a))", Some(true)),
            ("(color: var(--a, ;))", Some(false)),
            ("(color: var())", Some(false)),
            ("(color: env(10px))", Some(false)),
            ("(nonsense: var(--a))", Some(false)),
        ]);
    }

    #[test]
    fn not_and_or() {
        check(&[
            ("(color: red) and (display: flex)", Some(true)),
            (
                "(color: red) and (display: flex) and (margin: 0)",
                Some(true),
            ),
            ("(color: red) and (nonsense: 1)", Some(false)),
            ("(color: red) or (nonsense: 1)", Some(true)),
            ("(nonsense: 1) or (color: red)", Some(true)),
            ("(nonsense: 1) or (nonsense: 2)", Some(false)),
            ("not (nonsense: 1)", Some(true)),
            ("not (color: red)", Some(false)),
            ("not (not (color: red))", Some(true)),
            ("((color: red))", Some(true)),
            ("( (color: red) )", Some(true)),
            (
                "((color: red) and (display: flex)) or (nonsense: 1)",
                Some(true),
            ),
            ("(color: red)and (display: block)", Some(true)),
            ("(color: red)or (x: y)", Some(true)),
            // What a comment between them leaves.
            ("(color: red) and/**/(display: flex)", Some(true)),
            ("not/**/(color: red)", Some(false)),
            // Invalid, the rule dropped.
            ("(color: red) and (display: flex) or (x: y)", None),
            ("(color: red) (display: flex)", None),
            ("not not (color: red)", None),
            ("(color: red) and", None),
            ("and (color: red)", None),
            ("(color: red) or not (display: flex)", None),
            ("(color: red))", None),
            ("(color: red) and [x]", None),
            ("(color: red) and(display: flex)", None),
            ("selector(a)(b)", None),
            ("", None),
            ("color: red", None),
        ]);
    }

    #[test]
    fn general_enclosed_is_false() {
        check(&[
            ("(foo bar)", Some(false)),
            ("not (foo bar)", Some(true)),
            ("foo(bar)", Some(false)),
            ("not foo(bar)", Some(true)),
            ("not(color: red)", Some(false)),
            ("(color)", Some(false)),
            ("(color red)", Some(false)),
            ("(:)", Some(false)),
            ("()", Some(false)),
            // Not this engine's.
            ("font-tech(color-COLRv1)", Some(false)),
            ("font-format(woff2)", Some(false)),
        ]);
    }

    #[test]
    fn the_end_closes_blocks() {
        check(&[
            ("(color: red", Some(true)),
            ("((color: red)", Some(true)),
            ("selector(a", Some(true)),
            ("(color: red) or ((x: y)", Some(true)),
            ("not (color: red", Some(false)),
        ]);
    }

    #[test]
    fn selectors_are_parsed_strictly() {
        check(&[
            ("selector(a)", Some(true)),
            ("SELECTOR(a)", Some(true)),
            ("selector( a )", Some(true)),
            ("selector(a > b)", Some(true)),
            ("selector(a b c)", Some(true)),
            ("selector(*)", Some(true)),
            ("selector(a:hover)", Some(true)),
            ("selector(::before)", Some(true)),
            ("selector(div::before)", Some(true)),
            ("selector(:root)", Some(true)),
            ("selector(:lang(en))", Some(true)),
            ("selector(:nth-child(odd))", Some(true)),
            ("selector(:nth-child(2n+1 of a))", Some(true)),
            ("selector(:is(a))", Some(true)),
            ("selector(:where(a, b))", Some(true)),
            ("selector(:not(a, b))", Some(true)),
            ("(color: red) and selector(a)", Some(true)),
            ("not selector(:foo)", Some(true)),
            // A list, nothing, or what the parser does not know.
            ("selector(a, b)", Some(false)),
            ("selector(a b, c)", Some(false)),
            ("selector(a,)", Some(false)),
            ("selector()", Some(false)),
            ("selector(:foo)", Some(false)),
            ("selector(::-webkit-foo)", Some(false)),
            ("selector(a::before::after)", Some(false)),
            ("selector(::before span)", Some(false)),
            // Not this engine's, though the reference browser's.
            ("selector(:has(a))", Some(false)),
            ("selector(&)", Some(false)),
            // `:is()` and `:where()` forgive nothing here.
            ("selector(:is(:foo))", Some(false)),
            ("selector(:is(a, :foo))", Some(false)),
            ("selector(:where(:foo))", Some(false)),
            ("selector(:not(:foo))", Some(false)),
            ("selector(:is())", Some(false)),
            ("selector(:where())", Some(false)),
        ]);
        // The rule's own lists stay forgiving.
        let forgiving = css::tokenize(":is(a, :foo)");
        assert!(crate::parse_selector_list(&forgiving).is_ok());
    }

    #[test]
    fn nesting_past_the_cap_is_not_looked_into() {
        let nested =
            |depth: usize| format!("{}(color: red){}", "(".repeat(depth), ")".repeat(depth));
        assert_eq!(decide(&nested(MAX_NESTING)), Some(true));
        assert_eq!(decide(&nested(MAX_NESTING + 1)), Some(false));
        for depth in [100_000, 300_000] {
            assert_eq!(decide(&nested(depth)), Some(false));
            assert_eq!(decide(&"(".repeat(depth)), Some(false));
            assert_eq!(decide(&"not (".repeat(depth)), Some(true));
        }
    }
}
