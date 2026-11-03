//! `font-palette` and the descriptors of `@font-palette-values` (CSS
//! Fonts 4 sections 9.1 and 9.2), as the reference browser takes them
//! (Chrome 156, probed): `palette-mix()` names its interpolation method
//! and has two palettes, each with an optional percentage; a percentage
//! one of them lacks is written as what the other leaves of 100%, and
//! two of 50% not at all; a mix of a palette with itself computes to it.
//! An interpolation of two palettes is their mix in Oklab. Text is drawn
//! from no colour font's palettes here, so the property changes nothing
//! drawn.

use std::sync::Arc;

use css::{Declaration, Token};

use super::{LengthBases, Want, math};
use crate::calc;
use crate::color::{Hue, Method, Space, number, parse_method};
use crate::computed::{FontPalette, PaletteMix};

/// The deepest one `palette-mix()` nests in another: a deeper one is
/// invalid, as a hostile sheet could otherwise nest them past the stack.
const MAX_DEPTH: usize = 16;

/// A palette as written: its keyword or name, or a mix of two, each
/// percentage written or calculated.
enum Written<'t> {
    Palette(FontPalette),
    Mix(String, [(Box<Written<'t>>, Option<Percent<'t>>); 2]),
}

/// A percentage of `palette-mix()` as written.
#[derive(Clone, Copy)]
enum Percent<'t> {
    Given(f32),
    Math(&'t [Token]),
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

/// `tokens` split at their top-level commas.
fn split_commas(tokens: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Function(_) | Token::OpenParen => depth += 1,
            Token::CloseParen => depth = depth.saturating_sub(1),
            Token::Comma if depth == 0 => {
                out.push(&tokens[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&tokens[start..]);
    out
}

/// A palette keyword or `<palette-identifier>`, one token.
fn keyword(token: &Token) -> Option<FontPalette> {
    let Token::Ident(word) = token else {
        return None;
    };
    if word.starts_with("--") {
        return Some(FontPalette::Named(Arc::from(word.as_str())));
    }
    match word.to_ascii_lowercase().as_str() {
        "normal" => Some(FontPalette::Normal),
        "light" => Some(FontPalette::Light),
        "dark" => Some(FontPalette::Dark),
        _ => None,
    }
}

/// The interpolation method `palette-mix()` writes: `in` and its space,
/// `xyz` as `xyz-d65`, and a polar space's hue method but `shorter`.
fn method_text(method: Method) -> String {
    let space = match method.space {
        Space::Lab => "lab",
        Space::Lch => "lch",
        Space::Oklab => "oklab",
        Space::Oklch => "oklch",
        Space::Hsl => "hsl",
        Space::Hwb => "hwb",
        other => other.predefined_name().unwrap_or("srgb"),
    };
    let hue = match method.hue {
        Hue::Shorter => "",
        Hue::Longer => " longer hue",
        Hue::Increasing => " increasing hue",
        Hue::Decreasing => " decreasing hue",
    };
    format!("in {space}{hue}")
}

/// `tokens` as a palette written, `depth` mixes deep.
fn parse(tokens: &[Token], depth: usize) -> Option<Written<'_>> {
    match trim(tokens) {
        [one] => keyword(one).map(Written::Palette),
        [Token::Function(name), inside @ .., Token::CloseParen]
            if name.eq_ignore_ascii_case("palette-mix") && depth < MAX_DEPTH =>
        {
            let groups = split_commas(inside);
            let [method, a, b] = groups.as_slice() else {
                return None;
            };
            let words: Vec<&Token> = method
                .iter()
                .filter(|t| !matches!(t, Token::Whitespace))
                .collect();
            let method = method_text(parse_method(&words)?);
            Some(Written::Mix(method, [part(a, depth)?, part(b, depth)?]))
        }
        _ => None,
    }
}

/// One of `palette-mix()`'s palettes, with its percentage before or
/// after it: a written one from 0% to 100%, or a calculation of one.
fn part(tokens: &[Token], depth: usize) -> Option<(Box<Written<'_>>, Option<Percent<'_>>)> {
    fn percent(piece: &[Token]) -> Option<Percent<'_>> {
        match piece {
            [Token::Percentage { value }] => {
                let value = *value as f32;
                (0.0..=100.0)
                    .contains(&value)
                    .then_some(Percent::Given(value))
            }
            _ if calc::is_math_function(piece) => {
                calc::parse(piece, Want::Percentage).map(|_| Percent::Math(piece))
            }
            _ => None,
        }
    }
    let pieces = super::components(tokens);
    match pieces.as_slice() {
        [palette] => Some((Box::new(parse(palette, depth + 1)?), None)),
        [a, b] => {
            if let Some(p) = percent(a) {
                Some((Box::new(parse(b, depth + 1)?), Some(p)))
            } else {
                Some((Box::new(parse(a, depth + 1)?), Some(percent(b)?)))
            }
        }
        _ => None,
    }
}

/// The two percentages a mix writes, from those it has: none for two of
/// 50%, or one of 50% alone; the other's rest of 100% for one written
/// alone; a calculation alone as it is.
fn written_percents(percents: [Option<Percent<'_>>; 2]) -> [Option<String>; 2] {
    let text = |p: Percent<'_>| match p {
        Percent::Given(v) => format!("{}%", number(v)),
        Percent::Math(tokens) => crate::serialize::specified_of("width", tokens),
    };
    match percents {
        [None, None] => [None, None],
        [Some(Percent::Given(v)), None] | [None, Some(Percent::Given(v))] if v == 50.0 => {
            [None, None]
        }
        [Some(Percent::Given(v)), None] => [
            Some(text(Percent::Given(v))),
            Some(text(Percent::Given(100.0 - v))),
        ],
        [None, Some(Percent::Given(v))] => [
            Some(text(Percent::Given(100.0 - v))),
            Some(text(Percent::Given(v))),
        ],
        [Some(Percent::Given(a)), Some(Percent::Given(b))] if a == 50.0 && b == 50.0 => {
            [None, None]
        }
        [a, b] => [a.map(text), b.map(text)],
    }
}

/// `palette-mix()` written: its method, then each palette's text with
/// its percentage after it.
fn mix_text(method: &str, parts: [(String, Option<String>); 2]) -> String {
    let part = |(palette, percent): (String, Option<String>)| match percent {
        Some(percent) => format!("{palette} {percent}"),
        None => palette,
    };
    let [a, b] = parts;
    format!("palette-mix({method}, {}, {})", part(a), part(b))
}

/// A palette written as CSSOM serializes the value declared: keywords
/// in lowercase, names as written, a mix's calculations kept.
fn written_text(palette: &Written<'_>) -> String {
    match palette {
        Written::Palette(palette) => font_palette_text(palette),
        Written::Mix(method, [(a, pa), (b, pb)]) => {
            let [ta, tb] = written_percents([*pa, *pb]);
            mix_text(method, [(written_text(a), ta), (written_text(b), tb)])
        }
    }
}

/// A palette computed: its mixes' calculations resolved against
/// `bases`, a mix of a palette with itself that palette.
fn computed(palette: &Written<'_>, bases: &LengthBases<'_>) -> Option<FontPalette> {
    match palette {
        Written::Palette(palette) => Some(palette.clone()),
        Written::Mix(method, [(a, pa), (b, pb)]) => {
            let resolve = |p: &Option<Percent<'_>>| -> Option<Option<f32>> {
                match p {
                    None => Some(None),
                    Some(Percent::Given(v)) => Some(Some(*v)),
                    Some(Percent::Math(tokens)) => {
                        match math(tokens, Want::Percentage, bases, None) {
                            Some(calc::Computed::Percent(p)) if p.is_finite() => {
                                Some(Some(p.clamp(0.0, 100.0) as f32))
                            }
                            _ => None,
                        }
                    }
                }
            };
            let (a, b) = (computed(a, bases)?, computed(b, bases)?);
            if a == b {
                return Some(a);
            }
            Some(FontPalette::Mix(Arc::new(PaletteMix {
                method: Arc::from(method.as_str()),
                parts: [(a, resolve(pa)?), (b, resolve(pb)?)],
            })))
        }
    }
}

