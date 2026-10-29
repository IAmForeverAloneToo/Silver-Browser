//! The shorthands' resolved values (D66, G159), serialized from their
//! longhands' as the reference browser does it: at their shortest where
//! it writes them so, and the empty string where the longhands' values
//! cannot be written as the shorthand.

use crate::computed::{
    BreakBetween, BreakInside, ComputedStyle, FontStyle, GridAutoFlow, GridLine, LineHeight,
    TrackSize,
};

use super::longhands::{
    self, area_rows, background_box, background_size, easing, grid_line, line_names, repeat,
    shortest_sides, track_size, track_sizes,
};
use super::{Resolver, length_percentage, number, string};

/// The four sides' longhands of a box shorthand, `{prefix}{side}{suffix}`.
fn sides(prefix: &str, suffix: &str) -> [String; 4] {
    ["top", "right", "bottom", "left"].map(|side| format!("{prefix}{side}{suffix}"))
}

/// `name`'s value when it is a shorthand: `Some` of its value, or of
/// `None` where a longhand of it has none; `None` for a longhand or a
/// name this engine does not know.
pub(super) fn value(
    style: &ComputedStyle,
    name: &str,
    resolver: &Resolver<'_>,
) -> Option<Option<String>> {
    let get = |longhand: &str| super::resolved_value(style, longhand, resolver);
    let all = |names: &[String]| -> Option<Vec<String>> { names.iter().map(|n| get(n)).collect() };
    let box_sides = |prefix: &str, suffix: &str| -> Option<String> {
        let values = all(&sides(prefix, suffix))?;
        let values: [String; 4] = values.try_into().ok()?;
        Some(shortest_sides(&values))
    };
    // Two longhands written once where alike (`gap`, the `place-*` ones,
    // `overflow`).
    let pair = |a: &str, b: &str| -> Option<String> {
        let (a, b) = (get(a)?, get(b)?);
        Some(if a == b { a } else { format!("{a} {b}") })
    };
    let side_of = |side: &str| -> Option<String> {
        Some(format!(
            "{} {} {}",
            get(&format!("border-{side}-width"))?,
            get(&format!("border-{side}-style"))?,
            get(&format!("border-{side}-color"))?
        ))
    };
    Some(match name {
        "margin" => box_sides("margin-", ""),
        "padding" => box_sides("padding-", ""),
        "inset" => box_sides("", ""),
        "border-width" => box_sides("border-", "-width"),
        "border-style" => box_sides("border-", "-style"),
        "border-color" => box_sides("border-", "-color"),
        "border-top" => side_of("top"),
        "border-right" => side_of("right"),
        "border-bottom" => side_of("bottom"),
        "border-left" => side_of("left"),
        // One side's where the four are alike, the border image aside.
        "border" => {
            let top = side_of("top");
            let alike = ["right", "bottom", "left"]
                .iter()
                .all(|side| side_of(side) == top);
            if alike { top } else { Some(String::new()) }
        }
        "border-radius" => radius(style),
        "border-spacing" => Some(format!(
            "{} {}",
            get("-webkit-border-horizontal-spacing")?,
            get("-webkit-border-vertical-spacing")?
        )),
        "border-image" => border_image(style, resolver),
        "outline" => Some(format!(
            "{} {} {}",
            get("outline-color")?,
            get("outline-style")?,
            get("outline-width")?
        )),
        "gap" => pair("row-gap", "column-gap"),
        "place-content" => pair("align-content", "justify-content"),
        "place-items" => pair("align-items", "justify-items"),
        "place-self" => pair("align-self", "justify-self"),
        "overflow" => pair("overflow-x", "overflow-y"),
        "flex" => Some(format!(
            "{} {} {}",
            get("flex-grow")?,
            get("flex-shrink")?,
            get("flex-basis")?
        )),
        "flex-flow" => Some(format!("{} {}", get("flex-direction")?, get("flex-wrap")?)),
        "font" => font(style, &get),
        "font-variant" => get("font-variant-caps"),
        "text-wrap" => get("text-wrap-mode"),
        "list-style" => Some(format!(
            "{} {} {}",
            get("list-style-position")?,
            get("list-style-image")?,
            get("list-style-type")?
        )),
        // The thickness only where it is not `auto`, as the reference
        // browser writes it.
        "text-decoration" => {
            let thickness = get("text-decoration-thickness")?;
            let mut parts = vec![get("text-decoration-line")?];
            if thickness != "auto" {
                parts.push(thickness);
            }
            parts.push(get("text-decoration-style")?);
            parts.push(get("text-decoration-color")?);
            Some(parts.join(" "))
        }
        "background" => background(style, resolver),
        "background-position" => Some(longhands::layers(style, |layer| {
            let [x, y] = layer.position;
            format!(
                "{} {}",
                length_percentage(x).unwrap_or_default(),
                length_percentage(y).unwrap_or_default()
            )
        })),
        "page-break-before" | "page-break-after" => {
            let fragmentation = style.fragmentation();
            let value = if name == "page-break-before" {
                fragmentation.break_before
            } else {
                fragmentation.break_after
            };
            Some(
                match value {
                    BreakBetween::Auto => "auto",
                    BreakBetween::Page => "always",
                    BreakBetween::Left => "left",
                    BreakBetween::Right => "right",
                    BreakBetween::Avoid | BreakBetween::AvoidPage => "avoid",
                    _ => "",
                }
                .to_owned(),
            )
        }
        "page-break-inside" => Some(
            match style.fragmentation().break_inside {
                BreakInside::Auto => "auto",
                BreakInside::Avoid | BreakInside::AvoidPage => "avoid",
                _ => "",
            }
            .to_owned(),
        ),
        "grid-row" => Some(grid_pair(&style.grid().row_start, &style.grid().row_end)),
        "grid-column" => Some(grid_pair(
            &style.grid().column_start,
            &style.grid().column_end,
        )),
        "grid-area" => Some(grid_area(style)),
        "grid-template" => Some(grid_template(style)),
        "grid" => Some(grid(style)),
        "transition" => Some(transition(style)),
        "animation" => Some(animation(style)),
        _ => return None,
    })
}

