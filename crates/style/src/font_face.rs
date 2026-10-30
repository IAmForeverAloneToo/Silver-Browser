//! `@font-face` rules (CSS Fonts 4 section 4, gap batch D, D54): each
//! rule's family, its sources in order, the weights, slant and widths
//! its face covers, and the code points it is for (`unicode-range`). A
//! rule without a family or a source is no face. Each descriptor is
//! checked and written back as the reference browser does
//! ([`descriptor`]): what the rule's `style` gives and a `FontFace`'s
//! attributes say.

use css::Token;

use crate::computed::FontStyle;
use crate::font_family;

/// The most sources a rule's `src` may list.
pub const MAX_FONT_SOURCES: usize = 16;

/// The most ranges a `unicode-range` may list.
pub const MAX_UNICODE_RANGES: usize = 256;

/// One `@font-face` rule's face.
#[derive(Debug, Clone, PartialEq)]
pub struct FontFaceRule {
    pub family: String,
    /// The sources this engine can read, in order.
    pub sources: Vec<FontSource>,
    /// The weights, the least and the most, 1 to 1000.
    pub weight: (u16, u16),
    pub style: FontStyle,
    /// The widths, the least and the most, as percentages.
    pub stretch: (f32, f32),
    /// The code points the face is for, each range's first and last;
    /// none for all of them.
    pub ranges: Vec<(u32, u32)>,
    /// The descriptors as written back, a `FontFace`'s attributes.
    pub text: FaceText,
}

/// A face's descriptors as written back (CSS Font Loading 3's
/// `FontFace` attributes), each its initial value when not given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceText {
    pub style: String,
    pub weight: String,
    pub stretch: String,
    pub unicode_range: String,
    pub feature_settings: String,
    pub display: String,
}

impl Default for FaceText {
    fn default() -> Self {
        FaceText {
            style: "normal".into(),
            weight: "normal".into(),
            stretch: "normal".into(),
            unicode_range: "U+0-10FFFF".into(),
            feature_settings: "normal".into(),
            display: "auto".into(),
        }
    }
}

/// Where a face's data may be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontSource {
    /// A url, made absolute against its sheet's by the loader.
    Url(String),
    /// A face of the system's, by its full or PostScript name.
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
        ranges: Vec::new(),
        text: FaceText::default(),
    };
    // The last valid value of each stands.
    for declaration in declarations {
        let value = trim(&declaration.value);
        let Some((name, text)) = descriptor(&declaration.name, value) else {
            continue;
        };
        match name {
            "font-family" => family = font_family::parse_name(value),
            "src" => sources = Some(read_sources(value)),
            "font-weight" => {
                rule.weight = read_range(value, weight).unwrap_or((400, 400));
                rule.text.weight = text;
            }
            "font-stretch" => {
                rule.stretch = read_range(value, stretch).unwrap_or((100.0, 100.0));
                rule.text.stretch = text;
            }
            "font-style" => {
                rule.style = read_style(value).unwrap_or(FontStyle::Normal);
                rule.text.style = text;
            }
            "unicode-range" => {
                rule.ranges = ranges(value).unwrap_or_default();
                rule.text.unicode_range = text;
            }
            "font-feature-settings" => rule.text.feature_settings = text,
            "font-display" => rule.text.display = text,
            _ => {}
        }
    }
    rule.family = family?;
    rule.sources = sources?;
    // Every code point the ranges cover is all of them.
    if rule
        .ranges
        .iter()
        .any(|&(lo, hi)| lo == 0 && hi >= 0x10_FFFF)
    {
        rule.ranges.clear();
    }
    Some(rule)
}

