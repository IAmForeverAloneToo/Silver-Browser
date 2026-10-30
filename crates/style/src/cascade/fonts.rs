//! The font properties past `font-size` and `font-family` (CSS Fonts 4
//! sections 2 to 6, CSS 2 section 10.8; gap batch D, D53):
//! `font-weight`, `font-style`, `font-stretch`, the `font-variant`
//! longhands and shorthand, `font-feature-settings`, `line-height`, and
//! the `font` shorthand. Each parser takes a declaration's value as the
//! cascade has it and says `None` for one the property does not take.

use std::sync::Arc;

use css::Token;

use super::{LengthBases, Range, math, single_ident, single_token, trim};
use crate::calc::{Computed, Want};
use crate::computed::{FontFeatures, FontProps, FontStyle, FontVariant, LineHeight};

/// The most features a `font-feature-settings` may list: a longer list
/// is invalid.
pub const MAX_FONT_FEATURES: usize = 64;

/// `font-weight`: a number 1 to 1000 (a calculation clamped to them),
/// `normal`, `bold`, or `bolder` and `lighter` by the parent's weight
/// (section 2.2.1's table).
pub(super) fn parse_font_weight(
    tokens: &[Token],
    parent: f32,
    bases: &LengthBases<'_>,
) -> Option<f32> {
    if let Some(word) = single_ident(tokens) {
        return Some(match word.to_ascii_lowercase().as_str() {
            "normal" => 400.0,
            "bold" => 700.0,
            "bolder" => bolder(parent),
            "lighter" => lighter(parent),
            _ => return None,
        });
    }
    if let Some(Token::Number { value, .. }) = single_token(tokens) {
        return (1.0..=1000.0).contains(value).then_some(*value as f32);
    }
    Some(number(tokens, bases)?.clamp(1.0, 1000.0))
}

/// The weight `bolder` gives over `parent` (CSS Fonts 4 section 2.2.1).
pub fn bolder(parent: f32) -> f32 {
    if parent < 350.0 {
        400.0_f32.max(parent)
    } else if parent < 550.0 {
        700.0
    } else if parent < 900.0 {
        900.0
    } else {
        parent
    }
}

/// The weight `lighter` gives under `parent`.
pub fn lighter(parent: f32) -> f32 {
    if parent < 100.0 {
        parent
    } else if parent < 550.0 {
        100.0
    } else if parent < 750.0 {
        400.0
    } else {
        700.0
    }
}

/// A number, written or calculated, finite.
fn number(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    if let Some(Computed::Value(v)) = math(tokens, Want::Number, bases, None) {
        return v.is_finite().then_some(v as f32);
    }
    match single_token(tokens)? {
        Token::Number { value, .. } if value.is_finite() => Some(*value as f32),
        _ => None,
    }
}

/// `font-style`: `normal`, `italic`, or `oblique` with an angle from -90
/// to 90 degrees (a calculation clamped to them), 14 when none is
/// given; `oblique 0deg` is `normal`.
pub(super) fn parse_font_style(tokens: &[Token], bases: &LengthBases<'_>) -> Option<FontStyle> {
    let parts: Vec<&[Token]> = super::components(tokens);
    let word = |part: &[Token]| single_ident(part).map(str::to_ascii_lowercase);
    match parts.as_slice() {
        [one] => match word(one)?.as_str() {
            "normal" => Some(FontStyle::Normal),
            "italic" => Some(FontStyle::Italic),
            "oblique" => Some(FontStyle::DefaultOblique),
            _ => None,
        },
        [first, angle] if word(first)?.as_str() == "oblique" => {
            let a = super::effects::angle(angle, false, bases)?;
            let a = if matches!(angle, [Token::Dimension { .. }]) {
                (-90.0..=90.0).contains(&a).then_some(a)?
            } else {
                a.clamp(-90.0, 90.0)
            };
            Some(if a == 0.0 {
                FontStyle::Normal
            } else {
                FontStyle::Oblique(a)
            })
        }
        _ => None,
    }
}

/// `font-stretch`: a keyword or a percentage, as a percentage of the
/// normal width; a calculation below 0 is 0.
pub(super) fn parse_font_stretch(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    if let Some(word) = single_ident(tokens) {
        return stretch_keyword(&word.to_ascii_lowercase());
    }
    if let Some(Token::Percentage { value }) = single_token(tokens) {
        return (value.is_finite() && *value >= 0.0).then_some(*value as f32);
    }
    match math(tokens, Want::Percentage, bases, None) {
        Some(Computed::Percent(p)) if p.is_finite() => Some((p as f32).max(0.0)),
        _ => None,
    }
}