/// `border-radius`: the horizontal radii at their shortest, then `/`
/// and the vertical ones where any differs.
fn radius(style: &ComputedStyle) -> Option<String> {
    let corners = &style.radii().0;
    let axis = |i: usize| -> Option<[String; 4]> {
        let values: Option<Vec<String>> = corners
            .iter()
            .map(|corner| length_percentage(&corner[i]))
            .collect();
        values?.try_into().ok()
    };
    let (horizontal, vertical) = (axis(0)?, axis(1)?);
    let text = shortest_sides(&horizontal);
    Some(if horizontal == vertical {
        text
    } else {
        format!("{text} / {}", shortest_sides(&vertical))
    })
}

/// `border-image`: `none` without a source, as the reference browser
/// writes it, else every part.
fn border_image(style: &ComputedStyle, resolver: &Resolver<'_>) -> Option<String> {
    if style.border_image().source.is_none() {
        return Some("none".to_owned());
    }
    let get = |longhand: &str| super::resolved_value(style, longhand, resolver);
    Some(format!(
        "{} {} / {} / {} {}",
        get("border-image-source")?,
        get("border-image-slice")?,
        get("border-image-width")?,
        get("border-image-outset")?,
        get("border-image-repeat")?
    ))
}

/// `font`, as the reference browser writes it: the style, the small
/// capitals, the weight and the width where they are not `normal`, the
/// size, a `/` and the line height where it is not `normal`, then the
/// family; the empty string for a width no keyword names.
fn font(style: &ComputedStyle, get: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let font = style.font();
    let mut parts: Vec<String> = Vec::new();
    if font.style != FontStyle::Normal {
        parts.push(get("font-style")?);
    }
    if font.small_caps {
        parts.push("small-caps".to_owned());
    }
    if font.weight != 400 {
        parts.push(font.weight.to_string());
    }
    let stretch = match font.stretch {
        50.0 => "ultra-condensed",
        62.5 => "extra-condensed",
        75.0 => "condensed",
        87.5 => "semi-condensed",
        100.0 => "normal",
        112.5 => "semi-expanded",
        125.0 => "expanded",
        150.0 => "extra-expanded",
        200.0 => "ultra-expanded",
        _ => return Some(String::new()),
    };
    if stretch != "normal" {
        parts.push(stretch.to_owned());
    }
    let size = get("font-size")?;
    parts.push(match font.line_height {
        LineHeight::Normal => size,
        _ => format!("{size} / {}", get("line-height")?),
    });
    parts.push(get("font-family")?);
    Some(parts.join(" "))
}