/// `font-palette`'s value: `normal`, `light`, `dark`, a
/// `<palette-identifier>`, or `palette-mix()`; `None` for anything else.
pub(super) fn parse_font_palette(tokens: &[Token], bases: &LengthBases<'_>) -> Option<FontPalette> {
    computed(&parse(tokens, 0)?, bases)
}

/// `font-palette`'s declared value as CSSOM serializes it, as the
/// reference browser writes it; `None` when it is no value of the
/// property.
pub fn font_palette_specified(tokens: &[Token]) -> Option<String> {
    parse(tokens, 0).map(|palette| written_text(&palette))
}

/// `font-palette`'s computed value as `getComputedStyle` writes it.
pub fn font_palette_text(palette: &FontPalette) -> String {
    match palette {
        FontPalette::Normal => "normal".to_owned(),
        FontPalette::Light => "light".to_owned(),
        FontPalette::Dark => "dark".to_owned(),
        FontPalette::Named(name) => name.to_string(),
        FontPalette::Mix(mix) => {
            let [(a, pa), (b, pb)] = &mix.parts;
            let [ta, tb] = written_percents([pa.map(Percent::Given), pb.map(Percent::Given)]);
            mix_text(
                &mix.method,
                [(font_palette_text(a), ta), (font_palette_text(b), tb)],
            )
        }
    }
}

