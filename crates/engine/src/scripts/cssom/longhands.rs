//! A longhand's value read from a shorthand a declaration block keeps
//! as written (CSSOM 6.7.2's getPropertyValue, G50): the borders',
//! `border-image`'s and `background`'s, each part as given, what the
//! shorthand leaves out its initial value, or, for `background`,
//! `initial`, as the reference browser writes them.

use css::Token;

use super::{Declared, css_wide};

/// `name`'s value in `block` where the last declaration that sets it is
/// a shorthand; `None` where none sets it or the last is `name` itself.
pub(super) fn from_shorthand(name: &str, block: &[Declared]) -> Option<String> {
    if let Some(longhands) = border_parts(name) {
        // A shorthand within `border`, from its longhands read so.
        let values: Option<Vec<String>> =
            longhands.iter().map(|l| from_shorthand(l, block)).collect();
        return compose(&values?);
    }
    let declared = block
        .iter()
        .rev()
        .find(|d| d.name == name || covers(&d.name, name))?;
    if declared.name == name {
        return None;
    }
    if let Some(keyword) = css_wide(&declared.value) {
        return Some(keyword);
    }
    // A value with a substitution is the shorthand's alone.
    if declared.value.contains("var(") {
        return Some(String::new());
    }
    let tokens = css::tokenize(&declared.value);
    expand(&declared.name, &tokens, name)
}

const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

/// `border-image`'s longhands and their initial values.
const BORDER_IMAGE: [(&str, &str); 5] = [
    ("border-image-source", "none"),
    ("border-image-slice", "100%"),
    ("border-image-width", "1"),
    ("border-image-outset", "0"),
    ("border-image-repeat", "stretch"),
];

/// The longhands of `name` where it is one of the shorthands `border`
/// sets, in their order.
fn border_parts(name: &str) -> Option<Vec<String>> {
    let parts = |suffix: &str| Some(SIDES.map(|s| format!("border-{s}-{suffix}")).to_vec());
    match name {
        "border-width" => parts("width"),
        "border-style" => parts("style"),
        "border-color" => parts("color"),
        "border-image" => Some(BORDER_IMAGE.map(|(l, _)| l.to_owned()).to_vec()),
        _ => {
            let side = name.strip_prefix("border-")?;
            SIDES.contains(&side).then(|| {
                ["width", "style", "color"]
                    .map(|p| format!("{name}-{p}"))
                    .to_vec()
            })
        }
    }
}

/// A shorthand of `border` from its longhands' `values`, as the reference
/// browser writes it: the sides at their fewest, a side's parts but
/// those at their initial values, `border-image`'s `none` alone where
/// every part is initial; `None` where a CSS-wide keyword is not in all.
fn compose(values: &[String]) -> Option<String> {
    if values.iter().any(|v| css_wide(v).is_some() || v.is_empty()) {
        return values
            .iter()
            .all(|v| *v == values[0])
            .then(|| values[0].clone());
    }
    Some(match values {
        [top, right, bottom, left] => {
            let fewest = if left != right {
                4
            } else if bottom != top {
                3
            } else if right != top {
                2
            } else {
                1
            };
            values[..fewest].join(" ")
        }
        [width, style, color] => {
            let parts: Vec<&str> = [(width, "medium"), (style, "none"), (color, "currentcolor")]
                .iter()
                .filter(|(v, initial)| v != initial)
                .map(|(v, _)| v.as_str())
                .collect();
            if parts.is_empty() {
                width.clone()
            } else {
                parts.join(" ")
            }
        }
        [source, slice, width, outset, repeat] => {
            if values
                .iter()
                .zip(BORDER_IMAGE)
                .all(|(v, (_, initial))| v == initial)
            {
                "none".to_owned()
            } else {
                format!("{source} {slice} / {width} / {outset} {repeat}")
            }
        }
        _ => return None,
    })
}