/// `background`: each layer's image, repeat, attachment, position, `/`,
/// size, origin and clip, the last layer's colour first, as the
/// reference browser writes it.
fn background(style: &ComputedStyle, resolver: &Resolver<'_>) -> Option<String> {
    let layers: Vec<_> = style.background().layers().collect();
    let images = super::resolved_value(style, "background-image", resolver)?;
    let images = split_commas(&images);
    let colour = super::resolved_value(style, "background-color", resolver)?;
    let mut out = Vec::new();
    for (i, layer) in layers.iter().enumerate() {
        let mut text = String::new();
        if i + 1 == layers.len() {
            text.push_str(&colour);
            text.push(' ');
        }
        let attachment = match layer.attachment {
            crate::computed::Attachment::Scroll => "scroll",
            crate::computed::Attachment::Fixed => "fixed",
            crate::computed::Attachment::Local => "local",
        };
        let [x, y] = layer.position;
        text.push_str(&format!(
            "{} {} {attachment} {} {} / {} {} {}",
            images.get(i).map_or("none", |s| s.as_str()),
            repeat(layer.repeat),
            length_percentage(x).unwrap_or_default(),
            length_percentage(y).unwrap_or_default(),
            background_size(layer.size),
            background_box(layer.origin),
            background_box(layer.clip),
        ));
        out.push(text);
    }
    Some(out.join(", "))
}

