//! Media queries (M20): Media Queries Level 4's `<media-query-list>`,
//! parsed from the css crate's tokens with the draft's error handling,
//! and evaluated against a [`Media`], the environment the cascade is
//! given; with Media Queries Level 5's user-preference features. See
//! `docs/design/media.md`.
//!
//! A query that does not match the grammar is `not all` (Media Queries
//! 4 §3.2), the rest of its list standing. A feature this engine does
//! not know, a value its feature does not take, and anything else in
//! parentheses the grammar has no place for, are `<general-enclosed>`,
//! whose value is unknown: the evaluation is three-valued, `not` of
//! unknown is unknown, and a query whose value is unknown is false.
//! Parentheses nested past [`MAX_NESTING`] levels are unknown too, which
//! bounds the recursion; a block the tokens end inside is closed by
//! their end, as css-syntax-3 closes it.

use css::Token;

/// How deep conditions in parentheses may nest before what is inside is
/// left unknown.
pub const MAX_NESTING: usize = 32;

/// The kind of medium a document is shown on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaType {
    /// A screen: the window.
    #[default]
    Screen,
    /// Paper, or a PDF of it.
    Print,
}

/// The colour scheme the user prefers (`prefers-color-scheme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    /// Dark text on a light background.
    #[default]
    Light,
    /// Light text on a dark background.
    Dark,
}

/// The user's preferences that media queries can ask about (Media
/// Queries 5 §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Preferences {
    /// `prefers-color-scheme`.
    pub color_scheme: ColorScheme,
    /// `prefers-reduced-motion: reduce`.
    pub reduced_motion: bool,
}

/// What a media query is evaluated against: the medium, its size in
/// CSS pixels, the device's, its resolution in dots per CSS pixel, and
/// the user's preferences. The other features of the device have the
/// fixed values of a desktop browser drawing in colour with a mouse
/// (`docs/design/media.md`, "The media environment").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Media {
    /// The medium.
    pub media_type: MediaType,
    /// The viewport's width, or the page box's on paper.
    pub width: f32,
    /// The viewport's height, or the page box's on paper.
    pub height: f32,
    /// The deprecated `device-width`: the screen's, or the page box's.
    pub device_width: f32,
    /// The deprecated `device-height`.
    pub device_height: f32,
    /// Dots per CSS pixel.
    pub resolution: f32,
    /// The user's preferences.
    pub preferences: Preferences,
}

impl Default for Media {
    fn default() -> Self {
        Media::screen(0.0, 0.0)
    }
}

impl Media {
    /// A screen whose viewport, and device, are `width` by `height`, at
    /// one dot per pixel, with no preference.
    pub fn screen(width: f32, height: f32) -> Self {
        Media {
            media_type: MediaType::Screen,
            width,
            height,
            device_width: width,
            device_height: height,
            resolution: 1.0,
            preferences: Preferences::default(),
        }
    }
}

/// A parsed `<media-query-list>`: true when any of its queries is, and
/// when it has none.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaQueryList {
    queries: Vec<MediaQuery>,
}

/// One query of a list.
#[derive(Debug, Clone, PartialEq)]
enum MediaQuery {
    /// A query that did not match the grammar: `not all`.
    NotAll,
    /// `[not | only]? <media-type> [and <media-condition-without-or>]?`.
    Typed {
        not: bool,
        media_type: TypeName,
        condition: Option<Condition>,
    },
    /// A `<media-condition>` alone.
    Condition(Condition),
}

/// A media type as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeName {
    All,
    Screen,
    Print,
    /// Any other type: CSS2's, recognised and matching nothing
    /// (Media Queries 4 §2.3), or one unknown, which matches nothing
    /// too.
    Other,
}

/// A `<media-condition>`.
#[derive(Debug, Clone, PartialEq)]
enum Condition {
    Feature(Feature),
    Not(Box<Condition>),
    And(Vec<Condition>),
    Or(Vec<Condition>),
    /// `<general-enclosed>`, or a feature this engine cannot evaluate.
    Unknown,
}

/// A media feature.
#[derive(Debug, Clone, PartialEq)]
enum Feature {
    /// `(name: value)`, with `min-` or `max-` on a range feature.
    Plain {
        name: Name,
        prefix: Prefix,
        value: Value,
    },
    /// `(name)`.
    Boolean(Name),
    /// `(name op value)`, `(value op name)` or `(value op name op value)`:
    /// each bound as `value op actual`.
    Range {
        name: Name,
        bounds: Vec<(Value, Comparison)>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prefix {
    Exact,
    Min,
    Max,
}

/// A comparison of a bound with the actual value, the bound on its
/// left: `Lt` is `bound < actual`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comparison {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

/// The features this engine knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Name {
    Width,
    Height,
    AspectRatio,
    DeviceWidth,
    DeviceHeight,
    DeviceAspectRatio,
    Orientation,
    Resolution,
    Scan,
    Grid,
    Update,
    OverflowBlock,
    OverflowInline,
    Color,
    ColorIndex,
    Monochrome,
    ColorGamut,
    Pointer,
    Hover,
    AnyPointer,
    AnyHover,
    PrefersColorScheme,
    PrefersReducedMotion,
    PrefersContrast,
    PrefersReducedTransparency,
    ForcedColors,
    InvertedColors,
    Scripting,
    DynamicRange,
    VideoDynamicRange,
    DisplayMode,
    /// The Compatibility Standard's `-webkit-device-pixel-ratio`.
    WebkitDevicePixelRatio,
}

/// What a feature's values are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Length,
    Ratio,
    Resolution,
    Integer,
    Number,
    /// `<mq-boolean>`: 0 or 1.
    Boolean,
    /// One of the keywords.
    Keywords(&'static [&'static str]),
}

/// A value in a query, as written.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    /// A length: a number and its unit, lowercased; `0` is `0px`.
    Length(f64, LengthUnit),
    /// `a / b`, or `a` alone, which is `a / 1`.
    Ratio(f64, f64),
    /// Dots per CSS pixel, or `infinite`.
    Resolution(f64),
    /// An integer, or a number.
    Number(f64),
    /// A keyword, lowercased.
    Keyword(&'static str),
}

/// The units of a length in a query: absolute, or relative to the
/// initial values (Media Queries 4 §1.3) and the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LengthUnit {
    Px,
    In,
    Cm,
    Mm,
    Q,
    Pt,
    Pc,
    Em,
    Rem,
    Ex,
    Ch,
    Vw,
    Vh,
    Vmin,
    Vmax,
}