/// `name`'s value `value` in an `@font-face` rule as the reference
/// browser writes it back, with the descriptor's own name (`font-width`
/// is `font-stretch`'s); `None` for a descriptor it does not have or a
/// value the descriptor does not take.
pub fn descriptor(name: &str, value: &[Token]) -> Option<(&'static str, String)> {
    let value = trim(value);
    let name = name.to_ascii_lowercase();
    let words = words(value);
    let keyword = |allowed: &[&'static str]| match words.as_slice() {
        [Token::Ident(word)] => allowed
            .iter()
            .find(|k| k.eq_ignore_ascii_case(word))
            .map(|k| (*k).to_owned()),
        _ => None,
    };
    Some(match name.as_str() {
        "font-family" => (
            "font-family",
            font_family::write_name(&font_family::parse_name(value).filter(|name| {
                !(words.len() == 1
                    && font_family::GENERIC_FAMILIES.contains(&name.to_ascii_lowercase().as_str())
                    && matches!(words[0], Token::Ident(_)))
            })?),
        ),
        "src" => ("src", write_sources(value)?),
        "font-style" => ("font-style", write_style(value)?),
        "font-weight" => (
            "font-weight",
            write_range(value, weight, &["normal", "bold"])?,
        ),
        "font-stretch" | "font-width" => (
            "font-stretch",
            write_range(value, stretch, &STRETCH_KEYWORDS)?,
        ),
        "unicode-range" => {
            ranges(value)?;
            ("unicode-range", crate::serialize::tokens(value))
        }
        "font-display" => (
            "font-display",
            keyword(&["auto", "block", "swap", "fallback", "optional"])?,
        ),
        "font-feature-settings" => ("font-feature-settings", write_features(value)?),
        "font-variation-settings" => ("font-variation-settings", write_variations(value)?),
        "font-language-override" => (
            "font-language-override",
            keyword(&["normal"]).or_else(|| string_of(&words))?,
        ),
        "font-named-instance" => (
            "font-named-instance",
            keyword(&["auto"]).or_else(|| string_of(&words))?,
        ),
        "ascent-override" => ("ascent-override", write_overrides(value)?),
        "descent-override" => ("descent-override", write_overrides(value)?),
        "line-gap-override" => ("line-gap-override", write_overrides(value)?),
        "size-adjust" => match words.as_slice() {
            [Token::Percentage { value }] if *value >= 0.0 => {
                ("size-adjust", format!("{}%", number(*value)))
            }
            _ => return None,
        },
        _ => return None,
    })
}

/// The `font-stretch` keywords and their percentages (section 2.3).
const STRETCH_KEYWORDS: [&str; 9] = [
    "ultra-condensed",
    "extra-condensed",
    "condensed",
    "semi-condensed",
    "normal",
    "semi-expanded",
    "expanded",
    "extra-expanded",
    "ultra-expanded",
];

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

/// A number as the reference browser writes it, its shortest form.
fn number(value: f64) -> String {
    let rounded = (value * 1e6).round() / 1e6;
    format!("{rounded}")
}

/// A lone string, written back.
fn string_of(words: &[&Token]) -> Option<String> {
    match words {
        [Token::String { value, bad: false }] => Some(font_family::string(value)),
        _ => None,
    }
}

/// The ranges `unicode-range`'s tokens hold, as the parser read them
/// from its text (`css::unicode_range`); `None` for any that is not.
fn ranges(tokens: &[Token]) -> Option<Vec<(u32, u32)>> {
    let mut out = Vec::new();
    for part in tokens.split(|t| *t == Token::Comma) {
        match words(part).as_slice() {
            [Token::Ident(text)] => out.push(css::unicode_range::read(text)?),
            _ => return None,
        }
    }
    (!out.is_empty() && out.len() <= MAX_UNICODE_RANGES).then_some(out)
}

/// One `src` component, as the grammar reads it (section 4.3).
#[derive(Debug, Clone, PartialEq)]
enum Source {
    Url {
        url: String,
        /// `format()`'s argument, lowercase if a keyword.
        format: Option<String>,
        tech: Vec<String>,
    },
    Local(String),
}

/// The formats a `format()` keyword may name.
const FORMATS: [&str; 7] = [
    "collection",
    "embedded-opentype",
    "opentype",
    "svg",
    "truetype",
    "woff",
    "woff2",
];

/// The technologies `tech()` may name.
const TECHS: [&str; 11] = [
    "features-opentype",
    "features-aat",
    "features-graphite",
    "color-colrv0",
    "color-colrv1",
    "color-svg",
    "color-sbix",
    "color-cbdt",
    "variations",
    "palettes",
    "incremental",
];

/// `src`'s components in order, each `None` where it does not parse.
fn components(tokens: &[Token]) -> Vec<Option<Source>> {
    let mut depth = 0usize;
    tokens
        .split(|t| {
            match t {
                Token::Function(_) | Token::OpenParen => depth += 1,
                Token::CloseParen => depth = depth.saturating_sub(1),
                _ => {}
            }
            depth == 0 && *t == Token::Comma
        })
        .map(|part| component(&words(part)))
        .collect()
}

