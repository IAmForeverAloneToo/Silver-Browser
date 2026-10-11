//! The context's `font` (HTML 4.12.5.1.10): CSS's `font` shorthand as
//! the reference browser reads it on a canvas, its relative sizes
//! resolved when it is set, and as it serializes it: the style, the
//! weight, small caps, the size in pixels and the families; the line
//! height read and dropped, the stretch given to `fontStretch`. The
//! engine has one face (G22): every family, weight and style draws with
//! it.

use css::Token;

/// What a font's relative sizes are of: the canvas element's computed
/// `font-size`, the root element's, and the viewport's size; 10 pixels
/// each for a canvas with no style, as the reference browser has it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontBase {
    pub em: f64,
    pub rem: f64,
    pub viewport: (f64, f64),
}

impl Default for FontBase {
    fn default() -> FontBase {
        FontBase {
            em: 10.0,
            rem: 10.0,
            viewport: (0.0, 0.0),
        }
    }
}

/// `fontStretch`, and the stretch a `font` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stretch {
    UltraCondensed,
    ExtraCondensed,
    Condensed,
    SemiCondensed,
    Normal,
    SemiExpanded,
    Expanded,
    ExtraExpanded,
    UltraExpanded,
}

const STRETCHES: [(&str, Stretch); 9] = [
    ("ultra-condensed", Stretch::UltraCondensed),
    ("extra-condensed", Stretch::ExtraCondensed),
    ("condensed", Stretch::Condensed),
    ("semi-condensed", Stretch::SemiCondensed),
    ("normal", Stretch::Normal),
    ("semi-expanded", Stretch::SemiExpanded),
    ("expanded", Stretch::Expanded),
    ("extra-expanded", Stretch::ExtraExpanded),
    ("ultra-expanded", Stretch::UltraExpanded),
];

impl Stretch {
    pub fn from_name(name: &str) -> Option<Stretch> {
        STRETCHES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|&(_, stretch)| stretch)
    }

    pub fn name(self) -> &'static str {
        STRETCHES
            .iter()
            .find(|(_, s)| *s == self)
            .map_or("normal", |&(n, _)| n)
    }
}

/// A parsed `font`.
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    pub italic: bool,
    /// 1 to 1000; 400 is normal.
    pub weight: u16,
    pub small_caps: bool,
    pub stretch: Stretch,
    /// In pixels.
    pub size: f64,
    /// The families, each as it serializes: a name of more than one word
    /// in double quotes, a generic family in lowercase.
    pub families: Vec<String>,
}

impl Default for Font {
    /// `10px sans-serif`.
    fn default() -> Font {
        Font {
            italic: false,
            weight: 400,
            small_caps: false,
            stretch: Stretch::Normal,
            size: 10.0,
            families: vec!["sans-serif".to_owned()],
        }
    }
}

const GENERIC: [&str; 13] = [
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
    "emoji",
    "math",
    "fangsong",
];

const WIDE: [&str; 6] = [
    "inherit",
    "initial",
    "unset",
    "revert",
    "revert-layer",
    "default",
];

/// A number as the reference browser writes a font's size: at most six
/// significant digits, no trailing zeros.
fn size_text(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".to_owned();
    }
    let digits = value.abs().log10().floor() as i32 + 1;
    let places = (6 - digits).clamp(0, 12) as usize;
    let text = format!("{value:.places$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

impl Font {
    /// The font as `font` reads it back.
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        if self.italic {
            out.push_str("italic ");
        }
        match self.weight {
            400 => {}
            700 => out.push_str("bold "),
            weight => out.push_str(&format!("{weight} ")),
        }
        if self.small_caps {
            out.push_str("small-caps ");
        }
        out.push_str(&size_text(self.size));
        out.push_str("px ");
        out.push_str(&self.families.join(", "));
        out
    }
}

/// A length's pixels, its unit absolute or relative to `base` (`em` to
/// `font`, which is the font's size being set); `None` for a unit that
/// is neither.
pub(crate) fn length(value: f64, unit: &str, base: &FontBase, font: f64) -> Option<f64> {
    let unit = unit.to_ascii_lowercase();
    Some(match unit.as_str() {
        "px" => value,
        "pt" => value * 4.0 / 3.0,
        "pc" => value * 16.0,
        "in" => value * 96.0,
        "cm" => value * 96.0 / 2.54,
        "mm" => value * 96.0 / 25.4,
        "q" => value * 96.0 / 101.6,
        "em" => value * font,
        "ex" | "ch" => value * font / 2.0,
        "rem" => value * base.rem,
        "vw" => value * base.viewport.0 / 100.0,
        "vh" => value * base.viewport.1 / 100.0,
        "vmin" => value * base.viewport.0.min(base.viewport.1) / 100.0,
        "vmax" => value * base.viewport.0.max(base.viewport.1) / 100.0,
        _ => return None,
    })
}

/// The size an absolute or relative `font-size` keyword names.
fn keyword_size(name: &str, base: &FontBase) -> Option<f64> {
    Some(match name {
        "xx-small" => 9.0,
        "x-small" => 10.0,
        "small" => 13.0,
        "medium" => 16.0,
        "large" => 18.0,
        "x-large" => 24.0,
        "xx-large" => 32.0,
        "xxx-large" => 48.0,
        "larger" => base.em * 1.2,
        "smaller" => base.em / 1.2,
        _ => return None,
    })
}

