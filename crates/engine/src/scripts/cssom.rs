//! What a script's `style` object reads and writes (M25.2,
//! `docs/design/dom-scripting.md`): the `style` attribute's declarations
//! with their values' text as written, each checked as the cascade would
//! take it; the bindings write the block back. CSSOM's serialization of
//! specified values is G50's: a value is kept as it was given, trimmed.
//! The font's are written as the reference browser writes them, and the
//! `font` and `font-variant` shorthands are kept as their longhands and
//! written back from them (gap batch D).

use css::Token;

mod longhands;

/// One declaration of a declaration block, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Declared {
    /// The property's name, ASCII-lowercased, but a custom property's.
    pub(crate) name: String,
    /// The value's text, trimmed, without `!important`.
    pub(crate) value: String,
    pub(crate) important: bool,
}

/// The declarations of `text`, a declaration block's body (a `style`
/// attribute's, or `cssText`), that the cascade would take: those whose
/// value is valid for a property it knows. A property declared again
/// takes the later's value, in the later's place; a `font` or
/// `font-variant` is its longhands.
pub(crate) fn declarations(text: &str) -> Vec<Declared> {
    let text = without_comments(text);
    let mut found: Vec<Declared> = Vec::new();
    for piece in split_top_level(&text, ';') {
        let Some((name, value)) = split_once_top_level(piece, ':') else {
            continue;
        };
        let name = name.trim_matches(is_space);
        if name.is_empty() || name.contains(is_space) {
            continue;
        }
        let name = canonical(name);
        let (value, important) = strip_important(value.trim_matches(is_space));
        if !valid(&name, value) {
            continue;
        }
        let longhands =
            expand(&name, value).unwrap_or_else(|| vec![(name.clone(), written(&name, value))]);
        for (name, value) in longhands {
            found.retain(|declared| declared.name != name);
            found.push(Declared {
                name,
                value,
                important,
            });
        }
    }
    found
}

/// The descriptors of an `@page` rule's block (CSS Paged Media 3
/// section 7) that the reference browser takes: the margins, as the
/// properties take them, `size` and `page-orientation`; others left out.
pub(crate) fn page_descriptors(text: &str) -> Vec<Declared> {
    let text = without_comments(text);
    let mut found: Vec<Declared> = Vec::new();
    for piece in split_top_level(&text, ';') {
        let Some((name, value)) = split_once_top_level(piece, ':') else {
            continue;
        };
        let name = name.trim_matches(is_space).to_ascii_lowercase();
        let (value, important) = strip_important(value.trim_matches(is_space));
        let longhands = match name.as_str() {
            "margin" | "margin-top" | "margin-right" | "margin-bottom" | "margin-left"
                if valid(&name, value) =>
            {
                expand(&name, value).unwrap_or_else(|| vec![(name.clone(), written(&name, value))])
            }
            "size" | "page-orientation" if page_value(&name, value) => {
                let value = css_wide(value).unwrap_or_else(|| {
                    style::serialize::specified(&css::tokenize(value)).to_ascii_lowercase()
                });
                vec![(name.clone(), value)]
            }
            _ => continue,
        };
        for (name, value) in longhands {
            found.retain(|declared| declared.name != name);
            found.push(Declared {
                name,
                value,
                important,
            });
        }
    }
    found
}

/// Whether `value` is valid for the page descriptor `name`: `size`'s
/// `auto`, one or two lengths, or a page size and an orientation in
/// either order; `page-orientation`'s three keywords.
fn page_value(name: &str, value: &str) -> bool {
    if css_wide(value).is_some() {
        return true;
    }
    let parts = components(value);
    let words: Vec<String> = parts.iter().map(|p| p.to_ascii_lowercase()).collect();
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    if name == "page-orientation" {
        return matches!(words[..], ["upright" | "rotate-left" | "rotate-right"]);
    }
    let size = |w: &str| {
        matches!(
            w,
            "a5" | "a4" | "a3" | "b5" | "b4" | "jis-b5" | "jis-b4" | "letter" | "legal" | "ledger"
        )
    };
    let orientation = |w: &str| matches!(w, "portrait" | "landscape");
    let length = |w: &str| w != "auto" && valid("width", w) && !w.ends_with('%');
    match words[..] {
        ["auto"] => true,
        [one] => size(one) || orientation(one) || length(one),
        [a, b] => {
            (size(a) && orientation(b)) || (orientation(a) && size(b)) || (length(a) && length(b))
        }
        _ => false,
    }
}

/// The longhands `font` sets, in the order the reference browser lists
/// them.
const FONT_LONGHANDS: [&str; 15] = [
    "font-style",
    "font-variant-ligatures",
    "font-variant-caps",
    "font-variant-alternates",
    "font-variant-numeric",
    "font-variant-east-asian",
    "font-variant-position",
    "font-variant-emoji",
    "font-weight",
    "font-stretch",
    "font-size",
    "line-height",
    "font-family",
    "font-kerning",
    "font-feature-settings",
];

/// The CSS-wide keyword `value` is, lowercase.
fn css_wide(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer"
    )
    .then_some(lower)
}

/// The four sides' longhands of a box shorthand.
const fn sides(
    top: &'static str,
    right: &'static str,
    bottom: &'static str,
    left: &'static str,
) -> [&'static str; 4] {
    [top, right, bottom, left]
}

