//! SVG's properties (SVG 2 sections 7, 11 and 13, CSS Inline 3's
//! `dominant-baseline`; M32) and the presentation attributes that are
//! hints for them and for CSS's own (SVG 2 section 6.6). Each parser
//! takes a declaration's value as the cascade has it and says `None`
//! when the property does not take it.

use std::sync::Arc;

use css::{Declaration, Token};

use super::{
    LengthBases, MAX_LENGTH_PX, Range, components, effects, length_percentage, math, parse_color,
    single_ident, single_token, trim,
};
use crate::calc::{Computed, Want};
use crate::computed::{
    ComputedStyle, DominantBaseline, FillRule, LengthPercentage, LengthPercentageAuto, PaintLayer,
    PaintOrder, ShapeRendering, StrokeLinecap, StrokeLinejoin, SvgInherited, SvgOwn, SvgPaint,
    TextAnchor, VectorEffect,
};
use crate::svg_transform::parse_transform_list;

/// Whether `name` is one of SVG's properties an element inherits.
pub(super) fn is_inherited(name: &str) -> bool {
    matches!(
        name,
        "fill"
            | "fill-opacity"
            | "fill-rule"
            | "stroke"
            | "stroke-opacity"
            | "stroke-width"
            | "stroke-linecap"
            | "stroke-linejoin"
            | "stroke-miterlimit"
            | "stroke-dasharray"
            | "stroke-dashoffset"
            | "clip-rule"
            | "text-anchor"
            | "dominant-baseline"
            | "paint-order"
            | "shape-rendering"
    )
}

/// Applies `name: value` for SVG's properties: `None` when `name` is
/// none of them, else whether the value was taken.
pub(super) fn apply(
    style: &mut ComputedStyle,
    name: &str,
    value: &[Token],
    bases: &LengthBases<'_>,
) -> Option<bool> {
    fn inherited<T>(
        style: &mut ComputedStyle,
        v: Option<T>,
        update: impl FnOnce(&mut SvgInherited, T),
    ) -> bool {
        v.map(|v| update(style.svg_mut(), v)).is_some()
    }
    fn own<T>(
        style: &mut ComputedStyle,
        v: Option<T>,
        update: impl FnOnce(&mut SvgOwn, T),
    ) -> bool {
        v.map(|v| update(style.svg_own_mut(), v)).is_some()
    }
    let length = |range| svg_length(value, bases, range);
    Some(match name {
        "fill" => inherited(style, parse_paint(value), |s, v| s.fill = v),
        "fill-opacity" => inherited(style, effects::parse_opacity(value, bases), |s, v| {
            s.fill_opacity = v
        }),
        "fill-rule" => inherited(style, parse_fill_rule(value), |s, v| s.fill_rule = v),
        "stroke" => inherited(style, parse_paint(value), |s, v| s.stroke = v),
        "stroke-opacity" => inherited(style, effects::parse_opacity(value, bases), |s, v| {
            s.stroke_opacity = v
        }),
        "stroke-width" => inherited(style, length(Range::NonNegative), |s, v| s.stroke_width = v),
        "stroke-linecap" => inherited(style, parse_linecap(value), |s, v| s.stroke_linecap = v),
        "stroke-linejoin" => inherited(style, parse_linejoin(value), |s, v| s.stroke_linejoin = v),
        "stroke-miterlimit" => inherited(style, parse_miterlimit(value, bases), |s, v| {
            s.stroke_miterlimit = v
        }),
        "stroke-dasharray" => inherited(style, parse_dasharray(value, bases), |s, v| {
            s.stroke_dasharray = v
        }),
        "stroke-dashoffset" => inherited(style, length(Range::All), |s, v| s.stroke_dashoffset = v),
        "clip-rule" => inherited(style, parse_fill_rule(value), |s, v| s.clip_rule = v),
        "text-anchor" => inherited(style, parse_text_anchor(value), |s, v| s.text_anchor = v),
        "dominant-baseline" => inherited(style, parse_dominant_baseline(value), |s, v| {
            s.dominant_baseline = v
        }),
        "paint-order" => inherited(style, parse_paint_order(value), |s, v| s.paint_order = v),
        "shape-rendering" => inherited(style, parse_shape_rendering(value), |s, v| {
            s.shape_rendering = v
        }),
        "stop-color" => own(style, parse_stop_color(value), |s, v| s.stop_color = v),
        "stop-opacity" => own(style, effects::parse_opacity(value, bases), |s, v| {
            s.stop_opacity = v
        }),
        "vector-effect" => own(style, parse_vector_effect(value), |s, v| {
            s.vector_effect = v
        }),
        "clip-path" => own(style, parse_clip_path(value), |s, v| s.clip_path = v),
        "x" => own(style, length(Range::All), |s, v| s.x = v),
        "y" => own(style, length(Range::All), |s, v| s.y = v),
        "cx" => own(style, length(Range::All), |s, v| s.cx = v),
        "cy" => own(style, length(Range::All), |s, v| s.cy = v),
        "r" => own(style, length(Range::NonNegative), |s, v| s.r = v),
        "rx" => own(style, parse_radius(value, bases), |s, v| s.rx = v),
        "ry" => own(style, parse_radius(value, bases), |s, v| s.ry = v),
        _ => return None,
    })
}