/// The value of a three-valued evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Truth {
    True,
    False,
    Unknown,
}

impl Truth {
    fn of(b: bool) -> Truth {
        if b { Truth::True } else { Truth::False }
    }

    fn not(self) -> Truth {
        match self {
            Truth::True => Truth::False,
            Truth::False => Truth::True,
            Truth::Unknown => Truth::Unknown,
        }
    }
}

/// The initial font size, which `em` and `rem` in a query are of.
const INITIAL_FONT_SIZE: f64 = 16.0;

impl Name {
    fn of(name: &str) -> Option<(Name, Prefix)> {
        let lower = name.to_ascii_lowercase();
        // The vendor's prefix goes before `min-` and `max-`.
        let (vendor, unprefixed) = match lower.strip_prefix("-webkit-") {
            Some(rest) => ("-webkit-", rest),
            None => ("", lower.as_str()),
        };
        let (prefix, rest) = if let Some(rest) = unprefixed.strip_prefix("min-") {
            (Prefix::Min, rest)
        } else if let Some(rest) = unprefixed.strip_prefix("max-") {
            (Prefix::Max, rest)
        } else {
            (Prefix::Exact, unprefixed)
        };
        if rest.starts_with('-') {
            return None;
        }
        let bare = format!("{vendor}{rest}");
        let name = match bare.as_str() {
            "width" => Name::Width,
            "height" => Name::Height,
            "aspect-ratio" => Name::AspectRatio,
            "device-width" => Name::DeviceWidth,
            "device-height" => Name::DeviceHeight,
            "device-aspect-ratio" => Name::DeviceAspectRatio,
            "orientation" => Name::Orientation,
            "resolution" => Name::Resolution,
            "scan" => Name::Scan,
            "grid" => Name::Grid,
            "update" => Name::Update,
            "overflow-block" => Name::OverflowBlock,
            "overflow-inline" => Name::OverflowInline,
            "color" => Name::Color,
            "color-index" => Name::ColorIndex,
            "monochrome" => Name::Monochrome,
            "color-gamut" => Name::ColorGamut,
            "pointer" => Name::Pointer,
            "hover" => Name::Hover,
            "any-pointer" => Name::AnyPointer,
            "any-hover" => Name::AnyHover,
            "prefers-color-scheme" => Name::PrefersColorScheme,
            "prefers-reduced-motion" => Name::PrefersReducedMotion,
            "prefers-contrast" => Name::PrefersContrast,
            "prefers-reduced-transparency" => Name::PrefersReducedTransparency,
            "forced-colors" => Name::ForcedColors,
            "inverted-colors" => Name::InvertedColors,
            "scripting" => Name::Scripting,
            "dynamic-range" => Name::DynamicRange,
            "video-dynamic-range" => Name::VideoDynamicRange,
            "display-mode" => Name::DisplayMode,
            "-webkit-device-pixel-ratio" => Name::WebkitDevicePixelRatio,
            _ => return None,
        };
        // `min-` and `max-` go on range features alone.
        if prefix != Prefix::Exact && !name.is_range() {
            return None;
        }
        Some((name, prefix))
    }

    fn kind(self) -> Kind {
        match self {
            Name::Width | Name::Height | Name::DeviceWidth | Name::DeviceHeight => Kind::Length,
            Name::AspectRatio | Name::DeviceAspectRatio => Kind::Ratio,
            Name::Resolution => Kind::Resolution,
            Name::Color | Name::ColorIndex | Name::Monochrome => Kind::Integer,
            Name::WebkitDevicePixelRatio => Kind::Number,
            Name::Grid => Kind::Boolean,
            Name::Orientation => Kind::Keywords(&["portrait", "landscape"]),
            Name::Scan => Kind::Keywords(&["interlace", "progressive"]),
            Name::Update => Kind::Keywords(&["none", "slow", "fast"]),
            Name::OverflowBlock => Kind::Keywords(&["none", "scroll", "paged"]),
            Name::OverflowInline => Kind::Keywords(&["none", "scroll"]),
            Name::ColorGamut => Kind::Keywords(&["srgb", "p3", "rec2020"]),
            Name::Pointer | Name::AnyPointer => Kind::Keywords(&["none", "coarse", "fine"]),
            Name::Hover | Name::AnyHover => Kind::Keywords(&["none", "hover"]),
            Name::PrefersColorScheme => Kind::Keywords(&["light", "dark"]),
            Name::PrefersReducedMotion | Name::PrefersReducedTransparency => {
                Kind::Keywords(&["no-preference", "reduce"])
            }
            Name::PrefersContrast => Kind::Keywords(&["no-preference", "more", "less", "custom"]),
            Name::ForcedColors => Kind::Keywords(&["none", "active"]),
            Name::InvertedColors => Kind::Keywords(&["none", "inverted"]),
            Name::Scripting => Kind::Keywords(&["none", "initial-only", "enabled"]),
            Name::DynamicRange | Name::VideoDynamicRange => Kind::Keywords(&["standard", "high"]),
            Name::DisplayMode => Kind::Keywords(&[
                "fullscreen",
                "standalone",
                "minimal-ui",
                "browser",
                "picture-in-picture",
            ]),
        }
    }

    fn is_range(self) -> bool {
        matches!(
            self.kind(),
            Kind::Length | Kind::Ratio | Kind::Resolution | Kind::Integer | Kind::Number
        )
    }
}

/// The blocks open at a point of a run of tokens, by the token that
/// closes each: `(` and a function are closed by `)`, `[` by `]` and
/// `{` by `}` (css-syntax-3 §5.4.8). A closing token that closes none
/// of them is a token like any other.
#[derive(Default)]
pub(crate) struct Nesting {
    open: Vec<u8>,
}

impl Nesting {
    /// Takes `token` into account; says whether it closed a block.
    pub(crate) fn step(&mut self, token: &Token) -> bool {
        let closer = match token {
            Token::OpenParen | Token::Function(_) => b')',
            Token::OpenSquare => b']',
            Token::OpenCurly => b'}',
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => {
                let closes = match token {
                    Token::CloseParen => b')',
                    Token::CloseSquare => b']',
                    _ => b'}',
                };
                if self.open.last() == Some(&closes) {
                    self.open.pop();
                    return true;
                }
                return false;
            }
            _ => return false,
        };
        self.open.push(closer);
        false
    }

    /// Whether no block is open.
    pub(crate) fn is_empty(&self) -> bool {
        self.open.is_empty()
    }
}