/// One component: a url with an optional `format()` then `tech()`, or
/// `local()` with a family name.
fn component(parts: &[&Token]) -> Option<Source> {
    let (url, mut rest) = match parts {
        [Token::Url { value, bad: false }, rest @ ..] => (value.clone(), rest),
        [
            Token::Function(name),
            Token::String { value, bad: false },
            Token::CloseParen,
            rest @ ..,
        ] if name.eq_ignore_ascii_case("url") => (value.clone(), rest),
        [Token::Function(name), inner @ .., Token::CloseParen]
            if name.eq_ignore_ascii_case("local") =>
        {
            let inner: Vec<Token> = inner.iter().map(|t| (*t).clone()).collect();
            return font_family::parse_name(&inner).map(Source::Local);
        }
        _ => return None,
    };
    let mut format = None;
    if let [Token::Function(name), tail @ ..] = rest
        && name.eq_ignore_ascii_case("format")
    {
        format = Some(match tail {
            [Token::String { value, bad: false }, Token::CloseParen, ..] => value.clone(),
            [Token::Ident(word), Token::CloseParen, ..]
                if FORMATS.contains(&word.to_ascii_lowercase().as_str()) =>
            {
                word.to_ascii_lowercase()
            }
            _ => return None,
        });
        rest = &tail[2..];
    }
    let mut tech = Vec::new();
    if let [Token::Function(name), tail @ ..] = rest
        && name.eq_ignore_ascii_case("tech")
    {
        let close = tail.iter().position(|t| **t == Token::CloseParen)?;
        for item in tail[..close].split(|t| **t == Token::Comma) {
            match item {
                [Token::Ident(word)] if TECHS.contains(&word.to_ascii_lowercase().as_str()) => {
                    tech.push(word.to_ascii_lowercase())
                }
                _ => return None,
            }
        }
        rest = &tail[close + 1..];
    }
    rest.is_empty().then_some(Source::Url { url, format, tech })
}

/// Whether the reference browser takes a component: a url's format and
/// technologies all ones it reads, the older `*-variations` formats
/// among them.
fn supported(source: &Source) -> bool {
    match source {
        Source::Local(_) => true,
        Source::Url { format, tech, .. } => {
            format.as_deref().is_none_or(|f| {
                matches!(
                    f.to_ascii_lowercase().as_str(),
                    "collection"
                        | "opentype"
                        | "truetype"
                        | "woff"
                        | "woff2"
                        | "opentype-variations"
                        | "truetype-variations"
                        | "woff-variations"
                        | "woff2-variations"
                )
            }) && tech.iter().all(|t| {
                matches!(
                    t.as_str(),
                    "features-opentype"
                        | "features-aat"
                        | "color-colrv0"
                        | "color-colrv1"
                        | "color-sbix"
                        | "color-cbdt"
                        | "variations"
                        | "palettes"
                )
            })
        }
    }
}

/// Whether this engine reads a component's file as its hints say it is:
/// OpenType, TrueType or WOFF 1, no technology past OpenType's
/// features.
fn readable(source: &Source) -> bool {
    match source {
        Source::Local(_) => true,
        Source::Url { format, tech, .. } => {
            format.as_deref().is_none_or(|f| {
                matches!(
                    f.to_ascii_lowercase().as_str(),
                    "collection"
                        | "opentype"
                        | "truetype"
                        | "woff"
                        | "opentype-variations"
                        | "truetype-variations"
                        | "woff-variations"
                )
            }) && tech
                .iter()
                .all(|t| matches!(t.as_str(), "features-opentype" | "features-aat"))
        }
    }
}

