//! Gradients (CSS Images 3 section 3, M23.3): `linear-gradient()`,
//! `radial-gradient()`, their `repeating-` forms, and the reference
//! browser's `-webkit-linear-gradient()` and
//! `-webkit-repeating-linear-gradient()`, whose direction names the side
//! the line starts from and whose angles run from the right,
//! anticlockwise. A gradient's computed value is as written but for
//! its colours, computed, and a double position, which is two stops.
//! Images 4's `conic-gradient()` and `repeating-conic-gradient()`, and a
//! `<color-interpolation-method>` beside any gradient's direction or
//! shape, are taken since gap batch E; the other Images 4 additions are
//! not (G37).

use css::Token;

use super::{LengthBases, Range, components, length_percentage, single_ident};
use crate::computed::{
    Gradient, GradientItem, GradientShape, LengthPercentage, LineDirection, MAX_GRADIENT_STOPS,
    RadialSize,
};

/// The arguments of a function, each its components, split at the
/// top-level commas; `None` when one is empty.
fn arguments(inside: &[Token]) -> Option<Vec<Vec<&[Token]>>> {
    let mut out: Vec<Vec<&[Token]>> = vec![Vec::new()];
    for part in components(inside) {
        if matches!(part, [Token::Comma]) {
            out.push(Vec::new());
        } else {
            out.last_mut().expect("one at least").push(part);
        }
    }
    out.iter().all(|a| !a.is_empty()).then_some(out)
}

/// Whether `part` is the keyword `word`, in any case.
fn is(part: &[Token], word: &str) -> bool {
    single_ident(part).is_some_and(|w| w.eq_ignore_ascii_case(word))
}

/// A gradient function, `part` being one component; `None` for any
/// other value, or one whose arguments it does not take.
pub(super) fn gradient(part: &[Token], bases: &LengthBases<'_>) -> Option<Gradient> {
    let [Token::Function(name), inside @ .., Token::CloseParen] = part else {
        return None;
    };
    let lower = name.to_ascii_lowercase();
    let conic = lower.ends_with("conic-gradient");
    let (linear, repeating, legacy) = match lower.as_str() {
        "linear-gradient" => (true, false, false),
        "repeating-linear-gradient" => (true, true, false),
        "radial-gradient" | "conic-gradient" => (false, false, false),
        "repeating-radial-gradient" | "repeating-conic-gradient" => (false, true, false),
        "-webkit-linear-gradient" => (true, false, true),
        "-webkit-repeating-linear-gradient" => (true, true, true),
        _ => return None,
    };
    let mut args = arguments(inside)?;
    // A `<color-interpolation-method>` beside the first argument's
    // direction or shape, or alone as the first argument.
    let mut method = None;
    if !legacy && let Some(at) = args[0].iter().position(|p| is(p, "in")) {
        let after = &args[0][at + 1..];
        let words: Vec<&Token> = after.iter().take(3).filter_map(|p| p.first()).collect();
        let (found, used) = match words.as_slice() {
            [space, how, hue]
                if crate::color::parse_method(&[&Token::Ident("in".into()), space, how, hue])
                    .is_some() =>
            {
                (
                    crate::color::parse_method(&[&Token::Ident("in".into()), space, how, hue]),
                    3,
                )
            }
            [space, ..] => (
                crate::color::parse_method(&[&Token::Ident("in".into()), space]),
                1,
            ),
            [] => (None, 0),
        };
        let found = found?;
        // The method stands before or after the rest, not inside it.
        if at != 0 && at + 1 + used != args[0].len() {
            return None;
        }
        method = Some(found);
        args[0].drain(at..at + 1 + used);
        if args[0].is_empty() {
            args.remove(0);
            if args.is_empty() {
                return None;
            }
        }
    }
    let (shape, stops) = if conic {
        match conic_start(&args[0], bases) {
            Some(shape) => (shape, &args[1..]),
            None => (
                GradientShape::Conic {
                    from: 0.0,
                    center: [
                        LengthPercentage::Percent(50.0),
                        LengthPercentage::Percent(50.0),
                    ],
                },
                &args[..],
            ),
        }
    } else if linear {
        match direction(&args[0], legacy, bases) {
            Some(direction) => (GradientShape::Linear(direction), &args[1..]),
            None => (
                GradientShape::Linear(LineDirection::Angle(180.0)),
                &args[..],
            ),
        }
    } else {
        match radial(&args[0], bases) {
            Some(shape) => (shape, &args[1..]),
            None => (
                GradientShape::Radial {
                    ellipse: true,
                    size: RadialSize::FarthestCorner,
                    center: [
                        LengthPercentage::Percent(50.0),
                        LengthPercentage::Percent(50.0),
                    ],
                },
                &args[..],
            ),
        }
    };
    Some(Gradient {
        shape,
        repeating,
        items: stop_list(stops, bases, conic)?,
        method,
    })
}

