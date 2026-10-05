//! The values of the transform, opacity and will-change properties
//! (css-transforms-1 §7 to §9, css-transforms-2 §2, §9 to §13,
//! css-color-4 §3.2, css-will-change-1 §2; M19): the `transform` list
//! and its functions, the origins and their `<position>` forms, the
//! individual `translate`, `rotate` and `scale`, and the keywords. Each
//! parser takes a declaration's value as the cascade has it, trimmed and
//! without `!important`, and says `None` when the value is not one the
//! property takes, so that the declaration is dropped: a function the
//! engine does not know, a wrong count of arguments, a `<number>` where
//! a `<length>` must be, each invalidates the whole declaration, as the
//! grammar says.
//!
//! Lengths resolve as everywhere, to CSS pixels clamped to
//! [`MAX_LENGTH_PX`]; the numbers of a function, scale factors, matrix
//! components and the axis of a rotation, are clamped to the same
//! magnitude, so that a `scale(1e30)` is a scale by the largest length,
//! which draws the same, and the product of a few functions stays
//! finite; a number that is not finite is not a value. Angles are kept
//! in degrees; `0` is accepted where the grammar has `<zero>`.

use css::Token;

use super::{
    LengthBases, MAX_LENGTH_PX, parse_length_px, parse_non_negative_length_px, single_ident,
};
use crate::computed::{
    BackfaceVisibility, LengthPercentage, Offset, PerspectiveOrigin, Rotate, TransformBox,
    TransformFunction, TransformOrigin, TransformStyle, Translate, WillChange,
};

/// A cursor over a value's tokens that steps over white space.
struct Cursor<'t> {
    tokens: &'t [Token],
    pos: usize,
}

impl<'t> Cursor<'t> {
    fn new(tokens: &'t [Token]) -> Self {
        Cursor { tokens, pos: 0 }
    }

    /// The next token that is not white space, left where it is.
    fn peek(&mut self) -> Option<&'t Token> {
        while matches!(self.tokens.get(self.pos), Some(Token::Whitespace)) {
            self.pos += 1;
        }
        self.tokens.get(self.pos)
    }

    /// The next token that is not white space, taken.
    fn next(&mut self) -> Option<&'t Token> {
        let token = self.peek();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    /// Takes the next token if it is `token`.
    fn eat(&mut self, token: &Token) -> bool {
        let found = self.peek() == Some(token);
        if found {
            self.pos += 1;
        }
        found
    }

    fn at_end(&mut self) -> bool {
        self.peek().is_none()
    }

    /// The arguments of a function, the cursor just past its name: the
    /// tokens between its commas, up to its `)`, which is taken. An
    /// argument is one token, and there is at least one; the functions
    /// here take no other shape.
    fn arguments(&mut self) -> Option<Vec<&'t Token>> {
        let mut arguments = Vec::new();
        loop {
            let token = self.next()?;
            if matches!(token, Token::CloseParen | Token::Comma) {
                return None;
            }
            arguments.push(token);
            match self.next()? {
                Token::Comma => continue,
                Token::CloseParen => return Some(arguments),
                _ => return None,
            }
        }
    }
}

/// A `<number>`, finite and clamped to the largest length's magnitude.
fn number(token: &Token) -> Option<f32> {
    match token {
        Token::Number { value, .. } => {
            let v = *value as f32;
            v.is_finite()
                .then(|| v.clamp(-MAX_LENGTH_PX, MAX_LENGTH_PX))
        }
        _ => None,
    }
}

/// A `<number>` or a `<percentage>` that stands for one (css-transforms-2
/// §2.3, §13): `50%` is `0.5`.
fn number_or_percentage(token: &Token) -> Option<f32> {
    match token {
        Token::Percentage { value } => {
            let v = *value as f32 / 100.0;
            v.is_finite()
                .then(|| v.clamp(-MAX_LENGTH_PX, MAX_LENGTH_PX))
        }
        _ => number(token),
    }
}

/// An `<angle>` in degrees (css-values-4 §6.2): `deg`, `grad`, `rad` or
/// `turn`; `0` too when `zero` is allowed.
fn angle(token: &Token, zero: bool) -> Option<f32> {
    let degrees = match token {
        Token::Dimension { value, unit } => {
            let v = *value as f32;
            match unit.to_ascii_lowercase().as_str() {
                "deg" => v,
                "grad" => v * 0.9,
                "rad" => v.to_degrees(),
                "turn" => v * 360.0,
                _ => return None,
            }
        }
        Token::Number { value, .. } if zero && *value == 0.0 => 0.0,
        _ => return None,
    };
    degrees.is_finite().then_some(degrees)
}

/// A `<length>` in CSS pixels.
fn length(token: &Token, bases: &LengthBases<'_>) -> Option<f32> {
    parse_length_px(std::slice::from_ref(token), bases)
}