/// Copies SVG's property `name` from `src` into `style`, for a
/// CSS-wide keyword: `false` when `name` is none of them.
pub(super) fn copy(style: &mut ComputedStyle, src: &ComputedStyle, name: &str) -> bool {
    if is_inherited(name) {
        if style.svg.is_none() && src.svg.is_none() {
            return true;
        }
        let (from, to) = (src.svg().clone(), style.svg_mut());
        match name {
            "fill" => to.fill = from.fill,
            "fill-opacity" => to.fill_opacity = from.fill_opacity,
            "fill-rule" => to.fill_rule = from.fill_rule,
            "stroke" => to.stroke = from.stroke,
            "stroke-opacity" => to.stroke_opacity = from.stroke_opacity,
            "stroke-width" => to.stroke_width = from.stroke_width,
            "stroke-linecap" => to.stroke_linecap = from.stroke_linecap,
            "stroke-linejoin" => to.stroke_linejoin = from.stroke_linejoin,
            "stroke-miterlimit" => to.stroke_miterlimit = from.stroke_miterlimit,
            "stroke-dasharray" => to.stroke_dasharray = from.stroke_dasharray,
            "stroke-dashoffset" => to.stroke_dashoffset = from.stroke_dashoffset,
            "clip-rule" => to.clip_rule = from.clip_rule,
            "text-anchor" => to.text_anchor = from.text_anchor,
            "dominant-baseline" => to.dominant_baseline = from.dominant_baseline,
            "paint-order" => to.paint_order = from.paint_order,
            _ => to.shape_rendering = from.shape_rendering,
        }
        return true;
    }
    if !matches!(
        name,
        "stop-color"
            | "stop-opacity"
            | "vector-effect"
            | "clip-path"
            | "x"
            | "y"
            | "cx"
            | "cy"
            | "r"
            | "rx"
            | "ry"
    ) {
        return false;
    }
    if style.svg.is_none() && src.svg.is_none() {
        return true;
    }
    let (from, to) = (src.svg_own().clone(), style.svg_own_mut());
    match name {
        "stop-color" => to.stop_color = from.stop_color,
        "stop-opacity" => to.stop_opacity = from.stop_opacity,
        "vector-effect" => to.vector_effect = from.vector_effect,
        "clip-path" => to.clip_path = from.clip_path,
        "x" => to.x = from.x,
        "y" => to.y = from.y,
        "cx" => to.cx = from.cx,
        "cy" => to.cy = from.cy,
        "r" => to.r = from.r,
        "rx" => to.rx = from.rx,
        _ => to.ry = from.ry,
    }
    true
}

