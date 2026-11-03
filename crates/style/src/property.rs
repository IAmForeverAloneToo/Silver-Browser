//! Registered custom properties (CSS Properties and Values API 1, D73):
//! what `CSS.registerProperty` and `@property` give a custom property, a
//! syntax, whether it is inherited and an initial value, and the
//! registrations in force, which the cascade computes the properties by.
//!
//! A registration's syntax is read from its text ([`Syntax::parse_text`]);
//! its initial value, required unless the syntax is `*`, must match it,
//! hold no substitution function and be computationally independent: no
//! unit relative to the element, its font or the viewport. It is computed
//! once, when it is registered, as an element's value is. A property
//! registered with a syntax other than `*` is computed by it: a value
//! that does not match it is invalid at computed-value time (D73), and
//! its lengths, angles, times, resolutions, numbers and colours take
//! their computed forms, which `var()` substitutes and `getComputedStyle`
//! shows; one not inherited takes its initial value where it is not
//! declared. Of a name's registrations a script's wins over every
//! `@property` rule's, and of those the one in the layer last in the
//! order of normal declarations, then the last in the sheets' order, as
//! the other name-defining rules (Cascade 5 section 6.4.3).

use std::collections::HashMap;
use std::sync::Arc;

use css::Token;

use crate::custom::{self, Syntax};

/// A custom property registered.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyRegistration {
    /// The property's name, two hyphens and more.
    pub name: String,
    /// Its syntax as given, which the CSSOM shows.
    pub syntax_text: String,
    pub(crate) syntax: Syntax,
    /// Whether it is inherited.
    pub inherits: bool,
    /// Its initial value as given, white space at its ends left out.
    pub initial_text: Option<Vec<Token>>,
    /// Its initial value computed; `None` for the guaranteed-invalid
    /// value, a `*` property's without one.
    pub(crate) initial: Option<Arc<[Token]>>,
    /// Whether the initial value reads the viewport's size, which each
    /// cascade computes it against again.
    pub(crate) viewport: bool,
}

/// Why a registration was refused, the message of the `SyntaxError` a
/// script is given.
pub type RegistrationError = &'static str;

/// `CSS.registerProperty`'s checks (Properties and Values API section
/// 3.1): the registration of `name` with the syntax string `syntax`, the
/// inheritance `inherits` and the initial value `initial`, or why there
/// is none. That the name is registered already is the caller's to say.
pub fn register_property(
    name: &str,
    syntax: &str,
    inherits: bool,
    initial: Option<&str>,
) -> Result<PropertyRegistration, RegistrationError> {
    let tokens = initial.map(css::tokenize);
    registration(name, syntax, inherits, tokens.as_deref())
}

/// The registration an `@property` rule makes (Properties and Values API
/// section 4): its prelude a custom property's name, its descriptors a
/// `syntax` string and `inherits`, `true` or `false`, both required, and
/// an `initial-value` the syntax may require; `None` for a rule that is
/// invalid and makes none.
pub fn property_rule(
    prelude: &[Token],
    declarations: &[css::Declaration],
) -> Option<PropertyRegistration> {
    let name = match trim(prelude) {
        [Token::Ident(name)] => name,
        _ => return None,
    };
    let descriptor = |wanted: &str| {
        declarations
            .iter()
            .rev()
            .find(|d| d.name.eq_ignore_ascii_case(wanted) && !d.important)
            .map(|d| trim(&d.value))
    };
    let syntax = match descriptor("syntax")? {
        [Token::String { value, bad: false }] => value.clone(),
        _ => return None,
    };
    let inherits = match descriptor("inherits")? {
        [Token::Ident(word)] if word.eq_ignore_ascii_case("true") => true,
        [Token::Ident(word)] if word.eq_ignore_ascii_case("false") => false,
        _ => return None,
    };
    registration(name, &syntax, inherits, descriptor("initial-value")).ok()
}

/// The registration of `name`: see [`register_property`].
fn registration(
    name: &str,
    syntax_text: &str,
    inherits: bool,
    initial: Option<&[Token]>,
) -> Result<PropertyRegistration, RegistrationError> {
    if !custom::is_custom_property_name(name) {
        return Err("The name must be a custom property's.");
    }
    let syntax = Syntax::parse_text(syntax_text).ok_or("The syntax is not a valid one.")?;
    let initial_text = initial.map(|tokens| trim(tokens).to_vec());
    let initial = match &initial_text {
        None if syntax.is_universal() => None,
        None => return Err("An initial value is required for a syntax other than '*'."),
        Some(tokens) => Some(initial_value(&syntax, tokens)?),
    };
    let viewport = initial_text.as_deref().is_some_and(reads_viewport);
    Ok(PropertyRegistration {
        name: name.to_owned(),
        syntax_text: syntax_text.to_owned(),
        syntax,
        inherits,
        initial_text,
        initial,
        viewport,
    })
}

