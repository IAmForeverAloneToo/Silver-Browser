//! Counter styles (M21.4): a counter's value written in a style, by CSS
//! Counter Styles 3 §2 and §3, with the simple predefined styles of §6,
//! read once from their normative rules (`assets/counter-styles.css`).
//!
//! A style has a system — `cyclic`, `fixed`, `symbolic`, `alphabetic`,
//! `numeric` or `additive`, or one it `extends` — with its symbols, a
//! range outside which it falls back, a negative sign, a pad, and the
//! prefix and suffix a marker puts around its text. `counter()` and
//! `counters()` write a value without the prefix and suffix
//! ([`represent`](crate::counter_style::represent)); a list item's
//! marker writes them ([`marker`](crate::counter_style::marker)). A name
//! no style has writes as `decimal` (§5), the complex styles of §7 among
//! them, which are not here; `none` writes nothing.
//!
//! A representation is bounded, as §2 allows: one longer than 60 code
//! points is written by the fallback style instead, so no value makes a
//! symbolic style write a million symbols. A fallback that leads back to
//! a style already tried is `decimal` (§3.7).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

use css::Token;

/// The predefined styles' rules, compiled in.
const PREDEFINED: &str = include_str!("../assets/counter-styles.css");

/// The longest representation a style writes, in code points; a longer
/// one is its fallback's (Counter Styles 3 §2: at least 60 must be
/// supported).
pub const MAX_REPRESENTATION: usize = 60;

/// How many fallbacks are followed before `decimal` is used: more than
/// any chain of the predefined styles, which is two long.
const MAX_FALLBACKS: usize = 8;

/// A counter style's algorithm (§3.1).
#[derive(Debug, Clone, PartialEq)]
enum System {
    Cyclic,
    Numeric,
    Alphabetic,
    Symbolic,
    Additive,
    /// `fixed`, from its first symbol value.
    Fixed(i64),
}

/// A counter style, every descriptor given or taken from the style it
/// extends.
#[derive(Debug, Clone, PartialEq)]
struct CounterStyle {
    system: System,
    symbols: Vec<String>,
    /// The additive tuples, by strictly descending weight.
    additive: Vec<(u64, String)>,
    negative: (String, String),
    prefix: String,
    suffix: String,
    /// The ranges, inclusive; `None` for `auto`, the system's own.
    range: Option<Vec<(i64, i64)>>,
    pad: (usize, String),
    fallback: String,
}

impl CounterStyle {
    /// Whether the style writes a negative value with its negative sign
    /// (§3.2): every system but `cyclic` and `fixed`.
    fn uses_negative(&self) -> bool {
        !matches!(self.system, System::Cyclic | System::Fixed(_))
    }

    /// Whether `value` is in the style's range (§3.5).
    fn in_range(&self, value: i64) -> bool {
        match &self.range {
            Some(ranges) => ranges
                .iter()
                .any(|&(low, high)| (low..=high).contains(&value)),
            None => match self.system {
                System::Cyclic | System::Numeric | System::Fixed(_) => true,
                System::Alphabetic | System::Symbolic => value >= 1,
                System::Additive => value >= 0,
            },
        }
    }

