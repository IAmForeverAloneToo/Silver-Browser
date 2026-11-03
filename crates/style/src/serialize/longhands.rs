//! The computed values of the longhands past those [`super`] gives
//! itself (D66, G159): the text, table, grid, transform, motion, break,
//! list, decoration, background, image and border image properties, as
//! the reference browser writes them; with what they are made of, an
//! `<image>` (G232), a calculation (CSS Values 4 section 10.13) and an
//! easing function.

use crate::calc::{Calc, Expression, Op, Rounding};
use crate::color::{Hue, Method, Space};
use crate::computed::{
    AnimationDirection, AnimationFillMode, AnimationPlayState, Attachment, BackfaceVisibility,
    BackgroundBox, BackgroundSize, BorderImageRepeat, BorderImageWidth, BreakBetween, BreakInside,
    ComputedStyle, Content, ContentItem, DecorationStyle, DecorationThickness, Gradient,
    GradientItem, GradientShape, GridLine, Image, ImageOrientation, ImageRendering,
    LengthPercentage, LineDirection, ListStylePosition, ListStyleType, ObjectFit, Outset,
    PrintColorAdjust, Quotes, RadialSize, Repeat, RepeatCount, SliceSide, TemplateAreas,
    TextAlignLast, TextJustify, TrackBreadth, TrackList, TrackSection, TrackSize,
    TransformFunction, TransformStyle, TransitionBehavior, TransitionProperty, UnderlinePosition,
};
use crate::easing::{Easing, StepPosition};

use super::{color, length_percentage, number, px, self_alignment, string, text_align};
use crate::computed::{
    AutospaceMode, HangingEnd, Hyphens, LineBreak, TabSize, TextAutospace, TextFitKind,
    TextFitMethod, TextSpacingTrim, TextWrapStyle, WordSpaceTransform,
};

/// `hyphenate-limit-chars`, at its shortest: the third left out where it
/// is the second, then the second where it is `auto`.
fn hyphenate_limit_chars(limits: [Option<i32>; 3]) -> String {
    let word = |v: Option<i32>| v.map_or("auto".to_owned(), |n| n.to_string());
    let mut parts: Vec<String> = limits.iter().map(|&v| word(v)).collect();
    if limits[2] == limits[1] {
        parts.pop();
        if limits[1].is_none() {
            parts.pop();
        }
    }
    parts.join(" ")
}

/// `text-autospace`, its keywords in the grammar's order.
pub(super) fn text_autospace(value: TextAutospace) -> String {
    match value {
        TextAutospace::Normal => "normal".to_owned(),
        TextAutospace::NoAutospace => "no-autospace".to_owned(),
        TextAutospace::Spaces {
            ideograph_alpha,
            ideograph_numeric,
            punctuation,
            mode,
        } => {
            let mut parts = Vec::new();
            if ideograph_alpha {
                parts.push("ideograph-alpha");
            }
            if ideograph_numeric {
                parts.push("ideograph-numeric");
            }
            if punctuation {
                parts.push("punctuation");
            }
            match mode {
                AutospaceMode::Unset => {}
                AutospaceMode::Insert => parts.push("insert"),
                AutospaceMode::Replace => parts.push("replace"),
            }
            parts.join(" ")
        }
    }
}

/// `text-spacing-trim`.
pub(super) fn text_spacing_trim(value: TextSpacingTrim) -> &'static str {
    match value {
        TextSpacingTrim::Normal => "normal",
        TextSpacingTrim::SpaceAll => "space-all",
        TextSpacingTrim::SpaceFirst => "space-first",
        TextSpacingTrim::TrimStart => "trim-start",
        TextSpacingTrim::TrimBoth => "trim-both",
        TextSpacingTrim::TrimAll => "trim-all",
        TextSpacingTrim::Auto => "auto",
    }
}

/// `text-wrap-style`.
pub(super) fn text_wrap_style(value: TextWrapStyle) -> &'static str {
    match value {
        TextWrapStyle::Auto => "auto",
        TextWrapStyle::Balance => "balance",
        TextWrapStyle::Stable => "stable",
        TextWrapStyle::Pretty => "pretty",
    }
}

/// What makes a `url()`'s URL absolute, where the caller knows how.
pub(super) type Url<'a> = &'a dyn Fn(&str) -> Option<String>;