/// `font-palette` `p` of the way from `a` to `b` (CSS Fonts 4 section
/// 9.2: by computed value): `a` up to the start, `b` from the end, and
/// between them their mix in Oklab, each by its share, as the reference
/// browser mixes them; one palette with itself is that palette.
pub fn interpolate_font_palette(a: &FontPalette, b: &FontPalette, p: f64) -> FontPalette {
    if p <= 0.0 || a == b {
        return a.clone();
    }
    if p >= 1.0 {
        return b.clone();
    }
    FontPalette::Mix(Arc::new(PaletteMix {
        method: Arc::from("in oklab"),
        parts: [
            (a.clone(), Some(((1.0 - p) * 100.0) as f32)),
            (b.clone(), Some((p * 100.0) as f32)),
        ],
    }))
}

/// What an `@font-palette-values` rule's block declares, as the
/// reference browser keeps it: its `font-family`, `base-palette` and
/// `override-colors` descriptors, each the last valid one written,
/// `None` where there is none.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaletteValues {
    pub font_family: Option<String>,
    pub base_palette: Option<String>,
    pub override_colors: Option<String>,
}

impl PaletteValues {
    /// The descriptors of `list`.
    pub fn of(list: &[Declaration]) -> PaletteValues {
        let mut values = PaletteValues::default();
        for declaration in list.iter().filter(|d| !d.important) {
            let value = &declaration.value;
            match declaration.name.to_ascii_lowercase().as_str() {
                "font-family" => {
                    if let Some(family) = palette_family(value) {
                        values.font_family = Some(family);
                    }
                }
                "base-palette" => {
                    if let Some(base) = base_palette(value) {
                        values.base_palette = Some(base);
                    }
                }
                "override-colors" => {
                    if let Some(colors) = override_colors(value) {
                        values.override_colors = Some(colors);
                    }
                }
                _ => {}
            }
        }
        values
    }

    /// The block's text: each descriptor it has, in that order.
    pub fn text(&self) -> String {
        [
            ("font-family", &self.font_family),
            ("base-palette", &self.base_palette),
            ("override-colors", &self.override_colors),
        ]
        .iter()
        .filter_map(|(name, value)| value.as_ref().map(|v| format!("{name}: {v};")))
        .collect::<Vec<_>>()
        .join(" ")
    }
}

/// Whether `prelude` is an `@font-palette-values` rule's: one
/// `<dashed-ident>`.
pub fn palette_values_name(prelude: &[Token]) -> Option<String> {
    match trim(prelude) {
        [Token::Ident(name)] if name.starts_with("--") => Some(name.clone()),
        _ => None,
    }
}

/// `font-family`: family names, no generic family among them.
fn palette_family(tokens: &[Token]) -> Option<String> {
    let list = crate::font_family::parse_list(tokens)?;
    list.iter()
        .all(|family| matches!(family, crate::font_family::Family::Name(_)))
        .then(|| crate::font_family::write_list(&list))
}

/// `base-palette`: `light`, `dark`, or a non-negative integer, a
/// calculation of one as it is written.
fn base_palette(tokens: &[Token]) -> Option<String> {
    let tokens = trim(tokens);
    match tokens {
        [Token::Ident(word)] => match word.to_ascii_lowercase().as_str() {
            "light" | "dark" => Some(word.to_ascii_lowercase()),
            _ => None,
        },
        [
            Token::Number {
                value,
                is_integer: true,
            },
        ] if *value >= 0.0 => Some(number(*value as f32)),
        _ if calc::is_math_function(tokens) => {
            calc::parse(tokens, Want::Number)?;
            Some(crate::serialize::tokens(tokens).trim().to_owned())
        }
        _ => None,
    }
}