    /// The initial representation of `value` (§3.1), `None` when the
    /// system has none for it or it would be longer than
    /// [`MAX_REPRESENTATION`].
    fn initial(&self, value: i64) -> Option<String> {
        let symbols = &self.symbols;
        let n = symbols.len() as i64;
        let text = match self.system {
            System::Cyclic => symbols
                .get((value - 1).rem_euclid(n.max(1)) as usize)?
                .clone(),
            System::Fixed(first) => {
                let index = value.checked_sub(first)?;
                symbols.get(usize::try_from(index).ok()?)?.clone()
            }
            System::Symbolic => {
                if value < 1 || n == 0 {
                    return None;
                }
                let symbol = &symbols[((value - 1) % n) as usize];
                let length = (value + n - 1) / n;
                let chars = symbol.chars().count() as i64;
                if length.saturating_mul(chars) > MAX_REPRESENTATION as i64 {
                    return None;
                }
                symbol.repeat(length as usize)
            }
            System::Alphabetic => {
                if value < 1 || n < 2 {
                    return None;
                }
                let mut digits = Vec::new();
                let mut value = value;
                while value != 0 {
                    value -= 1;
                    digits.push(&symbols[(value % n) as usize]);
                    value /= n;
                }
                digits.iter().rev().map(|s| s.as_str()).collect()
            }
            System::Numeric => {
                if n < 2 {
                    return None;
                }
                if value == 0 {
                    return Some(symbols[0].clone());
                }
                let mut digits = Vec::new();
                let mut value = value;
                while value != 0 {
                    digits.push(&symbols[(value % n) as usize]);
                    value /= n;
                }
                digits.iter().rev().map(|s| s.as_str()).collect()
            }
            System::Additive => {
                if value == 0 {
                    return self
                        .additive
                        .iter()
                        .find(|(weight, _)| *weight == 0)
                        .map(|(_, symbol)| symbol.clone());
                }
                let mut value = u64::try_from(value).ok()?;
                let mut text = String::new();
                for (weight, symbol) in &self.additive {
                    if *weight == 0 || *weight > value {
                        continue;
                    }
                    let reps = value / weight;
                    let chars = (symbol.chars().count() as u64).saturating_mul(reps);
                    if (text.chars().count() as u64).saturating_add(chars)
                        > MAX_REPRESENTATION as u64
                    {
                        return None;
                    }
                    text.push_str(&symbol.repeat(reps as usize));
                    value -= weight * reps;
                    if value == 0 {
                        return Some(text);
                    }
                }
                return None;
            }
        };
        (text.chars().count() <= MAX_REPRESENTATION).then_some(text)
    }

    /// `value` written in this style by §2's algorithm, `None` when it
    /// falls back: outside the range, or with no initial representation.
    /// The pad counts code points where the draft counts grapheme
    /// clusters, which no predefined style's symbols tell apart.
    fn write(&self, value: i64) -> Option<String> {
        if !self.in_range(value) {
            return None;
        }
        let negative = value < 0 && self.uses_negative();
        let mut text = self.initial(if negative {
            value.checked_abs()?
        } else {
            value
        })?;
        let (pad, symbol) = &self.pad;
        let mut short = pad.saturating_sub(text.chars().count());
        if negative {
            let (before, after) = &self.negative;
            short = short.saturating_sub(before.chars().count() + after.chars().count());
        }
        if short > 0 {
            text = symbol.repeat(short) + &text;
        }
        if negative {
            let (before, after) = &self.negative;
            text = format!("{before}{text}{after}");
        }
        Some(text)
    }
}

/// The predefined styles by name, read from their rules once.
fn styles() -> &'static HashMap<String, CounterStyle> {
    static STYLES: OnceLock<HashMap<String, CounterStyle>> = OnceLock::new();
    STYLES.get_or_init(|| {
        let sheet = css::parse_stylesheet(PREDEFINED)
            .expect("the predefined counter styles (assets/counter-styles.css) are valid CSS");
        let mut rules: Vec<(String, Rule)> = Vec::new();
        for rule in &sheet.rules {
            let css::Rule::At(at) = rule else { continue };
            if !at.name.eq_ignore_ascii_case("counter-style") {
                continue;
            }
            let Some(css::Block::Declarations(declarations)) = &at.block else {
                continue;
            };
            let [Token::Ident(name)] = at.prelude.as_slice() else {
                continue;
            };
            rules.push((name.clone(), Rule::read(declarations)));
        }
        let by_name: HashMap<&str, &Rule> = rules.iter().map(|(n, r)| (n.as_str(), r)).collect();
        rules
            .iter()
            .filter_map(|(name, _)| Some((name.clone(), resolve(name, &by_name, 0)?)))
            .collect()
    })
}