/// `name`'s computed value in `style`, for the longhands this module
/// gives; `None` for any other name.
pub(super) fn value(style: &ComputedStyle, name: &str, url: Url<'_>) -> Option<String> {
    let lp = |v: &LengthPercentage| length_percentage(v).unwrap_or_default();
    Some(match name {
        "text-align-last" => match style.text_align_last {
            TextAlignLast::Auto => "auto".to_owned(),
            TextAlignLast::Align(a) => text_align(a).to_owned(),
        },
        "text-align-all" => text_align(style.text_align).to_owned(),
        "text-indent" => {
            let indent = &style.text().text_indent;
            let mut out = lp(&indent.length);
            if indent.hanging {
                out.push_str(" hanging");
            }
            if indent.each_line {
                out.push_str(" each-line");
            }
            out
        }
        "tab-size" => match style.text().tab_size {
            TabSize::Spaces(n) => number(n),
            TabSize::Length(v) => px(v),
        },
        "line-break" => match style.text().line_break {
            LineBreak::Auto => "auto",
            LineBreak::Loose => "loose",
            LineBreak::Normal => "normal",
            LineBreak::Strict => "strict",
            LineBreak::Anywhere => "anywhere",
        }
        .to_owned(),
        "hyphens" => match style.text().hyphens {
            Hyphens::None => "none",
            Hyphens::Manual => "manual",
            Hyphens::Auto => "auto",
        }
        .to_owned(),
        "hyphenate-character" => match &style.text().hyphenate_character {
            None => "auto".to_owned(),
            Some(text) => string(text),
        },
        "hyphenate-limit-chars" => hyphenate_limit_chars(style.text().hyphenate_limit_chars),
        "hyphenate-limit-lines" => match style.text().hyphenate_limit_lines {
            None => "no-limit".to_owned(),
            Some(n) => n.to_string(),
        },
        "text-autospace" => text_autospace(style.text().text_autospace),
        "text-spacing-trim" => text_spacing_trim(style.text().text_spacing_trim).to_owned(),
        "ruby-position" => match style.text().ruby_position {
            crate::computed::RubyPosition::Over => "over",
            crate::computed::RubyPosition::Under => "under",
        }
        .to_owned(),
        "math-style" => match style.text().math_style {
            crate::computed::MathStyle::Normal => "normal",
            crate::computed::MathStyle::Compact => "compact",
        }
        .to_owned(),
        "ruby-align" => match style.text().ruby_align {
            crate::computed::RubyAlign::SpaceAround => "space-around",
            crate::computed::RubyAlign::SpaceBetween => "space-between",
            crate::computed::RubyAlign::Start => "start",
            crate::computed::RubyAlign::Center => "center",
        }
        .to_owned(),
        "hanging-punctuation" => {
            let hanging = style.text().hanging_punctuation;
            let mut parts = Vec::new();
            if hanging.first {
                parts.push("first");
            }
            match hanging.end {
                HangingEnd::None => {}
                HangingEnd::ForceEnd => parts.push("force-end"),
                HangingEnd::AllowEnd => parts.push("allow-end"),
            }
            if hanging.last {
                parts.push("last");
            }
            if parts.is_empty() {
                "none".to_owned()
            } else {
                parts.join(" ")
            }
        }
        "text-wrap-style" => text_wrap_style(style.text().text_wrap_style).to_owned(),
        "word-space-transform" => match style.text().word_space_transform {
            WordSpaceTransform::None => "none".to_owned(),
            WordSpaceTransform::Space {
                ideographic,
                auto_phrase,
            } => {
                let space = if ideographic {
                    "ideographic-space"
                } else {
                    "space"
                };
                if auto_phrase {
                    format!("{space} auto-phrase")
                } else {
                    space.to_owned()
                }
            }
        },
        "text-fit" => {
            let fit = style.text().text_fit;
            let mut parts = vec![match fit.kind {
                TextFitKind::None => return Some("none".to_owned()),
                TextFitKind::Grow => "grow".to_owned(),
                TextFitKind::Shrink => "shrink".to_owned(),
            }];
            match fit.method {
                TextFitMethod::Consistent => {}
                TextFitMethod::PerLine => parts.push("per-line".to_owned()),
                TextFitMethod::PerLineAll => parts.push("per-line-all".to_owned()),
            }
            if let Some(limit) = fit.limit {
                parts.push(format!("{}%", number(limit)));
            }
            parts.join(" ")
        }
        "text-justify" => match style.text_justify {
            TextJustify::Auto => "auto",
            TextJustify::None => "none",
            TextJustify::InterWord => "inter-word",
            TextJustify::InterCharacter => "inter-character",
        }
        .to_owned(),
        // `border-spacing`'s longhands, as the reference browser has it.
        "-webkit-border-horizontal-spacing" => px(style.border_spacing.horizontal),
        "-webkit-border-vertical-spacing" => px(style.border_spacing.vertical),
        "content" => content(&style.content, url),
        "quotes" => match &style.quotes {
            Quotes::Auto => "auto".to_owned(),
            Quotes::None => "none".to_owned(),
            Quotes::Pairs(pairs) => pairs
                .iter()
                .map(|(open, close)| format!("{} {}", string(open), string(close)))
                .collect::<Vec<_>>()
                .join(" "),
        },
        "justify-items" => {
            let items = style.grid().justify_items;
            let value = self_alignment(items.value);
            if items.legacy {
                format!("legacy {value}")
            } else {
                value
            }
        }
        "justify-self" => self_alignment(style.grid().justify_self),
        "grid-template-columns" => track_list(style.grid().template_columns.as_ref()),
        "grid-template-rows" => track_list(style.grid().template_rows.as_ref()),
        "grid-template-areas" => match &style.grid().template_areas {
            None => "none".to_owned(),
            Some(areas) => area_rows(areas)
                .iter()
                .map(|row| string(row))
                .collect::<Vec<_>>()
                .join(" "),
        },
        "grid-auto-columns" => track_sizes(&style.grid().auto_columns),
        "grid-auto-rows" => track_sizes(&style.grid().auto_rows),
        "grid-auto-flow" => {
            let flow = style.grid().auto_flow;
            let axis = if flow.column { "column" } else { "row" };
            if flow.dense {
                format!("{axis} dense")
            } else {
                axis.to_owned()
            }
        }
        "grid-row-start" => grid_line(&style.grid().row_start),
        "grid-row-end" => grid_line(&style.grid().row_end),
        "grid-column-start" => grid_line(&style.grid().column_start),
        "grid-column-end" => grid_line(&style.grid().column_end),
        // The resolved value (CSSOM section 9): the list's matrix, which
        // a translation's percentage makes of the box, not known here.
        "transform" => transform(&style.effects().transform)?,
        "transform-origin" => {
            let origin = &style.effects().transform_origin;
            let mut text = format!("{} {}", lp(&origin.x), lp(&origin.y));
            if origin.z != 0.0 {
                text = format!("{text} {}", px(origin.z));
            }
            text
        }
        "translate" => match &style.effects().translate {
            None => "none".to_owned(),
            Some(t) if t.z != 0.0 => format!("{} {} {}", lp(&t.x), lp(&t.y), px(t.z)),
            Some(t) if t.y == LengthPercentage::Px(0.0) => lp(&t.x),
            Some(t) => format!("{} {}", lp(&t.x), lp(&t.y)),
        },
        "transform-style" => match style.effects().transform_style {
            TransformStyle::Flat => "flat",
            TransformStyle::Preserve3d => "preserve-3d",
        }
        .to_owned(),
        "perspective" => style
            .effects()
            .perspective
            .map_or_else(|| "none".to_owned(), px),
        "perspective-origin" => {
            let origin = &style.effects().perspective_origin;
            format!("{} {}", lp(&origin.x), lp(&origin.y))
        }
        "backface-visibility" => match style.effects().backface_visibility {
            BackfaceVisibility::Visible => "visible",
            BackfaceVisibility::Hidden => "hidden",
        }
        .to_owned(),
        "scroll-behavior" => if style.effects().scroll_behavior_smooth {
            "smooth"
        } else {
            "auto"
        }
        .to_owned(),
        "transition-property" => {
            let list = &style.motion().transition_property;
            if list.is_empty() {
                "none".to_owned()
            } else {
                list.iter()
                    .map(|p| match p {
                        TransitionProperty::All => "all".to_owned(),
                        TransitionProperty::Name(name) => name.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        }
        "transition-duration" => times(&style.motion().transition_duration),
        "transition-delay" => times(&style.motion().transition_delay),
        "transition-timing-function" => easings(&style.motion().transition_timing_function),
        "transition-behavior" => list(&style.motion().transition_behavior, |b| match b {
            TransitionBehavior::Normal => "normal".to_owned(),
            TransitionBehavior::AllowDiscrete => "allow-discrete".to_owned(),
        }),
        "animation-name" => list(&style.motion().animation_name, |n| {
            n.as_ref()
                .map_or_else(|| "none".to_owned(), |n| n.to_string())
        }),
        "animation-duration" => times(&style.motion().animation_duration),
        "animation-delay" => times(&style.motion().animation_delay),
        "animation-timing-function" => easings(&style.motion().animation_timing_function),
        "animation-iteration-count" => list(&style.motion().animation_iteration_count, |&n| {
            if n.is_infinite() {
                "infinite".to_owned()
            } else {
                number(n as f32)
            }
        }),
        "animation-direction" => list(&style.motion().animation_direction, |d| {
            match d {
                AnimationDirection::Normal => "normal",
                AnimationDirection::Reverse => "reverse",
                AnimationDirection::Alternate => "alternate",
                AnimationDirection::AlternateReverse => "alternate-reverse",
            }
            .to_owned()
        }),
        "animation-play-state" => list(&style.motion().animation_play_state, |s| {
            match s {
                AnimationPlayState::Running => "running",
                AnimationPlayState::Paused => "paused",
            }
            .to_owned()
        }),
        "animation-composition" => list(&style.motion().animation_composition, |c| {
            c.name().to_owned()
        }),
        "animation-fill-mode" => list(&style.motion().animation_fill_mode, |f| {
            match f {
                AnimationFillMode::None => "none",
                AnimationFillMode::Forwards => "forwards",
                AnimationFillMode::Backwards => "backwards",
                AnimationFillMode::Both => "both",
            }
            .to_owned()
        }),
        "break-before" => break_between(style.fragmentation().break_before).to_owned(),
        "break-after" => break_between(style.fragmentation().break_after).to_owned(),
        "break-inside" => match style.fragmentation().break_inside {
            BreakInside::Auto => "auto",
            BreakInside::Avoid => "avoid",
            BreakInside::AvoidPage => "avoid-page",
            BreakInside::AvoidColumn => "avoid-column",
            BreakInside::AvoidRegion => "avoid-region",
        }
        .to_owned(),
        "orphans" => style.fragmentation().orphans.to_string(),
        "widows" => style.fragmentation().widows.to_string(),
        "print-color-adjust" => match style.fragmentation().print_color_adjust {
            PrintColorAdjust::Economy => "economy",
            PrintColorAdjust::Exact => "exact",
        }
        .to_owned(),
        "counter-reset" => {
            let counters = &style.lists().counter_reset;
            if counters.is_empty() {
                "none".to_owned()
            } else {
                counters
                    .iter()
                    .map(|c| {
                        let name = if c.reversed {
                            format!("reversed({})", c.name)
                        } else {
                            c.name.to_string()
                        };
                        match c.value {
                            Some(v) => format!("{name} {v}"),
                            None => name,
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        }
        "counter-increment" | "counter-set" => {
            let lists = style.lists();
            let counters = if name == "counter-set" {
                &lists.counter_set
            } else {
                &lists.counter_increment
            };
            if counters.is_empty() {
                "none".to_owned()
            } else {
                counters
                    .iter()
                    .map(|c| format!("{} {}", c.name, c.value))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        }
        "list-style-type" => match &style.lists().list_style_type {
            ListStyleType::Style(name) => name.to_string(),
            ListStyleType::String(text) => string(text),
            ListStyleType::None => "none".to_owned(),
        },
        "list-style-position" => match style.lists().list_style_position {
            ListStylePosition::Outside => "outside",
            ListStylePosition::Inside => "inside",
        }
        .to_owned(),
        "list-style-image" => optional_image(style.lists().list_style_image.as_ref(), url),
        "text-decoration-line" => {
            let line = style.decoration().line;
            let words: Vec<&str> = [
                (line.underline, "underline"),
                (line.overline, "overline"),
                (line.line_through, "line-through"),
                (line.blink, "blink"),
            ]
            .into_iter()
            .filter_map(|(on, word)| on.then_some(word))
            .collect();
            if words.is_empty() {
                "none".to_owned()
            } else {
                words.join(" ")
            }
        }
        "text-decoration-style" => match style.decoration().style {
            DecorationStyle::Solid => "solid",
            DecorationStyle::Double => "double",
            DecorationStyle::Dotted => "dotted",
            DecorationStyle::Dashed => "dashed",
            DecorationStyle::Wavy => "wavy",
        }
        .to_owned(),
        "text-decoration-color" => color(style.decoration().color.unwrap_or(style.color)),
        "text-decoration-thickness" => match &style.decoration().thickness {
            DecorationThickness::Auto => "auto".to_owned(),
            DecorationThickness::FromFont => "from-font".to_owned(),
            DecorationThickness::Length(v) => lp(v),
        },
        "text-underline-offset" => style
            .decoration()
            .underline_offset
            .as_ref()
            .map_or_else(|| "auto".to_owned(), lp),
        "text-underline-position" => match style.decoration().underline_position {
            UnderlinePosition::Auto => "auto",
            UnderlinePosition::Under => "under",
            UnderlinePosition::FromFont => "from-font",
        }
        .to_owned(),
        // Each layer's value, as many as `background-image` has.
        "background-image" => layers(style, |l| optional_image(l.image, url)),
        "background-repeat" => layers(style, |l| repeat(l.repeat)),
        "background-size" => layers(style, |l| background_size(l.size)),
        "background-origin" => layers(style, |l| background_box(l.origin).to_owned()),
        "background-clip" => layers(style, |l| background_box(l.clip).to_owned()),
        "background-attachment" => layers(style, |l| {
            match l.attachment {
                Attachment::Scroll => "scroll",
                Attachment::Fixed => "fixed",
                Attachment::Local => "local",
            }
            .to_owned()
        }),
        "object-fit" => match style.image().object_fit {
            ObjectFit::Fill => "fill",
            ObjectFit::Contain => "contain",
            ObjectFit::Cover => "cover",
            ObjectFit::None => "none",
            ObjectFit::ScaleDown => "scale-down",
        }
        .to_owned(),
        "object-position" => {
            let position = &style.image().object_position;
            format!("{} {}", lp(&position.x), lp(&position.y))
        }
        "image-orientation" => match style.image().image_orientation {
            ImageOrientation::FromImage => "from-image",
            ImageOrientation::None => "none",
        }
        .to_owned(),
        "image-rendering" => match style.image().image_rendering {
            ImageRendering::Auto => "auto",
            ImageRendering::Smooth => "smooth",
            ImageRendering::HighQuality => "high-quality",
            ImageRendering::Pixelated => "pixelated",
            ImageRendering::CrispEdges => "crisp-edges",
        }
        .to_owned(),
        "border-image-source" => optional_image(style.border_image().source.as_ref(), url),
        "border-image-slice" => {
            let border = style.border_image();
            let sides = border.slice.map(|s| match s {
                SliceSide::Number(n) => number(n),
                SliceSide::Percent(p) => super::percent(p),
            });
            let text = shortest_sides(&sides);
            if border.fill {
                format!("{text} fill")
            } else {
                text
            }
        }
        "border-image-width" => {
            let sides = style.border_image().width.clone().map(|w| match w {
                BorderImageWidth::Number(n) => number(n),
                BorderImageWidth::Length(v) => lp(&v),
                BorderImageWidth::Auto => "auto".to_owned(),
            });
            shortest_sides(&sides)
        }
        "border-image-outset" => {
            let sides = style.border_image().outset.map(|o| match o {
                Outset::Number(n) => number(n),
                Outset::Px(v) => px(v),
            });
            shortest_sides(&sides)
        }
        "border-image-repeat" => {
            let word = |r: BorderImageRepeat| match r {
                BorderImageRepeat::Stretch => "stretch",
                BorderImageRepeat::Repeat => "repeat",
                BorderImageRepeat::Round => "round",
                BorderImageRepeat::Space => "space",
            };
            let [x, y] = style.border_image().repeat;
            if x == y {
                word(x).to_owned()
            } else {
                format!("{} {}", word(x), word(y))
            }
        }
        _ => return None,
    })
}

/// Four sides' values at their shortest (CSSOM section 6.7.2): one
/// where all are alike, two where the left is the right and the bottom
/// the top, three where the left is the right.
pub(super) fn shortest_sides(sides: &[String; 4]) -> String {
    let [top, right, bottom, left] = sides;
    if left != right {
        format!("{top} {right} {bottom} {left}")
    } else if bottom != top {
        format!("{top} {right} {bottom}")
    } else if right != top {
        format!("{top} {right}")
    } else {
        top.clone()
    }
}

/// A list's items, each written by `item`, separated by commas.
fn list<T>(items: &[T], item: impl Fn(&T) -> String) -> String {
    items.iter().map(item).collect::<Vec<_>>().join(", ")
}

/// Times in seconds, `0s`: past what an `f32` holds, the largest it does
/// (D78), as a computed infinity is written.
fn times(list: &[f64]) -> String {
    self::list(list, |&s| super::shorthands::seconds(s))
}

fn easings(list: &[Easing]) -> String {
    self::list(list, easing)
}

/// An easing function as the reference browser writes it: a keyword for
/// the curves that have one, `steps(n)` for `step-end`.
pub(super) fn easing(easing: &Easing) -> String {
    let n = |v: f64| number(v as f32);
    match easing {
        Easing::Linear => "linear".to_owned(),
        e if *e == Easing::EASE => "ease".to_owned(),
        e if *e == Easing::EASE_IN => "ease-in".to_owned(),
        e if *e == Easing::EASE_OUT => "ease-out".to_owned(),
        e if *e == Easing::EASE_IN_OUT => "ease-in-out".to_owned(),
        Easing::CubicBezier { x1, y1, x2, y2 } => {
            format!(
                "cubic-bezier({}, {}, {}, {})",
                n(*x1),
                n(*y1),
                n(*x2),
                n(*y2)
            )
        }
        Easing::Steps { count, position } => match position {
            StepPosition::JumpEnd => format!("steps({count})"),
            StepPosition::JumpStart => format!("steps({count}, start)"),
            StepPosition::JumpNone => format!("steps({count}, jump-none)"),
            StepPosition::JumpBoth => format!("steps({count}, jump-both)"),
        },
        Easing::Points(points) => format!(
            "linear({})",
            points
                .iter()
                .map(|(input, output)| format!("{} {}%", n(*output), n(input * 100.0)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn break_between(value: BreakBetween) -> &'static str {
    match value {
        BreakBetween::Auto => "auto",
        BreakBetween::Avoid => "avoid",
        BreakBetween::AvoidPage => "avoid-page",
        BreakBetween::Page => "page",
        BreakBetween::Left => "left",
        BreakBetween::Right => "right",
        BreakBetween::Recto => "recto",
        BreakBetween::Verso => "verso",
        BreakBetween::AvoidColumn => "avoid-column",
        BreakBetween::Column => "column",
        BreakBetween::AvoidRegion => "avoid-region",
        BreakBetween::Region => "region",
    }
}

/// `content`: its parts, a string quoted, `counter()` without its
/// `decimal`.
fn content(value: &Content, url: Url<'_>) -> String {
    let items = match value {
        Content::Normal => return "normal".to_owned(),
        Content::None => return "none".to_owned(),
        Content::Items(items) => items,
    };
    let counter_style = |style: &str| {
        if style == "decimal" {
            String::new()
        } else {
            format!(", {style}")
        }
    };
    items
        .iter()
        .map(|item| match item {
            ContentItem::Text(text) => string(text),
            ContentItem::Counter { name, style } => {
                format!("counter({name}{})", counter_style(style))
            }
            ContentItem::Counters {
                name,
                separator,
                style,
            } => format!(
                "counters({name}, {}{})",
                string(separator),
                counter_style(style)
            ),
            ContentItem::OpenQuote => "open-quote".to_owned(),
            ContentItem::CloseQuote => "close-quote".to_owned(),
            ContentItem::NoOpenQuote => "no-open-quote".to_owned(),
            ContentItem::NoCloseQuote => "no-close-quote".to_owned(),
            ContentItem::Image(image) => self::image(image, url),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Each background layer's value by `item`, the other lists repeated
/// to `background-image`'s length (Backgrounds 3 section 2.2).
pub(super) fn layers(
    style: &ComputedStyle,
    item: impl Fn(&crate::computed::BackgroundLayer<'_>) -> String,
) -> String {
    style
        .background()
        .layers()
        .map(|layer| item(&layer))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A layer's `background-repeat`: one keyword where one says both.
pub(super) fn repeat([x, y]: [Repeat; 2]) -> String {
    let word = |r: Repeat| match r {
        Repeat::Repeat => "repeat",
        Repeat::Space => "space",
        Repeat::Round => "round",
        Repeat::NoRepeat => "no-repeat",
    };
    match (x, y) {
        (Repeat::Repeat, Repeat::NoRepeat) => "repeat-x".to_owned(),
        (Repeat::NoRepeat, Repeat::Repeat) => "repeat-y".to_owned(),
        (x, y) if x == y => word(x).to_owned(),
        (x, y) => format!("{} {}", word(x), word(y)),
    }
}

/// A layer's `background-size`: one `auto` where both sides are, else
/// both sides, as the reference browser writes it (`15px auto`).
pub(super) fn background_size(size: &BackgroundSize) -> String {
    let side = |v: &Option<LengthPercentage>| {
        v.as_ref().map_or_else(
            || "auto".to_owned(),
            |v| length_percentage(v).unwrap_or_default(),
        )
    };
    match size {
        BackgroundSize::Cover => "cover".to_owned(),
        BackgroundSize::Contain => "contain".to_owned(),
        BackgroundSize::Sized(None, None) => "auto".to_owned(),
        BackgroundSize::Sized(w, h) => format!("{} {}", side(w), side(h)),
    }
}

pub(super) fn background_box(value: BackgroundBox) -> &'static str {
    match value {
        BackgroundBox::BorderBox => "border-box",
        BackgroundBox::PaddingBox => "padding-box",
        BackgroundBox::ContentBox => "content-box",
    }
}

/// `none`, or the image.
pub(super) fn optional_image(image: Option<&Image>, url: Url<'_>) -> String {
    image.map_or_else(|| "none".to_owned(), |image| self::image(image, url))
}

/// An `<image>` as the reference browser writes its computed value
/// (G232): a `url()` of a string, its URL absolute where `url` makes it
/// so; a gradient with its colours computed.
pub(super) fn image(image: &Image, url: Url<'_>) -> String {
    match image {
        Image::Url { url: written, .. } => url_value(written, url),
        Image::Gradient(gradient) => self::gradient(gradient),
    }
}

/// A `url()` of a string, its URL absolute where `url` makes it so; an
/// empty or fragment-only one as written (Values 4 sections 4.5.1 and
/// 4.5.2).
pub(super) fn url_value(written: &str, url: Url<'_>) -> String {
    let absolute = match written.is_empty() || written.starts_with('#') {
        true => written.to_string(),
        false => url(written).unwrap_or_else(|| written.to_string()),
    };
    format!("url({})", string(&absolute))
}

/// A gradient (CSS Images 3 and 4 section 3), as the reference browser
/// writes it: what was left to its default left out (`to bottom`, a
/// radial gradient's `ellipse farthest-corner at center`, a conic one's
/// `from 0deg at center`), its colours computed, its angles in degrees
/// to six figures (`114.592deg`, D78).
fn gradient(gradient: &Gradient) -> String {
    let lp = |v: &LengthPercentage| length_percentage(v).unwrap_or_default();
    let centre = |c: &[LengthPercentage; 2]| {
        let middle = LengthPercentage::Percent(50.0);
        (c[0] != middle || c[1] != middle).then(|| format!("at {} {}", lp(&c[0]), lp(&c[1])))
    };
    let (name, mut prelude, conic) = match &gradient.shape {
        GradientShape::Linear(direction) => {
            let prelude = match *direction {
                LineDirection::Side(degrees) => match degrees {
                    0.0 => Some("to top".to_owned()),
                    90.0 => Some("to right".to_owned()),
                    270.0 => Some("to left".to_owned()),
                    _ => None,
                },
                LineDirection::Angle(degrees) => {
                    Some(format!("{}deg", crate::color::number(degrees)))
                }
                // The horizontal side first, as the reference browser
                // writes a corner whichever order it was given in.
                LineDirection::Corner { right, bottom } => Some(format!(
                    "to {} {}",
                    if right { "right" } else { "left" },
                    if bottom { "bottom" } else { "top" }
                )),
            };
            ("linear-gradient", prelude, false)
        }
        GradientShape::Radial {
            ellipse,
            size,
            center,
        } => {
            let mut words: Vec<String> = Vec::new();
            let keyword = match size {
                RadialSize::ClosestSide => Some("closest-side"),
                RadialSize::FarthestSide => Some("farthest-side"),
                RadialSize::ClosestCorner => Some("closest-corner"),
                RadialSize::FarthestCorner => None,
                RadialSize::Circle(_) | RadialSize::Ellipse(..) => None,
            };
            // The shape is written where the size does not say it.
            let sized = matches!(size, RadialSize::Circle(_) | RadialSize::Ellipse(..));
            if !*ellipse && !sized {
                words.push("circle".to_owned());
            }
            if let Some(keyword) = keyword {
                words.push(keyword.to_owned());
            }
            match size {
                RadialSize::Circle(r) => words.push(px(*r)),
                RadialSize::Ellipse(rx, ry) => words.push(format!("{} {}", lp(rx), lp(ry))),
                _ => {}
            }
            words.extend(centre(center));
            (
                "radial-gradient",
                (!words.is_empty()).then(|| words.join(" ")),
                false,
            )
        }
        GradientShape::Conic { from, center } => {
            let mut words: Vec<String> = Vec::new();
            if *from != 0.0 {
                words.push(format!("from {}deg", crate::color::number(*from)));
            }
            words.extend(centre(center));
            (
                "conic-gradient",
                (!words.is_empty()).then(|| words.join(" ")),
                true,
            )
        }
    };
    if let Some(method) = gradient.method
        && !is_default_method(gradient, method)
    {
        let text = method_text(method);
        prelude = Some(match prelude {
            Some(p) => format!("{p} {text}"),
            None => text,
        });
    }
    let position = |v: &LengthPercentage| match v {
        LengthPercentage::Px(degrees) if conic => format!("{}deg", crate::color::number(*degrees)),
        other => lp(other),
    };
    let items = gradient.items.iter().map(|item| match item {
        GradientItem::Stop(c, None) => color(*c),
        GradientItem::Stop(c, Some(at)) => format!("{} {}", color(*c), position(at)),
        GradientItem::Hint(at) => position(at),
    });
    let parts: Vec<String> = prelude.into_iter().chain(items).collect();
    let repeating = if gradient.repeating { "repeating-" } else { "" };
    format!("{repeating}{name}({})", parts.join(", "))
}

/// Whether `method` is the one the gradient's colours would have
/// anyway: sRGB for legacy colours, Oklab otherwise (Images 4 section
/// 3.1).
fn is_default_method(gradient: &Gradient, method: Method) -> bool {
    let legacy = gradient.items.iter().all(|item| match item {
        GradientItem::Stop(c, _) => c.is_legacy(),
        GradientItem::Hint(_) => true,
    });
    let space = if legacy { Space::Srgb } else { Space::Oklab };
    method.space == space && method.hue == Hue::Shorter
}

/// A `<color-interpolation-method>`: `in` its space, and its hue's arc
/// where that is not the shorter.
fn method_text(method: Method) -> String {
    let space = match method.space {
        Space::Lab => "lab",
        Space::Lch => "lch",
        Space::Oklab => "oklab",
        Space::Oklch => "oklch",
        Space::Hsl => "hsl",
        Space::Hwb => "hwb",
        other => other.predefined_name().unwrap_or("srgb"),
    };
    let hue = match method.hue {
        Hue::Shorter => "",
        Hue::Longer => " longer hue",
        Hue::Increasing => " increasing hue",
        Hue::Decreasing => " decreasing hue",
    };
    if method.space.is_polar() {
        format!("in {space}{hue}")
    } else {
        format!("in {space}")
    }
}

/// `transform`'s resolved value: `none`, or the list's matrix as
/// `matrix()` or `matrix3d()`, its numbers of six significant figures;
/// `None` where a translation's percentage needs the box.
fn transform(list: &[TransformFunction]) -> Option<String> {
    if list.is_empty() {
        return Some("none".to_owned());
    }
    let has_percent = |f: &TransformFunction| match f {
        TransformFunction::Translate(x, y, _) => x.has_percent() || y.has_percent(),
        TransformFunction::Interpolated(_) => true,
        _ => false,
    };
    if list.iter().any(has_percent) {
        return None;
    }
    let m = crate::transform::list_matrix(list, 0.0, 0.0);
    // What single precision leaves of a zero (a quarter turn's cosine)
    // is zero.
    let n = |v: &f32| crate::color::number(if v.abs() < 1e-6 { 0.0 } else { *v });
    Some(match m.to_2d() {
        Some(two) if m.is_2d() => format!(
            "matrix({})",
            two.iter().map(n).collect::<Vec<_>>().join(", ")
        ),
        _ => format!(
            "matrix3d({})",
            m.0.iter().map(n).collect::<Vec<_>>().join(", ")
        ),
    })
}

/// A track list, or `none`.
fn track_list(list: Option<&TrackList>) -> String {
    let Some(list) = list else {
        return "none".to_owned();
    };
    let mut parts: Vec<String> = Vec::new();
    for (i, section) in list.sections.iter().enumerate() {
        parts.extend(line_names(&list.names[i]));
        parts.push(match section {
            TrackSection::Track(size) => track_size(size),
            TrackSection::Repeat(repeat) => {
                let count = match repeat.count {
                    RepeatCount::Count(n) => n.to_string(),
                    RepeatCount::AutoFill => "auto-fill".to_owned(),
                    RepeatCount::AutoFit => "auto-fit".to_owned(),
                };
                let mut inner: Vec<String> = Vec::new();
                for (j, size) in repeat.tracks.iter().enumerate() {
                    inner.extend(line_names(&repeat.names[j]));
                    inner.push(track_size(size));
                }
                inner.extend(repeat.names.last().and_then(|n| line_names(n)));
                format!("repeat({count}, {})", inner.join(" "))
            }
        });
    }
    parts.extend(list.names.last().and_then(|n| line_names(n)));
    parts.join(" ")
}

/// A line's names in brackets; `None` for a line without.
pub(super) fn line_names(names: &[std::sync::Arc<str>]) -> Option<String> {
    (!names.is_empty()).then(|| {
        format!(
            "[{}]",
            names.iter().map(|n| &**n).collect::<Vec<_>>().join(" ")
        )
    })
}

pub(super) fn track_sizes(sizes: &[TrackSize]) -> String {
    sizes.iter().map(track_size).collect::<Vec<_>>().join(" ")
}

pub(super) fn track_size(size: &TrackSize) -> String {
    match size {
        TrackSize::Breadth(b) => breadth(b),
        TrackSize::MinMax(min, max) => format!("minmax({}, {})", breadth(min), breadth(max)),
        TrackSize::FitContent(v) => {
            format!("fit-content({})", length_percentage(v).unwrap_or_default())
        }
    }
}

fn breadth(breadth: &TrackBreadth) -> String {
    match breadth {
        TrackBreadth::Fixed(v) => length_percentage(v).unwrap_or_default(),
        TrackBreadth::Flex(f) => format!("{}fr", number(*f)),
        TrackBreadth::MinContent => "min-content".to_owned(),
        TrackBreadth::MaxContent => "max-content".to_owned(),
        TrackBreadth::Auto => "auto".to_owned(),
    }
}

/// `grid-template-areas`' rows, each its cells' names, `.` for a cell
/// no area takes.
pub(super) fn area_rows(areas: &TemplateAreas) -> Vec<String> {
    let mut cells = vec![vec!["."; areas.columns as usize]; areas.rows as usize];
    for area in &areas.areas {
        for row in area.rows.0..area.rows.1 {
            for column in area.columns.0..area.columns.1 {
                if let Some(cell) = cells
                    .get_mut(row as usize)
                    .and_then(|r| r.get_mut(column as usize))
                {
                    *cell = &area.name;
                }
            }
        }
    }
    cells.into_iter().map(|row| row.join(" ")).collect()
}

/// A `<grid-line>`.
pub(super) fn grid_line(line: &GridLine) -> String {
    match line {
        GridLine::Auto => "auto".to_owned(),
        GridLine::Ident(name) => name.to_string(),
        GridLine::Line(n, None) => n.to_string(),
        GridLine::Line(n, Some(name)) => format!("{n} {name}"),
        GridLine::Span(1, Some(name)) => format!("span {name}"),
        GridLine::Span(n, None) => format!("span {n}"),
        GridLine::Span(n, Some(name)) => format!("span {n} {name}"),
    }
}

/// A computed calculation (CSS Values 4 section 10.13): `calc()` around
/// a sum or a product, a function's own name around its arguments; a
/// sum's leaves sorted, the percentage before the length, a negative
/// one subtracted. Its lengths are CSS pixels.
pub(super) fn calc(calc: &Calc) -> String {
    let text = node(calc.expression(), true);
    match calc.expression() {
        Expression::Sum(_)
        | Expression::Negate(_)
        | Expression::Product(_)
        | Expression::Invert(_)
        | Expression::Value(_)
        | Expression::Percent(_) => format!("calc({})", strip(&text)),
        _ => text,
    }
}

/// `text` without the parentheses around it, if it has them.
fn strip(text: &str) -> &str {
    text.strip_prefix('(')
        .and_then(|t| t.strip_suffix(')'))
        .filter(|inner| balanced(inner))
        .unwrap_or(text)
}

/// Whether `text`'s parentheses close as they open.
fn balanced(text: &str) -> bool {
    let mut depth = 0i32;
    for c in text.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// One node of a calculation tree (section 10.13's "serialize a
/// calculation tree"), `length` when its leaves are lengths rather than
/// numbers.
fn node(e: &Expression, length: bool) -> String {
    let leaf = |v: f32| {
        if length { px(v) } else { number(v) }
    };
    let arguments = |args: &[Expression]| {
        args.iter()
            .map(|a| strip(&node(a, length)).to_owned())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match e {
        Expression::Value(v) => leaf(*v),
        Expression::Percent(p) => super::percent(*p),
        Expression::Negate(child) => format!("(-1 * {})", node(child, length)),
        Expression::Invert(child) => format!("(1 / {})", node(child, false)),
        Expression::Sum(children) => {
            // The leaves first, sorted: a number, a percentage, then a
            // length (section 10.12's sort).
            let rank = |e: &Expression| match e {
                Expression::Value(_) if !length => 0,
                Expression::Percent(_) => 1,
                Expression::Value(_) => 2,
                _ => 3,
            };
            let mut sorted: Vec<&Expression> = children.iter().collect();
            sorted.sort_by_key(|e| rank(e));
            let mut text = String::from("(");
            for (i, child) in sorted.into_iter().enumerate() {
                let (minus, written) = match child {
                    Expression::Negate(inner) => (true, node(inner, length)),
                    Expression::Value(v) if *v < 0.0 => (true, leaf(-v)),
                    Expression::Percent(p) if *p < 0.0 => (true, super::percent(-p)),
                    other => (false, node(other, length)),
                };
                if i == 0 {
                    if minus {
                        text.push('-');
                    }
                } else {
                    text.push_str(if minus { " - " } else { " + " });
                }
                text.push_str(&written);
            }
            text.push(')');
            text
        }
        Expression::Product(children) => {
            // The factor with the length in it, if any, carries the
            // unit; the others are numbers.
            let carrier = children.iter().position(has_percent).or_else(|| {
                children
                    .iter()
                    .position(|c| !matches!(c, Expression::Invert(_)))
            });
            let mut text = String::from("(");
            for (i, child) in children.iter().enumerate() {
                let unit = length && Some(i) == carrier;
                if i > 0 {
                    text.push_str(if matches!(child, Expression::Invert(_)) {
                        " / "
                    } else {
                        " * "
                    });
                }
                text.push_str(&match child {
                    Expression::Invert(inner) if i > 0 => node(inner, false),
                    other => node(other, unit),
                });
            }
            text.push(')');
            text
        }
        Expression::Min(args) => format!("min({})", arguments(args)),
        Expression::Max(args) => format!("max({})", arguments(args)),
        Expression::Clamp(bounds) => {
            let bound = |e: &Expression| match e {
                Expression::Value(v) if v.is_infinite() => "none".to_owned(),
                other => strip(&node(other, length)).to_owned(),
            };
            let [low, value, high] = &**bounds;
            format!(
                "clamp({}, {}, {})",
                bound(low),
                strip(&node(value, length)),
                bound(high)
            )
        }
        Expression::Call(op, args) => {
            let name = match op {
                Op::Round(_) => "round",
                Op::Mod => "mod",
                Op::Rem => "rem",
                Op::Abs => "abs",
                Op::Sign => "sign",
                Op::Hypot => "hypot",
                Op::Sin(_) => "sin",
                Op::Cos(_) => "cos",
                Op::Tan(_) => "tan",
                Op::Asin => "asin",
                Op::Acos => "acos",
                Op::Atan => "atan",
                Op::Atan2 => "atan2",
                Op::Pow => "pow",
                Op::Sqrt => "sqrt",
                Op::Log => "log",
                Op::Exp => "exp",
            };
            let strategy = match op {
                Op::Round(Rounding::Up) => "up, ",
                Op::Round(Rounding::Down) => "down, ",
                Op::Round(Rounding::ToZero) => "to-zero, ",
                _ => "",
            };
            // `sign()` of a length is a number, as is what the others
            // past the stepped ones and `hypot()` take.
            let inner = matches!(
                op,
                Op::Round(_) | Op::Mod | Op::Rem | Op::Abs | Op::Sign | Op::Hypot
            );
            let args = args
                .iter()
                .map(|a| strip(&node(a, inner && (length || *op == Op::Sign))).to_owned())
                .collect::<Vec<_>>()
                .join(", ");
            format!("{name}({strategy}{args})")
        }
    }
}

/// Whether an expression has a percentage in it.
fn has_percent(e: &Expression) -> bool {
    match e {
        Expression::Value(_) => false,
        Expression::Percent(_) => true,
        Expression::Negate(c) | Expression::Invert(c) => has_percent(c),
        Expression::Sum(c) | Expression::Product(c) | Expression::Min(c) | Expression::Max(c) => {
            c.iter().any(has_percent)
        }
        Expression::Clamp(c) => c.iter().any(has_percent),
        Expression::Call(_, c) => c.iter().any(has_percent),
    }
}