/// `text` as a canvas's `font`, its relative sizes of `base`; `None` for
/// text that is not a `font` the reference browser takes on a canvas.
pub fn parse_font(text: &str, base: &FontBase) -> Option<Font> {
    let tokens = css::tokenize(text);
    let mut i = 0;
    let skip = |i: &mut usize| {
        while matches!(tokens.get(*i), Some(Token::Whitespace)) {
            *i += 1;
        }
    };
    let mut font = Font {
        families: Vec::new(),
        ..Font::default()
    };
    let (mut style, mut weight, mut variant, mut stretch) = (false, false, false, false);
    let mut normals = 0;
    // Style, variant, weight and stretch, in any order, each once.
    skip(&mut i);
    loop {
        match tokens.get(i) {
            Some(Token::Ident(name)) => {
                let name = name.to_ascii_lowercase();
                match name.as_str() {
                    "normal" => normals += 1,
                    "italic" if !style => {
                        style = true;
                        font.italic = true;
                    }
                    "oblique" if !style => {
                        style = true;
                        font.italic = true;
                        // An angle after it the reference browser takes
                        // and draws upright.
                        let mut j = i + 1;
                        skip(&mut j);
                        if let Some(Token::Dimension { unit, .. }) = tokens.get(j)
                            && matches!(
                                unit.to_ascii_lowercase().as_str(),
                                "deg" | "rad" | "grad" | "turn"
                            )
                        {
                            font.italic = false;
                            i = j;
                        }
                    }
                    "small-caps" if !variant => {
                        variant = true;
                        font.small_caps = true;
                    }
                    "bold" | "bolder" | "lighter" if !weight => {
                        weight = true;
                        font.weight = match name.as_str() {
                            "lighter" => 100,
                            _ => 700,
                        };
                    }
                    other => match Stretch::from_name(other) {
                        Some(found) if !stretch && found != Stretch::Normal => {
                            stretch = true;
                            font.stretch = found;
                        }
                        _ => break,
                    },
                }
            }
            Some(Token::Number { value, .. }) if !weight => {
                if !(1.0..=1000.0).contains(value) {
                    return None;
                }
                weight = true;
                font.weight = *value as u16;
            }
            _ => break,
        }
        i += 1;
        skip(&mut i);
    }
    let set = normals
        + [style, weight, variant, stretch]
            .iter()
            .filter(|s| **s)
            .count();
    if set > 4 {
        return None;
    }
    // The size.
    font.size = match tokens.get(i)? {
        Token::Dimension { value, unit } => length(*value, unit, base, base.em)?,
        Token::Percentage { value } => base.em * value / 100.0,
        Token::Ident(name) => keyword_size(&name.to_ascii_lowercase(), base)?,
        _ => return None,
    };
    if font.size.is_nan() || font.size < 0.0 || !font.size.is_finite() {
        return None;
    }
    i += 1;
    skip(&mut i);
    // A line height, read and dropped.
    if tokens.get(i) == Some(&Token::Delim('/')) {
        i += 1;
        skip(&mut i);
        match tokens.get(i)? {
            Token::Number { .. } | Token::Dimension { .. } | Token::Percentage { .. } => {}
            Token::Ident(name) if name.eq_ignore_ascii_case("normal") => {}
            _ => return None,
        }
        i += 1;
        skip(&mut i);
    }
    // The families, each a string or words, a comma between.
    loop {
        let family = match tokens.get(i)? {
            Token::String { value, bad: false } => {
                i += 1;
                quoted(value)
            }
            Token::Ident(_) => {
                let mut words = Vec::new();
                while let Some(Token::Ident(word)) = tokens.get(i) {
                    words.push(word.clone());
                    i += 1;
                    skip(&mut i);
                }
                if words.len() == 1 {
                    let lower = words[0].to_ascii_lowercase();
                    if WIDE.contains(&lower.as_str()) {
                        return None;
                    }
                    if GENERIC.contains(&lower.as_str()) {
                        lower
                    } else {
                        words.remove(0)
                    }
                } else {
                    quoted(&words.join(" "))
                }
            }
            _ => return None,
        };
        font.families.push(family);
        skip(&mut i);
        match tokens.get(i) {
            None => break,
            Some(Token::Comma) => {
                i += 1;
                skip(&mut i);
            }
            Some(_) => return None,
        }
    }
    Some(font)
}

/// A family name as it serializes: bare when it is one word, else in
/// double quotes.
fn quoted(name: &str) -> String {
    if !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii())
        && !name.starts_with(|c: char| c.is_ascii_digit())
    {
        name.to_owned()
    } else {
        format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// `letterSpacing` or `wordSpacing`: a length alone, its text as it was
/// given, in pixels and in ems of the font it is drawn with.
#[derive(Debug, Clone, PartialEq)]
pub struct Spacing {
    pub text: String,
    pub px: f64,
    pub em: f64,
}

impl Default for Spacing {
    fn default() -> Spacing {
        Spacing {
            text: "0px".to_owned(),
            px: 0.0,
            em: 0.0,
        }
    }
}

impl Spacing {
    /// `text` as a spacing: one length token and nothing round it, its
    /// relative units but `em`, `ex` and `ch` resolved now.
    pub fn parse(text: &str, base: &FontBase) -> Option<Spacing> {
        let tokens = css::tokenize(text);
        let [Token::Dimension { value, unit }] = tokens.as_slice() else {
            return None;
        };
        if !value.is_finite() {
            return None;
        }
        let lower = unit.to_ascii_lowercase();
        let (px, em) = match lower.as_str() {
            "em" => (0.0, *value),
            "ex" | "ch" => (0.0, value / 2.0),
            _ => (length(*value, &lower, base, 0.0)?, 0.0),
        };
        Some(Spacing {
            text: text.to_owned(),
            px,
            em,
        })
    }

    /// In pixels, for a font of `size`.
    pub fn pixels(&self, size: f64) -> f64 {
        self.px + self.em * size
    }
}