/// Whether the shorthand `shorthand` sets the longhand `longhand`.
fn covers(shorthand: &str, longhand: &str) -> bool {
    let side_part = |prefix: &str| {
        longhand
            .strip_prefix("border-")
            .and_then(|rest| rest.split_once('-'))
            .is_some_and(|(side, part)| SIDES.contains(&side) && part == prefix)
    };
    match shorthand {
        "border" => {
            ["width", "style", "color"].iter().any(|p| side_part(p))
                || BORDER_IMAGE.iter().any(|(l, _)| *l == longhand)
        }
        "border-width" => side_part("width"),
        "border-style" => side_part("style"),
        "border-color" => side_part("color"),
        "border-top" | "border-right" | "border-bottom" | "border-left" => longhand
            .strip_prefix(shorthand)
            .is_some_and(|rest| matches!(rest, "-width" | "-style" | "-color")),
        "border-image" => BORDER_IMAGE.iter().any(|(l, _)| *l == longhand),
        "background" => longhand.starts_with("background-"),
        "background-position" => {
            matches!(longhand, "background-position-x" | "background-position-y")
        }
        _ => false,
    }
}

/// The longhand `name`'s text from the shorthand `shorthand` declared
/// `tokens`.
fn expand(shorthand: &str, tokens: &[Token], name: &str) -> Option<String> {
    let parts = components(tokens);
    match shorthand {
        "border-width" | "border-style" | "border-color" => {
            let (side, _) = name.strip_prefix("border-")?.split_once('-')?;
            let side = SIDES.iter().position(|s| *s == side)?;
            Some(box_sides(&parts)?[side].clone())
        }
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
            if let Some((_, initial)) = BORDER_IMAGE.iter().find(|(l, _)| *l == name) {
                return Some((*initial).to_owned());
            }
            let [width, style, color] = border_side(&parts)?;
            Some(if name.ends_with("-width") {
                width.unwrap_or_else(|| "medium".to_owned())
            } else if name.ends_with("-style") {
                style.unwrap_or_else(|| "none".to_owned())
            } else {
                color.unwrap_or_else(|| "currentcolor".to_owned())
            })
        }
        "border-image" => {
            let i = BORDER_IMAGE.iter().position(|(l, _)| *l == name)?;
            let found = border_image(&parts)?;
            Some(
                found[i]
                    .clone()
                    .unwrap_or_else(|| BORDER_IMAGE[i].1.to_owned()),
            )
        }
        "background" => background(&parts, name),
        "background-position" => {
            let layers: Option<Vec<String>> = layers(&parts)
                .iter()
                .map(|layer| {
                    let [x, y] = position_axes(layer)?;
                    Some(if name.ends_with('x') { x } else { y })
                })
                .collect();
            Some(layers?.join(", "))
        }
        _ => None,
    }
}

/// The component values of `tokens`, the white space between them left
/// out: each a token, or a function or block up to its closer.
fn components(tokens: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if matches!(tokens[i], Token::Whitespace) {
            i += 1;
            continue;
        }
        let start = i;
        let mut depth = 0usize;
        loop {
            match tokens[i] {
                Token::Function(_) | Token::OpenParen | Token::OpenSquare | Token::OpenCurly => {
                    depth += 1
                }
                Token::CloseParen | Token::CloseSquare | Token::CloseCurly => {
                    depth = depth.saturating_sub(1)
                }
                _ => {}
            }
            i += 1;
            if depth == 0 || i == tokens.len() {
                break;
            }
        }
        out.push(&tokens[start..i]);
    }
    out
}