/// A conic gradient's first argument (Images 4 section 3.3): `from` an
/// angle, `at` a position, either or both in that order.
fn conic_start(parts: &[&[Token]], bases: &LengthBases<'_>) -> Option<GradientShape> {
    let mut from = 0.0;
    let mut rest = parts;
    if let [word, angle, tail @ ..] = rest
        && is(word, "from")
    {
        from = super::effects::angle(angle, true, bases)?;
        rest = tail;
    }
    let mut center = [
        LengthPercentage::Percent(50.0),
        LengthPercentage::Percent(50.0),
    ];
    if let [word, position @ ..] = rest
        && is(word, "at")
    {
        let (x, y) = super::effects::position(position, bases, true)?;
        center = [x, y];
        rest = &[];
    }
    (rest.is_empty() && rest.len() < parts.len()).then_some(GradientShape::Conic { from, center })
}

/// A linear gradient's direction: an angle, or `to` a side or corner;
/// in the legacy form a side or corner alone, where the line starts,
/// or an angle from the right, anticlockwise.
fn direction(parts: &[&[Token]], legacy: bool, bases: &LengthBases<'_>) -> Option<LineDirection> {
    if let [one] = parts
        && let Some(degrees) = super::effects::angle(one, true, bases)
    {
        return Some(LineDirection::Angle(if legacy {
            90.0 - degrees
        } else {
            degrees
        }));
    }
    let sides = match parts {
        [to, sides @ ..] if !legacy && is(to, "to") => sides,
        sides if legacy => sides,
        _ => return None,
    };
    let mut horizontal: Option<bool> = None;
    let mut vertical: Option<bool> = None;
    for side in sides {
        let word = single_ident(side)?.to_ascii_lowercase();
        match word.as_str() {
            "left" | "right" if horizontal.is_none() => horizontal = Some(word == "right"),
            "top" | "bottom" if vertical.is_none() => vertical = Some(word == "bottom"),
            _ => return None,
        }
    }
    // The legacy form names where the line starts: it points the other
    // way.
    let (horizontal, vertical) = if legacy {
        (horizontal.map(|r| !r), vertical.map(|b| !b))
    } else {
        (horizontal, vertical)
    };
    match (horizontal, vertical) {
        (Some(right), Some(bottom)) => Some(LineDirection::Corner { right, bottom }),
        (Some(right), None) => Some(LineDirection::Angle(if right { 90.0 } else { 270.0 })),
        (None, Some(bottom)) => Some(LineDirection::Angle(if bottom { 180.0 } else { 0.0 })),
        (None, None) => None,
    }
}