/// A `@counter-style` rule's descriptors as written: those it does not
/// give are `None`, to be taken from the style it extends or from their
/// initial values.
#[derive(Debug, Default)]
struct Rule {
    system: Option<RuleSystem>,
    symbols: Option<Vec<String>>,
    additive: Option<Vec<(u64, String)>>,
    negative: Option<(String, String)>,
    prefix: Option<String>,
    suffix: Option<String>,
    range: Option<Option<Vec<(i64, i64)>>>,
    pad: Option<(usize, String)>,
    fallback: Option<String>,
}

/// A rule's `system`.
#[derive(Debug, Clone)]
enum RuleSystem {
    Own(System),
    Extends(String),
}

impl Rule {
    /// The descriptors of `declarations` this module reads; a value that
    /// does not parse is passed over, the last valid one standing.
    fn read(declarations: &[css::Declaration]) -> Rule {
        let mut rule = Rule::default();
        for declaration in declarations {
            let value: Vec<&Token> = declaration
                .value
                .iter()
                .filter(|t| !matches!(t, Token::Whitespace))
                .collect();
            match declaration.name.as_str() {
                "system" => rule.system = read_system(&value).or(rule.system),
                "symbols" => {
                    rule.symbols = value
                        .iter()
                        .map(|t| symbol(t))
                        .collect::<Option<Vec<_>>>()
                        .filter(|s| !s.is_empty())
                        .or(rule.symbols)
                }
                "additive-symbols" => rule.additive = read_additive(&value).or(rule.additive),
                "negative" => {
                    rule.negative = match value.as_slice() {
                        [before] => symbol(before).map(|b| (b, String::new())),
                        [before, after] => symbol(before).zip(symbol(after)),
                        _ => None,
                    }
                    .or(rule.negative)
                }
                "prefix" => rule.prefix = one_symbol(&value).or(rule.prefix),
                "suffix" => rule.suffix = one_symbol(&value).or(rule.suffix),
                "range" => rule.range = read_range(&value).or(rule.range),
                "pad" => {
                    rule.pad = match value.as_slice() {
                        [
                            Token::Number {
                                value,
                                is_integer: true,
                            },
                            s,
                        ]
                        | [
                            s,
                            Token::Number {
                                value,
                                is_integer: true,
                            },
                        ] if *value >= 0.0 => symbol(s).map(|s| (*value as usize, s)),
                        _ => None,
                    }
                    .or(rule.pad)
                }
                "fallback" => {
                    rule.fallback = match value.as_slice() {
                        [Token::Ident(name)] => Some(name.clone()),
                        _ => None,
                    }
                    .or(rule.fallback)
                }
                _ => {}
            }
        }
        rule
    }
}

/// A `<symbol>` (§3.8): a string, or an identifier written as its
/// characters. Images are not taken.
fn symbol(token: &Token) -> Option<String> {
    match token {
        Token::String { value, bad: false } | Token::Ident(value) => Some(value.clone()),
        _ => None,
    }
}

fn one_symbol(value: &[&Token]) -> Option<String> {
    match value {
        [token] => symbol(token),
        _ => None,
    }
}

fn read_system(value: &[&Token]) -> Option<RuleSystem> {
    let word = |t: &Token, w: &str| matches!(t, Token::Ident(i) if i.eq_ignore_ascii_case(w));
    Some(match value {
        [t] if word(t, "cyclic") => RuleSystem::Own(System::Cyclic),
        [t] if word(t, "numeric") => RuleSystem::Own(System::Numeric),
        [t] if word(t, "alphabetic") => RuleSystem::Own(System::Alphabetic),
        [t] if word(t, "symbolic") => RuleSystem::Own(System::Symbolic),
        [t] if word(t, "additive") => RuleSystem::Own(System::Additive),
        [t] if word(t, "fixed") => RuleSystem::Own(System::Fixed(1)),
        [
            t,
            Token::Number {
                value,
                is_integer: true,
            },
        ] if word(t, "fixed") => RuleSystem::Own(System::Fixed(*value as i64)),
        [t, Token::Ident(name)] if word(t, "extends") => RuleSystem::Extends(name.clone()),
        _ => return None,
    })
}

