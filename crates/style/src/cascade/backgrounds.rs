//! The background properties (CSS Backgrounds 3 section 2, M23.3) and
//! `<image>` (CSS Images 3 section 2), with Images 4's `image-set()`
//! (section 2.4). Each parser takes a declaration's value as the cascade
//! has it and says `None` when the property does not take it.

use std::sync::Arc;

use css::Token;

use super::{LengthBases, Range, components, length_percentage, math, single_ident, single_token};
use crate::calc::{Computed, Want};
use crate::computed::{
    Attachment, BackgroundBox, BackgroundSize, Backgrounds, Color, Image, LengthPercentage,
    MAX_IMAGE_LAYERS, Repeat,
};

/// The image types this engine decodes, as a `type()` names them.
const SUPPORTED_TYPES: [&str; 5] = [
    "image/png",
    "image/apng",
    "image/jpeg",
    "image/gif",
    "image/webp",
];

/// The items of a comma-separated list, each its components; `None`
/// when an item is empty or there are more than [`MAX_IMAGE_LAYERS`].
fn items(tokens: &[Token]) -> Option<Vec<Vec<&[Token]>>> {
    let mut out: Vec<Vec<&[Token]>> = vec![Vec::new()];
    for part in components(tokens) {
        if matches!(part, [Token::Comma]) {
            out.push(Vec::new());
        } else {
            out.last_mut().expect("one at least").push(part);
        }
    }
    (out.len() <= MAX_IMAGE_LAYERS && out.iter().all(|item| !item.is_empty())).then_some(out)
}

/// Whether `part` is the keyword `word`, in any case.
fn is(part: &[Token], word: &str) -> bool {
    single_ident(part).is_some_and(|w| w.eq_ignore_ascii_case(word))
}

/// An `<image>`: a `url()`, a gradient, or an `image-set()`, which is
/// `None` when it has no option this engine can show, and so shows
/// nothing.
pub(super) fn image(part: &[Token], bases: &LengthBases<'_>) -> Option<Option<Image>> {
    if let Some(gradient) = super::gradients::gradient(part, bases) {
        return Some(Some(Image::Gradient(Arc::new(gradient))));
    }
    match part {
        [Token::Url { value, bad: false }] => Some(Some(Image::Url {
            url: Arc::from(value.as_str()),
            resolution: 1.0,
        })),
        [Token::Function(name), ..]
            if name.eq_ignore_ascii_case("image-set")
                || name.eq_ignore_ascii_case("-webkit-image-set") =>
        {
            image_set(part, bases)
        }
        _ => None,
    }
}

/// A `<resolution>`, in image pixels a CSS pixel, never negative.
fn resolution(part: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    let dppx = match math(part, Want::Resolution, bases, None) {
        Some(Computed::Value(v)) => v,
        Some(_) => return None,
        None => match single_token(part)? {
            Token::Dimension { value, unit } => {
                value
                    * match unit.to_ascii_lowercase().as_str() {
                        "x" | "dppx" => 1.0,
                        "dpi" => 1.0 / 96.0,
                        "dpcm" => 2.54 / 96.0,
                        _ => return None,
                    }
            }
            _ => return None,
        },
    };
    let dppx = dppx as f32;
    (dppx.is_finite() && dppx >= 0.0).then_some(dppx)
}

/// `type(<string>)`: whether the type names an image this engine reads,
/// by its essence.
fn supported_type(part: &[Token]) -> Option<bool> {
    let [Token::Function(name), inside @ .., Token::CloseParen] = part else {
        return None;
    };
    if !name.eq_ignore_ascii_case("type") {
        return None;
    }
    let [Token::String { value, bad: false }] = super::trim(inside) else {
        return None;
    };
    let essence = value.split(';').next().unwrap_or_default().trim();
    Some(SUPPORTED_TYPES.contains(&essence.to_ascii_lowercase().as_str()))
}

