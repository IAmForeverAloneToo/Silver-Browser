//! `@font-face` rules (CSS Fonts 4 section 4, gap batch D, D54): each
//! rule's family, its sources in order and the weights, slant and
//! widths its face covers. A rule without a family or a source is no
//! face. `unicode-range` is not read yet: the tokens keep a number's
//! value, not its text, which a range is read back from.

use css::Token;

use crate::computed::FontStyle;

/// The most sources a rule's `src` may list.
pub const MAX_FONT_SOURCES: usize = 16;

/// One `@font-face` rule's face.
#[derive(Debug, Clone, PartialEq)]
pub struct FontFaceRule {
    pub family: String,
    pub sources: Vec<FontSource>,
    /// The weights, the least and the most, 1 to 1000.
    pub weight: (u16, u16),
    pub style: FontStyle,
    /// The widths, the least and the most, as percentages.
    pub stretch: (f32, f32),
}

/// Where a face's data may be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontSource {
    /// A url, made absolute against its sheet's by the loader.
    Url(String),
    /// A face of the system's, by its name.
    Local(String),
}

/// The face `declarations` describe, an `@font-face` rule's; `None`
/// without a family or a source.
pub fn read(declarations: &[css::Declaration]) -> Option<FontFaceRule> {
    let mut family = None;
    let mut sources = None;
    let mut rule = FontFaceRule {
        family: String::new(),
        sources: Vec::new(),
        weight: (400, 400),
        style: FontStyle::Normal,
        stretch: (100.0, 100.0),
    };
    // The last valid value of each stands.
    for declaration in declarations {
        let value = trim(&declaration.value);
        match declaration.name.as_str() {
            "font-family" => family = read_family(value).or(family),
            "src" => sources = read_sources(value).or(sources),
            "font-weight" => rule.weight = read_range(value, weight).unwrap_or(rule.weight),
            "font-stretch" => rule.stretch = read_range(value, stretch).unwrap_or(rule.stretch),
            "font-style" => rule.style = read_style(value).unwrap_or(rule.style),
            _ => {}
        }
    }
    rule.family = family?;
    rule.sources = sources?;
    Some(rule)
}

fn trim(tokens: &[Token]) -> &[Token] {
    let start = tokens
        .iter()
        .position(|t| *t != Token::Whitespace)
        .unwrap_or(tokens.len());
    let end = tokens
        .iter()
        .rposition(|t| *t != Token::Whitespace)
        .map_or(start, |e| e + 1);
    &tokens[start..end]
}

/// The tokens of `tokens` other than whitespace.
fn words(tokens: &[Token]) -> Vec<&Token> {
    tokens.iter().filter(|t| **t != Token::Whitespace).collect()
}

/// One family: a string, or identifiers that are no generic family.
fn read_family(tokens: &[Token]) -> Option<String> {
    match words(tokens).as_slice() {
        [Token::String { value, bad: false }] if !value.is_empty() => Some(value.clone()),
        idents if !idents.is_empty() => {
            let names: Option<Vec<&str>> = idents
                .iter()
                .map(|t| match t {
                    Token::Ident(name) => Some(name.as_str()),
                    _ => None,
                })
                .collect();
            let names = names?;
            let generic = matches!(
                names.as_slice(),
                [one] if matches!(
                    one.to_ascii_lowercase().as_str(),
                    "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy" | "system-ui"
                        | "inherit" | "initial" | "unset" | "default" | "revert"
                )
            );
            (!generic).then(|| names.join(" "))
        }
        _ => None,
    }
}