/// `additive-symbols`: `<integer [0,∞]> && <symbol>`, comma-separated,
/// by strictly descending weight.
fn read_additive(value: &[&Token]) -> Option<Vec<(u64, String)>> {
    let mut tuples = Vec::new();
    for tuple in value.split(|t| matches!(t, Token::Comma)) {
        let (weight, s) = match tuple {
            [
                Token::Number {
                    value,
                    is_integer: true,
                },
                s,
            ]
            | [
                s,
                Token::Number {
                    value,
                    is_integer: true,
                },
            ] if *value >= 0.0 => (*value as u64, symbol(s)?),
            _ => return None,
        };
        if tuples.last().is_some_and(|&(last, _)| last <= weight) {
            return None;
        }
        tuples.push((weight, s));
    }
    (!tuples.is_empty()).then_some(tuples)
}

/// `range`: `auto`, or pairs of bounds, comma-separated, `infinite` for
/// either end.
fn read_range(value: &[&Token]) -> Option<Option<Vec<(i64, i64)>>> {
    if let [Token::Ident(word)] = value
        && word.eq_ignore_ascii_case("auto")
    {
        return Some(None);
    }
    let bound = |t: &Token, low: bool| match t {
        Token::Number {
            value,
            is_integer: true,
        } => Some(*value as i64),
        Token::Ident(word) if word.eq_ignore_ascii_case("infinite") => {
            Some(if low { i64::MIN } else { i64::MAX })
        }
        _ => None,
    };
    let mut ranges = Vec::new();
    for pair in value.split(|t| matches!(t, Token::Comma)) {
        let [low, high] = pair else { return None };
        let (low, high) = (bound(low, true)?, bound(high, false)?);
        if low > high {
            return None;
        }
        ranges.push((low, high));
    }
    Some(Some(ranges))
}

/// The style the rule named `name` defines, its `extends` followed; a
/// chain of extensions longer than any here, or a cycle, extends
/// `decimal` (§3.1.7). `None` when the rule defines no style: its system
/// lacks the symbols it needs (§3.1).
fn resolve(name: &str, rules: &HashMap<&str, &Rule>, depth: usize) -> Option<CounterStyle> {
    let rule = rules.get(name)?;
    let base = match &rule.system {
        Some(RuleSystem::Extends(parent)) => {
            let parent = if depth < MAX_FALLBACKS && *parent != name {
                resolve(parent, rules, depth + 1)
            } else {
                None
            };
            Some(parent.or_else(|| resolve("decimal", rules, MAX_FALLBACKS))?)
        }
        _ => None,
    };
    let system = match (&rule.system, &base) {
        (Some(RuleSystem::Own(system)), _) => system.clone(),
        (Some(RuleSystem::Extends(_)), Some(base)) => base.system.clone(),
        _ => System::Cyclic,
    };
    let inherit = |own: &Option<String>, from: fn(&CounterStyle) -> &String, initial: &str| {
        own.clone().unwrap_or_else(|| {
            base.as_ref()
                .map_or_else(|| initial.to_owned(), |b| from(b).clone())
        })
    };
    let style = CounterStyle {
        symbols: rule
            .symbols
            .clone()
            .or_else(|| base.as_ref().map(|b| b.symbols.clone()))
            .unwrap_or_default(),
        additive: rule
            .additive
            .clone()
            .or_else(|| base.as_ref().map(|b| b.additive.clone()))
            .unwrap_or_default(),
        negative: rule
            .negative
            .clone()
            .or_else(|| base.as_ref().map(|b| b.negative.clone()))
            .unwrap_or_else(|| ("-".into(), String::new())),
        prefix: inherit(&rule.prefix, |b| &b.prefix, ""),
        suffix: inherit(&rule.suffix, |b| &b.suffix, ". "),
        range: rule
            .range
            .clone()
            .or_else(|| base.as_ref().map(|b| b.range.clone()))
            .unwrap_or(None),
        pad: rule
            .pad
            .clone()
            .or_else(|| base.as_ref().map(|b| b.pad.clone()))
            .unwrap_or((0, String::new())),
        fallback: inherit(&rule.fallback, |b| &b.fallback, "decimal"),
        system,
    };
    let enough = match style.system {
        System::Cyclic | System::Fixed(_) | System::Symbolic => !style.symbols.is_empty(),
        System::Alphabetic | System::Numeric => style.symbols.len() >= 2,
        System::Additive => !style.additive.is_empty(),
    };
    enough.then_some(style)
}