const MARGIN: [&str; 4] = sides("margin-top", "margin-right", "margin-bottom", "margin-left");
const PADDING: [&str; 4] = sides(
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
);
const INSET: [&str; 4] = sides("top", "right", "bottom", "left");
const BORDER_WIDTH: [&str; 4] = sides(
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
);
const BORDER_STYLE: [&str; 4] = sides(
    "border-top-style",
    "border-right-style",
    "border-bottom-style",
    "border-left-style",
);
const BORDER_COLOR: [&str; 4] = sides(
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
);
const BORDER_TOP: [&str; 3] = ["border-top-width", "border-top-style", "border-top-color"];
const BORDER_RIGHT: [&str; 3] = [
    "border-right-width",
    "border-right-style",
    "border-right-color",
];
const BORDER_BOTTOM: [&str; 3] = [
    "border-bottom-width",
    "border-bottom-style",
    "border-bottom-color",
];
const BORDER_LEFT: [&str; 3] = [
    "border-left-width",
    "border-left-style",
    "border-left-color",
];
const BORDER_IMAGE: [&str; 5] = [
    "border-image-source",
    "border-image-slice",
    "border-image-width",
    "border-image-outset",
    "border-image-repeat",
];
/// `border-image`'s longhands' initial values, which `border` sets.
const BORDER_IMAGE_INITIAL: [&str; 5] = ["none", "100%", "1", "0", "stretch"];
const BORDER: [&str; 17] = [
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "border-top-style",
    "border-right-style",
    "border-bottom-style",
    "border-left-style",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "border-image-source",
    "border-image-slice",
    "border-image-width",
    "border-image-outset",
    "border-image-repeat",
];
const OUTLINE: [&str; 3] = ["outline-color", "outline-style", "outline-width"];
const LIST_STYLE: [&str; 3] = ["list-style-position", "list-style-image", "list-style-type"];

/// The shorthands a declaration block keeps as their longhands (G50),
/// each with its longhands in their canonical order.
const SHORTHANDS: [(&str, &[&str]); 18] = [
    ("font", &FONT_LONGHANDS),
    ("font-variant", &style::VARIANT_LONGHANDS),
    ("margin", &MARGIN),
    ("padding", &PADDING),
    ("inset", &INSET),
    ("border", &BORDER),
    ("border-width", &BORDER_WIDTH),
    ("border-style", &BORDER_STYLE),
    ("border-color", &BORDER_COLOR),
    ("border-top", &BORDER_TOP),
    ("border-right", &BORDER_RIGHT),
    ("border-bottom", &BORDER_BOTTOM),
    ("border-left", &BORDER_LEFT),
    ("border-image", &BORDER_IMAGE),
    ("outline", &OUTLINE),
    ("list-style", &LIST_STYLE),
    ("gap", &["row-gap", "column-gap"]),
    ("overflow", &["overflow-x", "overflow-y"]),
];

/// The longhands of the shorthand `name`, if it is one a block keeps as
/// them.
pub(crate) fn longhands(name: &str) -> Option<&'static [&'static str]> {
    static ALL: std::sync::LazyLock<Vec<&'static str>> =
        std::sync::LazyLock::new(|| style::all_longhands().collect());
    if name == "all" {
        return Some(ALL.as_slice());
    }
    SHORTHANDS
        .iter()
        .find(|(shorthand, _)| *shorthand == name)
        .map(|(_, longhands)| *longhands)
}

/// The pieces of `value` parted by white space outside brackets and
/// strings.
fn components(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut piece = String::new();
    let mut scan = Scanner::default();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if scan.top_level(c, chars.peek().copied()) && is_space(c) {
            if !piece.is_empty() {
                out.push(std::mem::take(&mut piece));
            }
            continue;
        }
        piece.push(c);
    }
    if !piece.is_empty() {
        out.push(piece);
    }
    out
}

/// One to four values given to the four sides (CSS Backgrounds 3's box
/// shorthands): top, right, bottom, left.
fn four_sides(value: &str) -> Option<[String; 4]> {
    let v = components(value);
    Some(match v.as_slice() {
        [a] => [a.clone(), a.clone(), a.clone(), a.clone()],
        [a, b] => [a.clone(), b.clone(), a.clone(), b.clone()],
        [a, b, c] => [a.clone(), b.clone(), c.clone(), b.clone()],
        [a, b, c, d] => [a.clone(), b.clone(), c.clone(), d.clone()],
        _ => return None,
    })
}

/// The components of `value` each given to the first of `slots` (each a
/// longhand and its initial value) it is valid for, at most one each;
/// the slots none was given keep their initial values. `None` when a
/// component fits no slot left.
fn by_validity(value: &str, slots: &[(&str, &str)]) -> Option<Vec<String>> {
    let mut given: Vec<Option<String>> = vec![None; slots.len()];
    for piece in components(value) {
        let slot = (0..slots.len()).find(|&i| given[i].is_none() && valid(slots[i].0, &piece))?;
        given[slot] = Some(piece);
    }
    Some(
        given
            .into_iter()
            .zip(slots)
            .map(|(v, (_, initial))| v.unwrap_or_else(|| (*initial).to_owned()))
            .collect(),
    )
}