/// `text` split at its commas outside parentheses, each part trimmed.
fn split_commas(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    let mut quoted = None;
    for (i, c) in text.char_indices() {
        match (quoted, c) {
            (Some(q), c) if c == q => quoted = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quoted = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ',') if depth == 0 => {
                out.push(text[start..i].trim().to_owned());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(text[start..].trim().to_owned());
    out
}

/// What a grid placement shorthand sets a line it leaves out to, from
/// the line before it: the same name, else `auto` (css-grid-2 section
/// 8.4).
fn implied(line: &GridLine) -> GridLine {
    match line {
        GridLine::Ident(name) => GridLine::Ident(name.clone()),
        _ => GridLine::Auto,
    }
}

/// `grid-row` or `grid-column`: the end left out where it is what the
/// start implies.
fn grid_pair(start: &GridLine, end: &GridLine) -> String {
    if *end == implied(start) {
        grid_line(start)
    } else {
        format!("{} / {}", grid_line(start), grid_line(end))
    }
}

/// `grid-area`: its four lines, those at the end left out while each is
/// what the one it follows from implies.
fn grid_area(style: &ComputedStyle) -> String {
    let grid = style.grid();
    let lines = [
        &grid.row_start,
        &grid.column_start,
        &grid.row_end,
        &grid.column_end,
    ];
    let mut count = 4;
    if *lines[3] == implied(lines[1]) {
        count = 3;
        if *lines[2] == implied(lines[0]) {
            count = 2;
            if *lines[1] == implied(lines[0]) {
                count = 1;
            }
        }
    }
    lines[..count]
        .iter()
        .map(|line| grid_line(line))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// `grid-template`: `none`; the rows and the columns; or, with areas,
/// each row's line names, string and size, then the columns. The empty
/// string where the areas' rows cannot be written so: a row list with a
/// `repeat()`, or of another number of rows.
fn grid_template(style: &ComputedStyle) -> String {
    let grid = style.grid();
    let get = |name: &str| longhands::value(style, name, &|_| None).unwrap_or_default();
    let columns = get("grid-template-columns");
    let Some(areas) = &grid.template_areas else {
        if grid.template_rows.is_none() && grid.template_columns.is_none() {
            return "none".to_owned();
        }
        return format!("{} / {columns}", get("grid-template-rows"));
    };
    let strings = area_rows(areas);
    let rows: Vec<(&[std::sync::Arc<str>], Option<&TrackSize>)> = match &grid.template_rows {
        None => strings.iter().map(|_| (&[][..], None)).collect(),
        Some(list) => {
            let mut rows = Vec::new();
            for (i, section) in list.sections.iter().enumerate() {
                let crate::computed::TrackSection::Track(size) = section else {
                    return String::new();
                };
                rows.push((&list.names[i][..], Some(size)));
            }
            rows
        }
    };
    if rows.len() != strings.len() {
        return String::new();
    }
    let mut parts: Vec<String> = Vec::new();
    for (i, ((names, size), row)) in rows.iter().zip(&strings).enumerate() {
        // Between two rows the names of one line, after the first's
        // size and before the second's string.
        if i == 0 {
            parts.extend(line_names(names));
        } else if let Some(names) = line_names(names) {
            parts.push(names);
        }
        parts.push(string(row));
        if let Some(size) = size
            && **size != TrackSize::AUTO
        {
            parts.push(track_size(size));
        }
    }
    if let Some(list) = &grid.template_rows {
        parts.extend(list.names.last().and_then(|n| line_names(n)));
    }
    if grid.template_columns.is_some() {
        format!("{} / {columns}", parts.join(" "))
    } else {
        parts.join(" ")
    }
}

/// `grid`: `grid-template`'s value where the auto-placement properties
/// are their initial values; the `auto-flow` forms where the template
/// on the flow's side is `none`; else the empty string.
fn grid(style: &ComputedStyle) -> String {
    let grid = style.grid();
    let auto = |sizes: &[TrackSize]| matches!(sizes, [TrackSize::AUTO]);
    let initial_rows = auto(&grid.auto_rows);
    let initial_columns = auto(&grid.auto_columns);
    if grid.auto_flow == GridAutoFlow::default() && initial_rows && initial_columns {
        return grid_template(style);
    }
    let get = |name: &str| longhands::value(style, name, &|_| None).unwrap_or_default();
    let flow = |sizes: &[TrackSize], initial: bool| {
        let mut text = "auto-flow".to_owned();
        if grid.auto_flow.dense {
            text.push_str(" dense");
        }
        if !initial {
            text = format!("{text} {}", track_sizes(sizes));
        }
        text
    };
    if grid.template_areas.is_some() {
        return String::new();
    }
    if !grid.auto_flow.column && grid.template_rows.is_none() && initial_columns {
        format!(
            "{} / {}",
            flow(&grid.auto_rows, initial_rows),
            get("grid-template-columns")
        )
    } else if grid.auto_flow.column && grid.template_columns.is_none() && initial_rows {
        format!(
            "{} / {}",
            get("grid-template-rows"),
            flow(&grid.auto_columns, initial_columns)
        )
    } else {
        String::new()
    }
}

/// The item of a coordinated list at `i`, the list repeated.
fn at<T>(list: &[T], i: usize) -> &T {
    &list[i % list.len()]
}

/// Seconds, as `0s`.
fn seconds(s: f64) -> String {
    format!("{}s", number(s as f32))
}

/// `transition`: each transition's property, duration, easing and
/// delay, and `allow-discrete` where it says so.
fn transition(style: &ComputedStyle) -> String {
    let motion = style.motion();
    let properties: Vec<String> = if motion.transition_property.is_empty() {
        vec!["none".to_owned()]
    } else {
        motion
            .transition_property
            .iter()
            .map(|p| match p {
                crate::computed::TransitionProperty::All => "all".to_owned(),
                crate::computed::TransitionProperty::Name(name) => name.to_string(),
            })
            .collect()
    };
    properties
        .iter()
        .enumerate()
        .map(|(i, property)| {
            let mut text = format!(
                "{property} {} {} {}",
                seconds(*at(&motion.transition_duration, i)),
                easing(at(&motion.transition_timing_function, i)),
                seconds(*at(&motion.transition_delay, i)),
            );
            if *at(&motion.transition_behavior, i)
                == crate::computed::TransitionBehavior::AllowDiscrete
            {
                text.push_str(" allow-discrete");
            }
            text
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `animation`: each animation's duration, easing, delay, iteration
/// count, direction, fill mode, play state and name, as the reference
/// browser orders them.
fn animation(style: &ComputedStyle) -> String {
    let motion = style.motion();
    let get = |name: &str| longhands::value(style, name, &|_| None).unwrap_or_default();
    let lists: Vec<Vec<String>> = [
        "animation-iteration-count",
        "animation-direction",
        "animation-fill-mode",
        "animation-play-state",
    ]
    .iter()
    .map(|name| split_commas(&get(name)))
    .collect();
    motion
        .animation_name
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let mut parts = vec![
                seconds(*at(&motion.animation_duration, i)),
                easing(at(&motion.animation_timing_function, i)),
                seconds(*at(&motion.animation_delay, i)),
            ];
            parts.extend(lists.iter().map(|list| at(list, i).clone()));
            parts.push(
                name.as_ref()
                    .map_or_else(|| "none".to_owned(), |n| n.to_string()),
            );
            parts.join(" ")
        })
        .collect::<Vec<_>>()
        .join(", ")
}