/// A cursor over a condition's tokens, which passes over whitespace
/// where asked.
pub(crate) struct Cursor<'a> {
    tokens: &'a [Token],
    pub(crate) pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(tokens: &'a [Token]) -> Self {
        Cursor { tokens, pos: 0 }
    }

    pub(crate) fn skip_ws(&mut self) {
        while matches!(self.tokens.get(self.pos), Some(Token::Whitespace)) {
            self.pos += 1;
        }
    }

    pub(crate) fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.pos)
    }

    pub(crate) fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    /// Takes the identifier `word`, in any case, when it is next.
    pub(crate) fn take_word(&mut self, word: &str) -> bool {
        match self.peek() {
            Some(Token::Ident(ident)) if ident.eq_ignore_ascii_case(word) => {
                self.pos += 1;
                true
            }
            _ => false,
        }
    }

    /// The tokens of the block opened by the `(` or function token at
    /// the cursor, without the opening and closing tokens, taking them
    /// all; a block the tokens end inside is closed by their end.
    pub(crate) fn take_block(&mut self) -> &'a [Token] {
        let start = self.pos + 1;
        let mut nesting = Nesting::default();
        while let Some(token) = self.tokens.get(self.pos) {
            self.pos += 1;
            if nesting.step(token) && nesting.is_empty() {
                return &self.tokens[start..self.pos - 1];
            }
        }
        &self.tokens[start.min(self.tokens.len())..]
    }
}

/// The tokens with the whitespace at either end taken off.
pub(crate) fn trim(tokens: &[Token]) -> &[Token] {
    let start = tokens
        .iter()
        .position(|t| *t != Token::Whitespace)
        .unwrap_or(tokens.len());
    let end = tokens
        .iter()
        .rposition(|t| *t != Token::Whitespace)
        .map_or(start, |i| i + 1);
    &tokens[start..end]
}

/// `tokens` split at the commas outside any block.
pub(crate) fn split_commas(tokens: &[Token]) -> Vec<&[Token]> {
    let mut parts = Vec::new();
    let mut nesting = Nesting::default();
    let mut start = 0;
    for (i, token) in tokens.iter().enumerate() {
        if *token == Token::Comma && nesting.is_empty() {
            parts.push(&tokens[start..i]);
            start = i + 1;
        }
        nesting.step(token);
    }
    parts.push(&tokens[start..]);
    parts
}

/// Parses a `<media-query-list>`: an empty one, all whitespace, holds
/// whatever the medium (Media Queries 4 §2.1), and a query that does
/// not parse is `not all`, the rest of the list standing.
pub fn parse_media_query_list(tokens: &[Token]) -> MediaQueryList {
    if trim(tokens).is_empty() {
        return MediaQueryList::default();
    }
    MediaQueryList {
        queries: split_commas(tokens)
            .into_iter()
            .map(|query| parse_query(trim(query)).unwrap_or(MediaQuery::NotAll))
            .collect(),
    }
}

/// One `<media-query>`; `None` when it does not match the grammar.
fn parse_query(tokens: &[Token]) -> Option<MediaQuery> {
    let mut c = Cursor::new(tokens);
    let typed = match tokens.first()? {
        // `not` before a type, rather than a condition's `not (...)`.
        Token::Ident(word)
            if word.eq_ignore_ascii_case("not") || word.eq_ignore_ascii_case("only") =>
        {
            c.pos = 1;
            c.skip_ws();
            matches!(c.peek(), Some(Token::Ident(_)))
        }
        Token::Ident(_) => true,
        _ => false,
    };
    if !typed {
        c.pos = 0;
        let condition = parse_condition(&mut c, true, 0)?;
        c.skip_ws();
        return c.at_end().then_some(MediaQuery::Condition(condition));
    }
    c.pos = 0;
    let not = c.take_word("not");
    if !not {
        c.take_word("only");
    }
    c.skip_ws();
    let Some(Token::Ident(name)) = c.peek() else {
        return None;
    };
    c.pos += 1;
    let media_type = match name.to_ascii_lowercase().as_str() {
        // Not a media type (Media Queries 4 §3).
        "only" | "not" | "and" | "or" | "layer" => return None,
        "all" => TypeName::All,
        "screen" => TypeName::Screen,
        "print" => TypeName::Print,
        _ => TypeName::Other,
    };
    c.skip_ws();
    if c.at_end() {
        return Some(MediaQuery::Typed {
            not,
            media_type,
            condition: None,
        });
    }
    if !c.take_word("and") {
        return None;
    }
    let condition = parse_condition(&mut c, false, 0)?;
    c.skip_ws();
    c.at_end().then_some(MediaQuery::Typed {
        not,
        media_type,
        condition: Some(condition),
    })
}

/// A `<media-condition>`, or `<media-condition-without-or>` when
/// `!or`: `not` and one term, or terms joined by `and` alone or by
/// `or` alone; `None` when it does not match the grammar. A keyword
/// needs no whitespace around it: what would follow it without is a
/// parenthesis, which a comment can put there, `and(` being a function.
fn parse_condition(c: &mut Cursor<'_>, or: bool, depth: usize) -> Option<Condition> {
    c.skip_ws();
    if c.take_word("not") {
        c.skip_ws();
        return Some(Condition::Not(Box::new(parse_in_parens(c, depth)?)));
    }
    let first = parse_in_parens(c, depth)?;
    let mut terms = vec![first];
    let mut joiner: Option<bool> = None; // Some(true) for `and`
    loop {
        let before = c.pos;
        c.skip_ws();
        let is_and = if c.take_word("and") {
            true
        } else if or && c.take_word("or") {
            false
        } else {
            c.pos = before;
            break;
        };
        if joiner.is_some_and(|j| j != is_and) {
            return None;
        }
        joiner = Some(is_and);
        c.skip_ws();
        terms.push(parse_in_parens(c, depth)?);
    }
    Some(match joiner {
        None => terms.pop().expect("one term"),
        Some(true) => Condition::And(terms),
        Some(false) => Condition::Or(terms),
    })
}