/// A radial gradient's first argument: its shape and size, either or
/// both, then `at` a position, or the position alone.
fn radial(parts: &[&[Token]], bases: &LengthBases<'_>) -> Option<GradientShape> {
    let at = parts.iter().position(|p| is(p, "at"));
    let (before, center) = match at {
        Some(at) => {
            let (x, y) = super::effects::position(&parts[at + 1..], bases, true)?;
            (&parts[..at], [x, y])
        }
        None => (
            parts,
            [
                LengthPercentage::Percent(50.0),
                LengthPercentage::Percent(50.0),
            ],
        ),
    };
    if at.is_none() && before.is_empty() {
        return None;
    }
    let mut shape: Option<bool> = None;
    let mut extent: Option<RadialSize> = None;
    let mut lengths: Vec<LengthPercentage> = Vec::new();
    for part in before {
        let word = single_ident(part).map(str::to_ascii_lowercase);
        match word.as_deref() {
            Some("circle") if shape.is_none() => shape = Some(false),
            Some("ellipse") if shape.is_none() => shape = Some(true),
            Some("closest-side") if extent.is_none() && lengths.is_empty() => {
                extent = Some(RadialSize::ClosestSide)
            }
            Some("farthest-side") if extent.is_none() && lengths.is_empty() => {
                extent = Some(RadialSize::FarthestSide)
            }
            Some("closest-corner") if extent.is_none() && lengths.is_empty() => {
                extent = Some(RadialSize::ClosestCorner)
            }
            Some("farthest-corner") if extent.is_none() && lengths.is_empty() => {
                extent = Some(RadialSize::FarthestCorner)
            }
            None if extent.is_none() && lengths.len() < 2 => {
                lengths.push(length_percentage(part, bases, Range::NonNegative)?)
            }
            _ => return None,
        }
    }
    // A shape and its size agree: a circle has one length, never a
    // percentage; an ellipse two.
    let (ellipse, size) = match (shape, extent, lengths.as_slice()) {
        (shape, Some(extent), []) => (shape.unwrap_or(true), extent),
        (Some(false) | None, None, [radius]) => {
            let LengthPercentage::Px(px) = radius else {
                return None;
            };
            (false, RadialSize::Circle(*px))
        }
        (Some(true) | None, None, [rx, ry]) => (true, RadialSize::Ellipse(rx.clone(), ry.clone())),
        (shape, None, []) => (shape.unwrap_or(true), RadialSize::FarthestCorner),
        _ => return None,
    };
    Some(GradientShape::Radial {
        ellipse,
        size,
        center,
    })
}

/// A `<color-stop-list>`: stops, a colour with a position or two, in
/// either order, or none, and transition hints, a position alone, each
/// between two stops; one stop at least, [`MAX_GRADIENT_STOPS`] at most.
fn stop_list(
    args: &[Vec<&[Token]>],
    bases: &LengthBases<'_>,
    conic: bool,
) -> Option<Vec<GradientItem>> {
    let mut items: Vec<GradientItem> = Vec::new();
    // A conic gradient's positions are angles, as pixels of degrees, or
    // percentages of a turn.
    let position = |part: &[Token]| {
        if conic {
            match super::single_token(part) {
                Some(Token::Percentage { value }) => Some(LengthPercentage::Percent(*value as f32)),
                _ => super::effects::angle(part, true, bases).map(LengthPercentage::Px),
            }
        } else {
            length_percentage(part, bases, Range::All)
        }
    };
    for arg in args {
        let color_at = arg
            .iter()
            .position(|part| super::color_value(part, bases.colors()).is_some());
        match (color_at, arg.len()) {
            (None, 1) => {
                // A hint between two stops, never two in a row.
                if !matches!(items.last(), Some(GradientItem::Stop(..))) {
                    return None;
                }
                items.push(GradientItem::Hint(position(arg[0])?));
            }
            (Some(at), 1..=3) => {
                let color = super::color_value(arg[at], bases.colors())?;
                let positions: Vec<&[Token]> = arg
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != at)
                    .map(|(_, part)| *part)
                    .collect();
                // The colour first or last, the positions together.
                if at != 0 && at != arg.len() - 1 {
                    return None;
                }
                if positions.is_empty() {
                    items.push(GradientItem::Stop(color, None));
                }
                for part in positions {
                    items.push(GradientItem::Stop(color, Some(position(part)?)));
                }
            }
            _ => return None,
        }
    }
    let stops = items
        .iter()
        .filter(|i| matches!(i, GradientItem::Stop(..)))
        .count();
    (matches!(items.last(), Some(GradientItem::Stop(..)))
        && (1..=MAX_GRADIENT_STOPS).contains(&stops))
    .then_some(items)
}

#[cfg(test)]
mod tests {
    use super::super::tests::with_bases;
    use super::*;
    use crate::computed::Color;

    fn parse(css: &str) -> Option<Gradient> {
        let tokens = css::tokenize(css);
        let parts = components(&tokens);
        let [part] = parts.as_slice() else {
            return None;
        };
        with_bases(|bases| gradient(part, bases))
    }