/// A border side's width, style and color from `value`.
fn border_side(value: &str) -> Option<Vec<String>> {
    let given = by_validity(
        value,
        &[
            ("border-top-style", "none"),
            ("border-top-width", "medium"),
            ("border-top-color", "currentcolor"),
        ],
    )?;
    Some(vec![given[1].clone(), given[0].clone(), given[2].clone()])
}

/// The longhands of the shorthand `name` declared `value`, each with
/// its value's text; `None` for a property kept as it is (a longhand, a
/// value with a substitution, a system font).
fn expand(name: &str, value: &str) -> Option<Vec<(String, String)>> {
    let longhands = longhands(name)?;
    if let Some(keyword) = css_wide(value) {
        return Some(
            longhands
                .iter()
                .map(|l| ((*l).to_owned(), keyword.clone()))
                .collect(),
        );
    }
    if ["var(", "attr(", "env("].iter().any(|f| value.contains(f)) {
        return None;
    }
    let tokens = css::tokenize(value);
    let pair = |values: Vec<String>| -> Option<Vec<(String, String)>> {
        Some(
            longhands
                .iter()
                .zip(values)
                .map(|(l, v)| ((*l).to_owned(), written(l, &v)))
                .collect(),
        )
    };
    match name {
        "margin" | "padding" | "inset" | "border-width" | "border-style" | "border-color" => {
            return pair(four_sides(value)?.to_vec());
        }
        "overflow" => {
            let mut parts = components(value);
            if parts.len() == 1 {
                parts.push(parts[0].clone());
            }
            return pair(parts);
        }
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            return pair(border_side(value)?);
        }
        "border" => {
            let side = border_side(value)?;
            let mut values = Vec::new();
            for part in &side {
                values.extend(std::iter::repeat_n(part.clone(), 4));
            }
            values.extend(BORDER_IMAGE_INITIAL.iter().map(|v| (*v).to_owned()));
            return pair(values);
        }
        "outline" => {
            return pair(by_validity(
                value,
                &[
                    ("outline-color", "currentcolor"),
                    ("outline-style", "none"),
                    ("outline-width", "medium"),
                ],
            )?);
        }
        "list-style" => {
            // `none` is the image's or the type's, whichever is not
            // given otherwise.
            let parts = components(value);
            let nones = parts
                .iter()
                .filter(|p| p.eq_ignore_ascii_case("none"))
                .count();
            let rest: Vec<String> = parts
                .iter()
                .filter(|p| !p.eq_ignore_ascii_case("none"))
                .cloned()
                .collect();
            let mut given = by_validity(
                &rest.join(" "),
                &[
                    ("list-style-position", "outside"),
                    ("list-style-image", ""),
                    ("list-style-type", ""),
                ],
            )?;
            let open = [1, 2].iter().filter(|&&i| given[i].is_empty()).count();
            if nones > open {
                return None;
            }
            if nones > 0 {
                for i in [1, 2] {
                    if given[i].is_empty() {
                        given[i] = "none".to_owned();
                    }
                }
            }
            if given[1].is_empty() {
                given[1] = "none".to_owned();
            }
            if given[2].is_empty() {
                given[2] = "disc".to_owned();
            }
            return pair(given);
        }
        "gap" => {
            let v = components(value);
            return match v.as_slice() {
                [a] => pair(vec![a.clone(), a.clone()]),
                [a, b] => pair(vec![a.clone(), b.clone()]),
                _ => None,
            };
        }
        // Its initial values alone, as `border` writes it.
        "border-image" if value.eq_ignore_ascii_case("none") => {
            return pair(
                BORDER_IMAGE_INITIAL
                    .iter()
                    .map(|v| (*v).to_owned())
                    .collect(),
            );
        }
        "font" | "font-variant" => {}
        _ => return None,
    }
    if name == "font" {
        let list = style::font_longhands(&tokens)?;
        return Some(
            list.into_iter()
                .map(|(l, v)| (l.to_owned(), written(l, &v)))
                .collect(),
        );
    }
    // `normal`, `none`, or each keyword its longhand's.
    let words: Vec<String> = value
        .split_ascii_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    let mut out: Vec<(String, String)> = longhands
        .iter()
        .map(|l| ((*l).to_owned(), "normal".to_owned()))
        .collect();
    if words == ["none"] {
        out[0].1 = "none".into();
        return Some(out);
    }
    if words == ["normal"] {
        return Some(out);
    }
    for (longhand, text) in out.iter_mut() {
        let mine: Vec<&str> = words
            .iter()
            .filter(|w| {
                style::parse_variant(longhand, &css::tokenize(w)).is_some_and(|v| v != "normal")
            })
            .map(String::as_str)
            .collect();
        if !mine.is_empty() {
            *text = style::parse_variant(longhand, &css::tokenize(&mine.join(" ")))?;
        }
    }
    Some(out)
}