/// Whether `tokens` name a unit of the viewport's size (Values 4
/// section 6.1.2), small, large and dynamic ones among them.
fn reads_viewport(tokens: &[Token]) -> bool {
    tokens.iter().any(|token| match token {
        Token::Dimension { unit, .. } => {
            let unit = unit.to_ascii_lowercase();
            let unit = unit.trim_start_matches(['s', 'l', 'd']);
            matches!(unit, "vw" | "vh" | "vi" | "vb" | "vmin" | "vmax")
        }
        _ => false,
    })
}

/// The computed initial value `tokens` give a property of `syntax`, or
/// why they give none: one that is a CSS-wide keyword, holds a
/// substitution function, does not match the syntax or is not
/// computationally independent.
fn initial_value(syntax: &Syntax, tokens: &[Token]) -> Result<Arc<[Token]>, RegistrationError> {
    let invalid = "The initial value is not valid for the syntax.";
    if crate::cascade::css_wide_keyword(tokens).is_some()
        || crate::cascade::revert_keyword(tokens).is_some()
        || custom::has_substitution(tokens)
        || !custom::check(tokens)
    {
        return Err(invalid);
    }
    if !syntax.matches(tokens) {
        return Err(invalid);
    }
    if !computationally_independent(tokens) {
        return Err("The initial value is not computationally independent.");
    }
    crate::cascade::compute_registered_alone(syntax, tokens)
        .ok_or(invalid)
        .map(Arc::from)
}

/// Whether `tokens` name no unit whose value depends on an element: a
/// length relative to a font, a line or a container. The viewport's
/// size is the document's, not the element's: its units are
/// computationally independent (Properties and Values API section
/// 3.1).
fn computationally_independent(tokens: &[Token]) -> bool {
    const DEPENDENT: [&str; 18] = [
        "em", "ex", "ch", "cap", "ic", "lh", "rem", "rex", "rch", "rcap", "ric", "rlh", "cqw",
        "cqh", "cqi", "cqb", "cqmin", "cqmax",
    ];
    tokens.iter().all(|token| match token {
        Token::Dimension { unit, .. } => !DEPENDENT.iter().any(|u| unit.eq_ignore_ascii_case(u)),
        _ => true,
    })
}

/// The value `p` of the way from `from` to `to`, two computed values of a
/// property registered with a syntax (Properties and Values API section
/// 2.5): a number, a dimension of one unit, a length and a percentage
/// together as `calc()`, a colour, and a transform function of the same
/// name and arguments, each interpolated, an integer rounded; lists of
/// them item by item, of the same length and separator. `None` for values
/// that do not interpolate, which step discretely, half way.
pub(crate) fn interpolate(from: &[Token], to: &[Token], p: f64) -> Option<Vec<Token>> {
    let items = |tokens: &[Token]| -> (Vec<Vec<Token>>, bool) {
        let tokens = trim(tokens);
        let commas = tokens.contains(&Token::Comma);
        let parts = crate::cascade::components(tokens)
            .into_iter()
            .filter(|part| !matches!(part, [Token::Comma]))
            .map(<[Token]>::to_vec)
            .collect();
        (parts, commas)
    };
    let ((a, a_commas), (b, b_commas)) = (items(from), items(to));
    if a.is_empty() || a.len() != b.len() || a_commas != b_commas {
        return None;
    }
    let mut written = Vec::with_capacity(a.len());
    for (a, b) in a.iter().zip(&b) {
        written.push(interpolate_one(a, b, p)?);
    }
    let separator = if a_commas { ", " } else { " " };
    Some(css::tokenize(&written.join(separator)))
}