/// `parts` as text, a space between two, a URL as a `url()` of a string
/// as the reference browser writes it.
fn text(parts: &[&[Token]]) -> String {
    let quoted = |part: &[Token]| match part {
        [Token::Url { value, .. }] => style::serialize::tokens(&[
            Token::Function("url".to_owned()),
            Token::String {
                value: value.clone(),
                bad: false,
            },
            Token::CloseParen,
        ]),
        other => style::serialize::tokens(other),
    };
    parts
        .iter()
        .map(|p| quoted(p))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `parts` together are a valid value of the longhand `name`.
fn valid(name: &str, parts: &[&[Token]]) -> bool {
    let mut tokens = Vec::new();
    for part in parts {
        if !tokens.is_empty() {
            tokens.push(Token::Whitespace);
        }
        tokens.extend_from_slice(part);
    }
    !tokens.is_empty() && style::supports(name, &tokens)
}

/// Whether `part` is the keyword `word`, in any case.
fn is(part: &[Token], word: &str) -> bool {
    matches!(part, [Token::Ident(w)] if w.eq_ignore_ascii_case(word))
}

/// One to four values for the top, right, bottom and left sides.
fn box_sides(parts: &[&[Token]]) -> Option<[String; 4]> {
    let t: Vec<String> = parts.iter().map(|&p| text(&[p])).collect();
    Some(match t.as_slice() {
        [a] => [a.clone(), a.clone(), a.clone(), a.clone()],
        [a, b] => [a.clone(), b.clone(), a.clone(), b.clone()],
        [a, b, c] => [a.clone(), b.clone(), c.clone(), b.clone()],
        [a, b, c, d] => [a.clone(), b.clone(), c.clone(), d.clone()],
        _ => return None,
    })
}

/// A border side's width, style and colour, in any order, each at most
/// once.
fn border_side(parts: &[&[Token]]) -> Option<[Option<String>; 3]> {
    let mut found: [Option<String>; 3] = [None, None, None];
    for &part in parts {
        let slot = ["border-top-width", "border-top-style", "border-top-color"]
            .iter()
            .enumerate()
            .position(|(i, longhand)| found[i].is_none() && valid(longhand, &[part]))?;
        found[slot] = Some(text(&[part]));
    }
    Some(found)
}

/// `border-image`'s source, slice (its `fill` last), width, outset and
/// repeat, those given.
fn border_image(parts: &[&[Token]]) -> Option<[Option<String>; 5]> {
    let mut found: [Option<String>; 5] = Default::default();
    let slash = |p: &[Token]| matches!(p, [Token::Delim('/')]);
    let repeat = |p: &[Token]| {
        ["stretch", "repeat", "round", "space"]
            .iter()
            .any(|w| is(p, w))
    };
    let mut i = 0;
    while i < parts.len() {
        let part = parts[i];
        if found[0].is_none() && !slash(part) && valid("border-image-source", &[part]) {
            found[0] = Some(text(&[part]));
            i += 1;
        } else if found[4].is_none() && repeat(part) {
            let n = parts[i..].iter().take(2).take_while(|p| repeat(p)).count();
            found[4] = Some(text(&parts[i..i + n]));
            i += n;
        } else if found[1].is_none() && (is(part, "fill") || valid("border-image-slice", &[part])) {
            let n = parts[i..]
                .iter()
                .take_while(|&&p| is(p, "fill") || (!slash(p) && valid("border-image-slice", &[p])))
                .count();
            let (fill, sides): (Vec<&[Token]>, Vec<&[Token]>) =
                parts[i..i + n].iter().partition(|p| is(p, "fill"));
            let mut slice = text(&sides);
            if !fill.is_empty() {
                slice.push_str(" fill");
            }
            found[1] = Some(slice);
            i += n;
            // `/ width`, `/ width / outset` or `/ / outset`.
            for slot in [2, 3] {
                if !parts.get(i).is_some_and(|p| slash(p)) {
                    break;
                }
                i += 1;
                let longhand = BORDER_IMAGE[slot].0;
                let n = parts[i..]
                    .iter()
                    .take_while(|&&p| !slash(p) && valid(longhand, &[p]))
                    .count();
                if n > 0 {
                    found[slot] = Some(text(&parts[i..i + n]));
                    i += n;
                }
            }
        } else {
            return None;
        }
    }
    Some(found)
}

/// The layers of a comma-separated list, each its components.
fn layers<'a>(parts: &[&'a [Token]]) -> Vec<Vec<&'a [Token]>> {
    let mut out: Vec<Vec<&[Token]>> = vec![Vec::new()];
    for part in parts {
        if matches!(part, [Token::Comma]) {
            out.push(Vec::new());
        } else {
            out.last_mut().expect("one at least").push(part);
        }
    }
    out
}

/// A position's horizontal and vertical parts.
fn position_axes(parts: &[&[Token]]) -> Option<[String; 2]> {
    let vertical = |p: &[Token]| is(p, "top") || is(p, "bottom");
    let horizontal = |p: &[Token]| is(p, "left") || is(p, "right");
    let keyword = |p: &[Token]| vertical(p) || horizontal(p) || is(p, "center");
    Some(match parts {
        [a] if vertical(a) => ["center".to_owned(), text(&[a])],
        [a] => [text(&[a]), "center".to_owned()],
        [a, b] if vertical(a) || horizontal(b) => [text(&[b]), text(&[a])],
        [a, b] => [text(&[a]), text(&[b])],
        _ => {
            // Keywords each with an offset after it, or not.
            let split = (1..parts.len()).find(|&i| keyword(parts[i]))?;
            let (first, second) = parts.split_at(split);
            if vertical(first[0]) || horizontal(second[0]) {
                [text(second), text(first)]
            } else {
                [text(first), text(second)]
            }
        }
    })
}

