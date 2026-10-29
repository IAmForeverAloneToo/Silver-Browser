//! The font properties past `font-size` and `font-family` (CSS Fonts 4
//! sections 2 to 6, CSS 2 section 10.8; gap batch D, D53):
//! `font-weight`, `font-style`, `font-stretch`, `font-variant` and
//! `font-variant-caps` (small capitals alone), `font-feature-settings`,
//! `line-height`, and the `font` shorthand. Each parser takes a
//! declaration's value as the cascade has it and says `None` for one the
//! property does not take.

use std::sync::Arc;

use css::Token;

use super::{LengthBases, Range, math, single_ident, single_token, trim};
use crate::calc::{Computed, Want};
use crate::computed::{FontFeatures, FontProps, FontStyle, LineHeight};

/// The most features a `font-feature-settings` may list: a longer list
/// is invalid.
pub const MAX_FONT_FEATURES: usize = 64;

/// `font-weight`: a number 1 to 1000, `normal`, `bold`, or `bolder` and
/// `lighter` by the parent's weight (section 2.2.1's table).
pub(super) fn parse_font_weight(
    tokens: &[Token],
    parent: u16,
    bases: &LengthBases<'_>,
) -> Option<u16> {
    if let Some(word) = single_ident(tokens) {
        return Some(match word.to_ascii_lowercase().as_str() {
            "normal" => 400,
            "bold" => 700,
            "bolder" => bolder(parent),
            "lighter" => lighter(parent),
            _ => return None,
        });
    }
    let n = number(tokens, bases)?;
    (1.0..=1000.0).contains(&n).then_some(n.round() as u16)
}

/// The weight `bolder` gives over `parent`.
pub fn bolder(parent: u16) -> u16 {
    match parent {
        0..350 => 400,
        350..550 => 700,
        _ => 900,
    }
}