/// One item of [`interpolate`], written.
fn interpolate_one(a: &[Token], b: &[Token], p: f64) -> Option<String> {
    let mix = |x: f64, y: f64| x + (y - x) * p;
    // A length, a percentage or both: pixels and a percentage.
    let length_percentage = |tokens: &[Token]| -> Option<(f64, f64)> {
        match tokens {
            [Token::Dimension { value, unit }] if unit.eq_ignore_ascii_case("px") => {
                Some((*value, 0.0))
            }
            [Token::Percentage { value }] => Some((0.0, *value)),
            [Token::Number { value, .. }] if *value == 0.0 => Some((0.0, 0.0)),
            [Token::Function(name), inside @ .., Token::CloseParen]
                if name.eq_ignore_ascii_case("calc") =>
            {
                let parts: Vec<&Token> = inside
                    .iter()
                    .filter(|t| !matches!(t, Token::Whitespace))
                    .collect();
                match parts.as_slice() {
                    [
                        Token::Percentage { value: pct },
                        Token::Delim(sign),
                        Token::Dimension { value: px, unit },
                    ] if unit.eq_ignore_ascii_case("px") && matches!(sign, '+' | '-') => {
                        Some((if *sign == '-' { -px } else { *px }, *pct))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    };
    let number = |value: f64| crate::cascade::registered_number(value);
    match (a, b) {
        (
            [
                Token::Number {
                    value: x,
                    is_integer: xi,
                },
            ],
            [
                Token::Number {
                    value: y,
                    is_integer: yi,
                },
            ],
        ) => {
            let value = mix(*x, *y);
            Some(number(if *xi && *yi { value.round() } else { value }))
        }
        ([Token::Dimension { value: x, unit: u }], [Token::Dimension { value: y, unit: v }])
            if u.eq_ignore_ascii_case(v) =>
        {
            Some(format!("{}{u}", number(mix(*x, *y))))
        }
        ([Token::Percentage { value: x }], [Token::Percentage { value: y }]) => {
            Some(format!("{}%", number(mix(*x, *y))))
        }
        ([Token::Function(f), ..], [Token::Function(g), ..])
            if f.eq_ignore_ascii_case(g) && !f.eq_ignore_ascii_case("calc") =>
        {
            let colors = (
                crate::cascade::parse_color(a),
                crate::cascade::parse_color(b),
            );
            if let (Some(x), Some(y)) = colors {
                return Some(crate::serialize::color(crate::animate::color(x, y, p)));
            }
            // A transform function: its arguments one by one.
            let arguments = |tokens: &[Token]| -> Vec<Vec<Token>> {
                match tokens {
                    [_, inside @ .., Token::CloseParen] => inside
                        .split(|t| matches!(t, Token::Comma))
                        .map(|argument| trim(argument).to_vec())
                        .collect(),
                    _ => Vec::new(),
                }
            };
            let (xs, ys) = (arguments(a), arguments(b));
            if xs.len() != ys.len() {
                return None;
            }
            let mut written = Vec::with_capacity(xs.len());
            for (x, y) in xs.iter().zip(&ys) {
                written.push(interpolate_one(x, y, p)?);
            }
            Some(format!("{f}({})", written.join(", ")))
        }
        _ => {
            if let (Some(x), Some(y)) = (
                crate::cascade::parse_color(a),
                crate::cascade::parse_color(b),
            ) {
                return Some(crate::serialize::color(crate::animate::color(x, y, p)));
            }
            let ((xl, xp), (yl, yp)) = (length_percentage(a)?, length_percentage(b)?);
            let (px, pct) = (mix(xl, yl), mix(xp, yp));
            // A length between lengths, a percentage between percentages,
            // and a `calc()` of the two between any others.
            let percent = |t: &[Token]| matches!(t, [Token::Percentage { .. }]);
            let plain = |t: &[Token]| matches!(t, [Token::Dimension { .. } | Token::Number { .. }]);
            Some(if plain(a) && plain(b) {
                format!("{}px", number(px))
            } else if percent(a) && percent(b) {
                format!("{}%", number(pct))
            } else if px < 0.0 {
                format!("calc({}% - {}px)", number(pct), number(-px))
            } else {
                format!("calc({}% + {}px)", number(pct), number(px))
            })
        }
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

/// The custom properties registered for a cascade, by name.
#[derive(Debug, Default)]
pub(crate) struct Registry {
    by_name: HashMap<Arc<str>, Arc<PropertyRegistration>>,
}

impl Registry {
    /// The registry of `rules`, the `@property` rules in force in their
    /// order of precedence, the least first, and `scripts`, the script's
    /// registrations, which win over them.
    pub(crate) fn new<'r>(
        rules: impl IntoIterator<Item = &'r PropertyRegistration>,
        scripts: &'r [PropertyRegistration],
    ) -> Registry {
        let mut by_name = HashMap::new();
        for registration in rules.into_iter().chain(scripts) {
            by_name.insert(
                Arc::from(registration.name.as_str()),
                Arc::new(registration.clone()),
            );
        }
        Registry { by_name }
    }

    /// The registration of `name`, if it has one.
    pub(crate) fn get(&self, name: &str) -> Option<&PropertyRegistration> {
        self.by_name.get(name).map(|r| &**r)
    }

    /// Whether nothing is registered.
    pub(crate) fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// Every registration, in no order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &PropertyRegistration> {
        self.by_name.values().map(|r| &**r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(syntax: &str, initial: &str) -> bool {
        register_property("--x", syntax, false, Some(initial)).is_ok()
    }

    #[test]
    fn a_registration_takes_a_syntax_and_an_initial_value_of_it() {
        for (syntax, initial) in [
            ("*", "a"),
            (" <length>+ | <percentage>", "2px 8px"),
            ("<length>", "calc(7in - 12px)"),
            ("<integer>", "calc(3.1415)"),
            ("<color>#", "red, blue"),
            ("banan\\61", "banana"),
            ("--foo | <color>", "--foo"),
            ("<transform-list>", "scale(2) rotate(20deg)"),
            ("<url>", "url(a)"),
            ("<image>", "linear-gradient(yellow, blue)"),
            ("<string>+", "'foo' 'bar'"),
        ] {
            assert!(valid(syntax, initial), "{syntax}: {initial}");
        }
        for (syntax, initial) in [
            ("<length>", "10em"),
            ("<length>", "var(--moo)"),
            ("<length>", "10"),
            ("<length>|initial", "10px"),
            ("<\\6c ength>", "10px"),
            ("<length> +", "10px"),
            ("<transform-list>+", "scale(2)"),
            ("*", "inherit"),
            ("*", "revert-layer"),
            ("banana", "bAnAnA"),
            ("<integer>", "1.0"),
            ("<resolution>", "-5.3dpcm"),
            ("<image>", "none"),
            ("foo bar", "foo bar"),
        ] {
            assert!(!valid(syntax, initial), "{syntax}: {initial}");
        }
        // Required unless the syntax is `*`.
        assert!(register_property("--x", "*", true, None).is_ok());
        assert!(register_property("--x", "<length>", true, None).is_err());
        assert!(register_property("x", "*", true, None).is_err());
    }

    #[test]
    fn computed_values_interpolate_by_their_types() {
        let mix = |a: &str, b: &str| {
            interpolate(&css::tokenize(a), &css::tokenize(b), 0.25)
                .map(|tokens| crate::serialize::tokens(&tokens))
        };
        assert_eq!(mix("100px", "200px").as_deref(), Some("125px"));
        assert_eq!(mix("1", "2").as_deref(), Some("1"), "an integer rounded");
        assert_eq!(mix("1.5", "2.5").as_deref(), Some("1.75"));
        assert_eq!(mix("10px, 20%", "50px, 60%").as_deref(), Some("20px, 30%"));
        assert_eq!(mix("0px", "40%").as_deref(), Some("calc(10% + 0px)"));
        assert_eq!(
            mix("rgb(0, 0, 0)", "rgb(200, 100, 40)").as_deref(),
            Some("rgb(50, 25, 10)")
        );
        assert_eq!(
            mix("rotate(0deg)", "rotate(100deg)").as_deref(),
            Some("rotate(25deg)")
        );
        // What does not interpolate steps.
        assert_eq!(mix("1px 2px", "1px"), None);
        assert_eq!(mix("a", "b"), None);
        assert_eq!(mix("1px", "1s"), None);
    }

    #[test]
    fn an_at_property_rule_needs_its_descriptors() {
        let rule = |source: &str| {
            let sheet = css::parse_stylesheet(source).expect("parses");
            let css::Rule::At(at) = &sheet.rules[0] else {
                panic!("an at-rule");
            };
            let Some(css::Block::Declarations(list)) = &at.block else {
                panic!("declarations");
            };
            property_rule(&at.prelude, list)
        };
        let made =
            rule("@property --a { syntax: '<length>'; inherits: false; initial-value: 1in }")
                .expect("valid");
        assert_eq!(
            made.initial.as_deref(),
            Some(css::tokenize("96px").as_slice())
        );
        // A URL is written quoted.
        let url = register_property("--u", "<url>", false, Some("url(http://a/)")).expect("valid");
        assert_eq!(
            crate::serialize::tokens(url.initial.as_deref().unwrap_or_default()),
            "url(\"http://a/\")"
        );
        assert!(!made.inherits);
        assert!(rule("@property --a { syntax: '*'; inherits: true }").is_some());
        assert!(rule("@property --a { syntax: '<length>'; inherits: true }").is_none());
        assert!(rule("@property --a { inherits: true; initial-value: 1px }").is_none());
        assert!(rule("@property --a { syntax: '*' }").is_none());
        assert!(rule("@property a { syntax: '*'; inherits: true }").is_none());
    }
}