/// A longhand of `background` (section 3.10) from its layers; what a
/// layer leaves out `initial`, as the reference browser writes it.
fn background(parts: &[&[Token]], name: &str) -> Option<String> {
    let layers = layers(parts);
    let last = layers.len() - 1;
    let mut values: Vec<String> = Vec::new();
    let mut colour = None;
    for (index, layer) in layers.iter().enumerate() {
        let mut image = None;
        let mut position: Option<&[&[Token]]> = None;
        let mut size = None;
        let mut repeat = None;
        let mut attachment = None;
        let mut boxes: Vec<String> = Vec::new();
        let mut at = 0;
        while at < layer.len() {
            let part = layer[at];
            let fits =
                |n: &usize, longhand| at + n <= layer.len() && valid(longhand, &layer[at..at + n]);
            if image.is_none() && valid("background-image", &[part]) {
                image = Some(text(&[part]));
                at += 1;
            } else if position.is_none()
                && let Some(n) = (1..=4).rev().find(|n| fits(n, "background-position"))
            {
                position = Some(&layer[at..at + n]);
                at += n;
                if matches!(layer.get(at), Some([Token::Delim('/')])) {
                    let rest = &layer[at + 1..];
                    let n = (1..=2.min(rest.len()))
                        .rev()
                        .find(|&n| valid("background-size", &rest[..n]))?;
                    size = Some(text(&rest[..n]));
                    at += 1 + n;
                }
            } else if repeat.is_none()
                && let Some(n) = (1..=2).rev().find(|n| fits(n, "background-repeat"))
            {
                repeat = Some(text(&layer[at..at + n]));
                at += n;
            } else if attachment.is_none() && valid("background-attachment", &[part]) {
                attachment = Some(text(&[part]));
                at += 1;
            } else if boxes.len() < 2 && valid("background-origin", &[part]) {
                boxes.push(text(&[part]));
                at += 1;
            } else if index == last && colour.is_none() && valid("background-color", &[part]) {
                colour = Some(text(&[part]));
                at += 1;
            } else {
                return None;
            }
        }
        let axes = match position {
            Some(parts) => Some(position_axes(parts)?),
            None => None,
        };
        let value = match name {
            "background-image" => image,
            "background-position" => axes.map(|[x, y]| format!("{x} {y}")),
            "background-position-x" => axes.map(|[x, _]| x),
            "background-position-y" => axes.map(|[_, y]| y),
            "background-size" => size,
            "background-repeat" => repeat,
            "background-attachment" => attachment,
            "background-origin" => boxes.first().cloned(),
            "background-clip" => boxes.last().cloned(),
            "background-color" => continue,
            _ => return None,
        };
        values.push(value.unwrap_or_else(|| "initial".to_owned()));
    }
    if name == "background-color" {
        return Some(colour.unwrap_or_else(|| "initial".to_owned()));
    }
    Some(values.join(", "))
}

#[cfg(test)]
mod tests {
    use super::super::declarations;
    use super::*;

    /// `name`'s value read from the block `text`.
    fn read(text: &str, name: &str) -> Option<String> {
        from_shorthand(name, &declarations(text))
    }

