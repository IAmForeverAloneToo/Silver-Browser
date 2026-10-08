//! `aspect-ratio` (css-sizing-4 section 5.1) and the image properties of
//! CSS Images 3 (sections 4.5, 4.6, 5.1 and 5.3; M23.2), and the
//! presentational hints an `img`'s `width` and `height` give (HTML
//! section 15.4.3). Each parser takes a declaration's value as the
//! cascade has it and says `None` when the property does not take it.

use css::Token;

use super::{LengthBases, components, math, single_ident, single_token};
use crate::calc::{Computed, Want};
use crate::computed::{
    AspectRatio, ImageOrientation, ImageRendering, ObjectFit, ObjectPosition, Ratio,
};

/// A non-negative `<number>`, or a math function's, finite.
fn non_negative_number(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    let value = match math(tokens, Want::Number, bases, None) {
        Some(Computed::Value(v)) => v,
        Some(_) => return None,
        None => match single_token(tokens)? {
            Token::Number { value, .. } => *value,
            _ => return None,
        },
    };
    let value = value as f32;
    (value.is_finite() && value >= 0.0).then_some(value)
}

/// A `<ratio>` (css-values-4 section 6.2) from its components: a number,
/// or two with a `/` between them.
fn ratio(parts: &[&[Token]], bases: &LengthBases<'_>) -> Option<Ratio> {
    match parts {
        [width] => Some(Ratio {
            width: non_negative_number(width, bases)?,
            height: 1.0,
        }),
        [width, [Token::Delim('/')], height] => Some(Ratio {
            width: non_negative_number(width, bases)?,
            height: non_negative_number(height, bases)?,
        }),
        _ => None,
    }
}

/// `aspect-ratio: auto || <ratio>`.
pub(super) fn parse_aspect_ratio(tokens: &[Token], bases: &LengthBases<'_>) -> Option<AspectRatio> {
    let parts = components(tokens);
    let is_auto =
        |part: &&[Token]| single_ident(part).is_some_and(|w| w.eq_ignore_ascii_case("auto"));
    let (auto, rest) = match parts.as_slice() {
        [first, rest @ ..] if is_auto(first) => (true, rest),
        [rest @ .., last] if is_auto(last) => (true, rest),
        rest => (false, rest),
    };
    match (auto, rest) {
        (true, []) => Some(AspectRatio::Auto),
        (true, rest) => ratio(rest, bases).map(AspectRatio::AutoOr),
        (false, rest) => ratio(rest, bases).map(AspectRatio::Ratio),
    }
}

/// `object-fit`.
pub(super) fn parse_object_fit(tokens: &[Token]) -> Option<ObjectFit> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "fill" => Some(ObjectFit::Fill),
        "contain" => Some(ObjectFit::Contain),
        "cover" => Some(ObjectFit::Cover),
        "none" => Some(ObjectFit::None),
        "scale-down" => Some(ObjectFit::ScaleDown),
        _ => None,
    }
}

/// `object-position: <position>`.
pub(super) fn parse_object_position(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<ObjectPosition> {
    let (x, y) = super::effects::position(&components(tokens), bases, true)?;
    Some(ObjectPosition { x, y })
}

/// `image-orientation: from-image | none`.
pub(super) fn parse_image_orientation(tokens: &[Token]) -> Option<ImageOrientation> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "from-image" => Some(ImageOrientation::FromImage),
        "none" => Some(ImageOrientation::None),
        _ => None,
    }
}

/// `image-rendering`, `-webkit-optimize-contrast` being the reference
/// browser's name for `crisp-edges`.
pub(super) fn parse_image_rendering(tokens: &[Token]) -> Option<ImageRendering> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "auto" => Some(ImageRendering::Auto),
        "smooth" => Some(ImageRendering::Smooth),
        "high-quality" => Some(ImageRendering::HighQuality),
        "pixelated" => Some(ImageRendering::Pixelated),
        "crisp-edges" | "-webkit-optimize-contrast" => Some(ImageRendering::CrispEdges),
        _ => None,
    }
}

/// Whether the attribute `name` of an element named `tag` is one of the
/// hints [`img_hints`] reads, whose change restyles the element.
pub(crate) fn is_img_hint(tag: &str, name: &str) -> bool {
    tag == "img" && matches!(name, "width" | "height")
}