/// A `<media-in-parens>`: a condition or a feature in parentheses, or
/// `<general-enclosed>`, unknown, which anything balanced in
/// parentheses or a function is; `None` when the cursor is at neither.
/// `depth` is how many parentheses are around the cursor.
fn parse_in_parens(c: &mut Cursor<'_>, depth: usize) -> Option<Condition> {
    match c.peek()? {
        Token::Function(_) => {
            c.take_block();
            Some(Condition::Unknown)
        }
        Token::OpenParen => {
            let inner = trim(c.take_block());
            let starts_condition = match inner.first() {
                Some(Token::OpenParen | Token::Function(_)) => true,
                Some(Token::Ident(word)) => word.eq_ignore_ascii_case("not"),
                _ => false,
            };
            if starts_condition {
                if depth >= MAX_NESTING {
                    return Some(Condition::Unknown);
                }
                let mut inside = Cursor::new(inner);
                if let Some(condition) = parse_condition(&mut inside, true, depth + 1) {
                    inside.skip_ws();
                    if inside.at_end() {
                        return Some(condition);
                    }
                }
                return Some(Condition::Unknown);
            }
            Some(parse_feature(inner).map_or(Condition::Unknown, Condition::Feature))
        }
        _ => None,
    }
}

/// A `<media-feature>`'s contents, between its parentheses; `None` for
/// one this engine does not know or a value its feature does not take.
fn parse_feature(tokens: &[Token]) -> Option<Feature> {
    // A range: split at the comparisons, `<`, `<=`, `>`, `>=` and `=`.
    let mut operands: Vec<&[Token]> = Vec::new();
    let mut comparisons: Vec<Comparison> = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < tokens.len() {
        if let Token::Delim(d @ ('<' | '>' | '=')) = tokens[i] {
            let or_equal = d != '=' && tokens.get(i + 1) == Some(&Token::Delim('='));
            comparisons.push(match (d, or_equal) {
                ('<', false) => Comparison::Lt,
                ('<', true) => Comparison::Le,
                ('>', false) => Comparison::Gt,
                ('>', true) => Comparison::Ge,
                _ => Comparison::Eq,
            });
            operands.push(trim(&tokens[start..i]));
            i += if or_equal { 2 } else { 1 };
            start = i;
            continue;
        }
        i += 1;
    }
    operands.push(trim(&tokens[start..]));
    if !comparisons.is_empty() {
        return parse_range(&operands, &comparisons);
    }
    let name_of = |token: &Token| match token {
        Token::Ident(name) => Name::of(name),
        _ => None,
    };
    match tokens {
        [name] => match name_of(name)? {
            (name, Prefix::Exact) => Some(Feature::Boolean(name)),
            _ => None,
        },
        [name, rest @ ..] => {
            let (name, prefix) = name_of(name)?;
            let rest = trim(rest);
            let [Token::Colon, value @ ..] = rest else {
                return None;
            };
            Some(Feature::Plain {
                name,
                prefix,
                value: parse_value(name.kind(), trim(value))?,
            })
        }
        [] => None,
    }
}

/// `<mf-range>` from its operands and the comparisons between them.
fn parse_range(operands: &[&[Token]], comparisons: &[Comparison]) -> Option<Feature> {
    let name_at = |tokens: &[Token]| match tokens {
        [Token::Ident(name)] => match Name::of(name) {
            Some((name, Prefix::Exact)) if name.is_range() => Some(name),
            _ => None,
        },
        _ => None,
    };
    // The bound's comparison read with the bound on its left.
    let flipped = |comparison: Comparison| match comparison {
        Comparison::Lt => Comparison::Gt,
        Comparison::Le => Comparison::Ge,
        Comparison::Gt => Comparison::Lt,
        Comparison::Ge => Comparison::Le,
        Comparison::Eq => Comparison::Eq,
    };
    match (operands, comparisons) {
        // `name op value`: `actual op value`, which is `value flipped actual`.
        ([left, right], [comparison]) if name_at(left).is_some() => {
            let name = name_at(left)?;
            Some(Feature::Range {
                name,
                bounds: vec![(parse_value(name.kind(), right)?, flipped(*comparison))],
            })
        }
        ([left, right], [comparison]) => {
            let name = name_at(right)?;
            Some(Feature::Range {
                name,
                bounds: vec![(parse_value(name.kind(), left)?, *comparison)],
            })
        }
        // `value op name op value`, both `<` or both `>`.
        ([low, name, high], [first, second]) => {
            let name = name_at(name)?;
            let lt = |c: &Comparison| matches!(c, Comparison::Lt | Comparison::Le);
            let gt = |c: &Comparison| matches!(c, Comparison::Gt | Comparison::Ge);
            if !((lt(first) && lt(second)) || (gt(first) && gt(second))) {
                return None;
            }
            Some(Feature::Range {
                name,
                bounds: vec![
                    (parse_value(name.kind(), low)?, *first),
                    (parse_value(name.kind(), high)?, flipped(*second)),
                ],
            })
        }
        _ => None,
    }
}

/// A value of a feature of `kind`; `None` when the feature does not
/// take it.
fn parse_value(kind: Kind, tokens: &[Token]) -> Option<Value> {
    let number = |token: &Token| match token {
        Token::Number { value, .. } if *value >= 0.0 && value.is_finite() => Some(*value),
        _ => None,
    };
    match kind {
        Kind::Length => match tokens {
            [Token::Number { value, .. }] if *value == 0.0 => {
                Some(Value::Length(0.0, LengthUnit::Px))
            }
            [Token::Dimension { value, unit }] if value.is_finite() => {
                let unit = match unit.to_ascii_lowercase().as_str() {
                    "px" => LengthUnit::Px,
                    "in" => LengthUnit::In,
                    "cm" => LengthUnit::Cm,
                    "mm" => LengthUnit::Mm,
                    "q" => LengthUnit::Q,
                    "pt" => LengthUnit::Pt,
                    "pc" => LengthUnit::Pc,
                    "em" => LengthUnit::Em,
                    "rem" => LengthUnit::Rem,
                    "ex" => LengthUnit::Ex,
                    "ch" => LengthUnit::Ch,
                    "vw" => LengthUnit::Vw,
                    "vh" => LengthUnit::Vh,
                    "vmin" => LengthUnit::Vmin,
                    "vmax" => LengthUnit::Vmax,
                    _ => return None,
                };
                Some(Value::Length(*value, unit))
            }
            _ => None,
        },
        Kind::Ratio => {
            let parts: Vec<&Token> = tokens.iter().filter(|t| **t != Token::Whitespace).collect();
            match parts.as_slice() {
                [a] => Some(Value::Ratio(number(a)?, 1.0)),
                [a, Token::Delim('/'), b] => Some(Value::Ratio(number(a)?, number(b)?)),
                _ => None,
            }
        }
        Kind::Resolution => match tokens {
            [Token::Ident(word)] if word.eq_ignore_ascii_case("infinite") => {
                Some(Value::Resolution(f64::INFINITY))
            }
            [Token::Dimension { value, unit }] if *value >= 0.0 && value.is_finite() => {
                let dppx = match unit.to_ascii_lowercase().as_str() {
                    "dppx" | "x" => *value,
                    "dpi" => value / 96.0,
                    "dpcm" => value * 2.54 / 96.0,
                    _ => return None,
                };
                Some(Value::Resolution(dppx))
            }
            _ => None,
        },
        Kind::Integer => match tokens {
            [
                Token::Number {
                    value,
                    is_integer: true,
                },
            ] if value.is_finite() => Some(Value::Number(*value)),
            _ => None,
        },
        Kind::Number => match tokens {
            [Token::Number { value, .. }] if value.is_finite() => Some(Value::Number(*value)),
            _ => None,
        },
        Kind::Boolean => match tokens {
            [
                Token::Number {
                    value,
                    is_integer: true,
                },
            ] if *value == 0.0 || *value == 1.0 => Some(Value::Number(*value)),
            _ => None,
        },
        Kind::Keywords(words) => match tokens {
            [Token::Ident(word)] => words
                .iter()
                .find(|w| word.eq_ignore_ascii_case(w))
                .map(|w| Value::Keyword(w)),
            _ => None,
        },
    }
}