/// `image-set( <image-set-option># )` (Images 4 section 2.4): each option
/// an image, a URL as a string or a `url()`, then a resolution and a
/// `type()` in either order, each optional, a missing resolution being
/// 1x. The image is the option the reference browser takes at 1x: of
/// those whose type, if given, this engine reads and whose resolution is
/// not zero, the one of the smallest resolution at or above 1x, else of
/// the largest, the first of equal ones. Its resolution is the image's
/// natural resolution.
fn image_set(part: &[Token], bases: &LengthBases<'_>) -> Option<Option<Image>> {
    let [_, inside @ .., Token::CloseParen] = part else {
        return None;
    };
    let mut chosen: Option<(Image, f32)> = None;
    for option in items(inside)? {
        let (first, rest) = option.split_first()?;
        let image = match first {
            [Token::Url { value, bad: false }] | [Token::String { value, bad: false }] => {
                Image::Url {
                    url: Arc::from(value.as_str()),
                    resolution: 1.0,
                }
            }
            part => Image::Gradient(Arc::new(super::gradients::gradient(part, bases)?)),
        };
        let (mut dppx, mut supported) = (None, None);
        for part in rest {
            if let Some(kind) = supported_type(part).filter(|_| supported.is_none()) {
                supported = Some(kind);
            } else if dppx.is_none() {
                dppx = Some(resolution(part, bases)?);
            } else {
                return None;
            }
        }
        let dppx = dppx.unwrap_or(1.0);
        if supported == Some(false) || dppx == 0.0 {
            continue;
        }
        let better = match &chosen {
            None => true,
            Some((_, best)) if *best >= 1.0 => dppx >= 1.0 && dppx < *best,
            Some((_, best)) => dppx > *best,
        };
        if better {
            chosen = Some((image, dppx));
        }
    }
    Some(chosen.map(|(image, resolution)| match image {
        Image::Url { url, .. } => Image::Url { url, resolution },
        gradient => gradient,
    }))
}