/// `src`: sources separated by commas, each a url (with any `format()`
/// or `tech()` after it, which say what the file is) or `local()`. A
/// source this engine cannot read is left out; none left is invalid.
fn read_sources(tokens: &[Token]) -> Option<Vec<FontSource>> {
    let mut out = Vec::new();
    for source in tokens.split(|t| *t == Token::Comma) {
        let parts = words(source);
        let (source, rest) = match parts.as_slice() {
            [Token::Url { value, bad: false }, rest @ ..] => (FontSource::Url(value.clone()), rest),
            [
                Token::Function(name),
                Token::String { value, bad: false },
                Token::CloseParen,
                rest @ ..,
            ] if name.eq_ignore_ascii_case("url") => (FontSource::Url(value.clone()), rest),
            [Token::Function(name), inner @ .., Token::CloseParen]
                if name.eq_ignore_ascii_case("local") =>
            {
                let local = match inner {
                    [Token::String { value, bad: false }] => value.clone(),
                    idents => {
                        let names: Option<Vec<&str>> = idents
                            .iter()
                            .map(|t| match t {
                                Token::Ident(name) => Some(name.as_str()),
                                _ => None,
                            })
                            .collect();
                        names.filter(|n| !n.is_empty())?.join(" ")
                    }
                };
                (FontSource::Local(local), &[][..])
            }
            _ => return None,
        };
        if !readable(rest)? {
            continue;
        }
        if out.len() < MAX_FONT_SOURCES {
            out.push(source);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Whether a url's `format()` and `tech()` hints say a file this engine
/// reads: OpenType, TrueType or WOFF 1, no font technology past them.
/// `None` for hints that do not parse, which make the `src` invalid.
fn readable(hints: &[&Token]) -> Option<bool> {
    let mut ok = true;
    let mut rest = hints;
    while let [Token::Function(name), tail @ ..] = rest {
        let close = tail.iter().position(|t| **t == Token::CloseParen)?;
        let args = &tail[..close];
        rest = &tail[close + 1..];
        match name.to_ascii_lowercase().as_str() {
            "format" => {
                let format = match args {
                    [Token::String { value, .. }] | [Token::Ident(value)] => {
                        value.to_ascii_lowercase()
                    }
                    _ => return None,
                };
                ok &= matches!(
                    format.as_str(),
                    "opentype" | "truetype" | "woff" | "collection"
                );
            }
            "tech" => ok = false,
            _ => return None,
        }
    }
    rest.is_empty().then_some(ok)
}

/// One value, or two from least to most (a pair written the other way
/// is taken in order); `auto` is the property's initial value.
fn read_range<T: Copy + PartialOrd>(
    tokens: &[Token],
    one: fn(&Token) -> Option<T>,
) -> Option<(T, T)> {
    match words(tokens).as_slice() {
        [a] => one(a).map(|v| (v, v)),
        [a, b] => {
            let (a, b) = (one(a)?, one(b)?);
            Some(if a <= b { (a, b) } else { (b, a) })
        }
        _ => None,
    }
}

fn weight(token: &Token) -> Option<u16> {
    match token {
        Token::Ident(word) => match word.to_ascii_lowercase().as_str() {
            "normal" | "auto" => Some(400),
            "bold" => Some(700),
            _ => None,
        },
        Token::Number { value, .. } if (1.0..=1000.0).contains(value) => Some(value.round() as u16),
        _ => None,
    }
}

fn stretch(token: &Token) -> Option<f32> {
    match token {
        Token::Ident(word) => Some(match word.to_ascii_lowercase().as_str() {
            "ultra-condensed" => 50.0,
            "extra-condensed" => 62.5,
            "condensed" => 75.0,
            "semi-condensed" => 87.5,
            "normal" | "auto" => 100.0,
            "semi-expanded" => 112.5,
            "expanded" => 125.0,
            "extra-expanded" => 150.0,
            "ultra-expanded" => 200.0,
            _ => return None,
        }),
        Token::Percentage { value } if *value >= 0.0 => Some(*value as f32),
        _ => None,
    }
}

/// `font-style`: `normal`, `italic`, or `oblique` with up to two angles,
/// the face's slant taken as their middle.
fn read_style(tokens: &[Token]) -> Option<FontStyle> {
    let parts = words(tokens);
    let word = |t: &Token| match t {
        Token::Ident(w) => Some(w.to_ascii_lowercase()),
        _ => None,
    };
    let angle = |t: &Token| match t {
        Token::Dimension { value, unit, .. } => {
            let deg = match unit.to_ascii_lowercase().as_str() {
                "deg" => *value,
                "rad" => value.to_degrees(),
                "grad" => value * 0.9,
                "turn" => value * 360.0,
                _ => return None,
            } as f32;
            (-90.0..=90.0).contains(&deg).then_some(deg)
        }
        _ => None,
    };
    match parts.as_slice() {
        [one] => match word(one)?.as_str() {
            "normal" | "auto" => Some(FontStyle::Normal),
            "italic" => Some(FontStyle::Italic),
            "oblique" => Some(FontStyle::Oblique(14.0)),
            _ => None,
        },
        [first, a] if word(first)? == "oblique" => angle(a).map(FontStyle::Oblique),
        [first, a, b] if word(first)? == "oblique" => {
            Some(FontStyle::Oblique((angle(a)? + angle(b)?) / 2.0))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(css: &str) -> Option<FontFaceRule> {
        let sheet = css::parse_stylesheet(&format!("@font-face {{ {css} }}")).expect("parses");
        let css::Rule::At(at) = &sheet.rules[0] else {
            panic!("an at-rule");
        };
        let Some(css::Block::Declarations(declarations)) = &at.block else {
            panic!("declarations");
        };
        read(declarations)
    }

    #[test]
    fn a_rule_is_a_family_and_its_sources() {
        let f = face("font-family: \"My Font\"; src: local(\"My Font\"), url(a.woff) format(\"woff\"), url(\"b.ttf\")")
            .expect("a face");
        assert_eq!(f.family, "My Font");
        assert_eq!(
            f.sources,
            [
                FontSource::Local("My Font".into()),
                FontSource::Url("a.woff".into()),
                FontSource::Url("b.ttf".into())
            ]
        );
        assert_eq!(
            (f.weight, f.style, f.stretch),
            ((400, 400), FontStyle::Normal, (100.0, 100.0))
        );
        assert_eq!(
            face("font-family: My Font; src: url(a)").unwrap().family,
            "My Font"
        );
        assert!(face("src: url(a)").is_none(), "no family");
        assert!(face("font-family: a").is_none(), "no source");
        assert!(
            face("font-family: serif; src: url(a)").is_none(),
            "a generic family"
        );
        // A format or technology this engine does not read is passed
        // over; a hint that does not parse makes the list invalid.
        let f =
            face("font-family: a; src: url(a.woff2) format(woff2), url(b.otf) format(opentype)");
        assert_eq!(f.unwrap().sources, [FontSource::Url("b.otf".into())]);
        assert!(face("font-family: a; src: url(a) tech(color-COLRv1)").is_none());
        assert!(face("font-family: a; src: url(a) bogus(x)").is_none());
    }

    #[test]
    fn the_faces_in_force_are_those_whose_media_hold() {
        let sheet = css::parse_stylesheet(
            "@font-face { font-family: a; src: url(a) }              @media print { @font-face { font-family: b; src: url(b) } }              @media screen { @font-face { font-family: c; src: url(c) } }              @supports (display: grid) { @layer x { @font-face { font-family: d; src: url(d) } } }              @font-face { font-family: e }",
        )
        .expect("parses");
        let sheets = crate::Stylesheets::new(vec![sheet]);
        let families: Vec<String> = sheets
            .font_faces(&crate::media::Media::screen(800.0, 600.0))
            .into_iter()
            .map(|f| f.family)
            .collect();
        assert_eq!(families, ["a", "c", "d"]);
    }

    #[test]
    fn descriptors_take_ranges() {
        let f = face(
            "font-family: a; src: url(a); font-weight: 700 300; font-stretch: condensed 150%; \
             font-style: oblique 10deg 20deg",
        )
        .unwrap();
        assert_eq!(f.weight, (300, 700));
        assert_eq!(f.stretch, (75.0, 150.0));
        assert_eq!(f.style, FontStyle::Oblique(15.0));
        // An invalid value leaves the one before.
        let f = face("font-family: a; src: url(a); font-weight: bold; font-weight: 2000").unwrap();
        assert_eq!(f.weight, (700, 700));
        assert_eq!(
            face("font-family: a; src: url(a); font-style: italic")
                .unwrap()
                .style,
            FontStyle::Italic
        );
    }
}