/// `override-colors`: a list of a non-negative integer and an absolute
/// colour each, the colours as their declared values are written.
fn override_colors(tokens: &[Token]) -> Option<String> {
    let mut out = Vec::new();
    for item in split_commas(tokens) {
        let pieces = super::components(item);
        let [
            Token::Number {
                value,
                is_integer: true,
            },
        ] = pieces.first()?
        else {
            return None;
        };
        if *value < 0.0 || pieces.len() < 2 {
            return None;
        }
        let color = trim(&item[item.iter().position(|t| !matches!(t, Token::Whitespace))? + 1..]);
        // A colour without an element: `currentcolor` none (CSS Fonts 4
        // section 9.1.3's absolute colours).
        super::parse_color(color)?;
        out.push(format!(
            "{} {}",
            number(*value as f32),
            crate::serialize::specified_of("color", color)
        ));
    }
    Some(out.join(", "))
}

#[cfg(test)]
mod tests {
    use super::super::tests::with_bases;
    use super::*;
    use crate::cascade::{CascadeContext, cascade_document};
    use crate::serialize::computed_value;
    use dom::Document;

    /// `css` as the property's declared value is written, and as its
    /// computed value is.
    fn both(css: &str) -> (Option<String>, Option<String>) {
        let tokens = css::tokenize(css);
        let computed = with_bases(|bases| parse_font_palette(&tokens, bases));
        (
            font_palette_specified(&tokens),
            computed.map(|p| font_palette_text(&p)),
        )
    }

    fn pair(a: &str, b: &str) -> (Option<String>, Option<String>) {
        (Some(a.to_owned()), Some(b.to_owned()))
    }

    #[test]
    fn font_palette_takes_its_keywords_names_and_mixes_as_the_reference() {
        // What Chrome 156 gave for each, probed.
        assert_eq!(both("Dark"), pair("dark", "dark"));
        assert_eq!(both("--A"), pair("--A", "--A"));
        assert_eq!(both("--"), pair("--", "--"));
        for invalid in [
            "none",
            "A",
            "normal none",
            "palette-mix(light 30%, dark)",
            "palette-mix(in srgb, dark)",
            "palette-mix(in srgb, dark, light, --foo)",
            "palette-mix(in oklab, dark -10%, light 40%)",
            "palette-mix(in oklab, dark 150%, light 40%)",
            "palette-mix(in oklch hue, dark 60%, light 40%)",
            "palette-mix(in oklch shorter, dark 60%, light 40%)",
            "palette-mix(in oklab dark 60%, light 40%)",
            "palette-mix(in srgb, light 30% 20%, dark)",
        ] {
            assert_eq!(both(invalid), (None, None), "{invalid}");
        }
        assert_eq!(
            both("palette-mix(in xyz, light 10%, dark)"),
            pair(
                "palette-mix(in xyz-d65, light 10%, dark 90%)",
                "palette-mix(in xyz-d65, light 10%, dark 90%)"
            )
        );
        assert_eq!(
            both("PALETTE-MIX(IN HSL SHORTER HUE, LIGHT, --Foo 20%)").0,
            Some("palette-mix(in hsl, light 80%, --Foo 20%)".to_owned())
        );
        assert_eq!(
            both("palette-mix(in oklch longer hue, 30% light, dark)").0,
            Some("palette-mix(in oklch longer hue, light 30%, dark 70%)".to_owned())
        );
        // Two of 50%, or one alone, are not written.
        for css in [
            "palette-mix(in srgb, light 50%, dark 50%)",
            "palette-mix(in srgb, light 50%, dark)",
            "palette-mix(in srgb, light, dark 50%)",
        ] {
            assert_eq!(
                both(css),
                pair(
                    "palette-mix(in srgb, light, dark)",
                    "palette-mix(in srgb, light, dark)"
                )
            );
        }
        assert_eq!(
            both("palette-mix(in srgb, light 30%, dark 40%)"),
            pair(
                "palette-mix(in srgb, light 30%, dark 40%)",
                "palette-mix(in srgb, light 30%, dark 40%)"
            )
        );
        // A calculation is kept as written, and computed.
        assert_eq!(
            both("palette-mix(in srgb, light calc(10% + 0%), dark)"),
            pair(
                "palette-mix(in srgb, light calc(10%), dark)",
                "palette-mix(in srgb, light 10%, dark 90%)"
            )
        );
        assert_eq!(
            both("palette-mix(in srgb, light, dark calc(50%))"),
            pair(
                "palette-mix(in srgb, light, dark calc(50%))",
                "palette-mix(in srgb, light, dark)"
            )
        );
        // A mix of a palette with itself computes to it; nested mixes
        // are written whole.
        assert_eq!(
            both("palette-mix(in oklab, light 30%, light)"),
            pair("palette-mix(in oklab, light 30%, light 70%)", "light")
        );
        assert_eq!(
            both("palette-mix(in srgb, palette-mix(in srgb, light 30%, normal) 20%, dark)").1,
            Some(
                "palette-mix(in srgb, palette-mix(in srgb, light 30%, normal 70%) 20%, dark 80%)"
                    .to_owned()
            )
        );
        let deep = "palette-mix(in srgb, ".repeat(40) + "light" + &", dark)".repeat(40);
        assert_eq!(both(&deep), (None, None), "too deep");
    }