/// `src` written back: the components the reference browser takes,
/// parted by `, `; `None` for none.
fn write_sources(tokens: &[Token]) -> Option<String> {
    let parts: Vec<String> = components(tokens)
        .into_iter()
        .flatten()
        .filter(supported)
        .map(|source| match source {
            Source::Local(name) => format!("local({})", font_family::string(&name)),
            Source::Url { url, format, tech } => {
                let mut out = format!("url({})", font_family::string(&url));
                if let Some(format) = format {
                    out.push_str(&format!(" format({})", font_family::string(&format)));
                }
                if !tech.is_empty() {
                    out.push_str(&format!(" tech({})", tech.join(", ")));
                }
                out
            }
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// The sources of `src`'s value `tokens` this engine can read, in
/// order (a `FontFace`'s source); `None` when it is no `src`.
pub fn sources(tokens: &[Token]) -> Option<Vec<FontSource>> {
    let tokens = trim(tokens);
    write_sources(tokens)?;
    Some(read_sources(tokens))
}

/// The sources of a valid `src` this engine can read, at most
/// [`MAX_FONT_SOURCES`]: those its hints say it reads, then, should
/// they all fail, those whose hints say a file it does not read (WOFF2,
/// colour fonts), which may yet be one it does.
fn read_sources(tokens: &[Token]) -> Vec<FontSource> {
    let taken: Vec<Source> = components(tokens)
        .into_iter()
        .flatten()
        .filter(supported)
        .collect();
    let (first, last): (Vec<Source>, Vec<Source>) = taken.into_iter().partition(readable);
    first
        .into_iter()
        .chain(last)
        .take(MAX_FONT_SOURCES)
        .map(|source| match source {
            Source::Local(name) => FontSource::Local(name),
            Source::Url { url, .. } => FontSource::Url(url),
        })
        .collect()
}

/// The component values of `tokens`: a token each, but a function
/// with all it holds, white space left out.
fn parts(tokens: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == Token::Whitespace {
            i += 1;
            continue;
        }
        let start = i;
        if matches!(tokens[i], Token::Function(_) | Token::OpenParen) {
            let mut depth = 0usize;
            while i < tokens.len() {
                match tokens[i] {
                    Token::Function(_) | Token::OpenParen => depth += 1,
                    Token::CloseParen => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        i += 1;
        out.push(&tokens[start..i.min(tokens.len())]);
    }
    out
}

/// One value, or two from least to most (a pair written the other way
/// is taken in order); `auto` is the property's initial value.
fn read_range<T: Copy + PartialOrd>(
    tokens: &[Token],
    one: fn(&[Token]) -> Option<T>,
) -> Option<(T, T)> {
    match parts(tokens).as_slice() {
        [a] => one(a).map(|v| (v, v)),
        [a, b] => {
            let (a, b) = (one(a)?, one(b)?);
            Some(if a <= b { (a, b) } else { (b, a) })
        }
        _ => None,
    }
}

/// A range descriptor written back as given: `auto`, or one or two
/// values, each a keyword of `keywords`, a number or percentage, or a
/// calculation of one.
fn write_range<T>(
    tokens: &[Token],
    one: fn(&[Token]) -> Option<T>,
    keywords: &[&str],
) -> Option<String> {
    let parts = parts(tokens);
    if let [[Token::Ident(word)]] = parts.as_slice()
        && word.eq_ignore_ascii_case("auto")
    {
        return Some("auto".into());
    }
    if parts.is_empty() || parts.len() > 2 {
        return None;
    }
    let written: Option<Vec<String>> = parts
        .iter()
        .map(|part| {
            one(part)?;
            Some(match part {
                [Token::Ident(word)] => keywords
                    .iter()
                    .find(|k| k.eq_ignore_ascii_case(word))?
                    .to_string(),
                [Token::Number { value, .. }] => number(*value),
                [Token::Percentage { value }] => format!("{}%", number(*value)),
                calculated => crate::serialize::tokens(calculated),
            })
        })
        .collect();
    Some(written?.join(" "))
}

fn weight(part: &[Token]) -> Option<u16> {
    match part {
        [Token::Ident(word)] => match word.to_ascii_lowercase().as_str() {
            "normal" | "auto" => Some(400),
            "bold" => Some(700),
            _ => None,
        },
        [Token::Number { value, .. }] => {
            (1.0..=1000.0).contains(value).then(|| value.round() as u16)
        }
        // A calculation is clamped to the range.
        [Token::Function(_), ..] => Some(
            crate::cascade::plain_number(part, false)?
                .clamp(1.0, 1000.0)
                .round() as u16,
        ),
        _ => None,
    }
}

fn stretch(part: &[Token]) -> Option<f32> {
    match part {
        [Token::Ident(word)] => Some(match word.to_ascii_lowercase().as_str() {
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
        [Token::Percentage { value }] => (*value >= 0.0).then_some(*value as f32),
        [Token::Function(_), ..] => Some(crate::cascade::plain_number(part, true)?.max(0.0) as f32),
        _ => None,
    }
}

/// An angle of `font-style`'s, in degrees, -90 to 90; a calculation is
/// clamped to them.
fn angle(part: &[Token]) -> Option<f32> {
    match part {
        [Token::Dimension { .. }] => {
            crate::cascade::plain_angle(part).filter(|a| (-90.0..=90.0).contains(a))
        }
        [Token::Function(_), ..] => Some(crate::cascade::plain_angle(part)?.clamp(-90.0, 90.0)),
        _ => None,
    }
}

/// `font-style`: `auto`, `normal`, `italic`, or `oblique` with up to two
/// angles, the face's slant taken as their middle.
fn read_style(tokens: &[Token]) -> Option<FontStyle> {
    let parts = parts(tokens);
    let word = |t: &[Token]| match t {
        [Token::Ident(w)] => Some(w.to_ascii_lowercase()),
        _ => None,
    };
    match parts.as_slice() {
        [one] => match word(one)?.as_str() {
            "normal" | "auto" => Some(FontStyle::Normal),
            "italic" => Some(FontStyle::Italic),
            "oblique" => Some(FontStyle::Oblique(14.0)),
            _ => None,
        },
        [first, a] if word(first)? == "oblique" => angle(a).map(|a| {
            if a == 0.0 {
                FontStyle::Normal
            } else {
                FontStyle::Oblique(a)
            }
        }),
        [first, a, b] if word(first)? == "oblique" => {
            Some(FontStyle::Oblique((angle(a)? + angle(b)?) / 2.0))
        }
        _ => None,
    }
}

/// `font-style` written back: an angle in degrees, `oblique 0deg` as
/// `normal`.
fn write_style(tokens: &[Token]) -> Option<String> {
    read_style(tokens)?;
    let parts = parts(tokens);
    let deg = |t: &[Token]| angle(t).map(|a| format!("{}deg", number(f64::from(a))));
    Some(match parts.as_slice() {
        [[Token::Ident(word)]] => word.to_ascii_lowercase(),
        [_, a] => match angle(a)? {
            0.0 => "normal".into(),
            _ => format!("oblique {}", deg(a)?),
        },
        [_, a, b] => format!("oblique {} {}", deg(a)?, deg(b)?),
        _ => return None,
    })
}

/// `font-feature-settings` written back: `normal`, or each feature's
/// tag and its value but 1.
fn write_features(tokens: &[Token]) -> Option<String> {
    let parts = words(tokens);
    if let [Token::Ident(word)] = parts.as_slice() {
        return word.eq_ignore_ascii_case("normal").then(|| "normal".into());
    }
    let mut out = Vec::new();
    for item in tokens.split(|t| *t == Token::Comma) {
        let item = words(item);
        let tag = match item.first() {
            Some(Token::String { value, bad: false })
                if value.len() == 4 && value.bytes().all(|b| (0x20..=0x7e).contains(&b)) =>
            {
                font_family::string(value)
            }
            _ => return None,
        };
        let value = match &item[1..] {
            [] => 1.0,
            [Token::Ident(word)] if word.eq_ignore_ascii_case("on") => 1.0,
            [Token::Ident(word)] if word.eq_ignore_ascii_case("off") => 0.0,
            [
                Token::Number {
                    value,
                    is_integer: true,
                },
            ] if *value >= 0.0 => *value,
            _ => return None,
        };
        out.push(if value == 1.0 {
            tag
        } else {
            format!("{tag} {}", number(value))
        });
    }
    Some(out.join(", "))
}

/// `font-variation-settings` written back: `normal`, or each axis's tag
/// and value.
fn write_variations(tokens: &[Token]) -> Option<String> {
    let parts = words(tokens);
    if let [Token::Ident(word)] = parts.as_slice() {
        return word.eq_ignore_ascii_case("normal").then(|| "normal".into());
    }
    let mut out = Vec::new();
    for item in tokens.split(|t| *t == Token::Comma) {
        match words(item).as_slice() {
            [
                Token::String {
                    value: tag,
                    bad: false,
                },
                Token::Number { value, .. },
            ] if tag.len() == 4 => {
                out.push(format!("{} {}", font_family::string(tag), number(*value)))
            }
            _ => return None,
        }
    }
    Some(out.join(", "))
}

/// A metric override written back: `normal`, or one or two percentages
/// not below 0.
fn write_overrides(tokens: &[Token]) -> Option<String> {
    let parts = words(tokens);
    if let [Token::Ident(word)] = parts.as_slice() {
        return word.eq_ignore_ascii_case("normal").then(|| "normal".into());
    }
    if parts.is_empty() || parts.len() > 2 {
        return None;
    }
    let written: Option<Vec<String>> = parts
        .iter()
        .map(|t| match t {
            Token::Percentage { value } if *value >= 0.0 => Some(format!("{}%", number(*value))),
            _ => None,
        })
        .collect();
    Some(written?.join(" "))
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

    /// `name: value` written back in a rule, or `None`.
    fn written(name: &str, value: &str) -> Option<String> {
        let sheet = css::parse_stylesheet(&format!("@font-face {{ {name}: {value} }}")).unwrap();
        let css::Rule::At(at) = &sheet.rules[0] else {
            panic!("an at-rule");
        };
        let Some(css::Block::Declarations(list)) = &at.block else {
            panic!("declarations");
        };
        let declaration = list.first()?;
        descriptor(&declaration.name, &declaration.value).map(|(_, text)| text)
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
        assert_eq!(f.text, FaceText::default());
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
        assert_eq!(
            face("font-family: \"serif\"; src: url(a)").unwrap().family,
            "serif"
        );
        // A format or technology this engine does not read is tried
        // last; a component that does not parse is passed over.
        let f =
            face("font-family: a; src: url(a.woff2) format(woff2), url(b.otf) format(opentype)");
        assert_eq!(
            f.unwrap().sources,
            [
                FontSource::Url("b.otf".into()),
                FontSource::Url("a.woff2".into())
            ]
        );
        assert_eq!(
            face("font-family: a; src: url(a) tech(color-COLRv1)")
                .unwrap()
                .sources,
            [FontSource::Url("a".into())]
        );
        assert!(face("font-family: a; src: url(a) bogus(x)").is_none());
        assert_eq!(
            face("font-family: a; src: url(a) bogus(x), local(b)")
                .unwrap()
                .sources,
            [FontSource::Local("b".into())]
        );
    }

    #[test]
    fn sources_parse_by_component() {
        let valid = |src: &str| written("src", src).is_some();
        for good in [
            "url(\"foo.ttf\")",
            "url(\"foo.ttf\") format(\"opentype\")",
            "url(\"foo.ttf\") format(woff2)",
            "url(\"foo.ttf\") format(collection)",
            "url(\"foo.ttf\") format(xyzz 200px), url(\"bar.html\")",
            "url(\"foo.ttf\") format(none), url(\"bar.html\")",
            "url(\"foo.ttf\") format(embedded-opentype), url(\"bar.html\")",
            "url(\"foo.ttf\") tech(color-COLRv1)",
            "url(\"foo.ttf\") format(opentype) tech(features-opentype)",
            "url(\"foo.ttf\") tech(features-opentype, color-COLRv0, variations, palettes)",
            "url(\"foo.ttf\") tech(color-SVG), url(\"bar.html\")",
            "local(  A  )",
            "local(inherit A)",
            "local(\"inherit\")",
            "local(), url(foo.ttf)",
            "local(12px monospace), url(foo.ttf)",
            "/* empty */, url(foo.ttf)",
            "url(not a valid url/bar.ttf), url(foo.ttf)",
            "local(\"\"), url(foo.ttf), unparseable-garbage, local(\"another font name\")",
        ] {
            assert!(valid(good), "{good}");
        }
        for bad in [
            "url(\"foo.ttf\") format()",
            "url(\"foo.ttf\") dummy()",
            "url(\"foo.ttf\") format(\"woff\") dummy()",
            "url(\"foo.ttf\") format(\"opentype\", \"truetype\")",
            "url(\"foo.ttf\") format(xyzzy)",
            "url(\"foo.ttf\") format(embedded-opentype)",
            "url(\"foo.ttf\") tech()",
            "url(\"foo.ttf\") tech(\"variations\")",
            "url(\"foo.ttf\") tech(features-opentype) format(opentype)",
            "url(\"foo.ttf\") tech(color-SVG)",
            "local(A) dummy()",
            "local(inherit)",
            "local(default)",
            "local(), local(initial)",
            "local(\"textfont\") format(opentype), local(\"emoji\") tech(color-COLRv0)",
        ] {
            assert!(!valid(bad), "{bad}");
        }
        assert_eq!(
            written(
                "src",
                "local(two idents), url(a) format(woff), url(b) tech(color-COLRv0)"
            )
            .as_deref(),
            Some(
                "local(\"two idents\"), url(\"a\") format(\"woff\"), url(\"b\") tech(color-colrv0)"
            )
        );
        assert_eq!(
            written("src", "url(\"N\\\"O\"), local(\"Z\\A A\")").as_deref(),
            Some("url(\"N\\\"O\"), local(\"Z\\a A\")")
        );
    }

    #[test]
    fn descriptors_are_written_back_as_given() {
        assert_eq!(
            written("font-weight", "900 100").as_deref(),
            Some("900 100")
        );
        assert_eq!(written("font-weight", "BOLD").as_deref(), Some("bold"));
        assert_eq!(written("font-weight", "auto").as_deref(), Some("auto"));
        assert_eq!(written("font-weight", "0"), None);
        assert_eq!(
            written("font-width", "75% 125%").as_deref(),
            Some("75% 125%")
        );
        assert_eq!(written("font-stretch", "auto").as_deref(), Some("auto"));
        assert_eq!(written("font-stretch", "125"), None);
        assert_eq!(written("font-stretch", "normal, expanded"), None);
        assert_eq!(
            written("font-style", "oblique 90deg -90deg").as_deref(),
            Some("oblique 90deg -90deg")
        );
        assert_eq!(
            written("font-style", "oblique 0deg").as_deref(),
            Some("normal")
        );
        assert_eq!(written("font-style", "normal 10deg"), None);
        // A calculation is clamped to the range.
        assert_eq!(
            written("font-weight", "calc(100.5) 300").as_deref(),
            Some("calc(100.5) 300")
        );
        assert!(written("font-weight", "calc(1001)").is_some());
        assert!(written("font-stretch", "calc(-100%)").is_some());
        assert_eq!(
            written("font-style", "oblique calc(91deg)").as_deref(),
            Some("oblique 90deg")
        );
        let f = face("font-family: a; src: url(a); font-weight: calc(1001) calc(50 * 2)").unwrap();
        assert_eq!(f.weight, (100, 1000));
        assert_eq!(written("ascent-override", "110%").as_deref(), Some("110%"));
        assert_eq!(
            written("line-gap-override", "100000000000%").as_deref(),
            Some("100000000000%")
        );
        assert_eq!(written("descent-override", "-1%"), None);
        assert_eq!(written("descent-override", "10px"), None);
        assert_eq!(written("size-adjust", "0%").as_deref(), Some("0%"));
        assert_eq!(written("size-adjust", "-1%"), None);
        assert_eq!(
            written("unicode-range", "u+0-7f, U+00e9").as_deref(),
            Some("U+0-7F, U+E9")
        );
        assert_eq!(written("unicode-range", "U+7F-0"), None);
        assert_eq!(
            written("font-feature-settings", "\"liga\" on, \"kern\" 0").as_deref(),
            Some("\"liga\", \"kern\" 0")
        );
        assert_eq!(written("font-display", "SWAP").as_deref(), Some("swap"));
        assert_eq!(written("color", "red"), None);
        let both =
            face("font-family: a; src: url(a); font-stretch: condensed; font-width: expanded")
                .unwrap();
        assert_eq!(both.text.stretch, "expanded");
    }

    #[test]
    fn ranges_say_which_code_points_a_face_is_for() {
        let f = face("font-family: a; src: url(a); unicode-range: U+0-7F, U+4??").unwrap();
        assert_eq!(f.ranges, [(0, 0x7F), (0x400, 0x4FF)]);
        assert_eq!(
            f.text.unicode_range,
            "U+0-7F, U+4??".replace("4??", "400-4FF")
        );
        let all = face("font-family: a; src: url(a); unicode-range: U+0-10FFFF").unwrap();
        assert!(all.ranges.is_empty());
        let bad = face("font-family: a; src: url(a); unicode-range: U+0-7F, x").unwrap();
        assert!(bad.ranges.is_empty());
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
        assert_eq!(
            (f.text.weight.as_str(), f.text.stretch.as_str()),
            ("700 300", "condensed 150%")
        );
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
