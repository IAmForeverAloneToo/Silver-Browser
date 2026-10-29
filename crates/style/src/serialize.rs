//! Computed values as text (M25, `getComputedStyle`): CSSOM's
//! serialization of the computed values of the properties this engine
//! computes, and of custom properties. Colors are `rgb()` or `rgba()`,
//! lengths `px` at their shortest, numbers and keywords as they are.
//! What the reference browser gives as a used value (`width` of a box
//! laid out, a percentage resolved) is given here as the computed value,
//! a percentage as written (G46); a `calc()` with a percentage, a
//! transform's matrix, are not given (the empty string).

use css::Token;

use crate::computed::{
    AlignPosition, BaselinePosition, BorderCollapse, BorderStyle, BoxSizing, CaptionSide, Clear,
    Color, ComputedStyle, ContentAlignment, Direction, Display, EmptyCells, FlexBasis,
    FlexDirection, FlexWrap, Float, LengthPercentage, LengthPercentageAuto, MaxSize, Overflow,
    OverflowAlign, Position, SelfAlignment, Size, TableLayout, TextAlign, TextTransform,
    TransformBox, VerticalAlign, WhiteSpace,
};
use crate::computed::{
    DominantBaseline, FillRule, PaintLayer, ShapeRendering, StrokeLinecap, StrokeLinejoin,
    SvgPaint, TextAnchor, VectorEffect,
};

/// The properties [`computed_value`] gives, in the order a computed
/// style's declaration block lists them (alphabetical, as the reference
/// browser's).
pub const COMPUTED_PROPERTIES: &[&str] = &[
    "align-content",
    "align-items",
    "align-self",
    "background-color",
    "border-bottom-color",
    "border-bottom-style",
    "border-bottom-width",
    "border-collapse",
    "border-left-color",
    "border-left-style",
    "border-left-width",
    "border-right-color",
    "border-right-style",
    "border-right-width",
    "border-top-color",
    "border-top-style",
    "border-top-width",
    "bottom",
    "box-sizing",
    "caption-side",
    "clear",
    "clip-path",
    "clip-rule",
    "color",
    "column-gap",
    "cursor",
    "cx",
    "cy",
    "direction",
    "display",
    "dominant-baseline",
    "empty-cells",
    "fill",
    "fill-opacity",
    "fill-rule",
    "flex-basis",
    "flex-direction",
    "flex-grow",
    "flex-shrink",
    "flex-wrap",
    "float",
    "font-family",
    "font-size",
    "height",
    "justify-content",
    "left",
    "letter-spacing",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "margin-top",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "opacity",
    "order",
    "outline-color",
    "outline-offset",
    "outline-style",
    "outline-width",
    "overflow-x",
    "overflow-y",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "padding-top",
    "paint-order",
    "position",
    "r",
    "right",
    "row-gap",
    "rx",
    "ry",
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
    "table-layout",
    "text-align",
    "text-anchor",
    "text-transform",
    "top",
    "transform-box",
    "vector-effect",
    "vertical-align",
    "white-space",
    "width",
    "word-spacing",
    "x",
    "y",
    "z-index",
];