/// The name a counter style is known by where `counter()`, `counters()`
/// or `list-style-type` names it (§3): the names Counter Styles 3
/// defines, §7's among them, ASCII-lowercased; any other as written,
/// names being case-sensitive.
pub fn canonical_name(name: &str) -> Cow<'_, str> {
    const COMPLEX: [&str; 13] = [
        "japanese-informal",
        "japanese-formal",
        "korean-hangul-formal",
        "korean-hanja-informal",
        "korean-hanja-formal",
        "simp-chinese-informal",
        "simp-chinese-formal",
        "trad-chinese-informal",
        "trad-chinese-formal",
        "cjk-ideographic",
        "ethiopic-numeric",
        "lower-hexadecimal",
        "upper-hexadecimal",
    ];
    let lower = name.to_ascii_lowercase();
    if lower == "none" || styles().contains_key(&lower) || COMPLEX.contains(&lower.as_str()) {
        if lower == name {
            Cow::Borrowed(name)
        } else {
            Cow::Owned(lower)
        }
    } else {
        Cow::Borrowed(name)
    }
}

/// `value` written in the style named `name`, as `counter()` and
/// `counters()` write it: by §2's algorithm, without the prefix and
/// suffix. A name no style has writes as `decimal`; `none` writes
/// nothing.
pub fn represent(name: &str, value: i32) -> String {
    if name == "none" {
        return String::new();
    }
    let value = i64::from(value);
    let styles = styles();
    let mut name = if styles.contains_key(name) {
        name
    } else {
        "decimal"
    };
    let mut tried: Vec<&str> = Vec::new();
    loop {
        let style = &styles[name];
        if let Some(text) = style.write(value) {
            return text;
        }
        tried.push(name);
        let fallback = style.fallback.as_str();
        name = if !styles.contains_key(fallback)
            || tried.contains(&fallback)
            || tried.len() >= MAX_FALLBACKS
        {
            "decimal"
        } else {
            fallback
        };
        if tried.contains(&name) {
            // `decimal` writes every value; this is never reached.
            return value.to_string();
        }
    }
}

/// A marker's text for `value` in the style named `name` (Lists 3 §3.1):
/// the style's prefix, `value` as [`represent`] writes it, and its
/// suffix, the prefix and suffix the style's own even when a fallback
/// writes the value; `decimal`'s for a name no style has. `None` for
/// `none`, which makes no marker.
pub fn marker(name: &str, value: i32) -> Option<String> {
    if name == "none" {
        return None;
    }
    let style = styles().get(name).unwrap_or_else(|| &styles()["decimal"]);
    let representation = represent(
        if styles().contains_key(name) {
            name
        } else {
            "decimal"
        },
        value,
    );
    Some(format!("{}{representation}{}", style.prefix, style.suffix))
}