/// A length of SVG's properties: a `<length-percentage>` in `range`, or
/// a number, which is pixels (SVG 2 section 13.5.2 lets `stroke-width`
/// and the rest take one, in a sheet too, as the reference browser
/// does).
fn svg_length(tokens: &[Token], bases: &LengthBases<'_>, range: Range) -> Option<LengthPercentage> {
    if let Some(Token::Number { value, .. }) = single_token(tokens) {
        let px = *value as f32;
        return (px.is_finite() && range.holds(px))
            .then(|| LengthPercentage::Px(px.clamp(-MAX_LENGTH_PX, MAX_LENGTH_PX)));
    }
    length_percentage(tokens, bases, range)
}

/// `rx` and `ry`: `auto`, or a length not negative.
fn parse_radius(tokens: &[Token], bases: &LengthBases<'_>) -> Option<LengthPercentageAuto> {
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("auto")) {
        return Some(LengthPercentageAuto::Auto);
    }
    Some(match svg_length(tokens, bases, Range::NonNegative)? {
        LengthPercentage::Px(v) => LengthPercentageAuto::Px(v),
        LengthPercentage::Percent(p) => LengthPercentageAuto::Percent(p),
        LengthPercentage::Calc(c) => LengthPercentageAuto::Calc(c),
    })
}