/// A longhand's value as a declaration block keeps it: the font's as
/// the reference browser writes them, a custom property's or one with a
/// substitution as given, the rest in their canonical forms (numbers at
/// their shortest, URLs and strings quoted, G50).
fn written(name: &str, value: &str) -> String {
    if let Some(keyword) = css_wide(value) {
        return keyword;
    }
    if name.starts_with("--") || ["var(", "attr(", "env("].iter().any(|f| value.contains(f)) {
        return value.to_owned();
    }
    let tokens = css::tokenize(value);
    let text = match name {
        "font-family" => style::font_family::parse_list(&tokens)
            .map(|list| style::font_family::write_list(&list)),
        "font-variant-ligatures"
        | "font-variant-caps"
        | "font-variant-alternates"
        | "font-variant-numeric"
        | "font-variant-east-asian"
        | "font-variant-position"
        | "font-variant-emoji" => style::parse_variant(name, &tokens),
        "font-stretch" | "font-weight" | "font-style" => {
            Some(style::serialize::tokens(&tokens).to_ascii_lowercase())
        }
        // Its shortest form (css-display-3 section 2, D77): `block flow`
        // is `block`, `list-item inline` `inline list-item`.
        "display" => style::cascade::parse_display_value(&tokens)
            .map(|(display, flow_root)| style::serialize::display_value(display, flow_root)),
        // A colour's keyword, named, a system colour's or `currentcolor`,
        // in lowercase (CSSOM section 6.7.2).
        _ if takes_color(name)
            && matches!(
                tokens
                    .iter()
                    .filter(|t| !matches!(t, Token::Whitespace))
                    .collect::<Vec<_>>()[..],
                [Token::Ident(_)]
            )
            && (style::parse_color_text(value).is_some()
                || value
                    .trim_matches(is_space)
                    .eq_ignore_ascii_case("currentcolor")) =>
        {
            Some(value.trim_matches(is_space).to_ascii_lowercase())
        }
        _ => None,
    };
    text.unwrap_or_else(|| style::serialize::specified_of(name, &tokens))
}

/// Whether `name` is a property whose value may be a colour alone.
fn takes_color(name: &str) -> bool {
    name.ends_with("color")
        || matches!(
            name,
            "fill" | "stroke" | "background" | "outline" | "text-decoration" | "column-rule"
        )
        || (name.starts_with("border") && !name.ends_with("radius") && !name.contains("image"))
}

/// The value of the shorthand `name` from `block`'s longhands, as CSSOM
/// serializes one (section 6.7.2): the empty string unless all its
/// longhands are there with one priority and it can say their values.
/// A longhand `block` does not declare is read from the shorthand that
/// sets it last, where one does.
pub(crate) fn shorthand_value(name: &str, block: &[Declared]) -> String {
    let Some(longhands) = longhands(name) else {
        // A shorthand not kept as longhands, or a longhand, read from the
        // shorthand that sets it as written.
        return longhands::from_shorthand(name, block).unwrap_or_default();
    };
    let found: Option<Vec<&Declared>> = longhands
        .iter()
        .map(|l| block.iter().find(|d| d.name == *l))
        .collect();
    let Some(found) = found else {
        return String::new();
    };
    if found.iter().any(|d| d.important != found[0].important) {
        return String::new();
    }
    let values: Vec<&str> = found.iter().map(|d| d.value.as_str()).collect();
    if values
        .iter()
        .any(|v| css_wide(v).is_some() || v.contains("var("))
    {
        return if values.iter().all(|v| *v == values[0]) && css_wide(values[0]).is_some() {
            values[0].to_owned()
        } else {
            String::new()
        };
    }
    match name {
        "font-variant" => style::write_font_variant(&values).unwrap_or_default(),
        "font" => font_value(&values).unwrap_or_default(),
        "border" => {
            // Its sides alike and its image as `border` sets it.
            let side = |i: usize| [values[i], values[i + 4], values[i + 8]];
            if (1..4).any(|i| side(i) != side(0)) || values[12..] != BORDER_IMAGE_INITIAL {
                return String::new();
            }
            initials_left_out(&side(0), &["medium", "none", "currentcolor"], "none")
        }
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            initials_left_out(&values, &["medium", "none", "currentcolor"], "none")
        }
        "border-image" => {
            if values == BORDER_IMAGE_INITIAL {
                "none".to_owned()
            } else {
                String::new()
            }
        }
        "outline" => initials_left_out(&values, &["currentcolor", "none", "medium"], "none"),
        "list-style" => initials_left_out(&values, &["outside", "none", "disc"], "outside"),
        "gap" | "overflow" => {
            if values[0] == values[1] {
                values[0].to_owned()
            } else {
                values.join(" ")
            }
        }
        // The box shorthands: at their shortest.
        _ => shortest_sides(&values),
    }
}

/// Values given to the four sides, at their shortest: the left left out
/// when it is the right, then the bottom when it is the top, then the
/// right when it is the top.
fn shortest_sides(values: &[&str]) -> String {
    let [top, right, bottom, left] = values else {
        return String::new();
    };
    let mut out = vec![*top, *right, *bottom, *left];
    if left == right {
        out.pop();
        if bottom == top {
            out.pop();
            if right == top {
                out.pop();
            }
        }
    }
    out.join(" ")
}

/// A shorthand's longhands' values with those at their `initials` left
/// out; `alone` when all are.
fn initials_left_out(values: &[&str], initials: &[&str], alone: &str) -> String {
    let kept: Vec<&str> = values
        .iter()
        .zip(initials)
        .filter(|(v, initial)| !v.eq_ignore_ascii_case(initial))
        .map(|(v, _)| *v)
        .collect();
    if kept.is_empty() {
        alone.to_owned()
    } else {
        kept.join(" ")
    }
}