/// A `<length-percentage>`, the percentage kept.
fn length_percentage(token: &Token, bases: &LengthBases<'_>) -> Option<LengthPercentage> {
    if let Token::Percentage { value } = token {
        let p = *value as f32;
        return p
            .is_finite()
            .then(|| LengthPercentage::Percent(p.clamp(-MAX_LENGTH_PX, MAX_LENGTH_PX)));
    }
    length(token, bases).map(LengthPercentage::Px)
}

const ZERO_LENGTH: LengthPercentage = LengthPercentage::Px(0.0);

/// One `<transform-function>` (css-transforms-1 §7, css-transforms-2
/// §13), the cursor at its name.
fn function(c: &mut Cursor<'_>, bases: &LengthBases<'_>) -> Option<TransformFunction> {
    let Token::Function(name) = c.next()? else {
        return None;
    };
    let args = c.arguments()?;
    let name = name.to_ascii_lowercase();
    let numbers = |n: usize| -> Option<Vec<f32>> {
        (args.len() == n)
            .then(|| args.iter().map(|t| number(t)).collect())
            .flatten()
    };
    Some(match (name.as_str(), args.as_slice()) {
        ("matrix", _) => {
            let v = numbers(6)?;
            TransformFunction::Matrix([v[0], v[1], v[2], v[3], v[4], v[5]])
        }
        ("matrix3d", _) => {
            let v = numbers(16)?;
            let mut m = [0.0; 16];
            m.copy_from_slice(&v);
            TransformFunction::Matrix3d(m)
        }
        ("translate", [x]) => {
            TransformFunction::Translate(length_percentage(x, bases)?, ZERO_LENGTH, 0.0)
        }
        ("translate", [x, y]) => TransformFunction::Translate(
            length_percentage(x, bases)?,
            length_percentage(y, bases)?,
            0.0,
        ),
        ("translatex", [x]) => {
            TransformFunction::Translate(length_percentage(x, bases)?, ZERO_LENGTH, 0.0)
        }
        ("translatey", [y]) => {
            TransformFunction::Translate(ZERO_LENGTH, length_percentage(y, bases)?, 0.0)
        }
        ("translatez", [z]) => {
            TransformFunction::Translate(ZERO_LENGTH, ZERO_LENGTH, length(z, bases)?)
        }
        ("translate3d", [x, y, z]) => TransformFunction::Translate(
            length_percentage(x, bases)?,
            length_percentage(y, bases)?,
            length(z, bases)?,
        ),
        ("scale", [s]) => {
            let s = number_or_percentage(s)?;
            TransformFunction::Scale(s, s, 1.0)
        }
        ("scale", [x, y]) => {
            TransformFunction::Scale(number_or_percentage(x)?, number_or_percentage(y)?, 1.0)
        }
        ("scalex", [x]) => TransformFunction::Scale(number_or_percentage(x)?, 1.0, 1.0),
        ("scaley", [y]) => TransformFunction::Scale(1.0, number_or_percentage(y)?, 1.0),
        ("scalez", [z]) => TransformFunction::Scale(1.0, 1.0, number_or_percentage(z)?),
        ("scale3d", [x, y, z]) => TransformFunction::Scale(
            number_or_percentage(x)?,
            number_or_percentage(y)?,
            number_or_percentage(z)?,
        ),
        ("rotate", [a]) | ("rotatez", [a]) => TransformFunction::Rotate {
            x: 0.0,
            y: 0.0,
            z: 1.0,
            angle: angle(a, true)?,
        },
        ("rotatex", [a]) => TransformFunction::Rotate {
            x: 1.0,
            y: 0.0,
            z: 0.0,
            angle: angle(a, true)?,
        },
        ("rotatey", [a]) => TransformFunction::Rotate {
            x: 0.0,
            y: 1.0,
            z: 0.0,
            angle: angle(a, true)?,
        },
        ("rotate3d", [x, y, z, a]) => TransformFunction::Rotate {
            x: number(x)?,
            y: number(y)?,
            z: number(z)?,
            angle: angle(a, true)?,
        },
        ("skew", [x]) => TransformFunction::Skew(angle(x, true)?, 0.0),
        ("skew", [x, y]) => TransformFunction::Skew(angle(x, true)?, angle(y, true)?),
        ("skewx", [x]) => TransformFunction::Skew(angle(x, true)?, 0.0),
        ("skewy", [y]) => TransformFunction::Skew(0.0, angle(y, true)?),
        ("perspective", [d]) => TransformFunction::Perspective(perspective_depth(d, bases)?),
        _ => return None,
    })
}

