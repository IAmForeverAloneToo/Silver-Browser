//! The border image properties (CSS Backgrounds 3 section 6, M23.3).
//! Each parser takes a declaration's value as the cascade has it and
//! says `None` when the property does not take it.

use css::Token;

use super::{LengthBases, Range, components, expand_1_to_4, length, length_percentage, math};
use crate::calc::{Computed, Want};
use crate::computed::{BorderImage, BorderImageRepeat, BorderImageWidth, Image, Outset, SliceSide};

/// Whether `part` is the keyword `word`, in any case.
fn is(part: &[Token], word: &str) -> bool {
    super::single_ident(part).is_some_and(|w| w.eq_ignore_ascii_case(word))
}

/// A non-negative `<number>`, or a math function's, finite.
fn number(part: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    let value = match math(part, Want::Number, bases, None) {
        Some(Computed::Value(v)) => return Some(Range::NonNegative.clamp(v)),
        Some(_) => return None,
        None => match super::single_token(part)? {
            Token::Number { value, .. } => *value as f32,
            _ => return None,
        },
    };
    (value.is_finite() && value >= 0.0).then_some(value)
}

/// A non-negative `<percentage>`, or a math function's.
fn percentage(part: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    let value = match math(part, Want::Percentage, bases, None) {
        Some(Computed::Percent(p)) => return Some(Range::NonNegative.clamp(p)),
        Some(_) => return None,
        None => match super::single_token(part)? {
            Token::Percentage { value } => *value as f32,
            _ => return None,
        },
    };
    (value.is_finite() && value >= 0.0).then_some(value)
}

/// `border-image-source`: `none`, or an `<image>`. `Some(None)` is
/// `none`, and so is an `image-set()` with no option this engine shows.
fn source(part: &[Token], bases: &LengthBases<'_>) -> Option<Option<Image>> {
    if is(part, "none") {
        return Some(None);
    }
    super::backgrounds::image(part, bases)
}