/// A `font-stretch` keyword's percentage (section 2.3).
fn stretch_keyword(word: &str) -> Option<f32> {
    Some(match word {
        "ultra-condensed" => 50.0,
        "extra-condensed" => 62.5,
        "condensed" => 75.0,
        "semi-condensed" => 87.5,
        "normal" => 100.0,
        "semi-expanded" => 112.5,
        "expanded" => 125.0,
        "extra-expanded" => 150.0,
        "ultra-expanded" => 200.0,
        _ => return None,
    })
}

/// The keyword of a `font-stretch` percentage that has one.
pub fn stretch_name(percentage: f32) -> Option<&'static str> {
    Some(match percentage {
        50.0 => "ultra-condensed",
        62.5 => "extra-condensed",
        75.0 => "condensed",
        87.5 => "semi-condensed",
        100.0 => "normal",
        112.5 => "semi-expanded",
        125.0 => "expanded",
        150.0 => "extra-expanded",
        200.0 => "ultra-expanded",
        _ => return None,
    })
}

/// The `font-variant` longhands, by name, each with its groups of
/// keywords: a value takes at most one of a group, and is written in
/// the groups' order (section 6).
const VARIANTS: [(&str, &[&[&str]]); 6] = [
    (
        "font-variant-ligatures",
        &[
            &["common-ligatures", "no-common-ligatures"],
            &["discretionary-ligatures", "no-discretionary-ligatures"],
            &["historical-ligatures", "no-historical-ligatures"],
            &["contextual", "no-contextual"],
        ],
    ),
    (
        "font-variant-caps",
        &[&[
            "small-caps",
            "all-small-caps",
            "petite-caps",
            "all-petite-caps",
            "unicase",
            "titling-caps",
        ]],
    ),
    ("font-variant-alternates", &[&["historical-forms"]]),
    (
        "font-variant-numeric",
        &[
            &["lining-nums", "oldstyle-nums"],
            &["proportional-nums", "tabular-nums"],
            &["diagonal-fractions", "stacked-fractions"],
            &["ordinal"],
            &["slashed-zero"],
        ],
    ),
    (
        "font-variant-east-asian",
        &[
            &[
                "jis78",
                "jis83",
                "jis90",
                "jis04",
                "simplified",
                "traditional",
            ],
            &["full-width", "proportional-width"],
            &["ruby"],
        ],
    ),
    ("font-variant-position", &[&["sub", "super"]]),
];

/// The keywords of `font-variant-emoji`.
const EMOJI: [&str; 3] = ["text", "emoji", "unicode"];