    #[test]
    fn font_palette_mixes_in_oklab_as_it_animates() {
        let (light, dark) = (FontPalette::Light, FontPalette::Dark);
        let at = |p: f64| font_palette_text(&interpolate_font_palette(&light, &dark, p));
        assert_eq!(at(-0.25), "light");
        assert_eq!(at(0.0), "light");
        assert_eq!(
            at(1.0 / 3.0),
            "palette-mix(in oklab, light 66.6667%, dark 33.3333%)"
        );
        assert_eq!(at(0.5), "palette-mix(in oklab, light, dark)");
        assert_eq!(at(1.0), "dark");
        assert_eq!(at(1.5), "dark");
        assert_eq!(
            font_palette_text(&interpolate_font_palette(&light, &light, 0.5)),
            "light"
        );
    }

    #[test]
    fn font_palette_is_inherited_and_left_by_the_font_shorthand() {
        let html = "<div id=a style=\"font-palette: dark\"><i id=b style=\"font: 12px serif\"></i>\
                    <i id=c style=\"font-palette: none\"></i>\
                    <i id=e style=\"font-palette: light\"><b id=f style=\"font-palette: unset\"></b></i>\
                    </div><p id=d></p>";
        let document = Document::from_node(&html::parse_document(html).expect("parses"));
        let styles = cascade_document(&document, &[], &CascadeContext::default());
        let value = |id: &str| {
            let node = document
                .subtree(document.root())
                .find(|&n| document.attr(n, "id") == Some(id))
                .expect("the element");
            computed_value(styles.get(node).expect("styled"), "font-palette").unwrap()
        };
        assert_eq!(value("a"), "dark");
        assert_eq!(value("b"), "dark");
        assert_eq!(value("c"), "dark", "an invalid value is dropped");
        assert_eq!(value("d"), "normal");
        // `unset` is `inherit`, the property being inherited.
        assert_eq!(value("f"), "light");
    }

    #[test]
    fn palette_values_keep_the_last_valid_of_each_descriptor() {
        let block = |css: &str| {
            let sheet = css::parse_stylesheet(&format!("@font-palette-values --p {{ {css} }}"))
                .expect("parses");
            let css::Rule::At(at) = &sheet.rules[0] else {
                panic!("an at-rule");
            };
            let Some(css::Block::Declarations(list)) = &at.block else {
                panic!("declarations");
            };
            PaletteValues::of(list).text()
        };
        assert_eq!(
            block(
                "base-palette: 1; base-palette: \"x\"; base-palette: dark; font-family: a; \
                 font-family: monospace; override-colors: 3 #123, 0 transparent"
            ),
            "font-family: a; base-palette: dark; override-colors: 3 rgb(17, 34, 51), 0 transparent;"
        );
        assert_eq!(
            block("base-palette: random(10, 30); override-colors: \"a\" red"),
            ""
        );
        assert_eq!(block("override-colors: 1 currentcolor"), "");
        assert_eq!(
            block("override-colors: 0 color-mix(in srgb, currentcolor, red)"),
            ""
        );
        assert_eq!(block("base-palette: 2 !important"), "");
        assert_eq!(
            palette_values_name(&css::tokenize(" --p ")).as_deref(),
            Some("--p")
        );
        assert_eq!(palette_values_name(&css::tokenize("p")), None);
        assert_eq!(palette_values_name(&css::tokenize("--a --b")), None);
    }
}