/// `name`'s computed value in `style` as text, `name` lowercase or a
/// custom property's; `None` for a property this engine does not give.
pub fn computed_value(style: &ComputedStyle, name: &str) -> Option<String> {
    if name.starts_with("--") {
        return style.custom_property(name).map(tokens);
    }
    const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];
    let side = |prefix: &str, suffix: &str| {
        SIDES.iter().position(|side| {
            name.strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(suffix))
                == Some(side)
        })
    };
    if let Some(i) = side("margin-", "") {
        return Some(auto_length(&style.margin[i]));
    }
    if let Some(i) = side("padding-", "") {
        return length_percentage(&style.padding[i]);
    }
    if let Some(i) = side("border-", "-width") {
        return Some(px(style.used_border_width()[i]));
    }
    if let Some(i) = side("border-", "-color") {
        return Some(color(style.border_color[i]));
    }
    if let Some(i) = side("border-", "-style") {
        return Some(border_style(style.border_style[i]).to_owned());
    }
    if let Some(i) = SIDES.iter().position(|side| *side == name) {
        return Some(auto_length(&style.inset[i]));
    }
    Some(match name {
        "color" => color(style.color),
        "background-color" => color(style.background_color),
        "font-size" => px(style.font_size_px),
        "font-family" => style.font_family.to_string(),
        "display" => display(style.display).to_owned(),
        "text-align" => text_align(style.text_align).to_owned(),
        "text-transform" => match style.text_transform {
            TextTransform::None => "none",
            TextTransform::Uppercase => "uppercase",
            TextTransform::Lowercase => "lowercase",
            TextTransform::Capitalize => "capitalize",
        }
        .to_owned(),
        "white-space" => match style.white_space {
            WhiteSpace::Normal => "normal",
            WhiteSpace::Pre => "pre",
            WhiteSpace::PreWrap => "pre-wrap",
            WhiteSpace::PreLine => "pre-line",
        }
        .to_owned(),
        "letter-spacing" => spacing(style.letter_spacing),
        "word-spacing" => px(style.word_spacing),
        "direction" => match style.direction {
            Direction::Ltr => "ltr",
            Direction::Rtl => "rtl",
        }
        .to_owned(),
        "width" => size(&style.width)?,
        "height" => size(&style.height)?,
        "min-width" => size(&style.min_width)?,
        "min-height" => size(&style.min_height)?,
        "max-width" => max_size(&style.max_width)?,
        "max-height" => max_size(&style.max_height)?,
        "overflow-x" | "overflow-y" => match style.overflow {
            Overflow::Visible => "visible",
            Overflow::Hidden => "hidden",
            Overflow::Scroll => "scroll",
            Overflow::Auto => "auto",
            Overflow::Clip => "clip",
        }
        .to_owned(),
        "box-sizing" => match style.box_sizing {
            BoxSizing::ContentBox => "content-box",
            BoxSizing::BorderBox => "border-box",
        }
        .to_owned(),
        "float" => match style.float {
            Float::None => "none",
            Float::Left => "left",
            Float::Right => "right",
        }
        .to_owned(),
        "clear" => match style.clear {
            Clear::None => "none",
            Clear::Left => "left",
            Clear::Right => "right",
            Clear::Both => "both",
        }
        .to_owned(),
        "position" => match style.position {
            Position::Static => "static",
            Position::Relative => "relative",
            Position::Absolute => "absolute",
            Position::Fixed => "fixed",
            Position::Sticky => "sticky",
        }
        .to_owned(),
        "z-index" => style
            .z_index
            .map_or_else(|| "auto".to_owned(), |z| z.to_string()),
        "table-layout" => match style.table_layout {
            TableLayout::Auto => "auto",
            TableLayout::Fixed => "fixed",
        }
        .to_owned(),
        "border-collapse" => match style.border_collapse {
            BorderCollapse::Separate => "separate",
            BorderCollapse::Collapse => "collapse",
        }
        .to_owned(),
        "outline-style" => style.outline().style.keyword().to_owned(),
        "outline-width" => px(style.used_outline_width()),
        "outline-color" => color(style.outline_color()),
        "outline-offset" => px(style.outline().offset),
        "cursor" => style.cursor.keyword().to_owned(),
        "caption-side" => match style.caption_side {
            CaptionSide::Top => "top",
            CaptionSide::Bottom => "bottom",
        }
        .to_owned(),
        "empty-cells" => match style.empty_cells {
            EmptyCells::Show => "show",
            EmptyCells::Hide => "hide",
        }
        .to_owned(),
        "vertical-align" => match &style.vertical_align {
            VerticalAlign::Baseline => "baseline".to_owned(),
            VerticalAlign::Sub => "sub".to_owned(),
            VerticalAlign::Super => "super".to_owned(),
            VerticalAlign::Top => "top".to_owned(),
            VerticalAlign::TextTop => "text-top".to_owned(),
            VerticalAlign::Middle => "middle".to_owned(),
            VerticalAlign::Bottom => "bottom".to_owned(),
            VerticalAlign::TextBottom => "text-bottom".to_owned(),
            VerticalAlign::Length(v) => px(*v),
            VerticalAlign::Percent(p) => percent(*p),
            VerticalAlign::Calc(_) => return None,
        },
        "flex-direction" => match style.flex_direction {
            FlexDirection::Row => "row",
            FlexDirection::RowReverse => "row-reverse",
            FlexDirection::Column => "column",
            FlexDirection::ColumnReverse => "column-reverse",
        }
        .to_owned(),
        "flex-wrap" => match style.flex_wrap {
            FlexWrap::NoWrap => "nowrap",
            FlexWrap::Wrap => "wrap",
            FlexWrap::WrapReverse => "wrap-reverse",
        }
        .to_owned(),
        "order" => style.order.to_string(),
        "flex-grow" => number(style.flex_grow),
        "flex-shrink" => number(style.flex_shrink),
        "flex-basis" => match &style.flex_basis {
            FlexBasis::Content => "content".to_owned(),
            FlexBasis::Size(s) => size(s)?,
        },
        "justify-content" => content_alignment(style.justify_content),
        "align-content" => content_alignment(style.align_content),
        "align-items" => self_alignment(style.align_items),
        "align-self" => self_alignment(style.align_self),
        "row-gap" => gap(&style.row_gap)?,
        "column-gap" => gap(&style.column_gap)?,
        "opacity" => number(style.effects().opacity),
        "transform-box" => match style.effects().transform_box {
            TransformBox::ContentBox => "content-box",
            TransformBox::BorderBox => "border-box",
            TransformBox::FillBox => "fill-box",
            TransformBox::StrokeBox => "stroke-box",
            TransformBox::ViewBox => "view-box",
        }
        .to_owned(),
        _ => return svg(style, name),
    })
}