/// The weight `lighter` gives under `parent`.
pub fn lighter(parent: u16) -> u16 {
    match parent {
        0..550 => 100,
        550..750 => 400,
        _ => 700,
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
/// to 90 degrees, 14 when none is given.
pub(super) fn parse_font_style(tokens: &[Token], bases: &LengthBases<'_>) -> Option<FontStyle> {
    let parts: Vec<&[Token]> = super::components(tokens);
    let word = |part: &[Token]| single_ident(part).map(str::to_ascii_lowercase);
    match parts.as_slice() {
        [one] => match word(one)?.as_str() {
            "normal" => Some(FontStyle::Normal),
            "italic" => Some(FontStyle::Italic),
            "oblique" => Some(FontStyle::Oblique(14.0)),
            _ => None,
        },
        [first, angle] if word(first)?.as_str() == "oblique" => {
            let a = super::effects::angle(angle, true, bases)?;
            (-90.0..=90.0).contains(&a).then_some(FontStyle::Oblique(a))
        }
        _ => None,
    }
}

/// `font-stretch`: a keyword or a percentage, as a percentage of the
/// normal width.
pub(super) fn parse_font_stretch(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    if let Some(word) = single_ident(tokens) {
        return stretch_keyword(&word.to_ascii_lowercase());
    }
    let p = match math(tokens, Want::Percentage, bases, None) {
        Some(Computed::Percent(p)) => p as f32,
        _ => match single_token(tokens)? {
            Token::Percentage { value } => *value as f32,
            _ => return None,
        },
    };
    (p.is_finite() && p >= 0.0).then_some(p)
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

/// `font-variant-caps` and the `font-variant` shorthand, of which only
/// `normal` and `small-caps` (and `font-variant: none`) are taken:
/// whether the text is in small capitals.
pub(super) fn parse_small_caps(tokens: &[Token], shorthand: bool) -> Option<bool> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "normal" => Some(false),
        "none" if shorthand => Some(false),
        "small-caps" => Some(true),
        _ => None,
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
                None => {
                    let n = number(v, bases)?;
                    if n < 0.0 || n.fract() != 0.0 {
                        return None;
                    }
                    n.min(u32::MAX as f32) as u32
                }
            },
            _ => return None,
        };
        out.push((tag, value));
        if out.len() > MAX_FONT_FEATURES {
            return None;
        }
    }
    (!out.is_empty()).then(|| out.into())
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
pub(super) fn parse_font(tokens: &[Token], parent: u16, bases: &LengthBases<'_>) -> Option<Font> {
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
        small_caps: split.small_caps,
        ..FontProps::default()
    };
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

    fn weight(css: &str) -> Option<u16> {
        with_bases(|bases| parse_font_weight(&css::tokenize(css), 400, bases))
    }

    fn font(css: &str) -> Option<(f32, String, FontProps)> {
        with_bases(|bases| parse_font(&css::tokenize(css), 400, bases))
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
        assert_eq!(weight("normal"), Some(400));
        assert_eq!(weight("BOLD"), Some(700));
        assert_eq!(weight("450"), Some(450));
        assert_eq!(weight("calc(100 * 3)"), Some(300));
        assert_eq!(weight("1000"), Some(1000));
        assert_eq!(weight("1001"), None);
        assert_eq!(weight("0"), None);
        assert_eq!(weight("400px"), None);
        assert_eq!(weight("bolder"), Some(700));
        assert_eq!(weight("lighter"), Some(100));
        let steps: Vec<_> = [100, 349, 350, 549, 550, 749, 750, 900]
            .into_iter()
            .map(|w| (bolder(w), lighter(w)))
            .collect();
        assert_eq!(
            steps,
            [
                (400, 100),
                (400, 100),
                (700, 100),
                (700, 100),
                (900, 400),
                (900, 400),
                (900, 700),
                (900, 700)
            ]
        );
    }

    #[test]
    fn styles_stretches_and_variants_parse_as_written() {
        let style = |css: &str| with_bases(|b| parse_font_style(&css::tokenize(css), b));
        assert_eq!(style("italic"), Some(FontStyle::Italic));
        assert_eq!(style("oblique"), Some(FontStyle::Oblique(14.0)));
        assert_eq!(style("oblique -10deg"), Some(FontStyle::Oblique(-10.0)));
        assert_eq!(style("oblique 0.25turn"), Some(FontStyle::Oblique(90.0)));
        assert_eq!(style("oblique 91deg"), None);
        assert_eq!(style("italic 10deg"), None);
        let stretch = |css: &str| with_bases(|b| parse_font_stretch(&css::tokenize(css), b));
        assert_eq!(stretch("condensed"), Some(75.0));
        assert_eq!(stretch("ultra-expanded"), Some(200.0));
        assert_eq!(stretch("50%"), Some(50.0));
        assert_eq!(stretch("calc(10% + 5%)"), Some(15.0));
        assert_eq!(stretch("-1%"), None);
        assert_eq!(stretch("50"), None);
        let caps = |css: &str, shorthand| parse_small_caps(&css::tokenize(css), shorthand);
        assert_eq!(caps("small-caps", false), Some(true));
        assert_eq!(caps("none", true), Some(false));
        assert_eq!(caps("none", false), None);
        assert_eq!(caps("all-small-caps", false), None);
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
                (*b"liga", 0),
                (*b"kern", 1),
                (*b"smcp", 1),
                (*b"ss01", 3)
            ])
        );
        assert_eq!(features("\"liga\" off"), Some(vec![(*b"liga", 0)]));
        assert_eq!(features("\"lig\" 1"), None);
        assert_eq!(features("\"ligat\""), None);
        assert_eq!(features("liga"), None);
        assert_eq!(features("\"liga\" -1"), None);
        assert_eq!(features("\"liga\" 1.5"), None);
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
        assert_eq!(props.weight, 700);
        assert_eq!(props.line_height, LineHeight::Px(30.0));
        let (_, _, props) = font("condensed 500 small-caps oblique 10deg 1em/1.2 a").unwrap();
        assert_eq!(
            (props.stretch, props.weight, props.small_caps, props.style),
            (75.0, 500, true, FontStyle::Oblique(10.0))
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
        assert_eq!(props.weight, 400);
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
        assert_eq!(c.font().weight, 300);
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
        assert_eq!(value("a", "font-feature-settings"), "\"liga\" 0, \"kern\"");
        assert_eq!(value("a", "font-variant-caps"), "small-caps");
        assert_eq!(value("a", "font-stretch"), "125%");
        assert_eq!(value("b", "font-style"), "oblique -20deg");
        assert_eq!(value("b", "font-feature-settings"), "normal");
        assert_eq!(value("b", "font-weight"), "400");
    }
}