/// `background-image`: a list of `none` or an `<image>`.
pub(super) fn parse_background_image(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Vec<Option<Image>>> {
    items(tokens)?
        .into_iter()
        .map(|item| match item.as_slice() {
            [part] if is(part, "none") => Some(None),
            [part] => image(part, bases),
            _ => None,
        })
        .collect()
}

/// One axis's repeat keyword.
fn repeat_keyword(part: &[Token]) -> Option<Repeat> {
    match single_ident(part)?.to_ascii_lowercase().as_str() {
        "repeat" => Some(Repeat::Repeat),
        "space" => Some(Repeat::Space),
        "round" => Some(Repeat::Round),
        "no-repeat" => Some(Repeat::NoRepeat),
        _ => None,
    }
}

/// A `<repeat-style>`: `repeat-x`, `repeat-y`, or one or two keywords,
/// one for both axes.
fn repeat(parts: &[&[Token]]) -> Option<[Repeat; 2]> {
    match parts {
        [one] if is(one, "repeat-x") => Some([Repeat::Repeat, Repeat::NoRepeat]),
        [one] if is(one, "repeat-y") => Some([Repeat::NoRepeat, Repeat::Repeat]),
        [one] => repeat_keyword(one).map(|r| [r, r]),
        [x, y] => Some([repeat_keyword(x)?, repeat_keyword(y)?]),
        _ => None,
    }
}

/// A `<bg-position>`, each axis from its start edge.
fn position(parts: &[&[Token]], bases: &LengthBases<'_>) -> Option<[LengthPercentage; 2]> {
    super::effects::background_position(parts, bases).map(|(x, y)| [x, y])
}

/// One side of a `<bg-size>`: `auto`, or a length or percentage never
/// negative.
fn size_side(part: &[Token], bases: &LengthBases<'_>) -> Option<Option<LengthPercentage>> {
    if is(part, "auto") {
        return Some(None);
    }
    length_percentage(part, bases, Range::NonNegative).map(Some)
}

/// A `<bg-size>`: `cover`, `contain`, or one or two sides, a missing
/// height `auto`.
fn size(parts: &[&[Token]], bases: &LengthBases<'_>) -> Option<BackgroundSize> {
    match parts {
        [one] if is(one, "cover") => Some(BackgroundSize::Cover),
        [one] if is(one, "contain") => Some(BackgroundSize::Contain),
        [one] => Some(BackgroundSize::Sized(size_side(one, bases)?, None)),
        [w, h] => Some(BackgroundSize::Sized(
            size_side(w, bases)?,
            size_side(h, bases)?,
        )),
        _ => None,
    }
}

/// A `<visual-box>` of `background-origin` and `background-clip`.
fn background_box(part: &[Token]) -> Option<BackgroundBox> {
    match single_ident(part)?.to_ascii_lowercase().as_str() {
        "border-box" => Some(BackgroundBox::BorderBox),
        "padding-box" => Some(BackgroundBox::PaddingBox),
        "content-box" => Some(BackgroundBox::ContentBox),
        _ => None,
    }
}

/// An `<attachment>`.
fn attachment(part: &[Token]) -> Option<Attachment> {
    match single_ident(part)?.to_ascii_lowercase().as_str() {
        "scroll" => Some(Attachment::Scroll),
        "fixed" => Some(Attachment::Fixed),
        "local" => Some(Attachment::Local),
        _ => None,
    }
}

/// A list of what `one` reads from each item's components.
fn list<T>(tokens: &[Token], one: impl Fn(&[&[Token]]) -> Option<T>) -> Option<Vec<T>> {
    items(tokens)?.iter().map(|item| one(item)).collect()
}

/// `background-repeat`.
pub(super) fn parse_background_repeat(tokens: &[Token]) -> Option<Vec<[Repeat; 2]>> {
    list(tokens, repeat)
}

/// `background-position`.
pub(super) fn parse_background_position(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Vec<[LengthPercentage; 2]>> {
    list(tokens, |parts| position(parts, bases))
}

/// `background-position-x` or, where not `x`, `-y` (gap batch E).
pub(super) fn parse_background_position_axis(
    tokens: &[Token],
    bases: &LengthBases<'_>,
    x: bool,
) -> Option<Vec<LengthPercentage>> {
    list(tokens, |parts| {
        super::effects::position_axis(parts, bases, x)
    })
}

/// `positions` with one axis's values `axis` (0 along x) taken from
/// `values`, each list repeated to the longer's length as layers repeat
/// their lists (Backgrounds 3 section 2.2).
pub(super) fn with_axis(
    positions: &[[LengthPercentage; 2]],
    values: Vec<LengthPercentage>,
    axis: usize,
) -> Vec<[LengthPercentage; 2]> {
    let n = positions.len().max(values.len());
    (0..n)
        .map(|i| {
            let mut pair = positions[i % positions.len().max(1)].clone();
            pair[axis] = values[i % values.len()].clone();
            pair
        })
        .collect()
}

/// `background-size`.
pub(super) fn parse_background_size(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Vec<BackgroundSize>> {
    list(tokens, |parts| size(parts, bases))
}

/// `background-origin` and `background-clip`.
pub(super) fn parse_background_box(tokens: &[Token]) -> Option<Vec<BackgroundBox>> {
    list(tokens, |parts| match parts {
        [one] => background_box(one),
        _ => None,
    })
}

/// `background-attachment`.
pub(super) fn parse_background_attachment(tokens: &[Token]) -> Option<Vec<Attachment>> {
    list(tokens, |parts| match parts {
        [one] => attachment(one),
        _ => None,
    })
}

/// The `background` shorthand (section 3.10): layers, each an image, a
/// position with a size after a `/`, a repeat, an attachment and up to
/// two boxes, in any order, each at most once, the last layer a colour
/// too. One box is the origin and the clip; two the origin, then the
/// clip. What a layer leaves out is its initial value, and the colour
/// transparent.
pub(super) fn parse_background(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<(Color, Backgrounds)> {
    let items = items(tokens)?;
    let last = items.len() - 1;
    let initial = Backgrounds::default();
    let mut out = Backgrounds {
        image: Vec::new(),
        repeat: Vec::new(),
        position: Vec::new(),
        size: Vec::new(),
        origin: Vec::new(),
        clip: Vec::new(),
        attachment: Vec::new(),
    };
    let mut color = Color::TRANSPARENT;
    for (index, item) in items.iter().enumerate() {
        let mut image_of = None;
        let mut repeat_of = None;
        let mut position_of = None;
        let mut size_of = None;
        let mut attachment_of = None;
        let mut boxes: Vec<BackgroundBox> = Vec::new();
        let mut color_of = None;
        let mut at = 0;
        while at < item.len() {
            let part = item[at];
            if image_of.is_none() {
                if is(part, "none") {
                    image_of = Some(None);
                    at += 1;
                    continue;
                }
                if let Some(found) = image(part, bases) {
                    image_of = Some(found);
                    at += 1;
                    continue;
                }
            }
            if position_of.is_none()
                && let Some((found, used)) = (1..=4)
                    .rev()
                    .filter(|n| at + n <= item.len())
                    .find_map(|n| position(&item[at..at + n], bases).map(|p| (p, n)))
            {
                position_of = Some(found);
                at += used;
                if matches!(item.get(at), Some([Token::Delim('/')])) {
                    let (found, used) = (1..=2)
                        .rev()
                        .filter(|n| at + 1 + n <= item.len())
                        .find_map(|n| size(&item[at + 1..at + 1 + n], bases).map(|s| (s, n)))?;
                    size_of = Some(found);
                    at += 1 + used;
                }
                continue;
            }
            if repeat_of.is_none()
                && let Some((found, used)) = (1..=2)
                    .rev()
                    .filter(|n| at + n <= item.len())
                    .find_map(|n| repeat(&item[at..at + n]).map(|r| (r, n)))
            {
                repeat_of = Some(found);
                at += used;
                continue;
            }
            if attachment_of.is_none()
                && let Some(found) = attachment(part)
            {
                attachment_of = Some(found);
                at += 1;
                continue;
            }
            if boxes.len() < 2
                && let Some(found) = background_box(part)
            {
                boxes.push(found);
                at += 1;
                continue;
            }
            if index == last
                && color_of.is_none()
                && let Some(found) = super::color_value(part, bases.colors())
            {
                color_of = Some(found);
                at += 1;
                continue;
            }
            return None;
        }
        out.image.push(image_of.unwrap_or(None));
        out.repeat.push(repeat_of.unwrap_or(initial.repeat[0]));
        out.position
            .push(position_of.unwrap_or_else(|| initial.position[0].clone()));
        out.size
            .push(size_of.unwrap_or_else(|| initial.size[0].clone()));
        out.attachment
            .push(attachment_of.unwrap_or(initial.attachment[0]));
        out.origin
            .push(boxes.first().copied().unwrap_or(initial.origin[0]));
        out.clip.push(
            boxes
                .get(1)
                .or(boxes.first())
                .copied()
                .unwrap_or(initial.clip[0]),
        );
        if let Some(found) = color_of {
            color = found;
        }
    }
    Some((color, out))
}

#[cfg(test)]
mod tests {
    use super::super::tests::with_bases;
    use super::*;

    fn tokens(css: &str) -> Vec<Token> {
        css::tokenize(css)
    }

    fn url(url: &str, resolution: f32) -> Option<Image> {
        Some(Image::Url {
            url: Arc::from(url),
            resolution,
        })
    }

    fn images(css: &str) -> Option<Vec<Option<Image>>> {
        with_bases(|bases| parse_background_image(&tokens(css), bases))
    }

    #[test]
    fn an_image_set_takes_the_option_the_reference_browser_takes() {
        let set = |css: &str| images(&format!("image-set({css})")).map(|mut v| v.remove(0));
        assert_eq!(set("\"a.png\" 0.5x, \"b.png\" 2x"), Some(url("b.png", 2.0)));
        assert_eq!(set("\"a.png\" 1x, \"b.png\" 1x"), Some(url("a.png", 1.0)));
        assert_eq!(set("\"a.png\" 2x, \"b.png\" 3x"), Some(url("a.png", 2.0)));
        assert_eq!(set("\"a.png\" 1.5x, url(b.png)"), Some(url("b.png", 1.0)));
        assert_eq!(
            set("\"a.png\" 0.25x, \"b.png\" 0.5x"),
            Some(url("b.png", 0.5))
        );
        assert_eq!(set("\"a.png\" 192dpi"), Some(url("a.png", 2.0)));
        assert_eq!(set("\"a.png\" calc(1x + 1x)"), Some(url("a.png", 2.0)));
        // An unsupported type is skipped, a supported one taken whatever
        // its case and parameters.
        assert_eq!(
            set("\"a.avif\" type(\"image/avif\"), \"b.png\" type(\"Image/PNG; x=1\")"),
            Some(url("b.png", 1.0))
        );
        assert_eq!(set("\"a.avif\" type(\"image/avif\")"), Some(None));
        assert_eq!(
            set("\"a.png\" type(\"image/png\") 2x"),
            Some(url("a.png", 2.0))
        );
        assert_eq!(set("\"a.png\" -1x"), None);
        assert_eq!(set("\"a.png\" 1x 2x"), None);
        assert_eq!(set("1x \"a.png\""), None);
        assert_eq!(set("image-set(\"a.png\")"), None);
        assert_eq!(
            images("-webkit-image-set(\"a.png\" 1x)"),
            Some(vec![url("a.png", 1.0)])
        );
    }

    #[test]
    fn a_gradient_is_an_image_and_an_image_sets_option() {
        let images = images("linear-gradient(red, blue), image-set(radial-gradient(red, blue) 2x)")
            .expect("valid");
        assert!(matches!(images[0], Some(Image::Gradient(_))));
        assert!(matches!(images[1], Some(Image::Gradient(_))));
        assert_eq!(images.len(), 2);
    }

    #[test]
    fn the_background_lists_parse_item_by_item() {
        assert_eq!(
            images("url(a.png), none, url(b.png)"),
            Some(vec![url("a.png", 1.0), None, url("b.png", 1.0)])
        );
        assert_eq!(images("url(a.png),"), None);
        assert_eq!(images(&vec!["none"; MAX_IMAGE_LAYERS + 1].join(",")), None);
        assert_eq!(
            parse_background_repeat(&tokens("repeat-x, space round, no-repeat")),
            Some(vec![
                [Repeat::Repeat, Repeat::NoRepeat],
                [Repeat::Space, Repeat::Round],
                [Repeat::NoRepeat; 2],
            ])
        );
        assert_eq!(parse_background_repeat(&tokens("repeat-x repeat")), None);
        let position =
            |css: &str| with_bases(|bases| parse_background_position(&tokens(css), bases));
        let p = LengthPercentage::Percent;
        assert_eq!(position("center"), Some(vec![[p(50.0), p(50.0)]]));
        assert_eq!(position("right 10% top"), Some(vec![[p(90.0), p(0.0)]]));
        assert_eq!(position("top 20% center"), Some(vec![[p(50.0), p(20.0)]]));
        assert_eq!(position("left bottom 25%"), Some(vec![[p(0.0), p(75.0)]]));
        assert_eq!(
            position("center top 10px"),
            Some(vec![[p(50.0), LengthPercentage::Px(10.0)]])
        );
        assert_eq!(position("left 10% right"), None);
        // Its longhands (gap batch E): an axis each, a list each.
        let axis = |css: &str, x: bool| {
            with_bases(|bases| parse_background_position_axis(&tokens(css), bases, x))
        };
        assert_eq!(axis("right 10%", true), Some(vec![p(90.0)]));
        assert_eq!(axis("center, 20%", false), Some(vec![p(50.0), p(20.0)]));
        assert_eq!(axis("left", true), Some(vec![p(0.0)]));
        assert_eq!(axis("top", true), None);
        assert_eq!(axis("left 10px 5px", true), None);
        let pair = |x: f32, y: f32| [p(x), p(y)];
        assert_eq!(
            with_axis(&[pair(1.0, 2.0)], vec![p(7.0), p(8.0)], 0),
            vec![pair(7.0, 2.0), pair(8.0, 2.0)]
        );
        assert_eq!(
            with_axis(&[pair(1.0, 2.0), pair(3.0, 4.0)], vec![p(9.0)], 1),
            vec![pair(1.0, 9.0), pair(3.0, 9.0)]
        );
        assert_eq!(position("10px 20px 30px"), None);
        let size = |css: &str| with_bases(|bases| parse_background_size(&tokens(css), bases));
        assert_eq!(
            size("cover, 10px, auto 50%"),
            Some(vec![
                BackgroundSize::Cover,
                BackgroundSize::Sized(Some(LengthPercentage::Px(10.0)), None),
                BackgroundSize::Sized(None, Some(p(50.0))),
            ])
        );
        assert_eq!(size("-1px"), None);
        assert_eq!(size("cover contain"), None);
        assert_eq!(
            parse_background_box(&tokens("content-box, border-box")),
            Some(vec![BackgroundBox::ContentBox, BackgroundBox::BorderBox])
        );
        assert_eq!(
            parse_background_attachment(&tokens("fixed, local")),
            Some(vec![Attachment::Fixed, Attachment::Local])
        );
    }

    #[test]
    fn the_shorthand_takes_each_part_once_in_any_order() {
        let background = |css: &str| with_bases(|bases| parse_background(&tokens(css), bases));
        let (color, b) =
            background("url(a.png) no-repeat center / 50% content-box fixed red").expect("valid");
        assert_eq!(color, Color::rgb8(255, 0, 0));
        assert_eq!(b.image, vec![url("a.png", 1.0)]);
        assert_eq!(b.repeat, vec![[Repeat::NoRepeat; 2]]);
        let p = LengthPercentage::Percent;
        assert_eq!(b.position, vec![[p(50.0), p(50.0)]]);
        assert_eq!(b.size, vec![BackgroundSize::Sized(Some(p(50.0)), None)]);
        assert_eq!(b.origin, vec![BackgroundBox::ContentBox]);
        assert_eq!(b.clip, vec![BackgroundBox::ContentBox]);
        assert_eq!(b.attachment, vec![Attachment::Fixed]);
        // Two boxes: the origin, then the clip; and two layers.
        let (color, b) =
            background("padding-box content-box, url(b.png) left 10px top 20px").expect("valid");
        assert_eq!(color, Color::TRANSPARENT);
        assert_eq!(b.origin[0], BackgroundBox::PaddingBox);
        assert_eq!(b.clip[0], BackgroundBox::ContentBox);
        assert_eq!(b.image, vec![None, url("b.png", 1.0)]);
        assert_eq!(
            b.position[1],
            [LengthPercentage::Px(10.0), LengthPercentage::Px(20.0)]
        );
        let (color, b) = background("left top green").expect("valid");
        assert_eq!((color, b.image.len()), (Color::rgb8(0, 128, 0), 1));
        assert!(
            background("green, url(a.png)").is_none(),
            "colour in the last layer only"
        );
        assert!(background("none none").is_none());
        assert!(background("center / ").is_none());
        assert!(background("/ 10px").is_none());
        assert!(background("red blue").is_none());
        assert!(background("border-box border-box border-box").is_none());
    }
}