impl MediaQueryList {
    /// Whether the list holds for `media`: an empty list does, and
    /// otherwise any query of it that is true.
    pub fn evaluate(&self, media: &Media) -> bool {
        self.queries.is_empty() || self.queries.iter().any(|q| q.evaluate(media))
    }

    /// Whether the list is empty, which holds for every medium.
    pub fn is_empty(&self) -> bool {
        self.queries.is_empty()
    }
}

impl MediaQuery {
    fn evaluate(&self, media: &Media) -> bool {
        let truth = match self {
            MediaQuery::NotAll => return false,
            MediaQuery::Condition(condition) => condition.evaluate(media),
            MediaQuery::Typed {
                not,
                media_type,
                condition,
            } => {
                let of_type = Truth::of(match media_type {
                    TypeName::All => true,
                    TypeName::Screen => media.media_type == MediaType::Screen,
                    TypeName::Print => media.media_type == MediaType::Print,
                    TypeName::Other => false,
                });
                let both = match condition {
                    None => of_type,
                    Some(condition) => and(of_type, condition.evaluate(media)),
                };
                if *not { both.not() } else { both }
            }
        };
        truth == Truth::True
    }
}

/// Kleene's `and`.
fn and(a: Truth, b: Truth) -> Truth {
    match (a, b) {
        (Truth::False, _) | (_, Truth::False) => Truth::False,
        (Truth::True, Truth::True) => Truth::True,
        _ => Truth::Unknown,
    }
}

impl Condition {
    fn evaluate(&self, media: &Media) -> Truth {
        match self {
            Condition::Feature(feature) => Truth::of(feature.evaluate(media)),
            Condition::Not(inner) => inner.evaluate(media).not(),
            Condition::And(terms) => terms
                .iter()
                .fold(Truth::True, |acc, t| and(acc, t.evaluate(media))),
            Condition::Or(terms) => terms.iter().fold(Truth::False, |acc, t| {
                and(acc.not(), t.evaluate(media).not()).not()
            }),
            Condition::Unknown => Truth::Unknown,
        }
    }
}