/// SVG's properties (M32), as the reference browser writes them:
/// `currentcolor` as the colour it is, a reference as a `url()` of a
/// string, `paint-order` as the layers it names.
fn svg(style: &ComputedStyle, name: &str) -> Option<String> {
    let (s, own) = (style.svg(), style.svg_own());
    let reference = |r: &str| {
        let quoted = tokens(&[Token::String {
            value: r.to_owned(),
            bad: false,
        }]);
        format!("url({quoted})")
    };
    let paint = |paint: &SvgPaint| -> String {
        match paint {
            SvgPaint::None => "none".to_owned(),
            SvgPaint::Color(c) => color(*c),
            SvgPaint::CurrentColor => color(style.color),
            SvgPaint::ContextFill => "context-fill".to_owned(),
            SvgPaint::ContextStroke => "context-stroke".to_owned(),
            SvgPaint::Url(r, None) => reference(r),
            SvgPaint::Url(r, Some(fallback)) => {
                let fallback = match &**fallback {
                    SvgPaint::None => "none".to_owned(),
                    SvgPaint::Color(c) => color(*c),
                    _ => color(style.color),
                };
                format!("{} {fallback}", reference(r))
            }
        }
    };
    let rule = |r: FillRule| match r {
        FillRule::NonZero => "nonzero",
        FillRule::EvenOdd => "evenodd",
    };
    Some(match name {
        "fill" => paint(&s.fill),
        "stroke" => paint(&s.stroke),
        "stop-color" => paint(&own.stop_color),
        "fill-opacity" => number(s.fill_opacity),
        "stroke-opacity" => number(s.stroke_opacity),
        "stop-opacity" => number(own.stop_opacity),
        "fill-rule" => rule(s.fill_rule).to_owned(),
        "clip-rule" => rule(s.clip_rule).to_owned(),
        "stroke-width" => length_percentage(&s.stroke_width)?,
        "stroke-dashoffset" => length_percentage(&s.stroke_dashoffset)?,
        "stroke-miterlimit" => number(s.stroke_miterlimit),
        "stroke-dasharray" => match &s.stroke_dasharray {
            None => "none".to_owned(),
            Some(dashes) => dashes
                .iter()
                .map(length_percentage)
                .collect::<Option<Vec<_>>>()?
                .join(", "),
        },
        "stroke-linecap" => match s.stroke_linecap {
            StrokeLinecap::Butt => "butt",
            StrokeLinecap::Round => "round",
            StrokeLinecap::Square => "square",
        }
        .to_owned(),
        "stroke-linejoin" => match s.stroke_linejoin {
            StrokeLinejoin::Miter => "miter",
            StrokeLinejoin::Round => "round",
            StrokeLinejoin::Bevel => "bevel",
        }
        .to_owned(),
        "text-anchor" => match s.text_anchor {
            TextAnchor::Start => "start",
            TextAnchor::Middle => "middle",
            TextAnchor::End => "end",
        }
        .to_owned(),
        "dominant-baseline" => match s.dominant_baseline {
            DominantBaseline::Auto => "auto",
            DominantBaseline::TextBottom => "text-bottom",
            DominantBaseline::Alphabetic => "alphabetic",
            DominantBaseline::Ideographic => "ideographic",
            DominantBaseline::Middle => "middle",
            DominantBaseline::Central => "central",
            DominantBaseline::Mathematical => "mathematical",
            DominantBaseline::Hanging => "hanging",
            DominantBaseline::TextTop => "text-top",
        }
        .to_owned(),
        "paint-order" => match s.paint_order.0 {
            None => "normal".to_owned(),
            Some(layers) => {
                let word = |l: PaintLayer| match l {
                    PaintLayer::Fill => "fill",
                    PaintLayer::Stroke => "stroke",
                    PaintLayer::Markers => "markers",
                };
                // The first layer alone when the other two follow in
                // their usual order, else the first two.
                if (layers[1] as u8) < (layers[2] as u8) {
                    word(layers[0]).to_owned()
                } else {
                    format!("{} {}", word(layers[0]), word(layers[1]))
                }
            }
        },
        "shape-rendering" => match s.shape_rendering {
            ShapeRendering::Auto => "auto",
            ShapeRendering::OptimizeSpeed => "optimizespeed",
            ShapeRendering::CrispEdges => "crispedges",
            ShapeRendering::GeometricPrecision => "geometricprecision",
        }
        .to_owned(),
        "vector-effect" => match own.vector_effect {
            VectorEffect::None => "none",
            VectorEffect::NonScalingStroke => "non-scaling-stroke",
        }
        .to_owned(),
        "clip-path" => match &own.clip_path {
            None => "none".to_owned(),
            Some(r) => reference(r),
        },
        "x" => length_percentage(&own.x)?,
        "y" => length_percentage(&own.y)?,
        "cx" => length_percentage(&own.cx)?,
        "cy" => length_percentage(&own.cy)?,
        "r" => length_percentage(&own.r)?,
        "rx" => auto_length(&own.rx),
        "ry" => auto_length(&own.ry),
        _ => return None,
    })
}