/// `none` or a `<length [0,∞]>`, the depth of a `perspective`.
fn perspective_depth(token: &Token, bases: &LengthBases<'_>) -> Option<Option<f32>> {
    if let Token::Ident(word) = token
        && word.eq_ignore_ascii_case("none")
    {
        return Some(None);
    }
    parse_non_negative_length_px(std::slice::from_ref(token), bases).map(Some)
}

/// `transform: none | <transform-list>` (css-transforms-1 §7): the
/// functions, in order; empty for `none`.
pub(super) fn parse_transform(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Vec<TransformFunction>> {
    if single_ident(tokens).is_some_and(|s| s.eq_ignore_ascii_case("none")) {
        return Some(Vec::new());
    }
    let mut c = Cursor::new(tokens);
    let mut functions = Vec::new();
    while !c.at_end() {
        functions.push(function(&mut c, bases)?);
    }
    (!functions.is_empty()).then_some(functions)
}

/// One value of a `<position>` (css-values-4 §4.3): a keyword, or a
/// length or percentage.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    Left,
    Right,
    Top,
    Bottom,
    Center,
    /// A length or a percentage, along whichever axis its place in the
    /// value gives it.
    Offset(LengthPercentage),
}

impl Place {
    /// The offset the place stands for on its own: `left` and `top` are
    /// `0%`, `right` and `bottom` `100%`, `center` `50%`.
    fn offset(self) -> Offset {
        match self {
            Place::Left | Place::Top => Offset {
                percent: 0.0,
                px: 0.0,
            },
            Place::Right | Place::Bottom => Offset {
                percent: 100.0,
                px: 0.0,
            },
            Place::Center => Offset::CENTER,
            Place::Offset(LengthPercentage::Px(px)) => Offset { percent: 0.0, px },
            Place::Offset(LengthPercentage::Percent(percent)) => Offset { percent, px: 0.0 },
        }
    }

    /// The offset of an edge keyword with an offset from its edge:
    /// `right 10px` is `100% - 10px`, `right 10%` is `90%`, `left 10px`
    /// is `10px`.
    fn offset_from_edge(self, delta: LengthPercentage) -> Offset {
        let from_end = matches!(self, Place::Right | Place::Bottom);
        match delta {
            LengthPercentage::Px(px) => Offset {
                percent: if from_end { 100.0 } else { 0.0 },
                px: if from_end { -px } else { px },
            },
            LengthPercentage::Percent(p) => Offset {
                percent: if from_end { 100.0 - p } else { p },
                px: 0.0,
            },
        }
    }

    /// Whether the place can be the first of a two-value position, the
    /// one along x.
    fn is_x(self) -> bool {
        matches!(
            self,
            Place::Left | Place::Right | Place::Center | Place::Offset(_)
        )
    }

    /// Whether the place can be the second of a two-value position, the
    /// one along y.
    fn is_y(self) -> bool {
        matches!(
            self,
            Place::Top | Place::Bottom | Place::Center | Place::Offset(_)
        )
    }
}

/// One value of a `<position>`.
fn place(token: &Token, bases: &LengthBases<'_>) -> Option<Place> {
    match token {
        Token::Ident(word) => match word.to_ascii_lowercase().as_str() {
            "left" => Some(Place::Left),
            "right" => Some(Place::Right),
            "top" => Some(Place::Top),
            "bottom" => Some(Place::Bottom),
            "center" => Some(Place::Center),
            _ => None,
        },
        _ => length_percentage(token, bases).map(Place::Offset),
    }
}

/// A `<position>` (css-values-4 §8.3), the whole of `tokens`: one
/// value, the other being `center`; two, the one along x first, or the
/// two keywords in either order; and, when `offsets` are allowed, as
/// `perspective-origin` allows them and `transform-origin` does not,
/// four, an edge keyword and an offset from its edge for each axis, in
/// either order. There is no three-value form: `background-position`'s
/// is its own (§8.3's note), and the reference browser refuses it here.
fn position(tokens: &[&Token], bases: &LengthBases<'_>, offsets: bool) -> Option<(Offset, Offset)> {
    let places: Vec<Place> = tokens
        .iter()
        .map(|t| place(t, bases))
        .collect::<Option<_>>()?;
    match places.as_slice() {
        [one] => Some(match one {
            Place::Top | Place::Bottom => (Offset::CENTER, one.offset()),
            _ => (one.offset(), Offset::CENTER),
        }),
        [x, y] if x.is_x() && y.is_y() => Some((x.offset(), y.offset())),
        [y, x]
            if matches!(y, Place::Top | Place::Bottom | Place::Center)
                && matches!(x, Place::Left | Place::Right | Place::Center) =>
        {
            Some((x.offset(), y.offset()))
        }
        [_, _, _, _] if offsets => position_with_offsets(&places),
        _ => None,
    }
}

