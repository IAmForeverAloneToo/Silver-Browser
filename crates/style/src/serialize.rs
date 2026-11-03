//! Computed values as text (M25, `getComputedStyle`): CSSOM's
//! serialization of the computed values of the properties this engine
//! computes, and of custom properties. Colors are `rgb()` or `rgba()`,
//! lengths `px` at their shortest, numbers and keywords as they are.
//! What the reference browser gives as a used value (`width` of a box
//! laid out, a percentage resolved) is given here as the computed value,
//! a percentage as written (G46); a `calc()` with a percentage as CSS
//! Values 4 section 10.13 writes it (D66).
//!
//! Every longhand the engine computes is given, those past the ones here
//! by `longhands`, and every shorthand of them by `shorthands`, from
//! their longhands' values (D66, G159); an `<image>` as the reference
//! browser writes it, its gradients among them (G232). What only the
//! caller knows, a used value or a URL made absolute, it gives through a
//! [`Resolver`].

mod longhands;
mod shorthands;

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
use crate::computed::{FontStyle, LineHeight};

/// The properties a computed style's declaration block lists, the
/// longhands [`computed_value`] gives, in the reference browser's order:
/// alphabetical, the `-webkit-` ones after (D66). `will-change`, whose
/// names the engine does not keep, is not among them.
pub const COMPUTED_PROPERTIES: &[&str] = &[
    "align-content",
    "align-items",
    "align-self",
    "animation-composition",
    "animation-delay",
    "animation-direction",
    "animation-duration",
    "animation-fill-mode",
    "animation-iteration-count",
    "animation-name",
    "animation-play-state",
    "animation-timing-function",
    "aspect-ratio",
    "backface-visibility",
    "background-attachment",
    "background-clip",
    "background-color",
    "background-image",
    "background-origin",
    "background-position-x",
    "background-position-y",
    "background-repeat",
    "background-size",
    "border-bottom-color",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-bottom-style",
    "border-bottom-width",
    "border-collapse",
    "border-image-outset",
    "border-image-repeat",
    "border-image-slice",
    "border-image-source",
    "border-image-width",
    "border-left-color",
    "border-left-style",
    "border-left-width",
    "border-right-color",
    "border-right-style",
    "border-right-width",
    "border-top-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-top-style",
    "border-top-width",
    "bottom",
    "box-shadow",
    "box-sizing",
    "break-after",
    "break-before",
    "break-inside",
    "caption-side",
    "clear",
    "clip-path",
    "clip-rule",
    "color",
    "color-scheme",
    "column-gap",
    "content",
    "counter-increment",
    "counter-reset",
    "counter-set",
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
    "filter",
    "flex-basis",
    "flex-direction",
    "flex-grow",
    "flex-shrink",
    "flex-wrap",
    "float",
    "font-family",
    "font-feature-settings",
    "font-kerning",
    "font-palette",
    "font-size",
    "font-stretch",
    "font-style",
    "font-variant-alternates",
    "font-variant-caps",
    "font-variant-east-asian",
    "font-variant-emoji",
    "font-variant-ligatures",
    "font-variant-numeric",
    "font-variant-position",
    "font-weight",
    "grid-auto-columns",
    "grid-auto-flow",
    "grid-auto-rows",
    "grid-column-end",
    "grid-column-start",
    "grid-row-end",
    "grid-row-start",
    "grid-template-areas",
    "grid-template-columns",
    "grid-template-rows",
    "hanging-punctuation",
    "height",
    "hyphenate-character",
    "hyphenate-limit-chars",
    "hyphenate-limit-lines",
    "hyphens",
    "image-orientation",
    "image-rendering",
    "justify-content",
    "justify-items",
    "justify-self",
    "left",
    "letter-spacing",
    "line-break",
    "line-height",
    "list-style-image",
    "list-style-position",
    "list-style-type",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "margin-top",
    "math-style",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "object-fit",
    "object-position",
    "opacity",
    "order",
    "orphans",
    "outline-color",
    "outline-offset",
    "outline-style",
    "outline-width",
    "overflow-wrap",
    "overflow-x",
    "overflow-y",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "padding-top",
    "paint-order",
    "perspective",
    "perspective-origin",
    "position",
    "print-color-adjust",
    "quotes",
    "r",
    "right",
    "rotate",
    "row-gap",
    "ruby-align",
    "ruby-position",
    "rx",
    "ry",
    "scale",
    "scroll-behavior",
    "scroll-margin-bottom",
    "scroll-margin-left",
    "scroll-margin-right",
    "scroll-margin-top",
    "scroll-padding-bottom",
    "scroll-padding-left",
    "scroll-padding-right",
    "scroll-padding-top",
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
    "tab-size",
    "table-layout",
    "text-align",
    "text-align-all",
    "text-align-last",
    "text-anchor",
    "text-autospace",
    "text-decoration-color",
    "text-decoration-line",
    "text-decoration-style",
    "text-decoration-thickness",
    "text-fit",
    "text-indent",
    "text-justify",
    "text-shadow",
    "text-spacing-trim",
    "text-transform",
    "text-underline-offset",
    "text-underline-position",
    "text-wrap-mode",
    "text-wrap-style",
    "top",
    "transform",
    "transform-box",
    "transform-origin",
    "transform-style",
    "transition-behavior",
    "transition-delay",
    "transition-duration",
    "transition-property",
    "transition-timing-function",
    "translate",
    "vector-effect",
    "vertical-align",
    "visibility",
    "white-space-collapse",
    "widows",
    "width",
    "word-break",
    "word-space-transform",
    "word-spacing",
    "x",
    "y",
    "z-index",
    "-webkit-border-horizontal-spacing",
    "-webkit-border-vertical-spacing",
];