/// A number at its shortest, `-0` as `0`.
fn number(value: f32) -> String {
    if value == 0.0 {
        "0".to_owned()
    } else {
        value.to_string()
    }
}

fn px(value: f32) -> String {
    format!("{}px", number(value))
}

fn percent(value: f32) -> String {
    format!("{}%", number(value))
}

/// `letter-spacing`: `normal` for zero, as the reference browser gives
/// it.
fn spacing(value: f32) -> String {
    if value == 0.0 {
        "normal".to_owned()
    } else {
        px(value)
    }
}

/// A color as CSSOM serializes an sRGB one: `rgb(r, g, b)`, or
/// `rgba(r, g, b, a)` when not opaque.
fn color(c: Color) -> String {
    if !c.is_legacy() {
        return crate::color::serialize(&c);
    }
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b) = (channel(c.r), channel(c.g), channel(c.b));
    if c.a >= 1.0 {
        format!("rgb({r}, {g}, {b})")
    } else {
        let alpha = (c.a.clamp(0.0, 1.0) * 1000.0).round() / 1000.0;
        format!("rgba({r}, {g}, {b}, {})", number(alpha))
    }
}

fn length_percentage(value: &LengthPercentage) -> Option<String> {
    match value {
        LengthPercentage::Px(v) => Some(px(*v)),
        LengthPercentage::Percent(p) => Some(percent(*p)),
        LengthPercentage::Calc(_) => None,
    }
}

fn auto_length(value: &LengthPercentageAuto) -> String {
    match value {
        LengthPercentageAuto::Auto => "auto".to_owned(),
        LengthPercentageAuto::Px(v) => px(*v),
        LengthPercentageAuto::Percent(p) => percent(*p),
        LengthPercentageAuto::Calc(_) => String::new(),
    }
}

fn size(value: &Size) -> Option<String> {
    Some(match value {
        Size::Auto => "auto".to_owned(),
        Size::Px(v) => px(*v),
        Size::Percent(p) => percent(*p),
        Size::Calc(_) => return None,
        Size::MinContent => "min-content".to_owned(),
        Size::MaxContent => "max-content".to_owned(),
        Size::FitContent => "fit-content".to_owned(),
    })
}

fn max_size(value: &MaxSize) -> Option<String> {
    Some(match value {
        MaxSize::None => "none".to_owned(),
        MaxSize::Px(v) => px(*v),
        MaxSize::Percent(p) => percent(*p),
        MaxSize::Calc(_) => return None,
        MaxSize::MinContent => "min-content".to_owned(),
        MaxSize::MaxContent => "max-content".to_owned(),
        MaxSize::FitContent => "fit-content".to_owned(),
    })
}