    #[test]
    fn a_longhand_is_read_from_its_shorthand_as_written() {
        let width = |side: &str| {
            read(
                "border-width: thick thin 1px",
                &format!("border-{side}-width"),
            )
        };
        assert_eq!(width("top").as_deref(), Some("thick"));
        assert_eq!(width("right").as_deref(), Some("thin"));
        assert_eq!(width("bottom").as_deref(), Some("1px"));
        assert_eq!(width("left").as_deref(), Some("thin"));
        assert_eq!(
            read("border-color: currentcolor", "border-left-color").as_deref(),
            Some("currentcolor")
        );
        assert_eq!(
            read("border-color: red yellow", "border-right-color").as_deref(),
            Some("yellow")
        );
        assert_eq!(
            read("border: 5px dotted blue", "border-left-style").as_deref(),
            Some("dotted")
        );
        assert_eq!(
            read("border: 5px dotted blue", "border-image-slice").as_deref(),
            Some("100%")
        );
        // What a side leaves out is its initial value.
        assert_eq!(
            read("border-top: thin", "border-top-color").as_deref(),
            Some("currentcolor")
        );
        assert_eq!(
            read("border-right: double", "border-right-width").as_deref(),
            Some("medium")
        );
        assert_eq!(read("border-right: double", "border-top-width"), None);
        // The last declaration wins; a longhand's own is not read here.
        assert_eq!(
            read(
                "border-top-width: 1px; border: 2px solid",
                "border-top-width"
            )
            .as_deref(),
            Some("2px")
        );
        assert_eq!(
            read("border: 2px; border-top-width: 1px", "border-top-width"),
            None
        );
        assert_eq!(
            read("border: inherit", "border-top-width").as_deref(),
            Some("inherit")
        );
    }

    #[test]
    fn a_shorthand_within_border_is_read_from_it() {
        let block = "border: 1px solid black";
        assert_eq!(read(block, "border-width").as_deref(), Some("1px"));
        assert_eq!(read(block, "border-color").as_deref(), Some("black"));
        assert_eq!(
            read(block, "border-left").as_deref(),
            Some("1px solid black")
        );
        assert_eq!(read(block, "border-image").as_deref(), Some("none"));
        assert_eq!(read("border: thin", "border-top").as_deref(), Some("thin"));
        assert_eq!(
            read("border: 1px; border-right: 2px dotted", "border-width").as_deref(),
            Some("1px 2px 1px 1px")
        );
        assert_eq!(
            read("border-image: 1 / / 2", "border-image").as_deref(),
            Some("none 1 / 1 / 2 stretch")
        );
        assert_eq!(
            read("border: inherit", "border-style").as_deref(),
            Some("inherit")
        );
        // Not every side from a shorthand: not said here.
        assert_eq!(read("border-top-width: 1px", "border-width"), None);
    }

    #[test]
    fn border_image_and_background_are_read_part_by_part() {
        let image = |value: &str, name: &str| read(&format!("border-image: {value}"), name);
        assert_eq!(
            image("url(\"a.png\") fill 1 2% 3 4%", "border-image-slice").as_deref(),
            Some("1 2% 3 4% fill")
        );
        assert_eq!(
            image("url(\"a.png\") fill 1 2% 3 4%", "border-image-source").as_deref(),
            Some("url(\"a.png\")")
        );
        assert_eq!(
            image("1 2% 3 4% / 5% / 6", "border-image-width").as_deref(),
            Some("5%")
        );
        assert_eq!(
            image("1 2% 3 4% / 5% / 6", "border-image-outset").as_deref(),
            Some("6")
        );
        assert_eq!(
            image("repeat round", "border-image-repeat").as_deref(),
            Some("repeat round")
        );
        assert_eq!(image("none", "border-image-width").as_deref(), Some("1"));
        let layers = "url(\"a.png\") 1px 2px / 3px 4px space round local padding-box content-box, \
                      rgb(5, 6, 7) none left 10px top";
        let background = |name: &str| read(&format!("background: {layers}"), name);
        assert_eq!(
            background("background-image").as_deref(),
            Some("url(\"a.png\"), none")
        );
        assert_eq!(
            background("background-position").as_deref(),
            Some("1px 2px, left 10px top")
        );
        assert_eq!(
            background("background-position-x").as_deref(),
            Some("1px, left 10px")
        );
        assert_eq!(
            background("background-size").as_deref(),
            Some("3px 4px, initial")
        );
        assert_eq!(
            background("background-repeat").as_deref(),
            Some("space round, initial")
        );
        assert_eq!(
            background("background-clip").as_deref(),
            Some("content-box, initial")
        );
        assert_eq!(
            background("background-color").as_deref(),
            Some("rgb(5, 6, 7)")
        );
        // What the shorthand leaves out is `initial`.
        assert_eq!(
            read("background: none", "background-attachment").as_deref(),
            Some("initial")
        );
        assert_eq!(
            read("background: border-box content-box", "background-origin").as_deref(),
            Some("border-box")
        );
    }
}