/// `font` from its longhands' values in [`FONT_LONGHANDS`]' order, as
/// the reference browser writes it; `None` when one has a value it
/// cannot say.
fn font_value(values: &[&str]) -> Option<String> {
    let [
        style,
        ligatures,
        caps,
        alternates,
        numeric,
        east_asian,
        position,
        emoji,
        weight,
        stretch,
        size,
        line_height,
        family,
        kerning,
        features,
    ] = values
    else {
        return None;
    };
    let others = [ligatures, alternates, numeric, east_asian, position, emoji];
    if *features != "normal"
        || *kerning != "auto"
        || others.iter().any(|v| **v != "normal")
        || !matches!(*caps, "normal" | "small-caps")
    {
        return None;
    }
    let stretch = if stretch.bytes().all(|b| b.is_ascii_lowercase() || b == b'-') {
        *stretch
    } else {
        // A percentage, or a calculation of one, that a keyword names.
        let inner = stretch
            .strip_prefix("calc(")
            .and_then(|s| s.strip_suffix(')'))
            .unwrap_or(stretch);
        let n: f32 = inner.trim().strip_suffix('%')?.parse().ok()?;
        style::stretch_name(n)?
    };
    let mut parts = Vec::new();
    for (value, initial) in [(*style, "normal"), (*caps, "normal"), (*weight, "normal")] {
        if value != initial {
            parts.push(value.to_owned());
        }
    }
    if stretch != "normal" {
        parts.push(stretch.to_owned());
    }
    parts.push(if *line_height == "normal" {
        (*size).to_owned()
    } else {
        format!("{size}/{line_height}")
    });
    parts.push((*family).to_owned());
    Some(parts.join(" "))
}

/// The declarations of `flat`, name, value and priority three in a row,
/// as `ParseDeclarations` gives them.
pub(crate) fn from_flat(flat: &[String]) -> Vec<Declared> {
    flat.as_chunks::<3>()
        .0
        .iter()
        .map(|d| Declared {
            name: d[0].clone(),
            value: d[1].clone(),
            important: !d[2].is_empty(),
        })
        .collect()
}

/// `block` as a declaration block's text (CSSOM 6.7.2): `name: value;`
/// each, parted by spaces, a shorthand whose longhands are all there
/// and that can say them in their first one's place, those of the most
/// longhands tried first.
pub(crate) fn block_text(block: &[Declared]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut done: Vec<&str> = Vec::new();
    for declared in block {
        if done.contains(&declared.name.as_str()) {
            continue;
        }
        let mut name = declared.name.as_str();
        let mut value = declared.value.clone();
        let mut candidates: Vec<(&str, &[&str])> = SHORTHANDS
            .iter()
            .copied()
            .chain(longhands("all").map(|all| ("all", all)))
            .filter(|(_, longhands)| longhands.contains(&name))
            .collect();
        candidates.sort_by_key(|(_, longhands)| std::cmp::Reverse(longhands.len()));
        for (shorthand, longhands) in candidates {
            if longhands.iter().any(|l| done.contains(l)) {
                continue;
            }
            // A width given as a percentage stays its longhand, which
            // the shorthand's keyword would not say as given.
            let percentage = shorthand == "font"
                && block
                    .iter()
                    .any(|d| d.name == "font-stretch" && d.value.contains('%'));
            let composed = shorthand_value(shorthand, block);
            // Only as the cascade reads it back.
            if !composed.is_empty() && !percentage && valid(shorthand, &composed) {
                done.extend(longhands.iter().copied());
                name = shorthand;
                value = composed;
                break;
            }
        }
        out.push(format!(
            "{name}: {}{};",
            closed(&value),
            if declared.important {
                " !important"
            } else {
                ""
            }
        ));
    }
    out.join(" ")
}

/// `value` with the string, functions and blocks it leaves open closed,
/// as the end of a value closes them (Syntax 3 section 5.4.8), so that
/// written into a block it ends where it did and not past the `;`.
fn closed(value: &str) -> String {
    let mut closers: Vec<char> = Vec::new();
    let mut quote: Option<char> = None;
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (_, '\\') => {
                chars.next();
            }
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => closers.push(')'),
            (None, '[') => closers.push(']'),
            (None, '{') => closers.push('}'),
            (None, ')' | ']' | '}') if closers.last() == Some(&c) => {
                closers.pop();
            }
            _ => {}
        }
    }
    let mut out = value.to_owned();
    out.extend(quote);
    out.extend(closers.iter().rev());
    out
}

/// A property's name as a declaration block keeps it: an alias's
/// property, any other name lowercased, but a custom property's, whose
/// case is its own.
pub(crate) fn canonical(name: &str) -> String {
    if name.starts_with("--") {
        name.to_owned()
    } else {
        style::property_name(&name.to_ascii_lowercase()).to_owned()
    }
}

/// Whether `name` is a property the cascade knows (any takes `initial`).
pub(crate) fn known(name: &str) -> bool {
    valid(name, "initial")
}