/// A feature's value on `media`.
enum Actual {
    /// A length in px, a resolution in dots per pixel, or an integer.
    Number(f64),
    /// A ratio, `width / height`.
    Ratio(f64, f64),
    /// A keyword; `None` when the feature matches none of its keywords,
    /// as `scan` on a device that does not scan.
    Keyword(Option<&'static str>),
}

impl Name {
    fn actual(self, media: &Media) -> Actual {
        let screen = media.media_type == MediaType::Screen;
        let keyword = |word: &'static str| Actual::Keyword(Some(word));
        match self {
            Name::Width => Actual::Number(media.width as f64),
            Name::Height => Actual::Number(media.height as f64),
            Name::DeviceWidth => Actual::Number(media.device_width as f64),
            Name::DeviceHeight => Actual::Number(media.device_height as f64),
            Name::AspectRatio => Actual::Ratio(media.width as f64, media.height as f64),
            Name::DeviceAspectRatio => {
                Actual::Ratio(media.device_width as f64, media.device_height as f64)
            }
            Name::Orientation => keyword(if media.height >= media.width {
                "portrait"
            } else {
                "landscape"
            }),
            Name::Resolution | Name::WebkitDevicePixelRatio => {
                Actual::Number(media.resolution as f64)
            }
            Name::Scan => Actual::Keyword(None),
            Name::Grid => Actual::Number(0.0),
            Name::Update => keyword(if screen { "fast" } else { "none" }),
            Name::OverflowBlock => keyword(if screen { "scroll" } else { "paged" }),
            Name::OverflowInline => keyword("none"),
            Name::Color => Actual::Number(8.0),
            Name::ColorIndex | Name::Monochrome => Actual::Number(0.0),
            Name::ColorGamut => keyword("srgb"),
            Name::Pointer | Name::AnyPointer => keyword(if screen { "fine" } else { "none" }),
            Name::Hover | Name::AnyHover => keyword(if screen { "hover" } else { "none" }),
            Name::PrefersColorScheme => keyword(match media.preferences.color_scheme {
                ColorScheme::Light => "light",
                ColorScheme::Dark => "dark",
            }),
            Name::PrefersReducedMotion => keyword(if media.preferences.reduced_motion {
                "reduce"
            } else {
                "no-preference"
            }),
            Name::PrefersContrast | Name::PrefersReducedTransparency => keyword("no-preference"),
            Name::ForcedColors | Name::InvertedColors | Name::Scripting => keyword("none"),
            Name::DynamicRange | Name::VideoDynamicRange => keyword("standard"),
            Name::DisplayMode => keyword("browser"),
        }
    }
}

impl Feature {
    fn evaluate(&self, media: &Media) -> bool {
        match self {
            // True unless the value is zero, `none` or `no-preference`
            // (Media Queries 4 §2.4.4).
            Feature::Boolean(name) => match name.actual(media) {
                Actual::Number(n) => n != 0.0,
                Actual::Ratio(w, h) => w != 0.0 && h != 0.0,
                Actual::Keyword(word) => word.is_some_and(|w| w != "none" && w != "no-preference"),
            },
            Feature::Plain {
                name,
                prefix,
                value,
            } => {
                let comparison = match prefix {
                    Prefix::Exact => Comparison::Eq,
                    // `min-` is `value <= actual`.
                    Prefix::Min => Comparison::Le,
                    Prefix::Max => Comparison::Ge,
                };
                compare(name.actual(media), value, comparison, media)
            }
            Feature::Range { name, bounds } => bounds
                .iter()
                .all(|(value, comparison)| compare(name.actual(media), value, *comparison, media)),
        }
    }
}

/// `value comparison actual`.
fn compare(actual: Actual, value: &Value, comparison: Comparison, media: &Media) -> bool {
    use std::cmp::Ordering;
    // The order of the value against the actual value.
    let order = match (&actual, value) {
        (Actual::Keyword(word), Value::Keyword(given)) => {
            return comparison == Comparison::Eq && *word == Some(*given);
        }
        (Actual::Number(actual), Value::Length(amount, unit)) => {
            px(*amount, *unit, media).partial_cmp(actual)
        }
        (Actual::Number(actual), Value::Resolution(number) | Value::Number(number)) => {
            number.partial_cmp(actual)
        }
        (Actual::Ratio(w, h), Value::Ratio(a, b)) => {
            // `0 / 0` is degenerate, and no comparison with it holds.
            if (*a == 0.0 && *b == 0.0) || (*w == 0.0 && *h == 0.0) {
                return false;
            }
            // a / b against w / h, without dividing: a * h against w * b.
            (a * h).partial_cmp(&(w * b))
        }
        _ => None,
    };
    let Some(order) = order else {
        return false;
    };
    match comparison {
        Comparison::Lt => order == Ordering::Less,
        Comparison::Le => order != Ordering::Greater,
        Comparison::Gt => order == Ordering::Greater,
        Comparison::Ge => order != Ordering::Less,
        Comparison::Eq => order == Ordering::Equal,
    }
}

/// A length in a query in px: relative units of the initial font size
/// and of the viewport (Media Queries 4 §1.3).
fn px(amount: f64, unit: LengthUnit, media: &Media) -> f64 {
    let (w, h) = (media.width as f64, media.height as f64);
    amount
        * match unit {
            LengthUnit::Px => 1.0,
            LengthUnit::In => 96.0,
            LengthUnit::Cm => 96.0 / 2.54,
            LengthUnit::Mm => 96.0 / 25.4,
            LengthUnit::Q => 96.0 / 25.4 / 4.0,
            LengthUnit::Pt => 96.0 / 72.0,
            LengthUnit::Pc => 16.0,
            LengthUnit::Em | LengthUnit::Rem => INITIAL_FONT_SIZE,
            LengthUnit::Ex | LengthUnit::Ch => INITIAL_FONT_SIZE / 2.0,
            LengthUnit::Vw => w / 100.0,
            LengthUnit::Vh => h / 100.0,
            LengthUnit::Vmin => w.min(h) / 100.0,
            LengthUnit::Vmax => w.max(h) / 100.0,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(source: &str) -> MediaQueryList {
        parse_media_query_list(&css::tokenize(source))
    }

    fn holds(source: &str, media: &Media) -> bool {
        list(source).evaluate(media)
    }

    fn screen(width: f32, height: f32) -> Media {
        Media::screen(width, height)
    }

    fn print(width: f32, height: f32) -> Media {
        Media {
            media_type: MediaType::Print,
            ..Media::screen(width, height)
        }
    }

    #[test]
    fn types_only_and_not() {
        let s = screen(800.0, 600.0);
        let p = print(800.0, 600.0);
        assert!(holds("", &s), "an empty list holds");
        assert!(holds("  ", &p));
        assert!(holds("all", &s) && holds("all", &p));
        assert!(holds("screen", &s) && !holds("screen", &p));
        assert!(holds("print", &p) && !holds("print", &s));
        assert!(holds("SCREEN", &s), "types are case-insensitive");
        assert!(holds("only screen", &s) && !holds("only print", &s));
        assert!(holds("not print", &s) && !holds("not screen", &s));
        // CSS2's other types are recognised and match nothing; so does
        // a type nobody knows.
        for t in [
            "tv", "handheld", "speech", "aural", "braille", "tty", "nonsense",
        ] {
            assert!(!holds(t, &s), "{t}");
            assert!(holds(&format!("not {t}"), &s), "not {t}");
        }
        // Words that are not types make the query `not all`, whatever
        // `not` says.
        for q in ["only", "not", "and", "or", "layer", "not and", "only not"] {
            assert!(!holds(q, &s), "{q}");
        }
        // A list holds when any query does.
        assert!(holds("print, screen", &s));
        assert!(holds("tv, , screen", &s), "a bad query leaves the rest");
        assert!(!holds("print, tv", &s));
    }

    #[test]
    fn the_drafts_examples() {
        let wide = screen(1280.0, 800.0);
        let narrow = screen(300.0, 800.0);
        // Media Queries 4's examples.
        assert!(holds("screen and (color), print and (color)", &wide));
        assert!(holds("(min-width: 20em)", &wide));
        assert!(!holds("(min-width: 20em)", &narrow));
        assert!(holds("(400px <= width <= 1300px)", &wide));
        assert!(!holds("(400px <= width <= 700px)", &wide));
        assert!(holds("(width >= 600px)", &wide) && !holds("(width >= 600px)", &narrow));
        assert!(holds("(600px <= width)", &wide));
        assert!(
            holds("not all and (monochrome)", &wide),
            "not (all and false)"
        );
        assert!(holds("not all and (grid)", &wide));
        assert!(!holds("not screen and (color)", &wide));
        assert!(holds("screen and not (monochrome)", &wide));
        assert!(!holds("(max-weight: 3kg)", &wide), "an unknown feature");
        assert!(
            !holds("not (max-weight: 3kg)", &wide),
            "not of unknown is unknown"
        );
        assert!(
            holds("(color) or (max-weight: 3kg)", &wide),
            "true or unknown"
        );
        assert!(!holds("(monochrome) or (max-weight: 3kg)", &wide));
        assert!(!holds("(color) and (max-weight: 3kg)", &wide));
        assert!(holds("(not (hover: none))", &wide));
        assert!(holds("((color) and (hover)) or (monochrome)", &wide));
    }

    #[test]
    fn what_does_not_parse_is_not_all() {
        let s = screen(800.0, 600.0);
        for q in [
            // `and` and `or` mixed without parentheses.
            "(color) and (hover) or (grid)",
            // `or` after a type.
            "screen and (color) or (hover)",
            // `only` before a condition.
            "only (color)",
            // No whitespace after `and`, which makes a function.
            "screen and(color)",
            // A condition after `not` then more.
            "not (color) and (hover)",
            // A stray token.
            "screen (color)",
            "(color) (hover)",
            "screen and",
            // A closing token that closes nothing, and brackets.
            "(color)]",
            "screen)",
            "(color) and (hover))",
            "(color) and [x]",
            "(color) or [x]",
            "[(color), screen]",
            "@media",
        ] {
            assert!(!holds(q, &s), "{q}");
            assert!(!holds(&format!("not {q}"), &s), "not {q}");
            assert!(holds(&format!("{q}, all"), &s), "{q} leaves the rest");
        }
    }

    #[test]
    fn the_end_closes_blocks_and_commas_in_blocks_do_not_split() {
        let s = screen(800.0, 600.0);
        // css-syntax-3 closes a block the tokens end inside.
        assert!(holds("(color", &s));
        assert!(holds("(width: 800px) and (color", &s));
        assert!(holds("(width < 900px", &s));
        assert!(holds("((color)", &s));
        assert!(holds("(color) or ((nonsense)", &s));
        assert!(!holds("not (color", &s));
        assert!(holds("screen, (color", &s));
        // A comma inside a block is the block's.
        assert!(!holds("[a, (color)]", &s));
        assert!(holds("(width: 800px) , [a, (color)], screen", &s));
        assert!(holds("(color), )", &s), "a stray `)` is a query of its own");
        // Empty queries are `not all`.
        assert!(holds("(color),", &s) && holds(",(color)", &s) && holds("screen,", &s));
        assert!(!holds(" , ", &s));
    }

    #[test]
    fn keywords_need_no_whitespace_but_a_function_is_not_one() {
        let s = screen(800.0, 600.0);
        assert!(holds("(color)and (update)", &s));
        assert!(holds("(color)or (grid)", &s));
        assert!(holds("screen and (color)and (update)", &s));
        // What a comment between them leaves.
        assert!(holds("screen and/**/(color)", &s));
        assert!(holds("(color) and/**/(update)", &s));
        assert!(!holds("not/**/(color)", &s));
        // `and(` is a function token.
        assert!(!holds("(color) and(update)", &s));
        assert!(holds("not  screen, screen", &s));
    }

    #[test]
    fn plain_min_max_and_range_forms() {
        let m = screen(800.0, 600.0);
        for (q, want) in [
            ("(width: 800px)", true),
            ("(width: 801px)", false),
            ("(min-width: 800px)", true),
            ("(max-width: 799.5px)", false),
            ("(MIN-WIDTH: 50EM)", true),
            ("(width > 800px)", false),
            ("(width >= 800px)", true),
            ("(800px < width)", false),
            ("(800px <= width)", true),
            ("(width = 800px)", true),
            ("(799px < width < 801px)", true),
            ("(801px > width > 799px)", true),
            ("(height < 601px)", true),
            ("(width: 0)", false),
            ("(min-width: 0)", true),
            // `<` and `=` must touch.
            ("(width < = 900px)", false),
            // Mixed directions, `=` between three, and two names.
            ("(799px < width > 700px)", false),
            ("(799px = width = 800px)", false),
            ("(width < height)", false),
            // A prefix on a discrete feature, in a boolean or a range.
            ("(min-orientation: portrait)", false),
            ("(min-width)", false),
            ("(min-width > 10px)", false),
            // A unitless length but zero, and a percentage.
            ("(min-width: 10)", false),
            ("(min-width: 10%)", false),
            ("(width: 0.0)", false),
            ("(width: 0px)", false),
            // Lengths of either sign, numbers in any notation.
            ("(min-width: -100px)", true),
            ("(max-width: -100px)", false),
            ("(width: 8e2px)", true),
            ("(width: +800px)", true),
            ("(width:800px)", true),
            ("( width : 800px )", true),
            ("(width >=800px)", true),
            ("(width => 800px)", false),
            ("(width> =800px)", false),
            ("(800px <= width <= 800px)", true),
            ("(400px < width) and (height < 700px)", true),
            // Anything after the value.
            ("(width: 800px !important)", false),
            ("(width: 800px 1)", false),
        ] {
            assert_eq!(holds(q, &m), want, "{q}");
        }
    }

    #[test]
    fn lengths_are_of_the_initial_font_and_the_viewport() {
        let m = screen(800.0, 600.0);
        assert!(holds("(width: 50em)", &m));
        assert!(holds("(width: 50rem)", &m));
        assert!(holds("(width: 100ex)", &m));
        assert!(holds("(width: 100vw)", &m));
        assert!(holds("(height: 100vh)", &m));
        assert!(holds("(height: 100vmin)", &m));
        assert!(holds("(width: 100vmax)", &m));
        assert!(holds("(width: 50pc)", &m));
        assert!(holds("(min-width: 211.6mm)", &m) && !holds("(min-width: 211.7mm)", &m));
        assert!(holds("(min-width: 846Q)", &m) && !holds("(min-width: 847q)", &m));
        assert!(holds("(width: 600pt)", &m));
        assert!(holds("(min-width: 8in)", &m) && !holds("(min-width: 8.5in)", &m));
        assert!(holds("(min-width: 21cm)", &m) && !holds("(min-width: 22cm)", &m));
        assert!(
            !holds("(min-width: 10lh)", &m),
            "a unit this engine has not"
        );
    }

    #[test]
    fn ratios_orientation_and_resolution() {
        let m = screen(1600.0, 900.0);
        for (q, want) in [
            ("(aspect-ratio: 16/9)", true),
            ("(aspect-ratio: 16 / 9)", true),
            ("(aspect-ratio: 32/18)", true),
            ("(min-aspect-ratio: 1)", true),
            ("(max-aspect-ratio: 1/1)", false),
            ("(aspect-ratio > 1.5)", true),
            ("(aspect-ratio: 0/0)", false),
            ("(aspect-ratio: -1/2)", false),
            ("(aspect-ratio: 16px/9)", false),
            ("(aspect-ratio: 1e1/5.625)", true),
            ("(aspect-ratio: 16.0/9.0)", true),
            ("(aspect-ratio: 0/1)", false),
            ("(min-aspect-ratio: 1/0)", false),
            ("(max-aspect-ratio: 1/0)", true),
            ("not (aspect-ratio: -16/9)", false),
            ("(orientation: landscape)", true),
            ("(orientation: portrait)", false),
            ("(orientation)", true),
            ("(resolution: 1dppx)", true),
            ("(resolution: 1x)", true),
            ("(resolution: 96dpi)", true),
            ("(min-resolution: 2dppx)", false),
            ("(resolution < 2x)", true),
            ("(max-resolution: 37.795275590551185dpcm)", true),
            ("(resolution: infinite)", false),
            ("(resolution)", true),
            ("(resolution: 1)", false),
            // The Compatibility Standard's.
            ("(-webkit-device-pixel-ratio: 1)", true),
            ("(-webkit-device-pixel-ratio: 1.0)", true),
            ("(-WEBKIT-MIN-DEVICE-PIXEL-RATIO: 1)", true),
            ("(-webkit-max-device-pixel-ratio: 0.5)", false),
            ("(-webkit-device-pixel-ratio > 0.5)", true),
            ("(-webkit-device-pixel-ratio)", true),
            ("(-webkit-device-pixel-ratio: 1dppx)", false),
            ("(min--webkit-device-pixel-ratio: 1)", false),
            ("(-webkit-min-width: 1px)", false),
            ("(min-device-pixel-ratio: 1)", false),
        ] {
            assert_eq!(holds(q, &m), want, "{q}");
        }
        let mut retina = m;
        retina.resolution = 2.0;
        assert!(holds("(-webkit-min-device-pixel-ratio: 2)", &retina));
        assert!(holds("(min-resolution: 192dpi)", &retina));
        assert!(holds("(orientation: portrait)", &screen(600.0, 600.0)));
        assert!(!holds("(aspect-ratio)", &screen(0.0, 600.0)));
    }

    #[test]
    fn the_devices_features() {
        let s = screen(800.0, 600.0);
        let p = print(800.0, 600.0);
        for (q, on_screen, on_paper) in [
            ("(color)", true, true),
            ("(color: 8)", true, true),
            ("(min-color: 1)", true, true),
            ("(color: 8.5)", false, false),
            ("(color: 8.0)", false, false),
            ("(color: +8)", true, true),
            // A negative integer is a value, which no device has.
            ("(color: -1)", false, false),
            ("not (color: -1)", true, true),
            ("(min-color: -1)", true, true),
            ("(color > -1)", true, true),
            ("(monochrome)", false, false),
            ("(monochrome: 0)", true, true),
            ("(color-index)", false, false),
            ("(color-gamut: srgb)", true, true),
            ("(color-gamut: p3)", false, false),
            ("(grid)", false, false),
            ("(grid: 0)", true, true),
            ("(grid: 2)", false, false),
            ("(scan)", false, false),
            ("(scan: progressive)", false, false),
            ("(update)", true, false),
            ("(update: fast)", true, false),
            ("(update: none)", false, true),
            ("(overflow-block: scroll)", true, false),
            ("(overflow-block: paged)", false, true),
            ("(overflow-inline)", false, false),
            ("(pointer: fine)", true, false),
            ("(any-pointer: coarse)", false, false),
            ("(hover)", true, false),
            ("(any-hover: hover)", true, false),
            ("(hover: none)", false, true),
            ("(pointer: mouse)", false, false),
        ] {
            assert_eq!(holds(q, &s), on_screen, "{q} on a screen");
            assert_eq!(holds(q, &p), on_paper, "{q} on paper");
        }
    }

    #[test]
    fn the_users_preferences() {
        let mut m = screen(800.0, 600.0);
        assert!(holds("(prefers-color-scheme: light)", &m));
        assert!(!holds("(prefers-color-scheme: dark)", &m));
        assert!(holds("(prefers-color-scheme)", &m), "no value is false");
        assert!(!holds("(prefers-reduced-motion)", &m));
        assert!(holds("(prefers-reduced-motion: no-preference)", &m));
        m.preferences = Preferences {
            color_scheme: ColorScheme::Dark,
            reduced_motion: true,
        };
        assert!(holds("(prefers-color-scheme: dark)", &m));
        assert!(holds("(prefers-reduced-motion: reduce)", &m));
        assert!(holds("(prefers-reduced-motion)", &m));
        for (q, want) in [
            ("(prefers-contrast)", false),
            ("(prefers-contrast: no-preference)", true),
            ("(prefers-contrast: more)", false),
            ("(prefers-reduced-transparency: no-preference)", true),
            ("(forced-colors: none)", true),
            ("(forced-colors)", false),
            ("(inverted-colors)", false),
            ("(scripting: none)", true),
            ("(scripting)", false),
            ("(scripting: enabled)", false),
            ("(dynamic-range: standard)", true),
            ("(dynamic-range: high)", false),
            ("(video-dynamic-range: standard)", true),
            ("(display-mode: browser)", true),
            ("(display-mode: standalone)", false),
            // Not this engine's.
            ("(prefers-reduced-data: no-preference)", false),
            ("not (prefers-reduced-data: reduce)", false),
        ] {
            assert_eq!(holds(q, &m), want, "{q}");
        }
    }

    #[test]
    fn general_enclosed_is_unknown() {
        let m = screen(800.0, 600.0);
        assert!(!holds("(color) and foo(bar)", &m));
        assert!(!holds("not foo(bar)", &m));
        assert!(holds("(color) or [x]", &m) || !holds("(color) or [x]", &m));
        assert!(!holds("(width: 800px) and ([nonsense])", &m));
        assert!(holds("(hover) or (anything at all here)", &m));
        assert!(!holds("(anything at all here)", &m));
        assert!(!holds("not (anything at all here)", &m));
        assert!(!holds("((nonsense) and (color))", &m));
        assert!(holds("((nonsense) or (color))", &m));
    }

    #[test]
    fn nesting_past_the_cap_is_unknown_and_costs_no_stack() {
        let m = screen(800.0, 600.0);
        let nested = |depth: usize, inner: &str| {
            format!("{}{inner}{}", "(".repeat(depth), ")".repeat(depth))
        };
        // A condition in `MAX_NESTING` parentheses, the feature's own
        // inside them.
        assert!(holds(&nested(MAX_NESTING, "(color)"), &m));
        assert!(!holds(&nested(MAX_NESTING + 1, "(color)"), &m));
        assert!(holds(
            &format!("(hover) or {}", nested(MAX_NESTING + 1, "(color)")),
            &m
        ));
        // Deeper than any stack could hold, closed or not.
        for depth in [100_000, 300_000] {
            assert!(!holds(&nested(depth, "(color)"), &m));
            assert!(!holds(&"(".repeat(depth), &m));
            assert!(!holds(&"not (".repeat(depth), &m));
            assert!(!holds(&"((color) and ".repeat(depth), &m));
        }
    }
}