/// What a caller knows of an element that its computed style does not
/// (D65, D66): a longhand's resolved value where it is not the computed
/// one (a used value of its box), and a `url()`'s URL made absolute.
pub struct Resolver<'a> {
    /// A longhand's resolved value, `None` where it is the computed one.
    pub longhand: &'a dyn Fn(&str) -> Option<String>,
    /// A URL as written made absolute, `None` where it is kept.
    pub url: &'a dyn Fn(&str) -> Option<String>,
}

impl Resolver<'_> {
    /// Nothing past the computed style.
    pub const COMPUTED: Resolver<'static> = Resolver {
        longhand: &|_| None,
        url: &|_| None,
    };
}

/// `name`'s resolved value in `style` as `getComputedStyle` gives it
/// (CSSOM section 9, D66), `name` lowercase or a custom property's: a
/// longhand's, `resolver`'s where it has one, else its computed value; a
/// shorthand's from its longhands'. `None` for a property this engine
/// does not give.
pub fn resolved_value(
    style: &ComputedStyle,
    name: &str,
    resolver: &Resolver<'_>,
) -> Option<String> {
    if let Some(value) = shorthands::value(style, name, resolver) {
        return value;
    }
    // A flow-relative longhand is its physical one's (css-logical-1).
    if let Some(physical) = crate::cascade::physical_longhand(name, style.direction) {
        return resolved_value(style, physical, resolver);
    }
    if let Some(value) = (resolver.longhand)(name) {
        return Some(value);
    }
    longhand(style, name, resolver.url)
}

/// `name`'s computed value in `style` as text, `name` lowercase or a
/// custom property's: [`resolved_value`] with nothing past the style.
pub fn computed_value(style: &ComputedStyle, name: &str) -> Option<String> {
    resolved_value(style, name, &Resolver::COMPUTED)
}