/// Whether `value` is valid for `name`, as the cascade decides
/// (`@supports`'s test).
pub(crate) fn valid(name: &str, value: &str) -> bool {
    let tokens: Vec<Token> = css::tokenize(value);
    if tokens
        .iter()
        .all(|token| matches!(token, Token::Whitespace))
    {
        return false;
    }
    // `revert` and `revert-layer` are CSS-wide keywords every property
    // takes, though the cascade does not yet roll back to them (G7).
    if matches!(
        value.trim_matches(is_space).to_ascii_lowercase().as_str(),
        "revert" | "revert-layer"
    ) {
        return style::supports(name, &css::tokenize("initial"));
    }
    style::supports(name, &tokens)
}

/// `CSS.supports()` (CSSOM 9, gap batch A): `value` valid for the
/// property `name`; or with none, `name` a supports condition that holds,
/// as it is or in parentheses.
pub(crate) fn supports(name: &str, value: Option<&str>) -> bool {
    match value {
        Some(value) => valid(&canonical(name), value),
        None => [name.to_owned(), format!("({name})")]
            .iter()
            .any(|text| style::supports_condition(&css::tokenize(text)) == Some(true)),
    }
}

fn is_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{c}' | '\r' | ' ')
}

/// `value` without a final `!important` (any case, white space allowed
/// after the `!`), and whether it had one.
fn strip_important(value: &str) -> (&str, bool) {
    let Some(bang) = value.rfind('!') else {
        return (value, false);
    };
    let word = value[bang + 1..].trim_matches(is_space);
    if word.eq_ignore_ascii_case("important") {
        (value[..bang].trim_matches(is_space), true)
    } else {
        (value, false)
    }
}

/// `text` without its comments, those outside strings, which CSS's
/// tokenizer drops as it reads.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                out.push(c);
                if c == '\\' {
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                } else if c == q {
                    quote = None;
                }
            }
            None if c == '/' && chars.peek() == Some(&'*') => {
                chars.next();
                let mut last = ' ';
                for inside in chars.by_ref() {
                    if last == '*' && inside == '/' {
                        break;
                    }
                    last = inside;
                }
            }
            None => {
                out.push(c);
                if c == '\\' {
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                } else if c == '"' || c == '\'' {
                    quote = Some(c);
                }
            }
        }
    }
    out
}

/// The pieces of `text` between each `separator` that is outside
/// strings, comments, and brackets of every kind.
fn split_top_level(text: &str, separator: char) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut scan = Scanner::default();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if scan.top_level(c, chars.peek().map(|&(_, next)| next)) && c == separator {
            pieces.push(&text[start..i]);
            start = i + c.len_utf8();
        }
    }
    pieces.push(&text[start..]);
    pieces
}

/// `text` split at its first top-level `separator`.
fn split_once_top_level(text: &str, separator: char) -> Option<(&str, &str)> {
    let mut scan = Scanner::default();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if scan.top_level(c, chars.peek().map(|&(_, next)| next)) && c == separator {
            return Some((&text[..i], &text[i + c.len_utf8()..]));
        }
    }
    None
}

/// Where a scan of a block's text stands: in a string (its quote), in a
/// comment, after a backslash, and how deep in brackets.
#[derive(Default)]
struct Scanner {
    quote: Option<char>,
    comment: bool,
    escaped: bool,
    depth: usize,
    /// The comment's opening `/*`'s star, or its closing `*/`'s star,
    /// seen: the next character is not looked at again.
    skip: bool,
}