fn gap(value: &Option<LengthPercentage>) -> Option<String> {
    match value {
        None => Some("normal".to_owned()),
        Some(value) => length_percentage(value),
    }
}

fn display(value: Display) -> &'static str {
    match value {
        Display::Block => "block",
        Display::Inline => "inline",
        Display::InlineBlock => "inline-block",
        Display::None => "none",
        Display::Table => "table",
        Display::InlineTable => "inline-table",
        Display::TableRowGroup => "table-row-group",
        Display::TableHeaderGroup => "table-header-group",
        Display::TableFooterGroup => "table-footer-group",
        Display::TableRow => "table-row",
        Display::TableColumnGroup => "table-column-group",
        Display::TableColumn => "table-column",
        Display::TableCell => "table-cell",
        Display::TableCaption => "table-caption",
        Display::Flex => "flex",
        Display::InlineFlex => "inline-flex",
        Display::Grid => "grid",
        Display::InlineGrid => "inline-grid",
        Display::ListItem => "list-item",
    }
}

fn text_align(value: TextAlign) -> &'static str {
    match value {
        TextAlign::Start => "start",
        TextAlign::End => "end",
        TextAlign::Left => "left",
        TextAlign::Right => "right",
        TextAlign::Center => "center",
        TextAlign::Justify => "justify",
    }
}

fn border_style(value: BorderStyle) -> &'static str {
    match value {
        BorderStyle::None => "none",
        BorderStyle::Hidden => "hidden",
        BorderStyle::Solid => "solid",
        BorderStyle::Double => "double",
        BorderStyle::Dashed => "dashed",
        BorderStyle::Dotted => "dotted",
        BorderStyle::Groove => "groove",
        BorderStyle::Ridge => "ridge",
        BorderStyle::Inset => "inset",
        BorderStyle::Outset => "outset",
    }
}

fn baseline(value: BaselinePosition) -> &'static str {
    match value {
        BaselinePosition::First => "baseline",
        BaselinePosition::Last => "last baseline",
    }
}

fn position(value: AlignPosition, overflow: OverflowAlign) -> String {
    let word = match value {
        AlignPosition::Center => "center",
        AlignPosition::Start => "start",
        AlignPosition::End => "end",
        AlignPosition::SelfStart => "self-start",
        AlignPosition::SelfEnd => "self-end",
        AlignPosition::FlexStart => "flex-start",
        AlignPosition::FlexEnd => "flex-end",
        AlignPosition::Left => "left",
        AlignPosition::Right => "right",
    };
    match overflow {
        OverflowAlign::Default => word.to_owned(),
        OverflowAlign::Safe => format!("safe {word}"),
        OverflowAlign::Unsafe => format!("unsafe {word}"),
    }
}

fn content_alignment(value: ContentAlignment) -> String {
    match value {
        ContentAlignment::Normal => "normal".to_owned(),
        ContentAlignment::Baseline(b) => baseline(b).to_owned(),
        ContentAlignment::SpaceBetween => "space-between".to_owned(),
        ContentAlignment::SpaceAround => "space-around".to_owned(),
        ContentAlignment::SpaceEvenly => "space-evenly".to_owned(),
        ContentAlignment::Stretch => "stretch".to_owned(),
        ContentAlignment::Position(p, o) => position(p, o),
    }
}

fn self_alignment(value: SelfAlignment) -> String {
    match value {
        SelfAlignment::Auto => "auto".to_owned(),
        SelfAlignment::Normal => "normal".to_owned(),
        SelfAlignment::Stretch => "stretch".to_owned(),
        SelfAlignment::Baseline(b) => baseline(b).to_owned(),
        SelfAlignment::Position(p, o) => position(p, o),
    }
}