/// The four-value form of `<position>`: `[ [ left | right ]
/// <length-percentage> ] && [ [ top | bottom ] <length-percentage> ]`.
fn position_with_offsets(places: &[Place]) -> Option<(Offset, Offset)> {
    let [
        first,
        Place::Offset(first_delta),
        second,
        Place::Offset(second_delta),
    ] = places
    else {
        return None;
    };
    let horizontal = |p: &Place| matches!(p, Place::Left | Place::Right);
    let vertical = |p: &Place| matches!(p, Place::Top | Place::Bottom);
    if horizontal(first) && vertical(second) {
        Some((
            first.offset_from_edge(*first_delta),
            second.offset_from_edge(*second_delta),
        ))
    } else if vertical(first) && horizontal(second) {
        Some((
            second.offset_from_edge(*second_delta),
            first.offset_from_edge(*first_delta),
        ))
    } else {
        None
    }
}

/// `transform-origin` (css-transforms-1 §8, css-transforms-2 §8): a
/// `<position>`'s one-value form, or its two-value forms followed by an
/// optional `<length>` along z. The forms with offsets from an edge are
/// `<position>`'s, not this property's.
pub(super) fn parse_transform_origin(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<TransformOrigin> {
    let tokens: Vec<&Token> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    let (xy, z) = match tokens.as_slice() {
        [_] | [_, _] => (&tokens[..], 0.0),
        [_, _, z] => (&tokens[..2], length(z, bases)?),
        _ => return None,
    };
    let (x, y) = position(xy, bases, false)?;
    Some(TransformOrigin { x, y, z })
}

/// `perspective-origin: <position>` (css-transforms-2 §11).
pub(super) fn parse_perspective_origin(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<PerspectiveOrigin> {
    let tokens: Vec<&Token> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    let (x, y) = position(&tokens, bases, true)?;
    Some(PerspectiveOrigin { x, y })
}

/// `transform-box` (css-transforms-1 §9).
pub(super) fn parse_transform_box(tokens: &[Token]) -> Option<TransformBox> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "content-box" => Some(TransformBox::ContentBox),
        "border-box" => Some(TransformBox::BorderBox),
        "fill-box" => Some(TransformBox::FillBox),
        "stroke-box" => Some(TransformBox::StrokeBox),
        "view-box" => Some(TransformBox::ViewBox),
        _ => None,
    }
}

/// `translate: none | <length-percentage> [ <length-percentage>
/// <length>? ]?` (css-transforms-2 §2.1).
pub(super) fn parse_translate(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Option<Translate>> {
    if single_ident(tokens).is_some_and(|s| s.eq_ignore_ascii_case("none")) {
        return Some(None);
    }
    let tokens: Vec<&Token> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    let (x, y, z) = match tokens.as_slice() {
        [x] => (length_percentage(x, bases)?, ZERO_LENGTH, 0.0),
        [x, y] => (
            length_percentage(x, bases)?,
            length_percentage(y, bases)?,
            0.0,
        ),
        [x, y, z] => (
            length_percentage(x, bases)?,
            length_percentage(y, bases)?,
            length(z, bases)?,
        ),
        _ => return None,
    };
    Some(Some(Translate { x, y, z }))
}

/// `rotate: none | <angle> | [ x | y | z | <number>{3} ] && <angle>`
/// (css-transforms-2 §2.2).
pub(super) fn parse_rotate(tokens: &[Token]) -> Option<Option<Rotate>> {
    if single_ident(tokens).is_some_and(|s| s.eq_ignore_ascii_case("none")) {
        return Some(None);
    }
    let tokens: Vec<&Token> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    let axis_keyword = |t: &Token| match t {
        Token::Ident(w) => match w.to_ascii_lowercase().as_str() {
            "x" => Some((1.0, 0.0, 0.0)),
            "y" => Some((0.0, 1.0, 0.0)),
            "z" => Some((0.0, 0.0, 1.0)),
            _ => None,
        },
        _ => None,
    };
    let (x, y, z, a) = match tokens.as_slice() {
        [a] => (0.0, 0.0, 1.0, angle(a, false)?),
        [k, a] | [a, k] if axis_keyword(k).is_some() && angle(a, false).is_some() => {
            let (x, y, z) = axis_keyword(k)?;
            (x, y, z, angle(a, false)?)
        }
        [x, y, z, a] if angle(a, false).is_some() => {
            (number(x)?, number(y)?, number(z)?, angle(a, false)?)
        }
        [a, x, y, z] if angle(a, false).is_some() => {
            (number(x)?, number(y)?, number(z)?, angle(a, false)?)
        }
        _ => return None,
    };
    Some(Some(Rotate { x, y, z, angle: a }))
}

/// `scale: none | [ <number> | <percentage> ]{1,3}` (css-transforms-2
/// §2.3): one value for x and y, a third for z.
pub(super) fn parse_scale(tokens: &[Token]) -> Option<Option<[f32; 3]>> {
    if single_ident(tokens).is_some_and(|s| s.eq_ignore_ascii_case("none")) {
        return Some(None);
    }
    let factors: Option<Vec<f32>> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .map(number_or_percentage)
        .collect();
    Some(Some(match factors?.as_slice() {
        [s] => [*s, *s, 1.0],
        [x, y] => [*x, *y, 1.0],
        [x, y, z] => [*x, *y, *z],
        _ => return None,
    }))
}

/// `transform-style: flat | preserve-3d` (css-transforms-2 §9).
pub(super) fn parse_transform_style(tokens: &[Token]) -> Option<TransformStyle> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "flat" => Some(TransformStyle::Flat),
        "preserve-3d" => Some(TransformStyle::Preserve3d),
        _ => None,
    }
}