impl Scanner {
    /// Takes `c`, `next` after it; whether `c` is at the top level, out of
    /// strings, comments and brackets, and not escaped.
    fn top_level(&mut self, c: char, next: Option<char>) -> bool {
        if self.skip {
            self.skip = false;
            return false;
        }
        if self.escaped {
            self.escaped = false;
            return false;
        }
        if self.comment {
            if c == '*' && next == Some('/') {
                self.comment = false;
                self.skip = true;
            }
            return false;
        }
        if let Some(quote) = self.quote {
            match c {
                '\\' => self.escaped = true,
                c if c == quote => self.quote = None,
                _ => {}
            }
            return false;
        }
        match c {
            '\\' => {
                self.escaped = true;
                false
            }
            '"' | '\'' => {
                self.quote = Some(c);
                false
            }
            '/' if next == Some('*') => {
                self.comment = true;
                self.skip = true;
                false
            }
            '(' | '[' | '{' => {
                self.depth += 1;
                false
            }
            ')' | ']' | '}' => {
                self.depth = self.depth.saturating_sub(1);
                false
            }
            _ => self.depth == 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The block's text: each declaration as `name: value;`, ` !important`
    /// before the semicolon when it is, a space between them.
    fn serialize(declarations: &[Declared]) -> String {
        let mut out = String::new();
        for declared in declarations {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&declared.name);
            out.push_str(": ");
            out.push_str(&declared.value);
            if declared.important {
                out.push_str(" !important");
            }
            out.push(';');
        }
        out
    }

    #[test]
    fn display_and_an_opacitys_calculation_are_written_in_their_shortest_forms() {
        // D77, D78: as the reference browser writes them.
        assert_eq!(written("display", "flow block"), "block");
        assert_eq!(
            written("display", "list-item inline flow-root"),
            "inline flow-root list-item"
        );
        assert_eq!(written("display", "math inline"), "math");
        assert_eq!(written("opacity", "min(50%, 0%)"), "calc(0%)");
        assert_eq!(written("width", "min(50%, 0%)"), "min(50%, 0%)");
        assert_eq!(written("width", "calc(1px * NaN)"), "calc(NaN * 1px)");
    }

    /// The block's declarations, as name, value and priority.
    fn read(text: &str) -> Vec<(String, String, bool)> {
        declarations(text)
            .into_iter()
            .map(|d| (d.name, d.value, d.important))
            .collect()
    }

    #[test]
    fn a_block_is_read_as_the_cascade_would_take_it() {
        assert_eq!(
            read("color:blue;;margin-top:3px"),
            [
                ("color".into(), "blue".into(), false),
                ("margin-top".into(), "3px".into(), false)
            ]
        );
        // Invalid values and unknown properties go; a later one wins.
        assert_eq!(
            read("COLOR: red; bogus: 1; width: banana; width: 2px !IMPORTANT ; color: RED"),
            [
                ("width".into(), "2px".into(), true),
                ("color".into(), "red".into(), false)
            ]
        );
        // Separators inside strings, comments and brackets are not.
        assert_eq!(
            read(
                "background-image: url(\"a;b\"); /* c: d; */ font-family: \"x;y\\\"\"; --My-Var: a(b;c) "
            ),
            [
                ("background-image".into(), "url(\"a;b\")".into(), false),
                ("font-family".into(), "\"x;y\\\"\"".into(), false),
                ("--My-Var".into(), "a(b;c)".into(), false)
            ]
        );
        assert_eq!(
            read("col/* x */or: red"),
            [("color".into(), "red".into(), false)]
        );
        assert_eq!(read("color"), []);
        assert_eq!(read("color:"), []);
        assert_eq!(read(": red"), []);
    }

    #[test]
    fn a_block_is_written_as_the_reference_browser_writes_it() {
        let block = declarations("color: red; width: 2px !important");
        assert_eq!(serialize(&block), "color: red; width: 2px !important;");
        assert_eq!(serialize(&[]), "");
        assert!(known("float") && known("-webkit-transform") && known("--x"));
        assert!(!known("bogus"));
        assert_eq!(canonical("-WebKit-Transform"), "transform");
        assert_eq!(canonical("--Mine"), "--Mine");
        assert_eq!(
            serialize(&declarations("-webkit-transform: none")),
            "transform: none;"
        );
    }

    #[test]
    fn a_value_left_open_is_written_closed() {
        let block = |name: &str, value: &str, important| {
            block_text(&[Declared {
                name: name.to_owned(),
                value: value.to_owned(),
                important,
            }])
        };
        assert_eq!(
            block("margin-left", "calc(1px * pow(2, sqrt(100))", false),
            "margin-left: calc(1px * pow(2, sqrt(100)));"
        );
        assert_eq!(
            block("width", "min(1px, 2px", true),
            "width: min(1px, 2px) !important;"
        );
        assert_eq!(block("content", "\"a(\\\"", false), "content: \"a(\\\"\";");
        assert_eq!(block("grid-area", "[a] (b]", false), "grid-area: [a] (b]);");
        // Written back, the block reads as it was set, its calculation
        // simplified as a specified value is (D78).
        let text = block("margin-left", "calc(1px * 2", false);
        assert_eq!(
            read(&format!("{text} color: red")),
            [
                ("margin-left".into(), "calc(2px)".into(), false),
                ("color".into(), "red".into(), false)
            ]
        );
    }

    #[test]
    fn the_box_shorthands_are_kept_as_their_longhands_and_said_from_them() {
        let text = |css: &str| block_text(&declarations(css));
        assert_eq!(
            text("margin: 1px 2px; margin-left: 4px"),
            "margin: 1px 2px 1px 4px;"
        );
        assert_eq!(text("padding-top: .5em"), "padding-top: 0.5em;");
        assert_eq!(text("border: thin solid"), "border: thin solid;");
        assert_eq!(
            text("border: 1px; border-top: 2px"),
            "border-width: 2px 1px 1px; border-style: none; border-color: currentcolor; \
             border-image: none;"
        );
        assert_eq!(
            text("border: 1px; border-top: 1px !important"),
            "border-right: 1px; border-bottom: 1px; border-left: 1px; border-image: none; \
             border-top: 1px !important;"
        );
        assert_eq!(
            text("outline: 2px dotted blue"),
            "outline: blue dotted 2px;"
        );
        assert_eq!(
            text("list-style: circle inside"),
            "list-style: inside circle;"
        );
        assert_eq!(text("list-style: none"), "list-style: none;");
        assert_eq!(
            shorthand_value("margin", &declarations("margin: inherit")),
            "inherit"
        );
        assert_eq!(longhands("border-top"), Some(&BORDER_TOP[..]));
        assert_eq!(longhands("color"), None);
        assert_eq!(
            text("color: rgb(1,2,3); background-image: url(a)"),
            "color: rgb(1, 2, 3); background-image: url(\"a\");"
        );
    }

    #[test]
    fn font_is_kept_as_its_longhands_and_said_from_them() {
        let block = declarations("font: italic 16px/2 \"A\",serif; color: red");
        assert_eq!(block.len(), FONT_LONGHANDS.len() + 1);
        let value = |block: &[Declared], name: &str| {
            block
                .iter()
                .find(|d| d.name == name)
                .map(|d| d.value.clone())
        };
        assert_eq!(value(&block, "font-style").as_deref(), Some("italic"));
        assert_eq!(value(&block, "line-height").as_deref(), Some("2"));
        assert_eq!(value(&block, "font-family").as_deref(), Some("A, serif"));
        assert_eq!(shorthand_value("font", &block), "italic 16px/2 A, serif");
        assert_eq!(
            block_text(&block),
            "font: italic 16px/2 A, serif; color: red;"
        );
        // A longhand the shorthand cannot say makes it the empty string,
        // and the block lists the longhands.
        let set = |extra: &str| declarations(&format!("font: medium serif; {extra}"));
        assert_eq!(
            shorthand_value("font", &set("font-stretch: ULTRA-condensed")),
            "ultra-condensed medium serif"
        );
        assert_eq!(
            shorthand_value("font", &set("font-stretch: 50.00000%")),
            "ultra-condensed medium serif"
        );
        let stretch = set("font-stretch: 50.00000%");
        assert_eq!(value(&stretch, "font-stretch").as_deref(), Some("50%"));
        // Written back, a percentage stays the longhand's.
        assert!(block_text(&stretch).contains("font-stretch: 50%;"));
        assert_eq!(
            shorthand_value("font", &set("font-stretch: calc(50%)")),
            "ultra-condensed medium serif"
        );
        for unsaid in [
            "font-stretch: 95%",
            "font-feature-settings: \"sinf\"",
            "font-variant: none",
            "font-variant-caps: all-small-caps",
            "font-variant-caps: inherit",
        ] {
            assert_eq!(shorthand_value("font", &set(unsaid)), "", "{unsaid}");
        }
        assert!(block_text(&set("font-variant: none")).starts_with("font-style: normal;"));
        assert_eq!(
            shorthand_value("font", &set("font-variant: small-caps")),
            "small-caps medium serif"
        );
        assert_eq!(
            shorthand_value("font", &declarations("font: inherit")),
            "inherit"
        );
        assert_eq!(
            shorthand_value("font", &set("font-stretch: revert")),
            "",
            "revert is kept"
        );
        // `font-variant` likewise.
        let variant = declarations("font: 12px serif; font-variant-numeric: tabular-nums");
        assert_eq!(shorthand_value("font-variant", &variant), "tabular-nums");
        assert_eq!(
            shorthand_value("font-variant", &declarations("font-variant: none")),
            "none"
        );
        assert_eq!(shorthand_value("font-variant", &[]), "");
        // A system font is kept whole.
        assert_eq!(block_text(&declarations("font: menu")), "font: menu;");
        assert_eq!(
            value(&declarations("font-family: Arial,Arial"), "font-family").as_deref(),
            Some("Arial, Arial")
        );
    }

    #[test]
    fn all_is_kept_as_every_longhand_but_direction_and_unicode_bidi() {
        let text = |css: &str| block_text(&declarations(css));
        assert_eq!(text("all: inherit"), "all: inherit;");
        assert_eq!(
            text("width: 100px; all: inherit; height: inherit"),
            "all: inherit;"
        );
        assert_eq!(
            text("width: 1px; --a: a; all: unset; --b: b"),
            "--a: a; all: unset; --b: b;"
        );
        let block = declarations("margin: 1px; all: revert");
        assert_eq!(shorthand_value("margin", &block), "revert");
        assert_eq!(shorthand_value("all", &block), "revert");
        assert!(longhands("all").is_some_and(|all| !all.contains(&"direction")));
        assert_eq!(
            text("overflow-x: scroll; overflow-y: hidden"),
            "overflow: scroll hidden;"
        );
        assert_eq!(text("overflow: clip"), "overflow: clip;");
        assert_eq!(text("overflow-y: auto"), "overflow-y: auto;");
        assert_eq!(text("color: CanvasText"), "color: canvastext;");
        assert_eq!(
            text("border-color: currentColor"),
            "border-color: currentcolor;"
        );
        assert_eq!(text("animation-name: Red"), "animation-name: Red;");
    }

    #[test]
    fn a_page_rules_block_keeps_its_descriptors() {
        let text = |css: &str| block_text(&page_descriptors(css));
        assert_eq!(
            text("size: A3; page-orientation: rotate-right; margin: 1em 24px; float: left"),
            "size: a3; page-orientation: rotate-right; margin: 1em 24px;"
        );
        assert_eq!(text("size: jis-b5 landscape"), "size: jis-b5 landscape;");
        assert_eq!(text("size: 216mm"), "size: 216mm;");
        assert_eq!(text("size: notarealsize; page-orientation: sideways"), "");
        assert_eq!(text("size: 10% 5px"), "");
        let rule = super::super::rules::page_selector;
        assert_eq!(rule("named:First").as_deref(), Some("named:first"));
        assert_eq!(rule(" :left ").as_deref(), Some(":left"));
        assert_eq!(rule("named :first"), None);
        assert_eq!(rule(":first :left"), None);
        assert_eq!(rule(":nope"), None);
    }
}