/// The declarations an `img`'s `width` and `height` are presentational
/// hints for (HTML section 15.4.3), as the text of a declaration block:
/// each a length or a percentage by the rules for parsing dimension
/// values, and both lengths together `aspect-ratio: auto w / h`. Empty
/// for any other element, and for an `img` with neither.
pub(super) fn img_hints(tag: &str, attr: impl Fn(&str) -> Option<String>) -> String {
    if tag != "img" {
        return String::new();
    }
    let dimension = |name: &str| attr(name).and_then(|value| dom::parse_dimension(&value));
    let (width, height) = (dimension("width"), dimension("height"));
    let mut out = String::new();
    for (property, value) in [("width", width), ("height", height)] {
        match value {
            Some(dom::Dimension::Length(px)) => out.push_str(&format!("{property}: {px}px;")),
            Some(dom::Dimension::Percentage(p)) => out.push_str(&format!("{property}: {p}%;")),
            None => {}
        }
    }
    if let (Some(dom::Dimension::Length(w)), Some(dom::Dimension::Length(h))) = (width, height) {
        out.push_str(&format!("aspect-ratio: auto {w} / {h};"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::tests::with_bases;
    use super::*;

    fn tokens(css: &str) -> Vec<Token> {
        css::tokenize(css)
    }

    fn aspect_ratio(css: &str) -> Option<AspectRatio> {
        with_bases(|bases| parse_aspect_ratio(&tokens(css), bases))
    }

    #[test]
    fn aspect_ratio_takes_auto_a_ratio_or_both() {
        let r = |width, height| Ratio { width, height };
        assert_eq!(aspect_ratio("auto"), Some(AspectRatio::Auto));
        assert_eq!(
            aspect_ratio("16 / 9"),
            Some(AspectRatio::Ratio(r(16.0, 9.0)))
        );
        assert_eq!(aspect_ratio("16/9"), Some(AspectRatio::Ratio(r(16.0, 9.0))));
        assert_eq!(aspect_ratio("2"), Some(AspectRatio::Ratio(r(2.0, 1.0))));
        assert_eq!(
            aspect_ratio("auto 3/2"),
            Some(AspectRatio::AutoOr(r(3.0, 2.0)))
        );
        assert_eq!(
            aspect_ratio("3 / 2 auto"),
            Some(AspectRatio::AutoOr(r(3.0, 2.0)))
        );
        assert_eq!(aspect_ratio("0 / 1"), Some(AspectRatio::Ratio(r(0.0, 1.0))));
        assert_eq!(
            aspect_ratio("calc(1 + 1) / 1"),
            Some(AspectRatio::Ratio(r(2.0, 1.0)))
        );
        assert_eq!(aspect_ratio("-1 / 2"), None);
        assert_eq!(aspect_ratio("1 / 2 / 3"), None);
        assert_eq!(aspect_ratio("auto auto"), None);
        assert_eq!(aspect_ratio("1px / 2"), None);
        assert_eq!(aspect_ratio(""), None);
        assert_eq!(r(0.0, 1.0).value(), None);
        assert_eq!(r(3.0, 2.0).value(), Some(1.5));
    }

    #[test]
    fn the_image_keywords_parse() {
        assert_eq!(
            parse_object_fit(&tokens("Scale-Down")),
            Some(ObjectFit::ScaleDown)
        );
        assert_eq!(parse_object_fit(&tokens("cover")), Some(ObjectFit::Cover));
        assert_eq!(parse_object_fit(&tokens("stretch")), None);
        assert_eq!(
            parse_image_orientation(&tokens("none")),
            Some(ImageOrientation::None)
        );
        assert_eq!(parse_image_orientation(&tokens("90deg")), None);
        assert_eq!(
            parse_image_rendering(&tokens("-webkit-optimize-contrast")),
            Some(ImageRendering::CrispEdges)
        );
        assert_eq!(
            parse_image_rendering(&tokens("pixelated")),
            Some(ImageRendering::Pixelated)
        );
        let position = |css: &str| with_bases(|bases| parse_object_position(&tokens(css), bases));
        let position_of = position;
        let position = position_of("right 10px top");
        assert_eq!(
            position, None,
            "three values are background-position's alone"
        );
        let position = position_of("left 10px top 20%").expect("four values");
        assert_eq!(position.y, crate::computed::LengthPercentage::Percent(20.0));
    }

    #[test]
    fn an_img_width_and_height_are_hints() {
        let hints = |pairs: &[(&str, &str)]| {
            let pairs: Vec<(String, String)> = pairs
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect();
            img_hints("img", |name| {
                pairs
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, v)| v.clone())
            })
        };
        assert_eq!(
            hints(&[("width", "300"), ("height", "150.5")]),
            "width: 300px;height: 150.5px;aspect-ratio: auto 300 / 150.5;"
        );
        assert_eq!(
            hints(&[("width", "50%"), ("height", "20")]),
            "width: 50%;height: 20px;"
        );
        assert_eq!(hints(&[("width", "x"), ("height", "20")]), "height: 20px;");
        assert_eq!(hints(&[]), "");
        assert_eq!(img_hints("video", |_| Some("10".into())), "");
        assert!(is_img_hint("img", "width") && !is_img_hint("img", "alt"));
    }
}