/// `perspective: none | <length [0,∞]>` (css-transforms-2 §10).
pub(super) fn parse_perspective(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Option<f32>> {
    let token = super::single_token(tokens)?;
    perspective_depth(token, bases)
}

/// `backface-visibility: visible | hidden` (css-transforms-2 §12).
pub(super) fn parse_backface_visibility(tokens: &[Token]) -> Option<BackfaceVisibility> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "visible" => Some(BackfaceVisibility::Visible),
        "hidden" => Some(BackfaceVisibility::Hidden),
        _ => None,
    }
}

/// `opacity: <alpha-value>` (css-color-4 §3.2): a number or a
/// percentage, clamped to `[0, 1]`.
pub(super) fn parse_opacity(tokens: &[Token]) -> Option<f32> {
    let v = match super::single_token(tokens)? {
        Token::Number { value, .. } => *value as f32,
        Token::Percentage { value } => *value as f32 / 100.0,
        _ => return None,
    };
    v.is_finite().then(|| v.clamp(0.0, 1.0))
}

/// `will-change: auto | <animateable-feature>#` (css-will-change-1 §2):
/// `scroll-position`, `contents` or a property's name, any name but the
/// CSS-wide keywords and `will-change`, `none`, `all`, `auto`, which make
/// the declaration invalid.
pub(super) fn parse_will_change(tokens: &[Token]) -> Option<WillChange> {
    if single_ident(tokens).is_some_and(|s| s.eq_ignore_ascii_case("auto")) {
        return Some(WillChange::AUTO);
    }
    let mut c = Cursor::new(tokens);
    let mut features = WillChange::AUTO;
    loop {
        let Token::Ident(name) = c.next()? else {
            return None;
        };
        let name = name.to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "will-change"
                | "none"
                | "all"
                | "auto"
                | "inherit"
                | "initial"
                | "unset"
                | "revert"
                | "revert-layer"
                | "default"
        ) {
            return None;
        }
        if let Some(feature) = WillChange::feature(&name) {
            features = features.with(feature);
        }
        if c.at_end() {
            return Some(features);
        }
        if !c.eat(&Token::Comma) {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::with_bases;
    use super::*;

    fn tokens(value: &str) -> Vec<Token> {
        css::tokenize(value)
    }

    fn transform(value: &str) -> Option<Vec<TransformFunction>> {
        with_bases(|bases| parse_transform(&tokens(value), bases))
    }

    #[test]
    fn transform_lists_parse_function_by_function() {
        use TransformFunction::*;
        assert_eq!(transform("none"), Some(vec![]));
        assert_eq!(transform("NONE"), Some(vec![]));
        assert_eq!(
            transform("translate(10px, 5%) rotate(0.5turn) scale(2) skewX(10deg)"),
            Some(vec![
                Translate(
                    LengthPercentage::Px(10.0),
                    LengthPercentage::Percent(5.0),
                    0.0
                ),
                Rotate {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                    angle: 180.0
                },
                Scale(2.0, 2.0, 1.0),
                Skew(10.0, 0.0),
            ])
        );
        assert_eq!(
            transform(
                "translateX(1em) translateY(-50%) translateZ(3px) translate3d(1px, 2px, 3px)"
            ),
            Some(vec![
                Translate(LengthPercentage::Px(16.0), ZERO_LENGTH, 0.0),
                Translate(ZERO_LENGTH, LengthPercentage::Percent(-50.0), 0.0),
                Translate(ZERO_LENGTH, ZERO_LENGTH, 3.0),
                Translate(LengthPercentage::Px(1.0), LengthPercentage::Px(2.0), 3.0),
            ])
        );
        assert_eq!(
            transform("scale(1, 2) scaleX(3) scaleY(4) scaleZ(5) scale3d(6, 7, 8) scale(50%)"),
            Some(vec![
                Scale(1.0, 2.0, 1.0),
                Scale(3.0, 1.0, 1.0),
                Scale(1.0, 4.0, 1.0),
                Scale(1.0, 1.0, 5.0),
                Scale(6.0, 7.0, 8.0),
                Scale(0.5, 0.5, 1.0),
            ])
        );
        assert_eq!(
            transform(
                "rotate(90deg) rotateX(100grad) rotateY(0) rotateZ(1rad) rotate3d(1, 1, 0, 45deg)"
            ),
            Some(vec![
                Rotate {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                    angle: 90.0
                },
                Rotate {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                    angle: 90.0
                },
                Rotate {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                    angle: 0.0
                },
                Rotate {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                    angle: 1f32.to_degrees()
                },
                Rotate {
                    x: 1.0,
                    y: 1.0,
                    z: 0.0,
                    angle: 45.0
                },
            ])
        );
        assert_eq!(
            transform("skew(10deg, 20deg) skew(0) skewY(5deg)"),
            Some(vec![Skew(10.0, 20.0), Skew(0.0, 0.0), Skew(0.0, 5.0)])
        );
        assert_eq!(
            transform("matrix(1, 0, 0, 1, 10, 20) perspective(500px) perspective(none)"),
            Some(vec![
                Matrix([1.0, 0.0, 0.0, 1.0, 10.0, 20.0]),
                Perspective(Some(500.0)),
                Perspective(None),
            ])
        );
        let m3d = transform("matrix3d(1,0,0,0, 0,1,0,0, 0,0,1,0, 1,2,3,1)").unwrap();
        assert_eq!(
            m3d,
            vec![Matrix3d([
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 2.0, 3.0, 1.0
            ])]
        );
        // A huge number is the largest length; a non-finite one is not
        // a value at all.
        assert_eq!(
            transform("scale(1e30)"),
            Some(vec![Scale(MAX_LENGTH_PX, MAX_LENGTH_PX, 1.0)])
        );
        assert_eq!(transform("scale(1e400)"), None);
    }

    #[test]
    fn a_wrong_function_or_argument_invalidates_the_whole_list() {
        for value in [
            "",
            "rotate(5deg) bogus(1)",
            "translate(10)",
            "translate(10px, 20px, 30px)",
            "translateZ(10%)",
            "translate3d(1px, 2px)",
            "scale()",
            "scale(1, 2, 3)",
            "scale(1px)",
            "rotate(5)",
            "rotate(90deg",
            "rotate3d(1, 0, 0)",
            "skew(10deg, 20deg, 30deg)",
            "matrix(1, 0, 0, 1, 10)",
            "matrix3d(1, 2, 3)",
            "perspective(-1px)",
            "perspective(10)",
            "rotate(5deg), rotate(5deg)",
            "rotate(5deg) 5px",
            "none rotate(5deg)",
            "translate(10px 20px)",
        ] {
            assert_eq!(transform(value), None, "{value:?}");
        }
    }

    fn origin(value: &str) -> Option<TransformOrigin> {
        with_bases(|bases| parse_transform_origin(&tokens(value), bases))
    }

    fn offset(percent: f32, px: f32) -> Offset {
        Offset { percent, px }
    }

    #[test]
    fn transform_origin_takes_its_three_forms() {
        // One value: the other is center; a vertical keyword goes to y.
        assert_eq!(
            origin("10px"),
            Some(TransformOrigin {
                x: offset(0.0, 10.0),
                y: Offset::CENTER,
                z: 0.0
            })
        );
        assert_eq!(
            origin("bottom"),
            Some(TransformOrigin {
                x: Offset::CENTER,
                y: offset(100.0, 0.0),
                z: 0.0
            })
        );
        assert_eq!(
            origin("left").map(|o| (o.x, o.y)),
            Some((offset(0.0, 0.0), Offset::CENTER))
        );
        // Two values: x then y, or the keywords either way round.
        assert_eq!(
            origin("right 25%").map(|o| (o.x, o.y)),
            Some((offset(100.0, 0.0), offset(25.0, 0.0)))
        );
        assert_eq!(
            origin("top left").map(|o| (o.x, o.y)),
            Some((offset(0.0, 0.0), offset(0.0, 0.0)))
        );
        assert_eq!(
            origin("bottom center").map(|o| (o.x, o.y)),
            Some((Offset::CENTER, offset(100.0, 0.0)))
        );
        assert_eq!(
            origin("center bottom").map(|o| (o.x, o.y)),
            Some((Offset::CENTER, offset(100.0, 0.0)))
        );
        assert_eq!(
            origin("1em 2em 3em"),
            Some(TransformOrigin {
                x: offset(0.0, 16.0),
                y: offset(0.0, 32.0),
                z: 48.0
            })
        );
        assert_eq!(origin("left top 10px").map(|o| o.z), Some(10.0));
        assert_eq!(
            origin("left 50px").map(|o| (o.x, o.y, o.z)),
            Some((offset(0.0, 0.0), offset(0.0, 50.0), 0.0))
        );
        // The reference browser refuses `top 50px`: the one-value form
        // takes no z, whatever css-values-4 §8.3.1's note says.
        for value in [
            "",
            "top 50px",
            "left left",
            "top bottom",
            "10px 20px 30%",
            "10px top 5px 20px",
            "right 10px bottom 20px",
            "auto",
            "10",
        ] {
            assert_eq!(origin(value), None, "{value:?}");
        }
    }

    fn perspective_origin(value: &str) -> Option<(Offset, Offset)> {
        with_bases(|bases| parse_perspective_origin(&tokens(value), bases)).map(|o| (o.x, o.y))
    }

    #[test]
    fn perspective_origin_takes_every_position_form() {
        assert_eq!(
            perspective_origin("10%"),
            Some((offset(10.0, 0.0), Offset::CENTER))
        );
        assert_eq!(
            perspective_origin("10% 20px"),
            Some((offset(10.0, 0.0), offset(0.0, 20.0)))
        );
        assert_eq!(
            perspective_origin("bottom right"),
            Some((offset(100.0, 0.0), offset(100.0, 0.0)))
        );
        // The four-value form: offsets from the named edges.
        assert_eq!(
            perspective_origin("right 10px bottom 20px"),
            Some((offset(100.0, -10.0), offset(100.0, -20.0)))
        );
        assert_eq!(
            perspective_origin("bottom 20px right 10px"),
            Some((offset(100.0, -10.0), offset(100.0, -20.0)))
        );
        assert_eq!(
            perspective_origin("left 10% top 5px"),
            Some((offset(10.0, 0.0), offset(0.0, 5.0)))
        );
        assert_eq!(
            perspective_origin("right 10% top 5px"),
            Some((offset(90.0, 0.0), offset(0.0, 5.0)))
        );
        // No three-value form, and no `center` with offsets.
        for value in [
            "",
            "10px 20px 30px",
            "right 10% top",
            "center bottom 20px",
            "bottom 20px center",
            "center 10px bottom 5px",
            "left 10px right 10px",
            "10px left",
            "left top 10px",
            "left 10px top 5px 1px",
        ] {
            assert_eq!(perspective_origin(value), None, "{value:?}");
        }
    }

    #[test]
    fn the_individual_properties_parse() {
        let translate = |v: &str| with_bases(|bases| parse_translate(&tokens(v), bases));
        assert_eq!(translate("none"), Some(None));
        assert_eq!(
            translate("10px"),
            Some(Some(Translate {
                x: LengthPercentage::Px(10.0),
                y: ZERO_LENGTH,
                z: 0.0
            }))
        );
        assert_eq!(
            translate("10% 20px 30px"),
            Some(Some(Translate {
                x: LengthPercentage::Percent(10.0),
                y: LengthPercentage::Px(20.0),
                z: 30.0
            }))
        );
        assert_eq!(translate("10px 20px 30%"), None);
        assert_eq!(translate("10"), None);
        assert_eq!(translate("1px 2px 3px 4px"), None);

        let rotate = |v: &str| parse_rotate(&tokens(v));
        let about = |x, y, z, angle| Some(Some(Rotate { x, y, z, angle }));
        assert_eq!(rotate("none"), Some(None));
        assert_eq!(rotate("45deg"), about(0.0, 0.0, 1.0, 45.0));
        assert_eq!(rotate("x 45deg"), about(1.0, 0.0, 0.0, 45.0));
        assert_eq!(rotate("45deg y"), about(0.0, 1.0, 0.0, 45.0));
        assert_eq!(rotate("1 1 0 45deg"), about(1.0, 1.0, 0.0, 45.0));
        assert_eq!(rotate("45deg 0 0 1"), about(0.0, 0.0, 1.0, 45.0));
        assert_eq!(rotate("0"), None, "the property takes no <zero>");
        assert_eq!(rotate("45"), None);
        assert_eq!(rotate("x y 45deg"), None);
        assert_eq!(rotate("1 1 45deg"), None);

        let scale = |v: &str| parse_scale(&tokens(v));
        assert_eq!(scale("none"), Some(None));
        assert_eq!(scale("2"), Some(Some([2.0, 2.0, 1.0])));
        assert_eq!(scale("2 3"), Some(Some([2.0, 3.0, 1.0])));
        assert_eq!(scale("2 3 4"), Some(Some([2.0, 3.0, 4.0])));
        assert_eq!(scale("50% 200%"), Some(Some([0.5, 2.0, 1.0])));
        assert_eq!(scale("2 3 4 5"), None);
        assert_eq!(scale("2px"), None);
        assert_eq!(scale(""), None);
    }

    #[test]
    fn the_keywords_the_depth_and_the_opacity_parse() {
        assert_eq!(
            parse_transform_box(&tokens("border-box")),
            Some(TransformBox::BorderBox)
        );
        assert_eq!(
            parse_transform_box(&tokens("view-box")),
            Some(TransformBox::ViewBox)
        );
        assert_eq!(
            parse_transform_box(&tokens("fill-box")),
            Some(TransformBox::FillBox)
        );
        assert_eq!(
            parse_transform_box(&tokens("content-box")),
            Some(TransformBox::ContentBox)
        );
        assert_eq!(parse_transform_box(&tokens("padding-box")), None);
        assert!(TransformBox::FillBox.uses_content_box());
        assert!(TransformBox::ContentBox.uses_content_box());
        assert!(!TransformBox::StrokeBox.uses_content_box());
        assert!(!TransformBox::ViewBox.uses_content_box());
        assert_eq!(
            parse_transform_style(&tokens("preserve-3d")),
            Some(TransformStyle::Preserve3d)
        );
        assert_eq!(
            parse_transform_style(&tokens("flat")),
            Some(TransformStyle::Flat)
        );
        assert_eq!(parse_transform_style(&tokens("auto")), None);
        assert_eq!(
            parse_backface_visibility(&tokens("hidden")),
            Some(BackfaceVisibility::Hidden)
        );
        assert_eq!(parse_backface_visibility(&tokens("none")), None);
        let perspective = |v: &str| with_bases(|bases| parse_perspective(&tokens(v), bases));
        assert_eq!(perspective("none"), Some(None));
        assert_eq!(perspective("500px"), Some(Some(500.0)));
        assert_eq!(perspective("0"), Some(Some(0.0)));
        assert_eq!(perspective("-1px"), None);
        assert_eq!(perspective("50%"), None);
        assert_eq!(perspective("500"), None);
        let opacity = |v: &str| parse_opacity(&tokens(v));
        assert_eq!(opacity("0.5"), Some(0.5));
        assert_eq!(opacity("50%"), Some(0.5));
        assert_eq!(opacity("2"), Some(1.0));
        assert_eq!(opacity("-1"), Some(0.0));
        assert_eq!(opacity("1px"), None);
        assert_eq!(opacity("auto"), None);
    }

    #[test]
    fn will_change_keeps_the_features_the_engine_acts_on() {
        let will_change = |v: &str| parse_will_change(&tokens(v));
        assert_eq!(will_change("auto"), Some(WillChange::AUTO));
        assert_eq!(will_change("transform"), Some(WillChange::TRANSFORM));
        assert_eq!(
            will_change("opacity, transform"),
            Some(WillChange::OPACITY.with(WillChange::TRANSFORM))
        );
        assert_eq!(
            will_change("scroll-position, contents, left"),
            Some(WillChange::SCROLL_POSITION.with(WillChange::CONTENTS))
        );
        assert_eq!(will_change("left"), Some(WillChange::AUTO));
        assert_eq!(
            will_change("Transform-Style"),
            Some(WillChange::TRANSFORM_STYLE)
        );
        // The properties this engine has not got whose other values
        // would make a stacking context or a containing block, and the
        // `-webkit-` aliases.
        assert_eq!(will_change("filter"), Some(WillChange::FILTER));
        assert_eq!(
            will_change("-webkit-backdrop-filter"),
            Some(WillChange::BACKDROP_FILTER)
        );
        assert_eq!(will_change("contain"), Some(WillChange::CONTAIN));
        assert_eq!(will_change("offset-path"), Some(WillChange::OFFSET_PATH));
        assert_eq!(will_change("position"), Some(WillChange::POSITION));
        assert_eq!(will_change("z-index"), Some(WillChange::Z_INDEX));
        for name in [
            "clip-path",
            "mask",
            "mask-image",
            "isolation",
            "mix-blend-mode",
            "view-transition-name",
        ] {
            assert_eq!(will_change(name), Some(WillChange::COMPOSITED), "{name}");
        }
        assert_eq!(
            will_change("-webkit-perspective"),
            Some(WillChange::PERSPECTIVE)
        );
        assert_eq!(
            will_change("-webkit-transform"),
            Some(WillChange::TRANSFORM)
        );
        for value in [
            "",
            "none",
            "all",
            "will-change",
            "transform, auto",
            "transform opacity",
            "transform,",
            "10px",
            "inherit, transform",
        ] {
            assert_eq!(will_change(value), None, "{value:?}");
        }
    }
}