/// Whether `name` is a style this engine writes in its own symbols: one
/// of §6's.
pub fn is_predefined(name: &str) -> bool {
    styles().contains_key(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(name: &str, value: i32) -> String {
        represent(name, value)
    }

    #[test]
    fn every_predefined_style_is_read() {
        // Counter Styles 3 §6: 28 numeric, 9 alphabetic, 5 symbolic and 2
        // fixed styles.
        assert_eq!(styles().len(), 44);
        for name in [
            "decimal",
            "decimal-leading-zero",
            "arabic-indic",
            "armenian",
            "upper-armenian",
            "lower-armenian",
            "bengali",
            "cambodian",
            "khmer",
            "cjk-decimal",
            "devanagari",
            "georgian",
            "gujarati",
            "gurmukhi",
            "hebrew",
            "kannada",
            "lao",
            "malayalam",
            "mongolian",
            "myanmar",
            "oriya",
            "persian",
            "lower-roman",
            "upper-roman",
            "tamil",
            "telugu",
            "thai",
            "tibetan",
            "lower-alpha",
            "lower-latin",
            "upper-alpha",
            "upper-latin",
            "lower-greek",
            "hiragana",
            "hiragana-iroha",
            "katakana",
            "katakana-iroha",
            "disc",
            "circle",
            "square",
            "disclosure-open",
            "disclosure-closed",
            "cjk-earthly-branch",
            "cjk-heavenly-stem",
        ] {
            assert!(is_predefined(name), "{name}");
        }
    }

    #[test]
    fn the_drafts_examples_write_as_it_says() {
        // §6's examples, each "e.g." of the draft.
        let seq = |name: &str, values: &[i32]| -> Vec<String> {
            values.iter().map(|&v| r(name, v)).collect()
        };
        assert_eq!(
            seq("decimal", &[1, 2, 98, 100, 0, -7]),
            ["1", "2", "98", "100", "0", "-7"]
        );
        assert_eq!(
            seq("decimal-leading-zero", &[1, 98, 100, -5]),
            ["01", "98", "100", "-5"]
        );
        assert_eq!(seq("arabic-indic", &[1, 100]), ["١", "١٠٠"]);
        assert_eq!(
            seq("armenian", &[1, 2, 3, 98, 99, 100]),
            ["Ա", "Բ", "Գ", "ՂԸ", "ՂԹ", "Ճ"]
        );
        assert_eq!(seq("upper-armenian", &[1, 100]), ["Ա", "Ճ"]);
        assert_eq!(seq("lower-armenian", &[1, 98, 100]), ["ա", "ղը", "ճ"]);
        assert_eq!(seq("bengali", &[1, 100]), ["১", "১০০"]);
        assert_eq!(seq("khmer", &[3, 98]), ["៣", "៩៨"]);
        assert_eq!(seq("cjk-decimal", &[1, 98, 100]), ["一", "九八", "一〇〇"]);
        assert_eq!(seq("devanagari", &[100]), ["१००"]);
        assert_eq!(seq("georgian", &[1, 98, 99, 100]), ["ა", "ჟჱ", "ჟთ", "რ"]);
        assert_eq!(
            seq("hebrew", &[1, 15, 16, 98, 100]),
            ["א", "טו", "טז", "צח", "ק"]
        );
        assert_eq!(
            seq("lower-roman", &[1, 2, 3, 98, 99, 100]),
            ["i", "ii", "iii", "xcviii", "xcix", "c"]
        );
        assert_eq!(
            seq("upper-roman", &[1999, 3999, 4]),
            ["MCMXCIX", "MMMCMXCIX", "IV"]
        );
        assert_eq!(seq("thai", &[1, 100]), ["๑", "๑๐๐"]);
        assert_eq!(seq("tibetan", &[98]), ["༩༨"]);
        assert_eq!(
            seq("lower-alpha", &[1, 2, 26, 27, 28]),
            ["a", "b", "z", "aa", "ab"]
        );
        assert_eq!(seq("upper-latin", &[3, 26, 27]), ["C", "Z", "AA"]);
        assert_eq!(seq("lower-greek", &[1, 24, 25, 26]), ["α", "ω", "αα", "αβ"]);
        assert_eq!(
            seq("hiragana", &[1, 2, 48, 49, 50]),
            ["あ", "い", "ん", "ああ", "あい"]
        );
        assert_eq!(seq("hiragana-iroha", &[1, 47, 48]), ["い", "す", "いい"]);
        assert_eq!(seq("katakana", &[1, 48, 49]), ["ア", "ン", "アア"]);
        assert_eq!(seq("katakana-iroha", &[2, 48]), ["ロ", "イイ"]);
        assert_eq!(seq("disc", &[1, 2, -3]), ["•", "•", "•"]);
        assert_eq!(seq("circle", &[1]), ["◦"]);
        assert_eq!(seq("square", &[5]), ["▪"]);
        assert_eq!(seq("disclosure-open", &[1]), ["▾"]);
        assert_eq!(seq("disclosure-closed", &[1]), ["▸"]);
        assert_eq!(seq("cjk-earthly-branch", &[1, 12]), ["子", "亥"]);
        assert_eq!(seq("cjk-heavenly-stem", &[1, 10]), ["甲", "癸"]);
    }

    #[test]
    fn values_out_of_range_fall_back() {
        // Roman to 3999, Armenian to 9999, then `decimal`; the fixed CJK
        // styles to their last symbol, then `cjk-decimal`, which takes no
        // negative value, and so `decimal`.
        assert_eq!(r("upper-roman", 4000), "4000");
        assert_eq!(r("lower-roman", 0), "0");
        assert_eq!(r("lower-roman", -3), "-3");
        assert_eq!(r("armenian", 10000), "10000");
        assert_eq!(r("georgian", 19999), "ჵჰშჟთ");
        assert_eq!(r("georgian", 20000), "20000");
        assert_eq!(r("hebrew", 11000), "11000");
        assert_eq!(r("lower-alpha", 0), "0");
        assert_eq!(r("lower-alpha", -1), "-1");
        assert_eq!(r("cjk-earthly-branch", 13), "一三");
        assert_eq!(r("cjk-earthly-branch", 0), "〇");
        assert_eq!(r("cjk-earthly-branch", -2), "-2");
        // The extremes of a 32-bit counter.
        assert_eq!(r("decimal", i32::MIN), "-2147483648");
        assert_eq!(r("decimal", i32::MAX), "2147483647");
        assert_eq!(r("lower-alpha", i32::MAX), "fxshrxw");
        assert_eq!(r("decimal-leading-zero", i32::MIN), "-2147483648");
    }

    #[test]
    fn names_case_and_unknown_styles() {
        assert_eq!(canonical_name("UPPER-ROMAN"), "upper-roman");
        assert_eq!(canonical_name("Japanese-Informal"), "japanese-informal");
        assert_eq!(canonical_name("NONE"), "none");
        assert_eq!(canonical_name("My-Style"), "My-Style");
        // A name no style has writes as `decimal`.
        assert_eq!(r("My-Style", 12), "12");
        assert_eq!(r("japanese-informal", 3), "3");
        assert_eq!(r("none", 3), "");
        assert_eq!(marker("none", 3), None);
    }

    #[test]
    fn markers_have_the_styles_prefix_and_suffix() {
        assert_eq!(marker("decimal", 3).as_deref(), Some("3. "));
        assert_eq!(marker("disc", 3).as_deref(), Some("• "));
        assert_eq!(marker("cjk-decimal", 3).as_deref(), Some("三、"));
        assert_eq!(marker("hiragana", 1).as_deref(), Some("あ、"));
        assert_eq!(marker("unknown", 3).as_deref(), Some("3. "));
        // The prefix and suffix are the style's even when its fallback
        // writes the value.
        assert_eq!(marker("cjk-earthly-branch", 13).as_deref(), Some("一三、"));
        assert_eq!(marker("lower-roman", 5000).as_deref(), Some("5000. "));
    }

    #[test]
    fn a_long_representation_is_the_fallbacks() {
        // A symbolic style past 60 code points, and an additive one: the
        // predefined styles have none, so these are built here.
        let style = CounterStyle {
            system: System::Symbolic,
            symbols: vec!["*".into()],
            additive: Vec::new(),
            negative: ("-".into(), String::new()),
            prefix: String::new(),
            suffix: String::new(),
            range: None,
            pad: (0, String::new()),
            fallback: "decimal".into(),
        };
        assert_eq!(style.write(60).as_deref(), Some("*".repeat(60).as_str()));
        assert_eq!(style.write(61), None);
        assert_eq!(style.write(i64::from(i32::MAX)), None);
        let additive = CounterStyle {
            system: System::Additive,
            additive: vec![(1, "|".into())],
            ..style
        };
        assert_eq!(additive.write(60).as_deref(), Some("|".repeat(60).as_str()));
        assert_eq!(additive.write(1_000_000_000), None);
    }
}