/// A `font-variant` longhand's value written as its computed value is,
/// its keywords in their groups' order; `None` when it is not one.
/// `font-variant-ligatures` takes `none` too.
pub fn parse_variant(name: &str, tokens: &[Token]) -> Option<String> {
    let words: Option<Vec<String>> = tokens
        .iter()
        .filter(|t| **t != Token::Whitespace)
        .map(|t| match t {
            Token::Ident(word) => Some(word.to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    let words = words?;
    match words.as_slice() {
        [one] if one == "normal" => return Some("normal".into()),
        [one] if one == "none" && name == "font-variant-ligatures" => {
            return Some("none".into());
        }
        [one] if name == "font-variant-emoji" => {
            return EMOJI.contains(&one.as_str()).then(|| one.clone());
        }
        [] => return None,
        _ => {}
    }
    let groups = VARIANTS.iter().find(|(n, _)| *n == name)?.1;
    let mut chosen: Vec<Option<&str>> = vec![None; groups.len()];
    for word in &words {
        let g = groups.iter().position(|g| g.contains(&word.as_str()))?;
        if chosen[g].is_some() {
            return None;
        }
        chosen[g] = Some(word.as_str());
    }
    Some(chosen.into_iter().flatten().collect::<Vec<_>>().join(" "))
}

/// The `font-variant` shorthand's longhands: `normal` all of them
/// normal, `none` the ligatures none and the rest normal, else each
/// keyword its longhand's; `None` when it is not one.
pub fn parse_font_variant(tokens: &[Token]) -> Option<FontVariant> {
    let words: Option<Vec<String>> = tokens
        .iter()
        .filter(|t| **t != Token::Whitespace)
        .map(|t| match t {
            Token::Ident(word) => Some(word.to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    let words = words?;
    let mut variant = FontVariant::default();
    match words.as_slice() {
        [] => return None,
        [one] if one == "normal" => return Some(variant),
        [one] if one == "none" => {
            variant.ligatures = "none".into();
            return Some(variant);
        }
        _ => {}
    }
    let mut parts: Vec<(&str, Vec<String>)> = Vec::new();
    for word in words {
        let name = if EMOJI.contains(&word.as_str()) {
            "font-variant-emoji"
        } else {
            VARIANTS
                .iter()
                .find(|(_, groups)| groups.iter().any(|g| g.contains(&word.as_str())))?
                .0
        };
        match parts.iter_mut().find(|(n, _)| *n == name) {
            Some((_, list)) => list.push(word),
            None => parts.push((name, vec![word])),
        }
    }
    for (name, list) in parts {
        let tokens: Vec<Token> = list.into_iter().map(Token::Ident).collect();
        let tokens: Vec<Token> = tokens
            .into_iter()
            .flat_map(|t| [t, Token::Whitespace])
            .collect();
        let value = parse_variant(name, &tokens)?;
        *variant_field(&mut variant, name)? = value;
    }
    Some(variant)
}

/// The field of `variant` that holds the longhand `name`.
pub fn variant_field<'v>(variant: &'v mut FontVariant, name: &str) -> Option<&'v mut String> {
    Some(match name {
        "font-variant-ligatures" => &mut variant.ligatures,
        "font-variant-caps" => &mut variant.caps,
        "font-variant-alternates" => &mut variant.alternates,
        "font-variant-numeric" => &mut variant.numeric,
        "font-variant-east-asian" => &mut variant.east_asian,
        "font-variant-position" => &mut variant.position,
        "font-variant-emoji" => &mut variant.emoji,
        _ => return None,
    })
}

/// The `font-variant` longhands, in the order the reference browser
/// lists them.
pub const VARIANT_LONGHANDS: [&str; 7] = [
    "font-variant-ligatures",
    "font-variant-caps",
    "font-variant-alternates",
    "font-variant-numeric",
    "font-variant-east-asian",
    "font-variant-position",
    "font-variant-emoji",
];

/// The `font-variant` shorthand written from its longhands' values in
/// [`VARIANT_LONGHANDS`]' order: `normal` or `none`, else each value but
/// `normal`; `None` when it cannot say them (the ligatures `none` with
/// another longhand not `normal`).
pub fn write_font_variant(values: &[&str]) -> Option<String> {
    let set: Vec<&str> = values.iter().copied().filter(|v| *v != "normal").collect();
    match set.as_slice() {
        [] => Some("normal".into()),
        ["none"] if values[0] == "none" => Some("none".into()),
        _ if values[0] == "none" => None,
        _ => Some(set.join(" ")),
    }
}

/// `font-feature-settings`: `normal`, or features separated by commas,
/// each a four-letter tag in a string and a value (1 when left out,
/// `on` 1 and `off` 0).
pub(super) fn parse_font_feature_settings(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<FontFeatures> {
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("normal")) {
        return Some(Arc::from([]));
    }
    let mut out = Vec::new();
    for item in super::split_top_level_commas(tokens) {
        let parts: Vec<&[Token]> = super::components(item);
        let tag = match parts.first().and_then(|p| single_token(p)) {
            Some(Token::String { value, bad: false }) => value.as_bytes(),
            _ => return None,
        };
        let tag: [u8; 4] = tag.try_into().ok()?;
        if !tag.iter().all(|b| (0x20..=0x7e).contains(b)) {
            return None;
        }
        let value = match &parts[1..] {
            [] => 1,
            [v] => match single_ident(v).map(str::to_ascii_lowercase).as_deref() {
                Some("on") => 1,
                Some("off") => 0,
                Some(_) => return None,
                // An integer as written, or calculated and rounded.
                None => {
                    let n = match math(v, Want::Number, bases, None) {
                        Some(Computed::Value(n)) => n.round(),
                        _ => match single_token(v)? {
                            Token::Number {
                                value,
                                is_integer: true,
                            } => *value,
                            _ => return None,
                        },
                    };
                    if n.is_nan() || n < 0.0 || !n.is_finite() {
                        return None;
                    }
                    n.min(f64::from(u32::MAX)) as u32
                }
            },
            _ => return None,
        };
        out.push((tag, value));
        if out.len() > MAX_FONT_FEATURES {
            return None;
        }
    }
    if out.is_empty() {
        return None;
    }
    // A tag's last value stands; the computed value lists them by tag.
    let mut kept: Vec<([u8; 4], u32)> = Vec::with_capacity(out.len());
    for (tag, value) in out {
        kept.retain(|(t, _)| *t != tag);
        kept.push((tag, value));
    }
    kept.sort_by_key(|(tag, _)| *tag);
    Some(kept.into())
}

/// `line-height`: `normal`, a non-negative number, length or percentage
/// (of the element's font size, computed to a length).
pub(super) fn parse_line_height(tokens: &[Token], bases: &LengthBases<'_>) -> Option<LineHeight> {
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("normal")) {
        return Some(LineHeight::Normal);
    }
    if let Some(n) = number(tokens, bases) {
        return (n >= 0.0).then_some(LineHeight::Number(n));
    }
    if let Some(Computed::Value(px)) = math(
        tokens,
        Want::LengthPercentage,
        bases,
        Some(f64::from(bases.em)),
    ) {
        return Some(LineHeight::Px(Range::NonNegative.clamp(px)));
    }
    if let Some(Token::Percentage { value }) = single_token(tokens) {
        let px = *value as f32 / 100.0 * bases.em;
        return (px.is_finite() && px >= 0.0).then_some(LineHeight::Px(px));
    }
    super::parse_non_negative_length_px(tokens, bases).map(LineHeight::Px)
}

/// The `font` shorthand's parts (section 6.1), each a slice of its
/// value for its longhand's parser: the style, variant, weight and
/// stretch in any order before the size, a line height after a `/`,
/// then the families. A system font keyword is the system's font, the
/// reference browser's 13 pixels of `system-ui`.
#[derive(Debug, Default)]
pub(super) struct FontShorthand<'t> {
    pub style: Option<&'t [Token]>,
    pub weight: Option<&'t [Token]>,
    pub stretch: Option<&'t [Token]>,
    pub small_caps: bool,
    pub size: Option<&'t [Token]>,
    pub line_height: Option<&'t [Token]>,
    pub families: Option<&'t [Token]>,
    /// A system font keyword: the size and families are the system's.
    pub system: bool,
}

/// The system font keywords.
const SYSTEM_FONTS: [&str; 6] = [
    "caption",
    "icon",
    "menu",
    "message-box",
    "small-caption",
    "status-bar",
];

/// Splits the `font` shorthand's value; `None` when it is not one.
pub(super) fn split_font(tokens: &[Token]) -> Option<FontShorthand<'_>> {
    let tokens = trim(tokens);
    if let Some(word) = single_ident(tokens)
        && SYSTEM_FONTS.contains(&word.to_ascii_lowercase().as_str())
    {
        return Some(FontShorthand {
            system: true,
            ..FontShorthand::default()
        });
    }
    let parts = super::components(tokens);
    let mut out = FontShorthand::default();
    let mut i = 0;
    let mut normals = 0;
    // Before the size: at most four, each once, `normal` standing for
    // any of them.
    while let Some(part) = parts.get(i) {
        let word = single_ident(part).map(str::to_ascii_lowercase);
        match word.as_deref() {
            Some("normal") => normals += 1,
            Some("italic" | "oblique") if out.style.is_none() => {
                // An angle after `oblique` is the style's.
                let angle = word.as_deref() == Some("oblique")
                    && parts.get(i + 1).is_some_and(|p| {
                        matches!(single_token(p), Some(Token::Dimension { unit, .. })
                            if matches!(unit.to_ascii_lowercase().as_str(), "deg" | "rad" | "grad" | "turn"))
                    });
                let end = if angle { i + 2 } else { i + 1 };
                out.style = Some(span(tokens, &parts[i..end]));
                i = end;
                continue;
            }
            Some("small-caps") if !out.small_caps => out.small_caps = true,
            Some("bold" | "bolder" | "lighter") if out.weight.is_none() => out.weight = Some(part),
            Some(w) if out.stretch.is_none() && stretch_keyword(w).is_some() => {
                out.stretch = Some(part)
            }
            None if out.weight.is_none()
                && matches!(single_token(part), Some(Token::Number { .. })) =>
            {
                out.weight = Some(part)
            }
            _ => break,
        }
        i += 1;
    }
    let set = normals
        + [
            out.style.is_some(),
            out.weight.is_some(),
            out.stretch.is_some(),
            out.small_caps,
        ]
        .iter()
        .filter(|s| **s)
        .count();
    if set > 4 {
        return None;
    }
    // The size, then a line height after `/`, then the families.
    let size = *parts.get(i)?;
    // A `/` written against the size is a token of its own.
    out.size = Some(size);
    i += 1;
    if parts
        .get(i)
        .is_some_and(|p| single_token(p) == Some(&Token::Delim('/')))
    {
        out.line_height = Some(*parts.get(i + 1)?);
        i += 2;
    }
    let rest = parts.get(i..).filter(|r| !r.is_empty())?;
    out.families = Some(span(tokens, rest));
    Some(out)
}

/// A number, or with `percentage` a percentage, written or calculated
/// with nothing in it that depends on an element: what an `@font-face`
/// descriptor's range takes.
pub(crate) fn plain_number(tokens: &[Token], percentage: bool) -> Option<f64> {
    super::with_default_bases(|bases| {
        let want = if percentage {
            Want::Percentage
        } else {
            Want::Number
        };
        match math(tokens, want, bases, None)? {
            Computed::Value(v) if !percentage => Some(v),
            Computed::Percent(p) if percentage => Some(p),
            _ => None,
        }
    })
    .filter(|v| v.is_finite())
}

/// An angle in degrees, written or calculated with nothing in it that
/// depends on an element.
pub(crate) fn plain_angle(tokens: &[Token]) -> Option<f32> {
    super::with_default_bases(|bases| super::effects::angle(tokens, false, bases))
}

/// The families a `font` shorthand names (CSS Font Loading 3's
/// `load()` and `check()` read them); a system font's are `system-ui`.
/// `None` when it is not one.
pub fn font_shorthand_families(tokens: &[Token]) -> Option<Vec<crate::font_family::Family>> {
    let split = split_font(tokens)?;
    if split.system {
        return Some(vec![crate::font_family::Family::Generic(
            "system-ui".into(),
        )]);
    }
    crate::font_family::parse_list(split.families?)
}

/// The `font` shorthand's values: the size (by `bases`, which the first
/// pass of the cascade gives the parent's) and the rest, each longhand
/// it does not name at its initial value.
pub(super) struct Font {
    pub size: f32,
    pub props: FontProps,
}

/// The reference browser's system font: 13 pixels of `system-ui`.
const SYSTEM_FONT_PX: f32 = 13.0;

/// Parses the `font` shorthand; `None` when any part of it is invalid.
pub(super) fn parse_font(tokens: &[Token], parent: f32, bases: &LengthBases<'_>) -> Option<Font> {
    let split = split_font(tokens)?;
    if split.system {
        return Some(Font {
            size: SYSTEM_FONT_PX,
            props: FontProps {
                family: "system-ui".into(),
                ..FontProps::default()
            },
        });
    }
    let mut props = FontProps {
        family: super::parse_font_family(split.families?)?.into(),
        ..FontProps::default()
    };
    if split.small_caps {
        props.variant = Arc::new(FontVariant {
            caps: "small-caps".into(),
            ..FontVariant::default()
        });
    }
    if let Some(style) = split.style {
        props.style = parse_font_style(style, bases)?;
    }
    if let Some(weight) = split.weight {
        props.weight = parse_font_weight(weight, parent, bases)?;
    }
    if let Some(stretch) = split.stretch {
        props.stretch = parse_font_stretch(stretch, bases)?;
    }
    if let Some(line_height) = split.line_height {
        props.line_height = parse_line_height(line_height, bases)?;
    }
    Some(Font {
        size: super::parse_font_size(split.size?, bases)?,
        props,
    })
}

/// The longhands of a `font` shorthand as written, each its value's
/// text, those it leaves out at their initial values, in the order the
/// reference browser lists them: what a declaration block keeps of it.
/// `None` for one that is invalid or a system font, which is kept
/// whole.
pub fn font_longhands(tokens: &[Token]) -> Option<Vec<(&'static str, String)>> {
    let split = split_font(tokens)?;
    if split.system {
        return None;
    }
    let text = |part: Option<&[Token]>| {
        part.map_or_else(
            || "normal".to_owned(),
            |p| crate::serialize::tokens(p).trim().to_ascii_lowercase(),
        )
    };
    let families = crate::font_family::parse_list(split.families?)?;
    let mut out = vec![("font-style", text(split.style))];
    for name in VARIANT_LONGHANDS {
        let value = if name == "font-variant-caps" && split.small_caps {
            "small-caps"
        } else {
            "normal"
        };
        out.push((name, value.to_owned()));
    }
    out.extend([
        ("font-weight", text(split.weight)),
        ("font-stretch", text(split.stretch)),
        (
            "font-size",
            crate::serialize::tokens(split.size?).trim().to_owned(),
        ),
        ("line-height", text(split.line_height)),
        ("font-family", crate::font_family::write_list(&families)),
        ("font-kerning", "auto".to_owned()),
        ("font-feature-settings", "normal".to_owned()),
    ]);
    Some(out)
}

/// The tokens of `tokens` from the first of `parts` to the last.
fn span<'t>(tokens: &'t [Token], parts: &[&'t [Token]]) -> &'t [Token] {
    let start = parts[0].as_ptr() as usize - tokens.as_ptr() as usize;
    let last = parts[parts.len() - 1];
    let end = last.as_ptr() as usize - tokens.as_ptr() as usize + std::mem::size_of_val(last);
    let size = std::mem::size_of::<Token>();
    &tokens[start / size..end / size]
}

#[cfg(test)]
mod tests {
    use super::super::tests::with_bases;
    use super::*;
    use crate::cascade::{CascadeContext, cascade_document};
    use crate::computed::ComputedStyle;
    use crate::serialize::computed_value;
    use dom::Document;

    fn weight(css: &str) -> Option<f32> {
        with_bases(|bases| parse_font_weight(&css::tokenize(css), 400.0, bases))
    }

    fn font(css: &str) -> Option<(f32, String, FontProps)> {
        with_bases(|bases| parse_font(&css::tokenize(css), 400.0, bases))
            .map(|f| (f.size, f.props.family.to_string(), f.props))
    }

    /// The style of the element with id `id` in `html`.
    fn style(html: &str, id: &str) -> ComputedStyle {
        let document = Document::from_node(&html::parse_document(html).expect("never fails"));
        let styles = cascade_document(&document, &[], &CascadeContext::default());
        let node = document
            .subtree(document.root())
            .find(|&n| document.attr(n, "id") == Some(id))
            .expect("the element");
        styles.get(node).expect("styled").clone()
    }

    #[test]
    fn weights_are_numbers_and_bolder_and_lighter_step_by_the_table() {
        assert_eq!(weight("normal"), Some(400.0));
        assert_eq!(weight("BOLD"), Some(700.0));
        assert_eq!(weight("450"), Some(450.0));
        assert_eq!(weight("150.25"), Some(150.25));
        assert_eq!(weight("calc(100 * 3)"), Some(300.0));
        assert_eq!(
            weight("calc(1001)"),
            Some(1000.0),
            "a calculation is clamped"
        );
        assert_eq!(weight("1000"), Some(1000.0));
        assert_eq!(weight("1001"), None);
        assert_eq!(weight("0"), None);
        assert_eq!(weight("400px"), None);
        assert_eq!(weight("bolder"), Some(700.0));
        assert_eq!(weight("lighter"), Some(100.0));
        let steps: Vec<_> = [
            1.0, 50.0, 100.0, 349.0, 350.0, 549.0, 550.0, 749.0, 750.0, 899.0, 950.0,
        ]
        .into_iter()
        .map(|w| (bolder(w), lighter(w)))
        .collect();
        assert_eq!(
            steps,
            [
                (400.0, 1.0),
                (400.0, 50.0),
                (400.0, 100.0),
                (400.0, 100.0),
                (700.0, 100.0),
                (700.0, 100.0),
                (900.0, 400.0),
                (900.0, 400.0),
                (900.0, 700.0),
                (900.0, 700.0),
                (950.0, 700.0)
            ]
        );
    }

    #[test]
    fn styles_stretches_and_variants_parse_as_written() {
        let style = |css: &str| with_bases(|b| parse_font_style(&css::tokenize(css), b));
        assert_eq!(style("italic"), Some(FontStyle::Italic));
        assert_eq!(style("oblique"), Some(FontStyle::DefaultOblique));
        assert_eq!(style("oblique 14deg"), Some(FontStyle::Oblique(14.0)));
        assert_eq!(style("oblique -10deg"), Some(FontStyle::Oblique(-10.0)));
        assert_eq!(style("oblique 0.25turn"), Some(FontStyle::Oblique(90.0)));
        assert_eq!(style("oblique 0deg"), Some(FontStyle::Normal));
        assert_eq!(
            style("oblique calc(10deg - 10deg)"),
            Some(FontStyle::Normal)
        );
        assert_eq!(
            style("oblique calc(100deg)"),
            Some(FontStyle::Oblique(90.0))
        );
        assert_eq!(style("oblique 91deg"), None);
        assert_eq!(style("italic 10deg"), None);
        let stretch = |css: &str| with_bases(|b| parse_font_stretch(&css::tokenize(css), b));
        assert_eq!(stretch("condensed"), Some(75.0));
        assert_eq!(stretch("ultra-expanded"), Some(200.0));
        assert_eq!(stretch("50%"), Some(50.0));
        assert_eq!(stretch("calc(10% + 5%)"), Some(15.0));
        assert_eq!(stretch("calc(-10%)"), Some(0.0));
        assert_eq!(stretch("-1%"), None);
        assert_eq!(stretch("50"), None);
        let variant = |name: &str, css: &str| parse_variant(name, &css::tokenize(css));
        assert_eq!(
            variant("font-variant-caps", "Small-Caps").as_deref(),
            Some("small-caps")
        );
        assert_eq!(
            variant("font-variant-caps", "all-small-caps").as_deref(),
            Some("all-small-caps")
        );
        assert_eq!(variant("font-variant-caps", "none"), None);
        assert_eq!(
            variant(
                "font-variant-numeric",
                "slashed-zero tabular-nums oldstyle-nums"
            )
            .as_deref(),
            Some("oldstyle-nums tabular-nums slashed-zero")
        );
        assert_eq!(
            variant("font-variant-numeric", "tabular-nums proportional-nums"),
            None
        );
        assert_eq!(
            variant("font-variant-ligatures", "none").as_deref(),
            Some("none")
        );
        assert_eq!(
            variant("font-variant-ligatures", "none common-ligatures"),
            None
        );
        assert_eq!(
            variant("font-variant-alternates", "HISTORICAL-FORMS").as_deref(),
            Some("historical-forms")
        );
        assert_eq!(
            variant("font-variant-emoji", "text").as_deref(),
            Some("text")
        );
        assert_eq!(variant("font-variant-position", "sub super"), None);
        let shorthand = |css: &str| parse_font_variant(&css::tokenize(css));
        let none = shorthand("none").unwrap();
        assert_eq!(
            (none.ligatures.as_str(), none.caps.as_str()),
            ("none", "normal")
        );
        let mixed = shorthand("tabular-nums small-caps ruby emoji").unwrap();
        assert_eq!(
            (
                mixed.numeric.as_str(),
                mixed.caps.as_str(),
                mixed.east_asian.as_str(),
                mixed.emoji.as_str()
            ),
            ("tabular-nums", "small-caps", "ruby", "emoji")
        );
        assert!(shorthand("small-caps none").is_none());
        assert!(shorthand("small-caps petite-caps").is_none());
        assert_eq!(
            write_font_variant(&["normal"; 7]).as_deref(),
            Some("normal")
        );
        let mut values = ["normal"; 7];
        values[0] = "none";
        assert_eq!(write_font_variant(&values).as_deref(), Some("none"));
        values[3] = "ordinal";
        assert_eq!(write_font_variant(&values), None);
        values[0] = "normal";
        assert_eq!(write_font_variant(&values).as_deref(), Some("ordinal"));
    }

    #[test]
    fn features_are_four_letter_tags_with_values() {
        let features = |css: &str| {
            with_bases(|b| parse_font_feature_settings(&css::tokenize(css), b)).map(|f| f.to_vec())
        };
        assert_eq!(features("normal"), Some(vec![]));
        assert_eq!(
            features("\"liga\" 0, \"kern\", \"smcp\" on, \"ss01\" 3"),
            Some(vec![
                (*b"kern", 1),
                (*b"liga", 0),
                (*b"smcp", 1),
                (*b"ss01", 3)
            ])
        );
        assert_eq!(features("\"liga\" off"), Some(vec![(*b"liga", 0)]));
        // A tag given again has its last value.
        assert_eq!(
            features("\"dlig\", \"smcp\", \"dlig\" 0"),
            Some(vec![(*b"dlig", 0), (*b"smcp", 1)])
        );
        assert_eq!(features("\"lig\" 1"), None);
        assert_eq!(features("\"ligat\""), None);
        assert_eq!(features("liga"), None);
        assert_eq!(features("\"liga\" -1"), None);
        assert_eq!(features("\"liga\" 1.5"), None);
        // An integer, not a number with an exponent; a calculation is
        // rounded.
        assert_eq!(features("\"liga\" 1e1"), None);
        assert_eq!(features("\"liga\" calc(10.1)"), Some(vec![(*b"liga", 10)]));
        assert_eq!(features("\"liga\" calc(-1)"), None);
        assert_eq!(features("\"liga\" 1 2"), None);
        assert_eq!(features("\"liga\","), None);
        let many = vec!["\"liga\""; MAX_FONT_FEATURES + 1].join(",");
        assert_eq!(features(&many), None);
    }

    #[test]
    fn line_heights_keep_numbers_and_compute_lengths() {
        let lh = |css: &str| with_bases(|b| parse_line_height(&css::tokenize(css), b));
        assert_eq!(lh("normal"), Some(LineHeight::Normal));
        assert_eq!(lh("1.5"), Some(LineHeight::Number(1.5)));
        assert_eq!(lh("0"), Some(LineHeight::Number(0.0)));
        assert_eq!(lh("20px"), Some(LineHeight::Px(20.0)));
        assert_eq!(lh("150%"), Some(LineHeight::Px(24.0)));
        assert_eq!(lh("2em"), Some(LineHeight::Px(32.0)));
        assert_eq!(lh("calc(100% + 2px)"), Some(LineHeight::Px(18.0)));
        assert_eq!(lh("-1"), None);
        assert_eq!(lh("-1px"), None);
        assert_eq!(lh("auto"), None);
    }

    #[test]
    fn the_shorthand_takes_its_parts_in_order() {
        let (size, family, props) = font("italic bold 12px/30px Georgia, serif").unwrap();
        assert_eq!((size, family.as_str()), (12.0, "Georgia, serif"));
        assert_eq!(props.style, FontStyle::Italic);
        assert_eq!(props.weight, 700.0);
        assert_eq!(props.line_height, LineHeight::Px(30.0));
        let (_, _, props) = font("condensed 500 small-caps oblique 10deg 1em/1.2 a").unwrap();
        assert_eq!(
            (props.stretch, props.weight, props.small_caps(), props.style),
            (75.0, 500.0, true, FontStyle::Oblique(10.0))
        );
        assert_eq!(props.line_height, LineHeight::Number(1.2));
        assert_eq!(font("12px sans-serif").unwrap().2, FontProps::default());
        assert!(font("normal normal normal normal 12px a").is_some());
        assert!(font("normal normal normal normal normal 12px a").is_none());
        assert!(font("bold bold 12px a").is_none());
        assert!(font("small-caps small-caps 12px a").is_none());
        assert!(font("bold 12px").is_none());
        assert!(font("bold serif").is_none());
        assert!(font("12px/ serif").is_none());
        assert!(font("12px/-1 serif").is_none());
        assert!(font("-1px serif").is_none());
        assert!(font("1001 12px a").is_none());
        let (size, family, props) = font("menu").unwrap();
        assert_eq!((size, family.as_str()), (13.0, "system-ui"));
        assert_eq!(props.weight, 400.0);
        assert!(font("menu 12px a").is_none());
    }

    #[test]
    fn the_cascade_inherits_and_resets_the_font() {
        let html = "<div id=p style=\"font: italic bold 20px/1.5 serif\">\
                    <span id=a style=\"font-weight: bolder\"></span>\
                    <span id=b style=\"font-size: 10px\"></span>\
                    <span id=c style=\"font: 8px sans-serif\"></span>\
                    <span id=d style=\"font-weight: lighter; line-height: inherit\"></span>\
                    <span id=e style=\"font: inherit; font-stretch: 50%\"></span>\
                    <span id=f style=\"font-style: initial; line-height: 150%\"></span>\n                    <span id=g style=\"line-height: unset\"></span></div>";
        let value = |id: &str, name: &str| computed_value(&style(html, id), name).unwrap();
        assert_eq!(value("p", "font-size"), "20px");
        assert_eq!(value("p", "font-style"), "italic");
        assert_eq!(value("a", "font-weight"), "900");
        // A number is inherited as a number.
        assert_eq!(value("b", "line-height"), "15px");
        assert_eq!(value("b", "font-weight"), "700");
        assert_eq!(value("c", "font-weight"), "400");
        assert_eq!(value("c", "font-style"), "normal");
        assert_eq!(value("c", "line-height"), "normal");
        assert_eq!(value("c", "font-size"), "8px");
        assert_eq!(value("c", "font-family"), "sans-serif");
        assert_eq!(value("d", "font-weight"), "400");
        assert_eq!(value("d", "line-height"), "30px");
        assert_eq!(value("e", "font-size"), "20px");
        assert_eq!(value("e", "font-family"), "serif");
        assert_eq!(value("e", "font-stretch"), "50%");
        assert_eq!(value("f", "font-style"), "normal");
        assert_eq!(value("f", "line-height"), "30px");
        assert_eq!(value("g", "line-height"), "30px");
        // Elements that set none have their parent's.
        let shared = "<div id=p style=\"font-weight: 300\"><span id=c></span></div>";
        let (p, c) = (style(shared, "p"), style(shared, "c"));
        assert_eq!(c.font().weight, 300.0);
        assert_eq!(p.font, c.font);
    }

    #[test]
    fn computed_values_serialize_as_the_reference_browser_has_them() {
        let html = "<i id=a style=\"font-style: oblique; font-variant: small-caps; \
                    font-feature-settings: &quot;liga&quot; 0, &quot;kern&quot;; \
                    font-stretch: expanded\"></i>\
                    <i id=b style=\"font-style: oblique -20deg\"></i>";
        let value = |id: &str, name: &str| computed_value(&style(html, id), name).unwrap();
        assert_eq!(value("a", "font-style"), "oblique");
        assert_eq!(value("a", "font-feature-settings"), "\"kern\", \"liga\" 0");
        assert_eq!(value("a", "font-variant-caps"), "small-caps");
        assert_eq!(value("a", "font-stretch"), "125%");
        assert_eq!(value("b", "font-style"), "oblique -20deg");
        assert_eq!(value("b", "font-feature-settings"), "normal");
        assert_eq!(value("b", "font-weight"), "400");
    }

    #[test]
    fn font_kerning_is_inherited_and_reset_by_font() {
        let html = "<div id=a style=\"font-kerning: NONE\"><i id=b></i>\
                    <i id=c style=\"font-kerning: normal; font: 12px serif\"></i>\
                    <i id=d style=\"font-kerning: sideways\"></i></div>";
        let value = |id: &str| computed_value(&style(html, id), "font-kerning").unwrap();
        assert_eq!(value("a"), "none");
        assert_eq!(value("b"), "none");
        assert_eq!(value("c"), "auto");
        assert_eq!(value("d"), "none", "an invalid value is dropped");
        assert_eq!(
            computed_value(&style(html, "c"), "font").unwrap(),
            "12px serif"
        );
        assert_eq!(computed_value(&style(html, "a"), "font").unwrap(), "");
    }
}