pub(super) fn parse_border_image_source(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Option<Image>> {
    match components(tokens)[..] {
        [part] => source(part, bases),
        _ => None,
    }
}

/// One side of `border-image-slice`.
fn slice_side(part: &[Token], bases: &LengthBases<'_>) -> Option<SliceSide> {
    number(part, bases)
        .map(SliceSide::Number)
        .or_else(|| percentage(part, bases).map(SliceSide::Percent))
}

/// `border-image-slice` from its components: one to four sides, and
/// `fill` before or after them.
fn slice(parts: &[&[Token]], bases: &LengthBases<'_>) -> Option<([SliceSide; 4], bool)> {
    let (fill, sides) = match parts {
        [first, rest @ ..] if is(first, "fill") => (true, rest),
        [rest @ .., last] if is(last, "fill") => (true, rest),
        _ => (false, parts),
    };
    let sides: Option<Vec<SliceSide>> = sides.iter().map(|p| slice_side(p, bases)).collect();
    Some((expand_1_to_4(&sides?)?, fill))
}

pub(super) fn parse_border_image_slice(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<([SliceSide; 4], bool)> {
    slice(&components(tokens), bases)
}

/// One side of `border-image-width`: a unitless number before a length.
fn width_side(part: &[Token], bases: &LengthBases<'_>) -> Option<BorderImageWidth> {
    if is(part, "auto") {
        return Some(BorderImageWidth::Auto);
    }
    number(part, bases)
        .map(BorderImageWidth::Number)
        .or_else(|| {
            length_percentage(part, bases, Range::NonNegative).map(BorderImageWidth::Length)
        })
}

fn width(parts: &[&[Token]], bases: &LengthBases<'_>) -> Option<[BorderImageWidth; 4]> {
    let sides: Option<Vec<BorderImageWidth>> = parts.iter().map(|p| width_side(p, bases)).collect();
    expand_1_to_4(&sides?)
}

pub(super) fn parse_border_image_width(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<[BorderImageWidth; 4]> {
    width(&components(tokens), bases)
}

/// One side of `border-image-outset`: a unitless number before a length.
fn outset_side(part: &[Token], bases: &LengthBases<'_>) -> Option<Outset> {
    number(part, bases)
        .map(Outset::Number)
        .or_else(|| length(part, bases, Range::NonNegative).map(Outset::Px))
}

fn outset(parts: &[&[Token]], bases: &LengthBases<'_>) -> Option<[Outset; 4]> {
    let sides: Option<Vec<Outset>> = parts.iter().map(|p| outset_side(p, bases)).collect();
    expand_1_to_4(&sides?)
}

pub(super) fn parse_border_image_outset(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<[Outset; 4]> {
    outset(&components(tokens), bases)
}

fn repeat_keyword(part: &[Token]) -> Option<BorderImageRepeat> {
    match super::single_ident(part)?.to_ascii_lowercase().as_str() {
        "stretch" => Some(BorderImageRepeat::Stretch),
        "repeat" => Some(BorderImageRepeat::Repeat),
        "round" => Some(BorderImageRepeat::Round),
        "space" => Some(BorderImageRepeat::Space),
        _ => None,
    }
}

/// `border-image-repeat`: one keyword for both axes, or two.
fn repeat(parts: &[&[Token]]) -> Option<[BorderImageRepeat; 2]> {
    match parts {
        [one] => repeat_keyword(one).map(|r| [r, r]),
        [x, y] => Some([repeat_keyword(x)?, repeat_keyword(y)?]),
        _ => None,
    }
}

pub(super) fn parse_border_image_repeat(tokens: &[Token]) -> Option<[BorderImageRepeat; 2]> {
    repeat(&components(tokens))
}

/// Whether `part` can be in a `border-image-slice`.
fn in_slice(part: &[Token], bases: &LengthBases<'_>) -> bool {
    is(part, "fill") || slice_side(part, bases).is_some()
}

/// The `border-image` shorthand (section 6.6): a source, a slice with a
/// width and an outset after it each after a `/`, and a repeat, in any
/// order, each at most once; what it leaves out is its initial value.
pub(super) fn parse_border_image(tokens: &[Token], bases: &LengthBases<'_>) -> Option<BorderImage> {
    let parts = components(tokens);
    let mut out = BorderImage::default();
    let (mut has_source, mut has_slice, mut has_repeat) = (false, false, false);
    let slash = |part: &[Token]| matches!(part, [Token::Delim('/')]);
    let mut i = 0;
    while i < parts.len() {
        let rest = &parts[i..];
        if !has_source && let Some(image) = source(rest[0], bases) {
            out.source = image;
            has_source = true;
            i += 1;
        } else if !has_repeat && repeat_keyword(rest[0]).is_some() {
            let n = rest
                .iter()
                .take(2)
                .take_while(|p| repeat_keyword(p).is_some())
                .count();
            out.repeat = repeat(&rest[..n])?;
            has_repeat = true;
            i += n;
        } else if !has_slice && in_slice(rest[0], bases) {
            let n = rest.iter().take_while(|p| in_slice(p, bases)).count();
            (out.slice, out.fill) = slice(&rest[..n], bases)?;
            has_slice = true;
            i += n;
            // `/ width`, `/ width / outset` or `/ / outset`.
            if parts.get(i).is_some_and(|p| slash(p)) {
                i += 1;
                let n = parts[i..]
                    .iter()
                    .take_while(|p| !slash(p) && width_side(p, bases).is_some())
                    .count();
                if n > 0 {
                    out.width = width(&parts[i..i + n], bases)?;
                    i += n;
                }
                let more = parts.get(i).is_some_and(|p| slash(p));
                if n == 0 && !more {
                    return None;
                }
                if more {
                    i += 1;
                    let n = parts[i..]
                        .iter()
                        .take_while(|p| outset_side(p, bases).is_some())
                        .count();
                    if n == 0 {
                        return None;
                    }
                    out.outset = outset(&parts[i..i + n], bases)?;
                    i += n;
                }
            }
        } else {
            return None;
        }
    }
    (!parts.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::super::tests::with_bases;
    use super::*;
    use crate::computed::LengthPercentage;

    fn tokens(src: &str) -> Vec<Token> {
        css::tokenize(src)
    }

    fn shorthand(src: &str) -> Option<BorderImage> {
        with_bases(|bases| parse_border_image(&tokens(src), bases))
    }

    #[test]
    fn the_longhands_take_their_sides() {
        with_bases(|b| {
            assert_eq!(
                parse_border_image_slice(&tokens("10 20% fill"), b),
                Some((
                    [
                        SliceSide::Number(10.0),
                        SliceSide::Percent(20.0),
                        SliceSide::Number(10.0),
                        SliceSide::Percent(20.0),
                    ],
                    true
                ))
            );
            assert!(parse_border_image_slice(&tokens("fill 1 2 3"), b).is_some_and(|s| s.1));
            assert_eq!(parse_border_image_slice(&tokens("-1"), b), None);
            assert_eq!(parse_border_image_slice(&tokens("1 fill 2"), b), None);
            assert_eq!(parse_border_image_slice(&tokens("1px"), b), None);
            assert_eq!(
                parse_border_image_slice(&tokens("calc(10% + 5%) calc(2 * 3)"), b),
                Some((
                    [
                        SliceSide::Percent(15.0),
                        SliceSide::Number(6.0),
                        SliceSide::Percent(15.0),
                        SliceSide::Number(6.0),
                    ],
                    false
                ))
            );
            assert_eq!(
                parse_border_image_width(&tokens("0 auto 10px 5%"), b),
                Some([
                    BorderImageWidth::Number(0.0),
                    BorderImageWidth::Auto,
                    BorderImageWidth::Length(LengthPercentage::Px(10.0)),
                    BorderImageWidth::Length(LengthPercentage::Percent(5.0)),
                ])
            );
            assert_eq!(parse_border_image_width(&tokens("-1px"), b), None);
            assert_eq!(
                parse_border_image_outset(&tokens("1 2px"), b),
                Some([
                    Outset::Number(1.0),
                    Outset::Px(2.0),
                    Outset::Number(1.0),
                    Outset::Px(2.0)
                ])
            );
            assert_eq!(parse_border_image_outset(&tokens("10%"), b), None);
            assert_eq!(
                parse_border_image_repeat(&tokens("round space")),
                Some([BorderImageRepeat::Round, BorderImageRepeat::Space])
            );
            assert_eq!(
                parse_border_image_repeat(&tokens("repeat")),
                Some([BorderImageRepeat::Repeat; 2])
            );
            assert_eq!(parse_border_image_repeat(&tokens("a b")), None);
            assert_eq!(parse_border_image_source(&tokens("none"), b), Some(None));
            assert!(
                parse_border_image_source(&tokens("url(a.png)"), b).is_some_and(|s| s.is_some())
            );
        });
    }

    #[test]
    fn the_shorthand_takes_its_parts_in_any_order() {
        let full = shorthand("url(a.png) 10 fill / 2px / 3 round").expect("valid");
        assert!(full.source.is_some() && full.fill);
        assert_eq!(full.slice, [SliceSide::Number(10.0); 4]);
        assert_eq!(
            full.width[0],
            BorderImageWidth::Length(LengthPercentage::Px(2.0))
        );
        assert_eq!(full.outset, [Outset::Number(3.0); 4]);
        assert_eq!(full.repeat, [BorderImageRepeat::Round; 2]);
        let reordered = shorthand("round 10 fill / 2px / 3 url(a.png)").expect("valid");
        assert_eq!(reordered, full);
        // An outset with no width.
        let outset = shorthand("10 / / 1px").expect("valid");
        assert_eq!(outset.width, BorderImage::default().width);
        assert_eq!(outset.outset, [Outset::Px(1.0); 4]);
        // What is left out is the initial value.
        assert_eq!(shorthand("none"), Some(BorderImage::default()));
        let only = shorthand("repeat stretch").expect("valid");
        assert_eq!(only.slice, BorderImage::default().slice);
        assert_eq!(
            only.repeat,
            [BorderImageRepeat::Repeat, BorderImageRepeat::Stretch]
        );
        for invalid in [
            "",
            "10 /",
            "10 / /",
            "/ 2px",
            "url(a.png) url(b.png)",
            "10 round 20",
            "round round round",
            "10 / 2px / 3 / 4",
            "none none",
        ] {
            assert_eq!(shorthand(invalid), None, "{invalid}");
        }
    }
}