/// A longhand's computed value, or `white-space`'s.
fn longhand(style: &ComputedStyle, name: &str, url: longhands::Url<'_>) -> Option<String> {
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
    let corner = match name {
        "border-top-left-radius" => Some(0),
        "border-top-right-radius" => Some(1),
        "border-bottom-right-radius" => Some(2),
        "border-bottom-left-radius" => Some(3),
        _ => None,
    };
    if let Some(i) = corner {
        // One value where the two are alike, as the reference browser
        // writes it (gap batch E).
        let [h, v] = &style.radii().0[i];
        let (h, v) = (length_percentage(h)?, length_percentage(v)?);
        return Some(if h == v { h } else { format!("{h} {v}") });
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
        "color-scheme" => style
            .color_scheme
            .as_ref()
            .map_or_else(|| "normal".to_owned(), |s| s.to_string()),
        "background-color" => color(style.background_color),
        // Gap batch E: each layer's position along one axis (G40).
        "background-position-x" | "background-position-y" => {
            let background = style.background();
            let list = if name.ends_with('y') {
                &background.position_y
            } else {
                &background.position_x
            };
            let parts: Option<Vec<String>> = list.iter().map(length_percentage).collect();
            parts?.join(", ")
        }
        // `auto`, a ratio, or both (G36).
        "aspect-ratio" => {
            let ratio =
                |r: &crate::computed::Ratio| format!("{} / {}", number(r.width), number(r.height));
            match &style.image().aspect_ratio {
                crate::computed::AspectRatio::Auto => "auto".to_owned(),
                crate::computed::AspectRatio::Ratio(r) => ratio(r),
                crate::computed::AspectRatio::AutoOr(r) => format!("auto {}", ratio(r)),
            }
        }
        "font-size" => px(style.font_size_px),
        "font-family" => style.font().family.to_string(),
        "font-weight" => number(style.font().weight),
        "font-style" => match style.font().style {
            FontStyle::Normal => "normal".to_owned(),
            FontStyle::Italic => "italic".to_owned(),
            FontStyle::DefaultOblique => "oblique".to_owned(),
            FontStyle::Oblique(a) => format!("oblique {}deg", number(a)),
        },
        "font-stretch" => format!("{}%", number(style.font().stretch)),
        "font-variant-ligatures" => style.font().variant.ligatures.clone(),
        "font-variant-caps" => style.font().variant.caps.clone(),
        "font-variant-alternates" => style.font().variant.alternates.clone(),
        "font-variant-numeric" => style.font().variant.numeric.clone(),
        "font-variant-east-asian" => style.font().variant.east_asian.clone(),
        "font-variant-position" => style.font().variant.position.clone(),
        "font-variant-emoji" => style.font().variant.emoji.clone(),
        "font-kerning" => style.font().kerning.name().to_owned(),
        "font-palette" => crate::cascade::font_palette_text(&style.text().font_palette),
        "font-feature-settings" => {
            let features = &style.font().features;
            if features.is_empty() {
                return Some("normal".to_owned());
            }
            let feature = |(tag, value): &([u8; 4], u32)| {
                let tag = String::from_utf8_lossy(tag);
                match value {
                    1 => format!("\"{tag}\""),
                    v => format!("\"{tag}\" {v}"),
                }
            };
            features.iter().map(feature).collect::<Vec<_>>().join(", ")
        }
        // The used value (CSSOM 1 section 9): a number is of the size.
        "line-height" => match style.font().line_height {
            LineHeight::Normal => "normal".to_owned(),
            LineHeight::Number(n) => px(n * style.font_size_px),
            LineHeight::Px(v) => px(v),
        },
        "display" => display_value(style.display, style.flow_root),
        "text-align" => match style.legacy_align {
            crate::computed::LegacyAlign::Left => "-webkit-left".to_owned(),
            crate::computed::LegacyAlign::Right => "-webkit-right".to_owned(),
            crate::computed::LegacyAlign::Center => "-webkit-center".to_owned(),
            crate::computed::LegacyAlign::None => text_align(style.text_align).to_owned(),
        },
        "text-transform" => {
            let extra = style.text().transform_extra;
            let mut parts = match style.text_transform {
                TextTransform::None => Vec::new(),
                TextTransform::Uppercase => vec!["uppercase"],
                TextTransform::Lowercase => vec!["lowercase"],
                TextTransform::Capitalize => vec!["capitalize"],
            };
            if extra.full_width {
                parts.push("full-width");
            }
            if extra.full_size_kana {
                parts.push("full-size-kana");
            }
            if extra.math_auto {
                parts.push("math-auto");
            }
            if parts.is_empty() {
                "none".to_owned()
            } else {
                parts.join(" ")
            }
        }
        "white-space" => match style.white_space {
            WhiteSpace::NORMAL => "normal".to_owned(),
            WhiteSpace::PRE => "pre".to_owned(),
            WhiteSpace::PRE_WRAP => "pre-wrap".to_owned(),
            WhiteSpace::PRE_LINE => "pre-line".to_owned(),
            WhiteSpace::NOWRAP => "nowrap".to_owned(),
            WhiteSpace::BREAK_SPACES => "break-spaces".to_owned(),
            other => format!(
                "{} {}",
                white_space_collapse(other.collapse),
                if other.wrap { "wrap" } else { "nowrap" }
            ),
        },
        "white-space-collapse" => white_space_collapse(style.white_space.collapse).to_owned(),
        "text-wrap-mode" => if style.white_space.wrap {
            "wrap"
        } else {
            "nowrap"
        }
        .to_owned(),
        "overflow-wrap" => match style.overflow_wrap {
            crate::computed::OverflowWrap::Normal => "normal",
            crate::computed::OverflowWrap::Anywhere => "anywhere",
            crate::computed::OverflowWrap::BreakWord => "break-word",
        }
        .to_owned(),
        "word-break" => match style.word_break {
            crate::computed::WordBreak::Normal => "normal",
            crate::computed::WordBreak::BreakAll => "break-all",
            crate::computed::WordBreak::KeepAll => "keep-all",
            crate::computed::WordBreak::BreakWord => "break-word",
            crate::computed::WordBreak::AutoPhrase => "auto-phrase",
            crate::computed::WordBreak::Manual => "manual",
        }
        .to_owned(),
        "visibility" => match style.visibility {
            crate::computed::Visibility::Visible => "visible",
            crate::computed::Visibility::Hidden => "hidden",
            crate::computed::Visibility::Collapse => "collapse",
        }
        .to_owned(),
        "text-shadow" => {
            let shadows = &style.decoration().text_shadow;
            if shadows.is_empty() {
                "none".to_owned()
            } else {
                shadows
                    .iter()
                    .map(|s| format!("{} {} {} {}", color(s.color), px(s.x), px(s.y), px(s.blur)))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        }
        .to_owned(),
        "letter-spacing" => match &style.text().letter_spacing {
            Some(spacing) => length_percentage(spacing)?,
            None => spacing(style.letter_spacing),
        },
        "word-spacing" => match &style.text().word_spacing {
            Some(spacing) => length_percentage(spacing)?,
            None => px(style.word_spacing),
        },
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
        "scroll-margin-top"
        | "scroll-margin-right"
        | "scroll-margin-bottom"
        | "scroll-margin-left" => {
            let side = crate::cascade::side_of(name);
            px(style.effects().scroll_margin[side])
        }
        "scroll-padding-top"
        | "scroll-padding-right"
        | "scroll-padding-bottom"
        | "scroll-padding-left" => {
            let side = crate::cascade::side_of(name);
            auto_length(&style.effects().scroll_padding[side])
        }
        "overflow-x" | "overflow-y" => match if name == "overflow-x" {
            style.overflow_x
        } else {
            style.overflow_y
        } {
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
        // Its images as written, a URL absolute where `url` makes it so,
        // then its keyword.
        "cursor" => {
            let mut out = String::new();
            for (written, hotspot) in style.image().cursor_images.iter() {
                out.push_str(&longhands::url_value(written, url));
                if let Some([x, y]) = hotspot {
                    out.push_str(&format!(" {} {}", number(*x), number(*y)));
                }
                out.push_str(", ");
            }
            out.push_str(style.cursor.keyword());
            out
        }
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
            VerticalAlign::Calc(c) => longhands::calc(c),
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
        // The individual transforms' (gap batch E: what the math
        // functions' tests read).
        "scale" => match style.effects().scale {
            None => "none".to_owned(),
            Some([x, y, z]) if z == 1.0 && x == y => number(x),
            Some([x, y, 1.0]) => format!("{} {}", number(x), number(y)),
            Some([x, y, z]) => format!("{} {} {}", number(x), number(y), number(z)),
        },
        "rotate" => match style.effects().rotate {
            None => "none".to_owned(),
            Some(r) => {
                let angle = format!("{}deg", number(r.angle));
                match (r.x, r.y, r.z) {
                    (x, y, z) if x == 0.0 && y == 0.0 && z > 0.0 => angle,
                    (x, y, z) if y == 0.0 && z == 0.0 && x > 0.0 => format!("x {angle}"),
                    (x, y, z) if x == 0.0 && z == 0.0 && y > 0.0 => format!("y {angle}"),
                    (x, y, z) => format!("{} {} {} {angle}", number(x), number(y), number(z)),
                }
            }
        },
        "filter" => {
            use crate::computed::Filter as F;
            let filters = &style.effects().filter;
            if filters.is_empty() {
                "none".to_owned()
            } else {
                filters
                    .iter()
                    .map(|f| match f {
                        F::Blur(v) => format!("blur({})", px(*v)),
                        F::Brightness(v) => format!("brightness({})", number(*v)),
                        F::Contrast(v) => format!("contrast({})", number(*v)),
                        F::Grayscale(v) => format!("grayscale({})", number(*v)),
                        F::HueRotate(v) => format!("hue-rotate({}deg)", number(*v)),
                        F::Invert(v) => format!("invert({})", number(*v)),
                        F::Opacity(v) => format!("opacity({})", number(*v)),
                        F::Saturate(v) => format!("saturate({})", number(*v)),
                        F::Sepia(v) => format!("sepia({})", number(*v)),
                        F::DropShadow {
                            color: c,
                            x,
                            y,
                            blur,
                        } => format!(
                            "drop-shadow({} {} {} {})",
                            color(*c),
                            px(*x),
                            px(*y),
                            px(*blur)
                        ),
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        }
        "box-shadow" => {
            let shadows = &style.effects().box_shadow;
            if shadows.is_empty() {
                "none".to_owned()
            } else {
                shadows
                    .iter()
                    .map(|s| {
                        let mut text = format!(
                            "{} {} {} {} {}",
                            color(s.color),
                            px(s.x),
                            px(s.y),
                            px(s.blur),
                            px(s.spread)
                        );
                        if s.inset {
                            text.push_str(" inset");
                        }
                        text
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        }
        "transform-box" => match style.effects().transform_box {
            TransformBox::ContentBox => "content-box",
            TransformBox::BorderBox => "border-box",
            TransformBox::FillBox => "fill-box",
            TransformBox::StrokeBox => "stroke-box",
            TransformBox::ViewBox => "view-box",
        }
        .to_owned(),
        _ => return svg(style, name).or_else(|| longhands::value(style, name, url)),
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

/// A `white-space-collapse` keyword.
fn white_space_collapse(collapse: crate::computed::WhiteSpaceCollapse) -> &'static str {
    use crate::computed::WhiteSpaceCollapse as W;
    match collapse {
        W::Collapse => "collapse",
        W::Preserve => "preserve",
        W::PreserveBreaks => "preserve-breaks",
        W::BreakSpaces => "break-spaces",
        W::PreserveSpaces => "preserve-spaces",
    }
}

pub fn px(value: f32) -> String {
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
pub(crate) fn color(c: Color) -> String {
    if !c.is_legacy() {
        return crate::color::serialize(&c);
    }
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b) = (channel(c.r), channel(c.g), channel(c.b));
    if c.a >= 1.0 {
        format!("rgb({r}, {g}, {b})")
    } else {
        // Two places where they read back as the same byte, else three
        // (CSSOM section 6.7.2).
        let a = c.a.clamp(0.0, 1.0);
        let byte = (a * 255.0).round();
        let two = (a * 100.0).round() / 100.0;
        let alpha = if (two * 255.0).round() == byte {
            two
        } else {
            (a * 1000.0).round() / 1000.0
        };
        format!("rgba({r}, {g}, {b}, {})", number(alpha))
    }
}

pub(crate) fn length_percentage(value: &LengthPercentage) -> Option<String> {
    match value {
        LengthPercentage::Px(v) => Some(px(*v)),
        LengthPercentage::Percent(p) => Some(percent(*p)),
        LengthPercentage::Calc(c) => Some(longhands::calc(c)),
    }
}

fn auto_length(value: &LengthPercentageAuto) -> String {
    match value {
        LengthPercentageAuto::Auto => "auto".to_owned(),
        LengthPercentageAuto::Px(v) => px(*v),
        LengthPercentageAuto::Percent(p) => percent(*p),
        LengthPercentageAuto::Calc(c) => longhands::calc(c),
    }
}

fn size(value: &Size) -> Option<String> {
    Some(match value {
        Size::Auto => "auto".to_owned(),
        Size::Px(v) => px(*v),
        Size::Percent(p) => percent(*p),
        Size::Calc(c) => longhands::calc(c),
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
        MaxSize::Calc(c) => longhands::calc(c),
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
        Display::InlineListItem => "inline list-item",
        Display::Contents => "contents",
        Display::Ruby => "ruby",
        Display::BlockRuby => "block ruby",
        Display::RubyBase => "ruby-base",
        Display::RubyText => "ruby-text",
        Display::RubyBaseContainer => "ruby-base-container",
        Display::RubyTextContainer => "ruby-text-container",
        Display::Math => "math",
        Display::BlockMath => "block math",
    }
}

/// `display`'s value and whether its box is a flow root, written as the
/// reference browser writes the computed value, and a specified one in
/// its shortest form (css-display-3 section 2): `flow-root` for a block
/// flow root, `inline flow-root list-item` for an inline one.
pub fn display_value(value: Display, flow_root: bool) -> String {
    match (value, flow_root) {
        (Display::Block, true) => "flow-root".to_owned(),
        (Display::ListItem, true) => "flow-root list-item".to_owned(),
        (Display::InlineListItem, true) => "inline flow-root list-item".to_owned(),
        (other, _) => display(other).to_owned(),
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

/// A CSS string of `text`, quoted, its quotes and backslashes escaped.
fn string(text: &str) -> String {
    tokens(&[Token::String {
        value: text.to_owned(),
        bad: false,
    }])
}

/// An easing function as the reference browser writes it, as an
/// effect's `easing` gives it (Web Animations 1 section 6.7.1).
pub fn easing(easing: &crate::easing::Easing) -> String {
    longhands::easing(easing)
}

/// A custom property's value as text: its tokens written back, white
/// space as one space, trimmed; and a selector's or a condition's, for
/// the DevTools' rules (M36).
pub fn tokens(value: &[Token]) -> String {
    write_tokens(value, false, false)
}

/// A property's specified value as CSSOM serializes it (section 6.7.2,
/// G50): [`tokens`], with each number at its shortest, each URL quoted
/// and each colour written as a hash or a colour function in its
/// canonical form (Color 4 section 15), as the reference browser writes
/// them.
pub fn specified(value: &[Token]) -> String {
    write_tokens(value, true, false)
}

/// The colour functions a specified value's colour is written from.
fn color_function(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "rgb"
            | "rgba"
            | "hsl"
            | "hsla"
            | "hwb"
            | "lab"
            | "lch"
            | "oklab"
            | "oklch"
            | "color"
            | "alpha"
    )
}

/// The colour a hash or a colour function at `value[at]` writes, and
/// where it ends; `None` for anything else, or one with a substitution
/// or a keyword that waits for the element in it. A relative colour is
/// written in its own form (D78).
fn color_at(value: &[Token], at: usize) -> Option<(usize, String)> {
    let end = match &value[at] {
        Token::Hash { .. } => at + 1,
        Token::Function(name) if color_function(name) => {
            let mut depth = 0usize;
            let mut end = None;
            for (i, token) in value.iter().enumerate().skip(at) {
                match token {
                    Token::Function(_) | Token::OpenParen => depth += 1,
                    Token::CloseParen => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            end?
        }
        _ => return None,
    };
    let run = &value[at..end];
    // A legacy function (`rgb()`, `hsl()`, `hwb()`) is written as its
    // `rgb()`, a calculation in it worked out, but `hsl()` and `hwb()`
    // with a missing component are kept; in the others a calculation is
    // written in its canonical form, as the reference browser keeps them.
    let name = match &value[at] {
        Token::Function(f) => f.to_ascii_lowercase(),
        _ => String::new(),
    };
    // A relative colour keeps its form, its origin written as a specified
    // colour is (D78).
    let arguments = match run {
        [_, arguments @ .., Token::CloseParen] => arguments,
        [_, arguments @ ..] => arguments,
        [] => return None,
    };
    if crate::color::is_relative(arguments) {
        let text = crate::color::relative_text(&name, arguments, &mut specified_color)?;
        return Some((end, text));
    }
    let legacy = matches!(name.as_str(), "rgb" | "rgba" | "hsl" | "hsla" | "hwb");
    let hue = matches!(name.as_str(), "hsl" | "hsla" | "hwb");
    let math = |t: &Token| crate::calc::is_math_function(std::slice::from_ref(t));
    let waits = run.iter().skip(1).any(|t| match t {
        Token::Function(_) => !math(t),
        Token::Ident(word) => {
            word.eq_ignore_ascii_case("from") || word.eq_ignore_ascii_case("currentcolor")
        }
        _ => false,
    });
    if waits {
        return None;
    }
    // `hsl()` and `hwb()` with a `none` keep their form, and the others
    // their calculations, in the modern syntax (D78).
    let none = run
        .iter()
        .any(|t| matches!(t, Token::Ident(word) if word.eq_ignore_ascii_case("none")));
    if (hue && none) || (!legacy && run.iter().skip(1).any(math)) {
        let text = crate::color::modern_text(&name, arguments, false)?;
        return Some((end, text));
    }
    let Some(parsed) = crate::cascade::parse_color(run) else {
        // A calculation that waits for the element, a relative length in
        // it, writes a legacy function in the modern syntax.
        let text = crate::color::modern_text(&name, arguments, true).filter(|_| legacy)?;
        return Some((end, text));
    };
    Some((end, color(parsed)))
}

/// A colour's tokens written as a specified colour is (D78): a keyword in
/// lowercase, a hash or a colour function as [`color_at`] writes it,
/// anything else as it is.
fn specified_color(tokens: &[Token]) -> String {
    let start = tokens
        .iter()
        .position(|t| !matches!(t, Token::Whitespace))
        .unwrap_or(tokens.len());
    let tokens = &tokens[start..];
    if let [Token::Ident(word), ..] = tokens {
        return word.to_ascii_lowercase();
    }
    match (!tokens.is_empty()).then(|| color_at(tokens, 0)).flatten() {
        Some((_, text)) => text,
        None => write_tokens(tokens, true, false),
    }
}

/// Where the function at `value[at]` ends: past its `)`, or the end of
/// the value, which closes what it leaves open.
fn function_end(value: &[Token], at: usize) -> usize {
    let mut depth = 0usize;
    for (i, token) in value.iter().enumerate().skip(at) {
        match token {
            Token::Function(_) | Token::OpenParen => depth += 1,
            Token::CloseParen => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    value.len()
}

/// The math function at `value[at]` written as a specified value is
/// (css-values-4 section 10.13, D78), and where it ends; `None` for
/// anything else, or one that does not parse, which is written as it is.
fn math_at(value: &[Token], at: usize, percentages: bool) -> Option<(usize, String)> {
    if !crate::calc::is_math_function(&value[at..=at]) {
        return None;
    }
    let end = function_end(value, at);
    Some((
        end,
        crate::calc::specified(&value[at..end], &[], percentages)?,
    ))
}

/// [`specified`] for the property `name` (D78): a math function's
/// percentages compared where they stand for numbers, as the opacities'
/// do, `min(50%, 0%)` written `calc(0%)` there.
pub fn specified_of(name: &str, value: &[Token]) -> String {
    let numbers = matches!(
        name,
        "opacity" | "fill-opacity" | "stroke-opacity" | "flood-opacity" | "stop-opacity"
    );
    write_tokens(value, true, numbers)
}

/// [`tokens`], with `url()`s quoted when `quote_urls`, a math function's
/// percentages compared where `percentages` ([`specified_of`]).
fn write_tokens(value: &[Token], quote_urls: bool, percentages: bool) -> String {
    let mut out = String::new();
    let mut at = 0;
    while at < value.len() {
        if quote_urls && let Some((end, color)) = color_at(value, at) {
            out.push_str(&color);
            at = end;
            continue;
        }
        if quote_urls && let Some((end, math)) = math_at(value, at, percentages) {
            out.push_str(&math);
            at = end;
            continue;
        }
        let token = &value[at];
        at += 1;
        match token {
            // Canonical: no space just inside brackets, and a comma
            // followed by one space.
            Token::Whitespace if quote_urls && out.ends_with('(') => {}
            Token::Comma if quote_urls => {
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push_str(", ");
            }
            Token::CloseParen if quote_urls => {
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push(')');
            }
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
            Token::Url { value, .. } if quote_urls => {
                out.push_str("url(");
                out.push_str(&string(value));
                out.push(')');
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
    fn a_specified_value_is_written_in_its_canonical_form() {
        let written = |text: &str| specified(&css::tokenize(text));
        assert_eq!(written(".5% -.1em -0px"), "0.5% -0.1em 0px");
        assert_eq!(written("url(http://a/)"), "url(\"http://a/\")");
        assert_eq!(written("'a\"b'"), "\"a\\\"b\"");
        assert_eq!(written("rgb( 1,2 , 3 )"), "rgb(1, 2, 3)");
        assert_eq!(written("#FF0000"), "rgb(255, 0, 0)");
        assert_eq!(
            written("hsla(120, 100%, 50%, 0.25)"),
            "rgba(0, 255, 0, 0.25)"
        );
        assert_eq!(
            written("1px 2px #00F, inset 0 0 3px rgb(0 0 0 / 50%)"),
            "1px 2px rgb(0, 0, 255), inset 0 0 3px rgba(0, 0, 0, 0.5)"
        );
        assert_eq!(
            written("linear-gradient(#fff,red 10%)"),
            "linear-gradient(rgb(255, 255, 255), red 10%)"
        );
        assert_eq!(written("rgb(from red r g b)"), "rgb(from red r g b)");
        assert_eq!(written("lab(none 20 calc(0.5))"), "lab(none 20 calc(0.5))");
        assert_eq!(written("hsl(none 100% 50%)"), "hsl(none 100% 50%)");
        assert_eq!(written("rgb(calc(infinity), 0, 0)"), "rgb(255, 0, 0)");
        assert_eq!(tokens(&css::tokenize("url(x)")), "url(x)");
    }

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
        // An alpha of a byte: two places where they read back as it.
        assert_eq!(
            color(Color::rgba(0.0, 0.5, 0.0, 128.0 / 255.0)),
            "rgba(0, 128, 0, 0.5)"
        );
        assert_eq!(
            color(Color::rgba(0.0, 0.0, 0.0, 0.125)),
            "rgba(0, 0, 0, 0.125)"
        );
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
        // Alphabetical, the `-webkit-` ones after (D66).
        let (webkit, standard): (Vec<&str>, Vec<&str>) = COMPUTED_PROPERTIES
            .iter()
            .partition(|name| name.starts_with("-webkit-"));
        let mut sorted = standard.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, standard);
        let mut sorted = webkit.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, webkit);
        assert_eq!(
            COMPUTED_PROPERTIES.last(),
            Some(&"-webkit-border-vertical-spacing")
        );
    }

    /// The computed style of the element `#t` in `html`, styled by
    /// `sheet`.
    fn styled(html: &str, sheet: &str) -> ComputedStyle {
        let document = dom::Document::from_node(&html::parse_document(html).expect("never fails"));
        let sheets = [css::parse_stylesheet(sheet).expect("a valid sheet")];
        let styles = crate::cascade_document(&document, &sheets, &crate::CascadeContext::default());
        let list = crate::parse_selector_list(&css::tokenize("#t")).expect("valid selector");
        document
            .subtree(document.root())
            .find(|&id| crate::matches_selector(&document, id, &list.0[0]))
            .and_then(|id| styles.get(id).cloned())
            .expect("an element #t")
    }

    /// `name`'s computed value on a `div#t` with `declarations`.
    fn of(declarations: &str, name: &str) -> String {
        let style = styled("<div id=t></div>", &format!("#t {{ {declarations} }}"));
        computed_value(&style, name).unwrap_or_else(|| panic!("{name} of {declarations}"))
    }

    #[test]
    fn display_ruby_and_math_are_written_as_the_reference_browser_writes_them() {
        assert_eq!(
            of("display: list-item inline flow-root", "display"),
            "inline flow-root list-item"
        );
        assert_eq!(of("display: ruby block", "display"), "block ruby");
        assert_eq!(of("display: contents", "display"), "contents");
        assert_eq!(of("display: block math", "display"), "block", "on a div");
        assert_eq!(display_value(Display::BlockMath, false), "block math");
        assert_eq!(of("ruby-position: under", "ruby-position"), "under");
        assert_eq!(of("ruby-align: center", "ruby-align"), "center");
        assert_eq!(of("math-style: compact", "math-style"), "compact");
        assert_eq!(of("", "ruby-align"), "space-around");
    }

    #[test]
    fn a_calculations_infinity_is_the_largest_value_its_place_holds() {
        // D78: what a script reads is a number, not an infinity.
        let largest = |text: String, unit: &str| {
            let v: f64 = text.trim_end_matches(unit).parse().expect("a number");
            assert!(v.is_finite() && v > 1e6, "{text}");
        };
        largest(
            of("transition-delay: calc(infinity * 1s)", "transition-delay"),
            "s",
        );
        largest(of("translate: calc(infinity * 100%)", "translate"), "%");
        // A resolution's is taken, clamped to its range.
        let image_set = "image-set(url(a.png) calc(1dppx * -infinity))";
        assert!(crate::supports(
            "background-image",
            &css::tokenize(image_set)
        ));
        assert_eq!(
            of("transform: rotate(calc(infinity * 1deg))", "transform"),
            of("transform: rotate(0deg)", "transform")
        );
        assert_eq!(
            of(
                "background-image: conic-gradient(from 2rad, red, blue)",
                "background-image"
            ),
            "conic-gradient(from 114.592deg, rgb(255, 0, 0), rgb(0, 0, 255))"
        );
    }

    #[test]
    fn shorthands_are_written_from_their_longhands() {
        // D66, G159: the reference browser's values for an element with
        // every initial value.
        for (name, expected) in [
            ("margin", "0px"),
            ("padding", "0px"),
            ("inset", "auto"),
            ("border", "0px none rgb(0, 0, 0)"),
            ("border-top", "0px none rgb(0, 0, 0)"),
            ("border-width", "0px"),
            ("border-style", "none"),
            ("border-color", "rgb(0, 0, 0)"),
            ("border-radius", "0px"),
            ("border-image", "none"),
            ("outline", "rgb(0, 0, 0) none 0px"),
            ("gap", "normal"),
            ("flex", "0 1 auto"),
            ("flex-flow", "row nowrap"),
            ("place-content", "normal"),
            ("place-items", "normal"),
            ("place-self", "auto"),
            ("overflow", "visible"),
            ("list-style", "outside none disc"),
            ("text-decoration", "none solid rgb(0, 0, 0)"),
            (
                "background",
                "rgba(0, 0, 0, 0) none repeat scroll 0% 0% / auto padding-box border-box",
            ),
            ("transition", "all 0s ease 0s"),
            ("font-variant", "normal"),
            ("font", "16px sans-serif"),
            ("grid-template", "none"),
        ] {
            assert_eq!(of("", name), expected, "{name}");
        }
        // At their shortest.
        assert_eq!(of("margin: 1px 2px", "margin"), "1px 2px");
        assert_eq!(of("margin: 1px 2px 3px", "margin"), "1px 2px 3px");
        assert_eq!(of("padding: 1px 2px 1px 4px", "padding"), "1px 2px 1px 4px");
        assert_eq!(of("border-radius: 10px 20px", "border-radius"), "10px 20px");
        assert_eq!(
            of("border-radius: 10px / 5px", "border-radius"),
            "10px / 5px"
        );
        assert_eq!(
            of("border: 1px solid red", "border"),
            "1px solid rgb(255, 0, 0)"
        );
        assert_eq!(of("gap: 10px 20px", "gap"), "10px 20px");
        assert_eq!(of("flex: 1", "flex"), "1 1 0%");
        assert_eq!(
            of("font: italic bold 12px/30px Georgia", "font"),
            "italic 700 12px / 30px Georgia"
        );
        assert_eq!(of("font-variant: small-caps", "font-variant"), "small-caps");
        assert_eq!(
            of("text-decoration: underline", "text-decoration"),
            "underline solid rgb(0, 0, 0)"
        );
        assert_eq!(
            of("transition: opacity 1s ease-in 0.5s", "transition"),
            "opacity 1s ease-in 0.5s"
        );
        assert_eq!(of("grid-row: 1 / 3", "grid-row"), "1 / 3");
        // Not to be written as the shorthand: the empty string.
        assert_eq!(
            of("border: 1px solid red; border-left-width: 2px", "border"),
            ""
        );
        assert_eq!(of("font-stretch: 80%", "font"), "");
        // Not a property the engine has.
        assert_eq!(computed_value(&ComputedStyle::default(), "columns"), None);
    }

    #[test]
    fn the_longhands_past_the_first_are_given() {
        for (declarations, name, expected) in [
            ("", "transform", "none"),
            (
                "transform: translate(10px, 20px) scale(2)",
                "transform",
                "matrix(2, 0, 0, 2, 10, 20)",
            ),
            ("", "transform-origin", "50% 50%"),
            ("", "translate", "none"),
            ("", "text-decoration-line", "none"),
            ("", "list-style-type", "disc"),
            ("", "background-repeat", "repeat"),
            (
                "background-repeat: repeat-x",
                "background-repeat",
                "repeat-x",
            ),
            ("", "background-size", "auto"),
            // Both sides where one is not `auto`, as the reference
            // browser writes them.
            ("background-size: 10px auto", "background-size", "10px auto"),
            ("background-size: 10px", "background-size", "10px auto"),
            ("background-size: auto 10%", "background-size", "auto 10%"),
            ("background-size: auto auto", "background-size", "auto"),
            ("", "animation-name", "none"),
            ("", "transition-timing-function", "ease"),
            (
                "transition-timing-function: step-start, steps(3)",
                "transition-timing-function",
                "steps(1, start), steps(3)",
            ),
            ("", "grid-template-columns", "none"),
            (
                "grid-template-columns: [a] 1fr 10px",
                "grid-template-columns",
                "[a] 1fr 10px",
            ),
            ("", "-webkit-border-horizontal-spacing", "0px"),
            ("", "content", "normal"),
            ("", "counter-reset", "none"),
            ("", "orphans", "2"),
            ("", "object-position", "50% 50%"),
            ("", "border-image-slice", "100%"),
            ("", "border-image-width", "1"),
        ] {
            assert_eq!(of(declarations, name), expected, "{name}: {declarations}");
        }
    }

    #[test]
    fn images_and_gradients_are_written_as_the_reference_browser_writes_them() {
        // G232.
        let image = |value: &str| of(&format!("background-image: {value}"), "background-image");
        assert_eq!(
            image("linear-gradient(red, blue)"),
            "linear-gradient(rgb(255, 0, 0), rgb(0, 0, 255))"
        );
        assert_eq!(
            image("linear-gradient(to bottom, red, blue)"),
            "linear-gradient(rgb(255, 0, 0), rgb(0, 0, 255))"
        );
        assert_eq!(
            image("linear-gradient(to right, red 10%, blue)"),
            "linear-gradient(to right, rgb(255, 0, 0) 10%, rgb(0, 0, 255))"
        );
        assert_eq!(
            image("linear-gradient(45deg, red, blue)"),
            "linear-gradient(45deg, rgb(255, 0, 0), rgb(0, 0, 255))"
        );
        assert_eq!(
            image("repeating-linear-gradient(red 0px, blue 10px)"),
            "repeating-linear-gradient(rgb(255, 0, 0) 0px, rgb(0, 0, 255) 10px)"
        );
        assert_eq!(
            image("radial-gradient(circle, red, blue)"),
            "radial-gradient(circle, rgb(255, 0, 0), rgb(0, 0, 255))"
        );
        assert_eq!(
            image("radial-gradient(red, blue)"),
            "radial-gradient(rgb(255, 0, 0), rgb(0, 0, 255))"
        );
        // A corner's horizontal side first, whichever order it is in.
        for corner in ["to bottom left", "to left bottom"] {
            assert_eq!(
                image(&format!("linear-gradient({corner}, red, blue)")),
                "linear-gradient(to left bottom, rgb(255, 0, 0), rgb(0, 0, 255))"
            );
        }
        assert_eq!(
            image("linear-gradient(to top right in lab, red, blue)"),
            "linear-gradient(to right top in lab, rgb(255, 0, 0), rgb(0, 0, 255))"
        );
        assert_eq!(
            image("conic-gradient(from 90deg, red, blue)"),
            "conic-gradient(from 90deg, rgb(255, 0, 0), rgb(0, 0, 255))"
        );
        assert_eq!(
            image("url(a.png), linear-gradient(red, blue)"),
            "url(\"a.png\"), linear-gradient(rgb(255, 0, 0), rgb(0, 0, 255))"
        );
        assert_eq!(
            of(
                "list-style-image: linear-gradient(red, blue)",
                "list-style-image"
            ),
            "linear-gradient(rgb(255, 0, 0), rgb(0, 0, 255))"
        );
        assert_eq!(
            of(
                "border-image-source: linear-gradient(red, blue)",
                "border-image-source"
            ),
            "linear-gradient(rgb(255, 0, 0), rgb(0, 0, 255))"
        );
        // A URL made absolute by the caller.
        let style = styled("<div id=t></div>", "#t { background-image: url(a.png) }");
        let resolver = Resolver {
            longhand: &|_| None,
            url: &|url| Some(format!("https://example.test/{url}")),
        };
        assert_eq!(
            resolved_value(&style, "background-image", &resolver).as_deref(),
            Some("url(\"https://example.test/a.png\")")
        );
        // An empty or fragment-only one is kept as written, a cursor's
        // images too, before its keyword.
        let style = styled(
            "<div id=t></div>",
            "#t { background-image: url(#foo); cursor: url(a.png) 2 3, url(), url('#b'), pointer }",
        );
        assert_eq!(
            resolved_value(&style, "background-image", &resolver).as_deref(),
            Some("url(\"#foo\")")
        );
        assert_eq!(
            resolved_value(&style, "cursor", &resolver).as_deref(),
            Some("url(\"https://example.test/a.png\") 2 3, url(\"\"), url(\"#b\"), pointer")
        );
        let style = styled(
            "<div id=t><p id=c></p></div>",
            "#t { cursor: url(a.png), auto } #c { cursor: url(a.png) 1, auto }",
        );
        assert_eq!(
            computed_value(&style, "cursor").as_deref(),
            Some("url(\"a.png\"), auto")
        );
    }

    #[test]
    fn calculations_are_written_as_values_4_says() {
        // Section 10.13: the percentage before the length, a negative
        // one subtracted, a function's own name at the top.
        assert_eq!(
            of(
                "background-position-x: calc(10px + 50%)",
                "background-position-x"
            ),
            "calc(50% + 10px)"
        );
        assert_eq!(of("width: calc(50% - 10px)", "width"), "calc(50% - 10px)");
        assert_eq!(of("left: min(50%, 10px)", "left"), "min(50%, 10px)");
        assert_eq!(
            of("padding-left: calc(1em + 10%)", "padding-left"),
            "calc(10% + 16px)"
        );
    }

    #[test]
    fn a_used_value_from_the_caller_makes_the_shorthand() {
        let style = ComputedStyle::default();
        let resolver = Resolver {
            longhand: &|name| (name == "margin-left").then(|| "8px".to_owned()),
            url: &|_| None,
        };
        assert_eq!(
            resolved_value(&style, "margin", &resolver).as_deref(),
            Some("0px 0px 0px 8px")
        );
        assert_eq!(
            resolved_value(&style, "margin-left", &resolver).as_deref(),
            Some("8px")
        );
    }

    #[test]
    fn currentcolor_is_the_elements_own_colour() {
        // D66, G226: inherited as the keyword, the `p`'s blue.
        let style = styled(
            "<div style=\"color: red; border-color: currentcolor\">\
             <p id=t style=\"color: blue; border-color: inherit\"></p></div>",
            "",
        );
        assert_eq!(
            computed_value(&style, "border-color").as_deref(),
            Some("rgb(0, 0, 255)")
        );
    }

    #[test]
    fn a_custom_propertys_tokens_are_written_back() {
        let written = tokens(&css::tokenize(" 10px  calc(1px + 2%) \"a\\\"b\" #f00 , x "));
        assert_eq!(written, "10px calc(1px + 2%) \"a\\\"b\" #f00 , x");
    }
}