/// A custom property's value as text: its tokens written back, white
/// space as one space, trimmed; and a selector's or a condition's, for
/// the DevTools' rules (M36).
pub(crate) fn tokens(value: &[Token]) -> String {
    let mut out = String::new();
    for token in value {
        match token {
            Token::Whitespace => {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            Token::Ident(s) => out.push_str(s),
            Token::Function(s) => {
                out.push_str(s);
                out.push('(');
            }
            Token::AtKeyword(s) => {
                out.push('@');
                out.push_str(s);
            }
            Token::Hash { value, .. } => {
                out.push('#');
                out.push_str(value);
            }
            Token::String { value, .. } => {
                out.push('"');
                for c in value.chars() {
                    if c == '"' || c == '\\' {
                        out.push('\\');
                    }
                    out.push(c);
                }
                out.push('"');
            }
            Token::Url { value, .. } => {
                out.push_str("url(");
                out.push_str(value);
                out.push(')');
            }
            Token::Number { value, .. } => out.push_str(&number(*value as f32)),
            Token::Percentage { value } => out.push_str(&percent(*value as f32)),
            Token::Dimension { value, unit, .. } => {
                out.push_str(&number(*value as f32));
                out.push_str(unit);
            }
            Token::Delim(c) => out.push(*c),
            Token::Colon => out.push(':'),
            Token::Semicolon => out.push(';'),
            Token::Comma => out.push(','),
            Token::OpenSquare => out.push('['),
            Token::CloseSquare => out.push(']'),
            Token::OpenParen => out.push('('),
            Token::CloseParen => out.push(')'),
            Token::OpenCurly => out.push('{'),
            Token::CloseCurly => out.push('}'),
            _ => {}
        }
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_written_as_the_reference_browser_writes_them() {
        let mut style = ComputedStyle {
            color: Color::rgb8(10, 20, 30),
            ..ComputedStyle::default()
        };
        style.margin[3] = LengthPercentageAuto::Px(24.0);
        style.width = Size::Percent(50.0);
        style.effects_mut().opacity = 0.25;
        let value = |name| computed_value(&style, name);
        assert_eq!(value("color").as_deref(), Some("rgb(10, 20, 30)"));
        assert_eq!(value("margin-left").as_deref(), Some("24px"));
        assert_eq!(value("margin-top").as_deref(), Some("0px"));
        assert_eq!(value("width").as_deref(), Some("50%"));
        assert_eq!(value("opacity").as_deref(), Some("0.25"));
        assert_eq!(value("transform-box").as_deref(), Some("view-box"));
        let mut fill = ComputedStyle::default();
        fill.effects_mut().transform_box = TransformBox::FillBox;
        assert_eq!(
            computed_value(&fill, "transform-box").as_deref(),
            Some("fill-box")
        );
        assert_eq!(value("font-size").as_deref(), Some("16px"));
        assert_eq!(value("display").as_deref(), Some("inline"));
        assert_eq!(
            value("background-color").as_deref(),
            Some("rgba(0, 0, 0, 0)")
        );
        assert_eq!(value("border-top-style").as_deref(), Some("none"));
        assert_eq!(value("border-top-width").as_deref(), Some("0px"));
        assert_eq!(value("position").as_deref(), Some("static"));
        assert_eq!(value("z-index").as_deref(), Some("auto"));
        assert_eq!(value("top").as_deref(), Some("auto"));
        assert_eq!(value("letter-spacing").as_deref(), Some("normal"));
        assert_eq!(value("max-width").as_deref(), Some("none"));
        assert_eq!(value("row-gap").as_deref(), Some("normal"));
        assert_eq!(value("bogus"), None);
        // A channel between two bytes goes to the nearer.
        let half = Color::rgba(0.5, 0.1, 1.0, 1.0);
        assert_eq!(color(half), "rgb(128, 26, 255)");
        assert_eq!(value("--x"), None);
        assert_eq!(value("fill").as_deref(), Some("rgb(0, 0, 0)"));
        assert_eq!(value("stroke").as_deref(), Some("none"));
        assert_eq!(value("stroke-miterlimit").as_deref(), Some("4"));
        assert_eq!(value("paint-order").as_deref(), Some("normal"));
        assert_eq!(value("rx").as_deref(), Some("auto"));
        // Every property listed is given.
        for name in COMPUTED_PROPERTIES {
            assert!(value(name).is_some(), "{name}");
        }
        let mut sorted = COMPUTED_PROPERTIES.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, COMPUTED_PROPERTIES);
    }

    #[test]
    fn a_custom_propertys_tokens_are_written_back() {
        let written = tokens(&css::tokenize(" 10px  calc(1px + 2%) \"a\\\"b\" #f00 , x "));
        assert_eq!(written, "10px calc(1px + 2%) \"a\\\"b\" #f00 , x");
    }
}