    const RED: Color = Color::rgb8(255, 0, 0);
    const BLUE: Color = Color::rgb8(0, 0, 255);

    #[test]
    fn a_linear_gradients_direction_is_an_angle_or_a_side_or_corner() {
        let direction = |css: &str| match parse(css)?.shape {
            GradientShape::Linear(d) => Some(d),
            _ => None,
        };
        assert_eq!(
            direction("linear-gradient(red, blue)"),
            Some(LineDirection::Angle(180.0))
        );
        assert_eq!(
            direction("linear-gradient(45deg, red, blue)"),
            Some(LineDirection::Angle(45.0))
        );
        assert_eq!(
            direction("linear-gradient(0.25turn, red, blue)"),
            Some(LineDirection::Angle(90.0))
        );
        assert_eq!(
            direction("linear-gradient(0, red, blue)"),
            Some(LineDirection::Angle(0.0))
        );
        assert_eq!(
            direction("linear-gradient(to left, red, blue)"),
            Some(LineDirection::Angle(270.0))
        );
        assert_eq!(
            direction("linear-gradient(to top right, red, blue)"),
            Some(LineDirection::Corner {
                right: true,
                bottom: false
            })
        );
        assert_eq!(
            direction("linear-gradient(to bottom left, red, blue)"),
            Some(LineDirection::Corner {
                right: false,
                bottom: true
            })
        );
        // The legacy form names where the line starts, its angles from
        // the right, anticlockwise.
        assert_eq!(
            direction("-webkit-linear-gradient(top, red, blue)"),
            Some(LineDirection::Angle(180.0))
        );
        assert_eq!(
            direction("-webkit-linear-gradient(0deg, red, blue)"),
            Some(LineDirection::Angle(90.0))
        );
        assert_eq!(
            direction("-webkit-linear-gradient(left top, red, blue)"),
            Some(LineDirection::Corner {
                right: true,
                bottom: true
            })
        );
        assert!(parse("linear-gradient(to, red, blue)").is_none());
        assert!(parse("linear-gradient(to left left, red, blue)").is_none());
        assert!(parse("linear-gradient(top, red, blue)").is_none());
        assert!(parse("linear-gradient(1, red, blue)").is_none());
        // A method beside the direction or alone (gap batch E).
        let method = |css: &str| parse(css).and_then(|g| g.method);
        let oklch = crate::color::Method {
            space: crate::color::Space::Oklch,
            hue: crate::color::Hue::Longer,
        };
        assert_eq!(
            method("linear-gradient(in oklch longer hue, red, blue)"),
            Some(oklch)
        );
        assert_eq!(
            method("linear-gradient(to right in oklch longer hue, red, blue)"),
            Some(oklch)
        );
        assert_eq!(
            parse("linear-gradient(in lab to right, red, blue)").map(|g| g.shape),
            Some(GradientShape::Linear(LineDirection::Angle(90.0)))
        );
        assert!(parse("linear-gradient(in nope, red, blue)").is_none());
        assert!(parse("linear-gradient(in srgb longer hue, red, blue)").is_none());
        assert!(parse("linear-gradient(to in lab right, red, blue)").is_none());
    }

    #[test]
    fn a_conic_gradient_starts_from_an_angle_at_a_centre() {
        let g = parse("conic-gradient(from 90deg at 10% 20%, red 0deg, blue 0.5turn, green 75%)")
            .expect("a conic gradient");
        assert_eq!(
            g.shape,
            GradientShape::Conic {
                from: 90.0,
                center: [
                    LengthPercentage::Percent(10.0),
                    LengthPercentage::Percent(20.0)
                ]
            }
        );
        assert_eq!(
            g.items,
            [
                GradientItem::Stop(RED, Some(LengthPercentage::Px(0.0))),
                GradientItem::Stop(BLUE, Some(LengthPercentage::Px(180.0))),
                GradientItem::Stop(
                    Color::rgb8(0, 128, 0),
                    Some(LengthPercentage::Percent(75.0))
                ),
            ]
        );
        assert!(
            parse("repeating-conic-gradient(red 0 10%, blue 10% 20%)").is_some_and(|g| g.repeating)
        );
        assert!(parse("conic-gradient(red, blue)").is_some());
        assert!(
            parse("conic-gradient(red 10px, blue)").is_none(),
            "no lengths"
        );
        assert!(parse("conic-gradient(from 1px, red, blue)").is_none());
        assert!(parse("conic-gradient(at 10%, red, blue)").is_some());
    }

