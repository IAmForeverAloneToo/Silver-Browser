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
    match name {
        "width" | "height" => matches!(
            tag,
            "img" | "video" | "canvas" | "embed" | "object" | "marquee" | "input"
        ),
        "hspace" | "vspace" => matches!(
            tag,
            "img" | "embed" | "object" | "iframe" | "marquee" | "input"
        ),
        "type" => tag == "input",
        _ => false,
    }
}

/// The declarations the attributes of embedded content and images are
/// presentational hints for (HTML sections 15.4.3 and 15.5.12), as the
/// text of a declaration block. `width` and `height` map to the
/// dimension properties on an `img`, a `video`, an `embed`, an `object`,
/// a `marquee` and an image button (`input type=image`), each a length
/// or a percentage by the rules for parsing dimension values; on an
/// `img`, a `video` and an image button both lengths together are also
/// `aspect-ratio: auto w / h`, and on a `canvas`, whose attributes are
/// its surface's size and no dimension, both read as non-negative
/// integers are. `hspace` maps to the left and right margins and
/// `vspace` to the top and bottom ones, by the same rules, on an `img`,
/// an `embed`, an `object`, an `iframe`, a `marquee` and an image
/// button. Empty for any other element.
pub(super) fn img_hints(tag: &str, attr: impl Fn(&str) -> Option<String>) -> String {
    let image_button =
        tag == "input" && attr("type").is_some_and(|t| t.trim().eq_ignore_ascii_case("image"));
    if tag == "input" && !image_button {
        return String::new();
    }
    let dimension = |name: &str| attr(name).and_then(|value| dom::parse_dimension(&value));
    let css = |value: dom::Dimension| match value {
        dom::Dimension::Length(px) => format!("{px}px"),
        dom::Dimension::Percentage(p) => format!("{p}%"),
    };
    let mut out = String::new();
    if matches!(
        tag,
        "img" | "video" | "embed" | "object" | "marquee" | "input"
    ) {
        for name in ["width", "height"] {
            if let Some(value) = dimension(name) {
                out.push_str(&format!("{name}: {};", css(value)));
            }
        }
    }
    if matches!(tag, "img" | "video" | "input")
        && let (Some(dom::Dimension::Length(w)), Some(dom::Dimension::Length(h))) =
            (dimension("width"), dimension("height"))
    {
        out.push_str(&format!("aspect-ratio: auto {w} / {h};"));
    }
    if tag == "canvas"
        && let (Some(w), Some(h)) = (
            attr("width").and_then(|v| dom::parse_non_negative_integer(&v)),
            attr("height").and_then(|v| dom::parse_non_negative_integer(&v)),
        )
    {
        out.push_str(&format!("aspect-ratio: auto {w} / {h};"));
    }
    if matches!(
        tag,
        "img" | "embed" | "object" | "iframe" | "marquee" | "input"
    ) {
        if let Some(h) = dimension("hspace").map(css) {
            out.push_str(&format!("margin-left: {h}; margin-right: {h};"));
        }
        if let Some(v) = dimension("vspace").map(css) {
            out.push_str(&format!("margin-top: {v}; margin-bottom: {v};"));
        }
    }
    out
}

/// Whether `name` on an element named `tag` is one of a frame's hints.
pub(crate) fn is_frame_hint(tag: &str, name: &str) -> bool {
    tag == "iframe" && matches!(name, "width" | "height" | "frameborder")
}

/// The presentational hints of an `iframe` (M33.2, HTML sections 15.4.3
/// and 15.4.4): its `width` and `height` map to the dimension
/// properties, with no ratio, and a `frameborder` that parses as the
/// integer 0, or does not parse, takes its border's widths to 0.
pub(super) fn frame_hints(tag: &str, attr: impl Fn(&str) -> Option<String>) -> String {
    if tag != "iframe" {
        return String::new();
    }
    let mut out = String::new();
    for name in ["width", "height"] {
        match attr(name).and_then(|value| dom::parse_dimension(&value)) {
            Some(dom::Dimension::Length(px)) => out.push_str(&format!("{name}: {px}px;")),
            Some(dom::Dimension::Percentage(p)) => out.push_str(&format!("{name}: {p}%;")),
            None => {}
        }
    }
    if attr("frameborder").is_some_and(|value| dom::parse_integer(&value).is_none_or(|n| n == 0)) {
        out.push_str("border-width: 0;");
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
    fn an_iframes_hints_are_its_size_and_frameborder() {
        // M33.2: no ratio, and a frameborder of 0 or of no integer takes
        // the border off.
        let hints = |attrs: &[(&str, &str)]| {
            frame_hints("iframe", |name| {
                attrs
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| (*v).to_owned())
            })
        };
        assert_eq!(
            hints(&[("width", "100"), ("height", "50%")]),
            "width: 100px;height: 50%;"
        );
        assert_eq!(hints(&[("frameborder", "0")]), "border-width: 0;");
        assert_eq!(hints(&[("frameborder", "no")]), "border-width: 0;");
        assert_eq!(hints(&[("frameborder", " 1")]), "");
        assert_eq!(hints(&[]), "");
        assert_eq!(frame_hints("img", |_| Some("0".to_owned())), "");
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
        assert_eq!(hints(&[("width", "200.%")]), "width: 200%;");
        assert_eq!(hints(&[("width", "+200")]), "");
        assert_eq!(img_hints("div", |_| Some("10".into())), "");
        assert!(is_img_hint("img", "width") && !is_img_hint("img", "alt"));
        assert!(is_img_hint("embed", "hspace") && !is_img_hint("canvas", "hspace"));
    }

    #[test]
    fn embedded_contents_sizes_ratios_and_spaces_are_hints() {
        // HTML sections 15.4.3 and 15.5.12, as the reference browser
        // computes them.
        let hints = |tag: &str, pairs: &[(&str, &str)]| {
            img_hints(tag, |name| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| (*v).to_owned())
            })
        };
        let size = [("width", "10"), ("height", "20")];
        assert_eq!(
            hints("video", &size),
            "width: 10px;height: 20px;aspect-ratio: auto 10 / 20;"
        );
        assert_eq!(hints("embed", &size), "width: 10px;height: 20px;");
        assert_eq!(hints("object", &size), "width: 10px;height: 20px;");
        assert_eq!(hints("marquee", &size), "width: 10px;height: 20px;");
        // A canvas keeps its size and takes the ratio, by integers.
        assert_eq!(
            hints("canvas", &[("width", "0.5"), ("height", "1.5")]),
            "aspect-ratio: auto 0 / 1;"
        );
        assert_eq!(hints("canvas", &[("width", "10%")]), "");
        // An input only as an image button.
        assert_eq!(hints("input", &size), "");
        assert_eq!(
            hints(
                "input",
                &[("type", " IMAGE"), ("width", "10"), ("height", "20")]
            ),
            "width: 10px;height: 20px;aspect-ratio: auto 10 / 20;"
        );
        // Spaces are dimensions: a percentage kept, a sign refused, and a
        // video takes none.
        assert_eq!(
            hints("iframe", &[("hspace", "5%"), ("vspace", "3")]),
            "margin-left: 5%; margin-right: 5%;margin-top: 3px; margin-bottom: 3px;"
        );
        assert_eq!(hints("img", &[("hspace", "+3")]), "");
        assert_eq!(hints("video", &[("hspace", "3")]), "");
    }
}
