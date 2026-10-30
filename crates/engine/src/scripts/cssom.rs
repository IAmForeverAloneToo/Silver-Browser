//! What a script's `style` object reads and writes (M25.2,
//! `docs/design/dom-scripting.md`): the `style` attribute's declarations
//! with their values' text as written, each checked as the cascade would
//! take it; the bindings write the block back. CSSOM's serialization of
//! specified values is G50's: a value is kept as it was given, trimmed.
//! The font's are written as the reference browser writes them, and the
//! `font` and `font-variant` shorthands are kept as their longhands and
//! written back from them (gap batch D).

use css::Token;

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

/// The longhands of the shorthand `name` declared `value`, each with
/// its value's text; `None` for a property kept as it is (a longhand, a
/// value with a substitution, a system font).
fn expand(name: &str, value: &str) -> Option<Vec<(String, String)>> {
    let longhands: &[&str] = match name {
        "font" => &FONT_LONGHANDS,
        "font-variant" => &style::VARIANT_LONGHANDS,
        _ => return None,
    };
    if let Some(keyword) = css_wide(value) {
        return Some(
            longhands
                .iter()
                .map(|l| ((*l).to_owned(), keyword.clone()))
                .collect(),
        );
    }
    let tokens = css::tokenize(value);
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
/// the reference browser writes them, the rest as given.
fn written(name: &str, value: &str) -> String {
    if css_wide(value).is_some() || value.contains("var(") {
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
        _ => None,
    };
    text.unwrap_or_else(|| value.to_owned())
}

/// The value of the shorthand `name` from `block`'s longhands, as CSSOM
/// serializes one (section 6.7.2): the empty string unless all its
/// longhands are there with one priority and it can say their values.
pub(crate) fn shorthand_value(name: &str, block: &[Declared]) -> String {
    let longhands: &[&str] = match name {
        "font" => &FONT_LONGHANDS,
        "font-variant" => &style::VARIANT_LONGHANDS,
        _ => return String::new(),
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
    if name == "font-variant" {
        return style::write_font_variant(&values).unwrap_or_default();
    }
    font_value(&values).unwrap_or_default()
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
/// and that can say them in their first one's place.
pub(crate) fn block_text(block: &[Declared]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut done: Vec<&str> = Vec::new();
    for declared in block {
        if done.contains(&declared.name.as_str()) {
            continue;
        }
        let mut name = declared.name.as_str();
        let mut value = declared.value.clone();
        for (shorthand, longhands) in [
            ("font", &FONT_LONGHANDS[..]),
            ("font-variant", &style::VARIANT_LONGHANDS[..]),
        ] {
            if !longhands.contains(&name) {
                continue;
            }
            // A width given as a percentage stays its longhand, which
            // the shorthand's keyword would not say as given.
            let percentage = shorthand == "font"
                && block
                    .iter()
                    .any(|d| d.name == "font-stretch" && d.value.contains('%'));
            let composed = shorthand_value(shorthand, block);
            if !composed.is_empty() && !percentage {
                done.extend(longhands.iter().copied());
                name = shorthand;
                value = composed;
                break;
            }
        }
        out.push(format!(
            "{name}: {value}{};",
            if declared.important {
                " !important"
            } else {
                ""
            }
        ));
    }
    out.join(" ")
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
                ("color".into(), "RED".into(), false)
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
}