    #[test]
    fn stops_take_one_or_two_positions_and_hints_go_between_them() {
        let items = |css: &str| parse(css).map(|g| g.items);
        let p = LengthPercentage::Percent;
        assert_eq!(
            items("linear-gradient(red 10%, 30%, blue 20% 40%)"),
            Some(vec![
                GradientItem::Stop(RED, Some(p(10.0))),
                GradientItem::Hint(p(30.0)),
                GradientItem::Stop(BLUE, Some(p(20.0))),
                GradientItem::Stop(BLUE, Some(p(40.0))),
            ])
        );
        assert_eq!(
            items("linear-gradient(10px red, blue)"),
            Some(vec![
                GradientItem::Stop(RED, Some(LengthPercentage::Px(10.0))),
                GradientItem::Stop(BLUE, None),
            ])
        );
        assert_eq!(items("linear-gradient(red)").map(|i| i.len()), Some(1));
        assert!(
            items("linear-gradient(10%, red, blue)").is_none(),
            "a hint first"
        );
        assert!(
            items("linear-gradient(red, blue, 10%)").is_none(),
            "a hint last"
        );
        assert!(
            items("linear-gradient(red, 10%, 20%, blue)").is_none(),
            "two hints"
        );
        assert!(items("linear-gradient(red 1% 2% 3%, blue)").is_none());
        assert!(items("linear-gradient(1% red 2%, blue)").is_none());
        assert!(items("linear-gradient(red blue)").is_none());
        let many = vec!["red"; MAX_GRADIENT_STOPS + 1].join(",");
        assert!(items(&format!("linear-gradient({many})")).is_none());
        let most = vec!["red"; MAX_GRADIENT_STOPS].join(",");
        assert!(items(&format!("linear-gradient({most})")).is_some());
    }

    #[test]
    fn a_radial_gradients_shape_and_size_agree() {
        let shape = |css: &str| match parse(css)?.shape {
            GradientShape::Radial {
                ellipse,
                size,
                center,
            } => Some((ellipse, size, center)),
            _ => None,
        };
        let p = LengthPercentage::Percent;
        let middle = [p(50.0), p(50.0)];
        assert_eq!(
            shape("radial-gradient(red, blue)"),
            Some((true, RadialSize::FarthestCorner, middle.clone()))
        );
        assert_eq!(
            shape("radial-gradient(circle, red, blue)"),
            Some((false, RadialSize::FarthestCorner, middle.clone()))
        );
        assert_eq!(
            shape("radial-gradient(10px, red, blue)"),
            Some((false, RadialSize::Circle(10.0), middle.clone()))
        );
        assert_eq!(
            shape("radial-gradient(10px 20%, red, blue)"),
            Some((
                true,
                RadialSize::Ellipse(LengthPercentage::Px(10.0), p(20.0)),
                middle.clone()
            ))
        );
        assert_eq!(
            shape("radial-gradient(closest-side circle at 10px top, red, blue)"),
            Some((
                false,
                RadialSize::ClosestSide,
                [LengthPercentage::Px(10.0), p(0.0)]
            ))
        );
        assert_eq!(
            shape("repeating-radial-gradient(at 20% 30%, red, blue)").map(|s| s.2),
            Some([p(20.0), p(30.0)])
        );
        assert!(parse("radial-gradient(circle 10%, red, blue)").is_none());
        assert!(parse("radial-gradient(circle 1px 2px, red, blue)").is_none());
        assert!(parse("radial-gradient(ellipse 1px, red, blue)").is_none());
        assert!(parse("radial-gradient(-1px, red, blue)").is_none());
        assert!(parse("radial-gradient(at, red, blue)").is_none());
        assert!(
            parse("radial-gradient(at left 10px top, red, blue)").is_none(),
            "three values are background-position's alone"
        );
        assert!(parse("repeating-linear-gradient(red, blue)").is_some_and(|g| g.repeating));
    }
}