/// A paint's `url()`: the reference as written.
fn url(tokens: &[Token]) -> Option<Arc<str>> {
    match trim(tokens) {
        [Token::Url { value, bad: false }] => Some(Arc::from(value.as_str())),
        [Token::Function(name), inside @ .., Token::CloseParen]
            if name.eq_ignore_ascii_case("url") =>
        {
            match trim(inside) {
                [Token::String { value, bad: false }] => Some(Arc::from(value.as_str())),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `none`, a colour or `currentcolor`: what a paint's fallback may be.
fn color_or_none(tokens: &[Token]) -> Option<SvgPaint> {
    match single_ident(tokens).map(str::to_ascii_lowercase).as_deref() {
        Some("none") => Some(SvgPaint::None),
        Some("currentcolor") => Some(SvgPaint::CurrentColor),
        _ => parse_color(tokens).map(SvgPaint::Color),
    }
}

/// `<paint>` (SVG 2 section 13.2): `none`, a colour, `currentcolor`,
/// `url()` and a fallback of `none` or a colour, `context-fill` or
/// `context-stroke`.
pub(super) fn parse_paint(tokens: &[Token]) -> Option<SvgPaint> {
    match components(tokens).as_slice() {
        [one] => {
            if let Some(reference) = url(one) {
                return Some(SvgPaint::Url(reference, None));
            }
            match single_ident(one).map(str::to_ascii_lowercase).as_deref() {
                Some("context-fill") => Some(SvgPaint::ContextFill),
                Some("context-stroke") => Some(SvgPaint::ContextStroke),
                _ => color_or_none(one),
            }
        }
        [reference, fallback] => Some(SvgPaint::Url(
            url(reference)?,
            Some(Box::new(color_or_none(fallback)?)),
        )),
        _ => None,
    }
}

/// `stop-color`: a colour or `currentcolor`.
fn parse_stop_color(tokens: &[Token]) -> Option<SvgPaint> {
    color_or_none(tokens).filter(|paint| *paint != SvgPaint::None)
}

/// One of `words`, ASCII case-insensitively.
fn keyword<T: Copy>(tokens: &[Token], words: &[(&str, T)]) -> Option<T> {
    let word = single_ident(tokens)?;
    words
        .iter()
        .find(|(name, _)| word.eq_ignore_ascii_case(name))
        .map(|&(_, value)| value)
}

fn parse_fill_rule(tokens: &[Token]) -> Option<FillRule> {
    keyword(
        tokens,
        &[
            ("nonzero", FillRule::NonZero),
            ("evenodd", FillRule::EvenOdd),
        ],
    )
}

fn parse_linecap(tokens: &[Token]) -> Option<StrokeLinecap> {
    keyword(
        tokens,
        &[
            ("butt", StrokeLinecap::Butt),
            ("round", StrokeLinecap::Round),
            ("square", StrokeLinecap::Square),
        ],
    )
}

fn parse_linejoin(tokens: &[Token]) -> Option<StrokeLinejoin> {
    keyword(
        tokens,
        &[
            ("miter", StrokeLinejoin::Miter),
            ("round", StrokeLinejoin::Round),
            ("bevel", StrokeLinejoin::Bevel),
        ],
    )
}

fn parse_text_anchor(tokens: &[Token]) -> Option<TextAnchor> {
    keyword(
        tokens,
        &[
            ("start", TextAnchor::Start),
            ("middle", TextAnchor::Middle),
            ("end", TextAnchor::End),
        ],
    )
}

fn parse_dominant_baseline(tokens: &[Token]) -> Option<DominantBaseline> {
    use DominantBaseline::*;
    keyword(
        tokens,
        &[
            ("auto", Auto),
            ("text-bottom", TextBottom),
            ("alphabetic", Alphabetic),
            ("ideographic", Ideographic),
            ("middle", Middle),
            ("central", Central),
            ("mathematical", Mathematical),
            ("hanging", Hanging),
            ("text-top", TextTop),
        ],
    )
}

fn parse_shape_rendering(tokens: &[Token]) -> Option<ShapeRendering> {
    keyword(
        tokens,
        &[
            ("auto", ShapeRendering::Auto),
            ("optimizespeed", ShapeRendering::OptimizeSpeed),
            ("crispedges", ShapeRendering::CrispEdges),
            ("geometricprecision", ShapeRendering::GeometricPrecision),
        ],
    )
}

fn parse_vector_effect(tokens: &[Token]) -> Option<VectorEffect> {
    keyword(
        tokens,
        &[
            ("none", VectorEffect::None),
            ("non-scaling-stroke", VectorEffect::NonScalingStroke),
        ],
    )
}

/// `stroke-miterlimit`: a number not negative (the reference browser
/// takes one below 1).
fn parse_miterlimit(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    let value = match math(tokens, Want::Number, bases, None) {
        Some(Computed::Value(v)) => v.max(0.0),
        Some(_) => return None,
        None => match single_token(tokens)? {
            Token::Number { value, .. } if *value >= 0.0 => *value,
            _ => return None,
        },
    };
    let value = value as f32;
    value.is_finite().then_some(value)
}

/// `stroke-dasharray: none | [<length-percentage> | <number>]+#`, the
/// values separated by commas or white space, none negative.
fn parse_dasharray(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Option<Arc<[LengthPercentage]>>> {
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("none")) {
        return Some(None);
    }
    let mut out = Vec::new();
    let mut after_comma = true;
    for part in components(tokens) {
        if matches!(part, [Token::Comma]) {
            if after_comma {
                return None;
            }
            after_comma = true;
            continue;
        }
        out.push(svg_length(part, bases, Range::NonNegative)?);
        after_comma = false;
    }
    if after_comma {
        return None;
    }
    Some(Some(Arc::from(out)))
}

/// `paint-order: normal | [fill || stroke || markers]`: the layers
/// named first, the rest after them in their usual order.
fn parse_paint_order(tokens: &[Token]) -> Option<PaintOrder> {
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("normal")) {
        return Some(PaintOrder(None));
    }
    let mut layers = Vec::new();
    for part in components(tokens) {
        let layer = keyword(
            part,
            &[
                ("fill", PaintLayer::Fill),
                ("stroke", PaintLayer::Stroke),
                ("markers", PaintLayer::Markers),
            ],
        )?;
        if layers.contains(&layer) {
            return None;
        }
        layers.push(layer);
    }
    if layers.is_empty() {
        return None;
    }
    for layer in [PaintLayer::Fill, PaintLayer::Stroke, PaintLayer::Markers] {
        if !layers.contains(&layer) {
            layers.push(layer);
        }
    }
    Some(PaintOrder(Some([layers[0], layers[1], layers[2]])))
}

/// `clip-path` for SVG's clipping paths: `none` or a `url()`; a basic
/// shape or a box is no value here (G87).
fn parse_clip_path(tokens: &[Token]) -> Option<Option<Arc<str>>> {
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("none")) {
        return Some(None);
    }
    url(tokens).map(Some)
}

/// The properties every SVG element has a presentation attribute for
/// (SVG 2 section 6.6), sorted: those this engine knows apply, the rest
/// are ignored as any declaration of an unknown property is.
const PRESENTATION: &[&str] = &[
    "alignment-baseline",
    "baseline-shift",
    "clip-path",
    "clip-rule",
    "color",
    "color-interpolation",
    "color-interpolation-filters",
    "color-rendering",
    "cursor",
    "direction",
    "display",
    "dominant-baseline",
    "fill",
    "fill-opacity",
    "fill-rule",
    "filter",
    "flood-color",
    "flood-opacity",
    "font-family",
    "font-size",
    "font-size-adjust",
    "font-stretch",
    "font-style",
    "font-variant",
    "font-weight",
    "image-rendering",
    "letter-spacing",
    "lighting-color",
    "marker-end",
    "marker-mid",
    "marker-start",
    "mask",
    "mask-type",
    "opacity",
    "overflow",
    "paint-order",
    "pointer-events",
    "shape-rendering",
    "stop-color",
    "stop-opacity",
    "stroke",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-opacity",
    "stroke-width",
    "text-anchor",
    "text-decoration",
    "text-overflow",
    "text-rendering",
    "transform",
    "transform-origin",
    "unicode-bidi",
    "vector-effect",
    "visibility",
    "white-space",
    "word-spacing",
    "writing-mode",
];

/// Whether `name` is a geometry property's presentation attribute on an
/// SVG element named `tag` (SVG 2 sections 7 and 10; `x` and `y` of a
/// `text` are lists, and no hint).
fn is_geometry(tag: &str, name: &str) -> bool {
    match name {
        "x" | "y" | "width" | "height" => {
            matches!(tag, "svg" | "rect" | "image" | "foreignObject" | "use")
        }
        "cx" | "cy" => matches!(tag, "circle" | "ellipse"),
        "r" => tag == "circle",
        "rx" | "ry" => matches!(tag, "rect" | "ellipse"),
        _ => false,
    }
}

/// Whether the attribute `name` of an SVG element named `tag` is a
/// presentation attribute, whose change restyles the element.
pub(crate) fn is_svg_hint(tag: &str, name: &str) -> bool {
    PRESENTATION.binary_search(&name).is_ok() || is_geometry(tag, name)
}

/// CSS's properties whose presentation attribute takes a number for
/// pixels where the property takes a length (SVG's own take one in a
/// sheet too).
fn takes_unitless(name: &str) -> bool {
    matches!(
        name,
        "baseline-shift"
            | "font-size"
            | "letter-spacing"
            | "word-spacing"
            | "transform-origin"
            | "width"
            | "height"
    )
}

/// The declarations an SVG element named `tag` has from its
/// presentation attributes (SVG 2 section 6.6), in the attributes'
/// order: each attribute's value parsed by its property's grammar, one
/// declaration and never `!important`, white space at its ends no
/// error, a number a pixel length where the property takes a length; a
/// value the property does not take is a declaration the cascade
/// ignores. The `transform` attribute's grammar is SVG's own, and it is
/// declared as its `matrix()`; a negative `width` or `height` is 0, as
/// the reference browser has an `<svg>`'s.
pub(super) fn svg_hints(tag: &str, attrs: &[(String, String)]) -> Vec<Declaration> {
    let mut out = Vec::new();
    for (name, value) in attrs {
        if !is_svg_hint(tag, name) {
            continue;
        }
        let value = if name == "transform" {
            if value.trim_matches(is_space).is_empty() {
                continue;
            }
            let Some(m) = parse_transform_list(value) else {
                continue;
            };
            let mut tokens = vec![Token::Function("matrix".to_owned())];
            for (i, v) in m.iter().enumerate() {
                if i > 0 {
                    tokens.push(Token::Comma);
                }
                tokens.push(Token::Number {
                    value: *v,
                    is_integer: v.fract() == 0.0,
                });
            }
            tokens.push(Token::CloseParen);
            tokens
        } else {
            let mut tokens = trim(&css::tokenize(value)).to_vec();
            if tokens.is_empty() {
                continue;
            }
            if takes_unitless(name) {
                unitless_to_px(&mut tokens);
            }
            if matches!(name.as_str(), "width" | "height")
                && let [Token::Dimension { value, .. }] = tokens.as_mut_slice()
                && *value < 0.0
            {
                *value = 0.0;
            }
            tokens
        };
        out.push(Declaration {
            name: name.clone(),
            value,
            important: false,
        });
    }
    out
}

/// White space as the attribute grammars have it.
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// Each number outside a function made that many pixels.
fn unitless_to_px(tokens: &mut [Token]) {
    let mut depth = 0usize;
    for token in tokens {
        match token {
            Token::Function(_) | Token::OpenParen => depth += 1,
            Token::CloseParen => depth = depth.saturating_sub(1),
            Token::Number { value, .. } if depth == 0 => {
                *token = Token::Dimension {
                    value: *value,
                    unit: "px".to_owned(),
                };
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::with_bases;
    use super::*;

    fn tokens(text: &str) -> Vec<Token> {
        css::tokenize(text)
    }

    #[test]
    fn a_paint_is_none_a_colour_currentcolor_or_a_reference_with_a_fallback() {
        let red = parse_color(&tokens("red")).unwrap();
        assert_eq!(parse_paint(&tokens("none")), Some(SvgPaint::None));
        assert_eq!(parse_paint(&tokens("RED")), Some(SvgPaint::Color(red)));
        assert_eq!(
            parse_paint(&tokens("currentColor")),
            Some(SvgPaint::CurrentColor)
        );
        assert_eq!(
            parse_paint(&tokens("context-fill")),
            Some(SvgPaint::ContextFill)
        );
        assert_eq!(
            parse_paint(&tokens("url(#g)")),
            Some(SvgPaint::Url("#g".into(), None))
        );
        assert_eq!(
            parse_paint(&tokens("url(\"#g\") none")),
            Some(SvgPaint::Url("#g".into(), Some(Box::new(SvgPaint::None))))
        );
        assert_eq!(
            parse_paint(&tokens("url(#g) red")),
            Some(SvgPaint::Url(
                "#g".into(),
                Some(Box::new(SvgPaint::Color(red)))
            ))
        );
        for bad in [
            "bogus",
            "red !important",
            "red;stroke:blue",
            "url(#g) context-fill",
            "red url(#g)",
            "url(#g) red blue",
            "",
        ] {
            assert_eq!(parse_paint(&tokens(bad)), None, "{bad}");
        }
        assert_eq!(parse_stop_color(&tokens("none")), None);
    }

    #[test]
    fn a_dash_array_is_lengths_or_numbers_by_commas_or_white_space() {
        with_bases(|bases| {
            let px = |v| LengthPercentage::Px(v);
            assert_eq!(
                parse_dasharray(&tokens("1, 2 3"), bases),
                Some(Some(Arc::from(vec![px(1.0), px(2.0), px(3.0)])))
            );
            assert_eq!(
                parse_dasharray(&tokens("10% 2"), bases),
                Some(Some(Arc::from(vec![
                    LengthPercentage::Percent(10.0),
                    px(2.0)
                ])))
            );
            assert_eq!(parse_dasharray(&tokens("none"), bases), Some(None));
            assert_eq!(
                parse_dasharray(&tokens("0"), bases),
                Some(Some(Arc::from(vec![px(0.0)])))
            );
            for bad in ["1,,2", "-1 2", ",1", "1,", "", "red"] {
                assert_eq!(parse_dasharray(&tokens(bad), bases), None, "{bad}");
            }
        });
    }

    #[test]
    fn paint_order_names_the_first_layers_and_keeps_normal_apart() {
        use PaintLayer::*;
        assert_eq!(parse_paint_order(&tokens("normal")), Some(PaintOrder(None)));
        assert_eq!(
            parse_paint_order(&tokens("stroke")),
            Some(PaintOrder(Some([Stroke, Fill, Markers])))
        );
        assert_eq!(
            parse_paint_order(&tokens("markers stroke")),
            Some(PaintOrder(Some([Markers, Stroke, Fill])))
        );
        assert_eq!(
            parse_paint_order(&tokens("fill stroke markers")),
            Some(PaintOrder(Some([Fill, Stroke, Markers])))
        );
        assert_eq!(parse_paint_order(&tokens("stroke stroke")), None);
        assert_eq!(parse_paint_order(&tokens("normal fill")), None);
    }

    #[test]
    fn the_lengths_take_numbers_and_the_radii_auto() {
        with_bases(|bases| {
            assert_eq!(
                svg_length(&tokens("3"), bases, Range::NonNegative),
                Some(LengthPercentage::Px(3.0))
            );
            assert_eq!(svg_length(&tokens("-1"), bases, Range::NonNegative), None);
            assert_eq!(
                svg_length(&tokens("-3"), bases, Range::All),
                Some(LengthPercentage::Px(-3.0))
            );
            assert_eq!(
                parse_radius(&tokens("auto"), bases),
                Some(LengthPercentageAuto::Auto)
            );
            assert_eq!(
                parse_radius(&tokens("10%"), bases),
                Some(LengthPercentageAuto::Percent(10.0))
            );
            assert_eq!(parse_miterlimit(&tokens("0.5"), bases), Some(0.5));
            assert_eq!(parse_miterlimit(&tokens("-1"), bases), None);
        });
        assert_eq!(parse_linejoin(&tokens("miter-clip")), None);
        assert_eq!(
            parse_shape_rendering(&tokens("crispEdges")),
            Some(ShapeRendering::CrispEdges)
        );
        assert_eq!(parse_clip_path(&tokens("url(#c)")), Some(Some("#c".into())));
        assert_eq!(parse_clip_path(&tokens("circle(5px)")), None);
    }

    fn hints(tag: &str, attrs: &[(&str, &str)]) -> Vec<(String, String)> {
        let attrs: Vec<(String, String)> = attrs
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect();
        svg_hints(tag, &attrs)
            .into_iter()
            .map(|d| {
                let value: Vec<String> = d.value.iter().map(|t| format!("{t:?}")).collect();
                (d.name, value.join(" "))
            })
            .collect()
    }

    #[test]
    fn presentation_attributes_are_one_declaration_each_and_never_important() {
        let got = hints(
            "rect",
            &[
                ("fill", "  red  "),
                ("id", "r"),
                ("width", "10"),
                ("stroke-width", "2em"),
                ("transform", "translate(3 4)"),
                ("cx", "5"),
                ("FILL", "blue"),
            ],
        );
        let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["fill", "width", "stroke-width", "transform"]);
        assert_eq!(got[0].1, "Ident(\"red\")");
        assert!(got[1].1.contains("unit: \"px\""), "{}", got[1].1);
        assert!(got[3].1.starts_with("Function(\"matrix\")"), "{}", got[3].1);
        // A list in error, or empty, declares nothing.
        assert!(hints("g", &[("transform", "scale(2) foo")]).is_empty());
        assert!(hints("g", &[("transform", "  ")]).is_empty());
        // The geometry is the element's own.
        assert_eq!(hints("circle", &[("r", "2"), ("x", "9")]).len(), 1);
        assert!(hints("rect", &[("r", "2"), ("cx", "1")]).is_empty());
        assert!(hints("text", &[("x", "5"), ("y", "6")]).is_empty());
        assert!(hints("svg", &[("width", "-5")])[0].1.contains("value: 0.0"));
        assert!(is_svg_hint("rect", "rx") && !is_svg_hint("circle", "rx"));
        assert!(is_svg_hint("g", "font-size") && !is_svg_hint("g", "style"));
        let mut sorted = PRESENTATION.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, PRESENTATION, "sorted for the binary search");
    }
}
