//! ECMA-402's `Intl` (gap batch D, D63, `docs/design/text-dom-scripts.md`):
//! what the bootstrap's `Intl` classes compute, by ICU4X's formatters
//! with their compiled data, as natives of the bootstrap's `native`
//! object. Pure computation, which never reaches the host: locales
//! parsed and canonicalized (ECMA-402 section 6.2), negotiated against
//! the languages ICU4X knows (section 9.2), numbers rounded and
//! formatted (section 15.5), plural categories (section 17), strings
//! compared (section 10) and dates formatted (section 11). The
//! bootstrap resolves the options, as section 15.1.2 and its kin say;
//! the natives take them as small integer codes. Formatters are kept
//! per locale and options, a bounded few of each kind.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;
use std::sync::OnceLock;

use fixed_decimal::{Decimal, RoundingIncrement, Sign, SignedRoundingMode, UnsignedRoundingMode};
use icu_calendar::AnyCalendarKind;
use icu_collator::options::{AlternateHandling, CaseLevel, CollatorOptions, Strength};
use icu_collator::{CollatorBorrowed, CollatorPreferences};
use icu_datetime::fieldsets::builder::{DateFields, FieldSetBuilder, ZoneStyle};
use icu_datetime::fieldsets::enums::CompositeFieldSet;
use icu_datetime::input::{Date, DateTime, Time, TimeZone, TimeZoneInfo, UtcOffset, ZonedDateTime};
use icu_datetime::options::{Length, SubsecondDigits, TimePrecision, YearStyle};
use icu_datetime::{DateTimeFormatter, DateTimeFormatterPreferences};
use icu_decimal::options::{DecimalFormatterOptions, GroupingStrategy};
use icu_decimal::{DecimalFormatter, DecimalFormatterPreferences};
use icu_locale::extensions::unicode::{Key, Value as KeywordValue};
use icu_locale::subtags::{Language, Region, Script};
use icu_locale::{
    LanguageIdentifier, Locale, LocaleCanonicalizer, LocaleExpander, TransformResult,
};
use icu_plurals::{PluralCategory, PluralRuleType, PluralRules, PluralRulesOptions};
use rquickjs::{Ctx, Exception, Function, Object, Value};

/// The longest language tag taken, and the most tags a list may hold:
/// bounds the work a hostile list makes, far past any real tag's.
const MAX_TAG: usize = 256;
const MAX_TAGS: usize = 1000;

/// The longest numeric string formatted exactly (ECMA-402 section
/// 15.5.16's strings); a longer one is read as a double.
const MAX_NUMBER_STRING: usize = 1100;

/// The formatters of one kind kept at once; past it, all are let go.
const MAX_CACHED: usize = 32;

/// The locale a negotiation falls back to when nothing asked for is
/// available (ECMA-402 section 6.2.4's DefaultLocale).
const DEFAULT_LOCALE: &str = "en-US";

/// The formatters made, by locale and options.
#[derive(Default)]
struct Cache {
    decimal: HashMap<(String, u8), Rc<DecimalFormatter>>,
    symbols: HashMap<String, Rc<Symbols>>,
    plurals: HashMap<(String, bool), Rc<PluralRules>>,
    collators: HashMap<(String, Vec<i32>), Rc<CollatorBorrowed<'static>>>,
    datetimes: HashMap<(String, Vec<i32>), Rc<DateTimeFormatter<CompositeFieldSet>>>,
}

type Shared = Rc<RefCell<Cache>>;

/// The value of `key` in `map`, made by `make` if it is not there.
fn cached<K: Eq + Hash, V>(
    map: &mut HashMap<K, Rc<V>>,
    key: K,
    make: impl FnOnce() -> Option<V>,
) -> Option<Rc<V>> {
    if let Some(found) = map.get(&key) {
        return Some(found.clone());
    }
    let made = Rc::new(make()?);
    if map.len() >= MAX_CACHED {
        map.clear();
    }
    map.insert(key, made.clone());
    Some(made)
}

fn range_error(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    Exception::throw_range(ctx, message)
}

// ---- Locales (ECMA-402 section 6.2, section 9.2, section 14) ----------

/// A language tag parsed and canonicalized as ECMA-402 section 6.2.1's
/// IsStructurallyValidLanguageTag and CanonicalizeUnicodeLocaleId say:
/// a Unicode BCP 47 locale identifier, its aliases replaced. `None` if
/// it is not one.
fn parse_tag(tag: &str) -> Option<Locale> {
    if tag.is_empty() || tag.len() > MAX_TAG || !tag.is_ascii() || tag.contains('_') {
        return None;
    }
    let mut locale = Locale::try_from_str(tag).ok()?;
    LocaleCanonicalizer::new_extended().canonicalize(&mut locale);
    Some(locale)
}

/// Whether ICU4X has data for the locale's language (ECMA-402 section
/// 9.1's [[AvailableLocales]]): a language the likely-subtags data of
/// the locales CLDR covers knows. Its script, region and variants fall
/// back as ICU4X's data does.
fn available(locale: &Locale) -> bool {
    let language = locale.id.language;
    if language.is_unknown() {
        return false;
    }
    let mut id = LanguageIdentifier::from((language, None, None));
    LocaleExpander::new_common().maximize(&mut id) == TransformResult::Modified
}

/// The tag's Unicode extension keywords, as pairs of key and value.
fn keywords(locale: &Locale) -> Vec<String> {
    let mut pairs = Vec::new();
    for (key, value) in locale.extensions.unicode.keywords.iter() {
        pairs.push(key.to_string());
        pairs.push(value.to_string());
    }
    pairs
}

/// `intlCanonical(tags)`: ECMA-402 section 9.2.1's
/// CanonicalizeLocaleList of strings, duplicates dropped; a tag that is
/// not one throws a RangeError.
fn canonical(ctx: &Ctx<'_>, tags: Vec<String>) -> rquickjs::Result<Vec<String>> {
    if tags.len() > MAX_TAGS {
        return Err(range_error(ctx, "Too many locales"));
    }
    let mut seen: Vec<String> = Vec::new();
    for tag in tags {
        let Some(locale) = parse_tag(&tag) else {
            return Err(range_error(ctx, "Incorrect locale information provided"));
        };
        let text = locale.to_string();
        if !seen.contains(&text) {
            seen.push(text);
        }
    }
    Ok(seen)
}

/// `intlResolve(requested, preferred)`: ECMA-402 section 9.2.2's
/// LookupMatchingLocaleByPrefix over canonical tags: the first that is
/// available, its base name and its Unicode keywords' pairs after it;
/// none, the first of the user's languages that is, or `en-US`.
fn resolve(requested: Vec<String>, preferred: Vec<String>) -> Vec<String> {
    for tag in requested.iter().take(MAX_TAGS) {
        if let Some(locale) = parse_tag(tag)
            && available(&locale)
        {
            let mut found = vec![locale.id.to_string()];
            found.extend(keywords(&locale));
            return found;
        }
    }
    for tag in preferred.iter().take(MAX_TAGS) {
        if let Some(locale) = parse_tag(tag)
            && available(&locale)
        {
            return vec![locale.id.to_string()];
        }
    }
    vec![DEFAULT_LOCALE.to_owned()]
}

/// `intlSupported(tags)`: of canonical tags, those available (ECMA-402
/// section 9.2.10's LookupSupportedLocales).
fn supported(tags: Vec<String>) -> Vec<String> {
    tags.into_iter()
        .take(MAX_TAGS)
        .filter(|tag| parse_tag(tag).is_some_and(|locale| available(&locale)))
        .collect()
}

/// The Unicode keys `Intl.Locale` reads and writes (ECMA-402 section
/// 14.1.1), in the order its options name them.
const LOCALE_KEYS: [&str; 6] = ["ca", "co", "hc", "kf", "kn", "nu"];

/// `intlLocale(tag, options)`: ECMA-402 section 14.1.1's
/// UpdateLanguageId and InsertUnicodeExtensionAndCanonicalize. The
/// options are the language, script and region and then the keys of
/// [`LOCALE_KEYS`], "" for those not given. The answer is the tag, its
/// language, script, region and base name, then the values of
/// [`LOCALE_KEYS`] ("" for those it has not).
fn locale_with(ctx: &Ctx<'_>, tag: &str, options: &[String]) -> rquickjs::Result<Vec<String>> {
    let invalid = || range_error(ctx, "Incorrect locale information provided");
    let mut locale = parse_tag(tag).ok_or_else(invalid)?;
    let option = |i: usize| {
        options
            .get(i)
            .filter(|s| !s.is_empty() && s.len() <= MAX_TAG)
    };
    if let Some(language) = option(0) {
        let language = Language::try_from_str(language).map_err(|_| invalid())?;
        if language.is_unknown() && language.as_str() != "und" {
            return Err(invalid());
        }
        locale.id.language = language;
    }
    if let Some(script) = option(1) {
        locale.id.script = Some(Script::try_from_str(script).map_err(|_| invalid())?);
    }
    if let Some(region) = option(2) {
        locale.id.region = Some(Region::try_from_str(region).map_err(|_| invalid())?);
    }
    for (i, key) in LOCALE_KEYS.iter().enumerate() {
        if let Some(value) = option(3 + i) {
            let key = Key::try_from_str(key).map_err(|_| invalid())?;
            let value = KeywordValue::try_from_str(value).map_err(|_| invalid())?;
            locale.extensions.unicode.keywords.set(key, value);
        }
    }
    // Canonical again, the new subtags' aliases replaced.
    let mut locale = parse_tag(&locale.to_string()).ok_or_else(invalid)?;
    LocaleCanonicalizer::new_extended().canonicalize(&mut locale);
    let mut answer = vec![
        locale.to_string(),
        locale.id.language.to_string(),
        locale.id.script.map(|s| s.to_string()).unwrap_or_default(),
        locale.id.region.map(|r| r.to_string()).unwrap_or_default(),
        locale.id.to_string(),
    ];
    for key in LOCALE_KEYS {
        let value = Key::try_from_str(key)
            .ok()
            .and_then(|key| locale.extensions.unicode.keywords.get(&key))
            .map(|value| value.to_string());
        answer.push(match value.as_deref() {
            Some("") => "true".to_owned(),
            Some(value) => value.to_owned(),
            None => String::new(),
        });
    }
    Ok(answer)
}

/// `intlLikely(tag, maximize)`: the tag with its likely subtags added
/// (UTS #35's Add Likely Subtags), or with those that are likely
/// removed (Remove Likely Subtags), as `Intl.Locale`'s `maximize()` and
/// `minimize()` (ECMA-402 section 14.3.3 and section 14.3.4).
fn likely(ctx: &Ctx<'_>, tag: &str, maximize: bool) -> rquickjs::Result<String> {
    let mut locale =
        parse_tag(tag).ok_or_else(|| range_error(ctx, "Incorrect locale information provided"))?;
    let expander = LocaleExpander::new_extended();
    if maximize {
        expander.maximize(&mut locale.id);
    } else {
        expander.minimize(&mut locale.id);
    }
    Ok(locale.to_string())
}

/// A tag known good, for a formatter's preferences: `en-US` if it is
/// not.
fn locale_of(tag: &str) -> Locale {
    parse_tag(tag)
        .unwrap_or_else(|| Locale::try_from_str(DEFAULT_LOCALE).unwrap_or(Locale::UNKNOWN))
}

// ---- Numbers (ECMA-402 section 15.5) -----------------------------------

/// A number to format: ECMA-402 section 15.5.16's ToIntlMathematicalValue,
/// a finite one with the double it came from, if it did.
enum Num {
    NaN,
    Infinite(bool),
    Finite(Decimal, Option<f64>),
}

/// The digit options of ECMA-402 section 15.1.3, SetNumberFormatDigitOptions,
/// as the bootstrap resolved them, in its integer codes: minimum
/// integer digits, minimum and maximum fraction digits, minimum and
/// maximum significant digits, the rounding type (0 fraction digits, 1
/// significant digits, 2 more precision, 3 less precision), the rounding
/// mode (section 15.5.13's table), whether trailing zeros are
/// stripped from an integer, and the rounding increment (section
/// 15.1.3's 1, 2, 5, 10, 20, 25, 50 up to 5000) as a power of ten and
/// ICU4X's increment of one, two, five or twenty-five of it.
struct Digits {
    min_int: i16,
    min_frac: i16,
    max_frac: i16,
    min_sig: i16,
    max_sig: i16,
    kind: i32,
    mode: SignedRoundingMode,
    strip_integer: bool,
    increment: (i16, RoundingIncrement),
}

/// The integer codes' places, shared by `intlNumber` and `intlPlural`.
const STYLE: usize = 0;
const MIN_INT: usize = 1;
const MIN_FRAC: usize = 2;
const MAX_FRAC: usize = 3;
const MIN_SIG: usize = 4;
const MAX_SIG: usize = 5;
const ROUNDING_TYPE: usize = 6;
const GROUPING: usize = 7;
const SIGN_DISPLAY: usize = 8;
const NOTATION: usize = 9;
const ROUNDING_MODE: usize = 10;
const TRAILING_ZEROS: usize = 11;
const INCREMENT: usize = 12;

fn code(codes: &[i32], at: usize) -> i32 {
    codes.get(at).copied().unwrap_or(0)
}

impl Digits {
    /// The options from their codes, each held to ECMA-402's range:
    /// integer digits 1 to 21, fraction digits 0 to 100, significant
    /// digits 1 to 21.
    fn of(codes: &[i32]) -> Digits {
        let clamp = |at: usize, low: i32, high: i32| code(codes, at).clamp(low, high) as i16;
        let min_frac = clamp(MIN_FRAC, 0, 100);
        let min_sig = clamp(MIN_SIG, 1, 21);
        let unsigned = |mode| SignedRoundingMode::Unsigned(mode);
        Digits {
            min_int: clamp(MIN_INT, 1, 21),
            min_frac,
            max_frac: clamp(MAX_FRAC, 0, 100).max(min_frac),
            min_sig,
            max_sig: clamp(MAX_SIG, 1, 21).max(min_sig),
            kind: code(codes, ROUNDING_TYPE),
            mode: match code(codes, ROUNDING_MODE) {
                1 => SignedRoundingMode::Ceil,
                2 => SignedRoundingMode::Floor,
                3 => unsigned(UnsignedRoundingMode::Expand),
                4 => unsigned(UnsignedRoundingMode::Trunc),
                5 => SignedRoundingMode::HalfCeil,
                6 => SignedRoundingMode::HalfFloor,
                7 => unsigned(UnsignedRoundingMode::HalfTrunc),
                8 => unsigned(UnsignedRoundingMode::HalfEven),
                _ => unsigned(UnsignedRoundingMode::HalfExpand),
            },
            strip_integer: code(codes, TRAILING_ZEROS) == 1,
            increment: increment_of(code(codes, INCREMENT)),
        }
    }

    /// ECMA-402 section 15.5.3, FormatNumericToString's rounding: the
    /// value rounded to its fraction or significant digits (or, for the
    /// rounding priorities, the one of the two with more or less
    /// precision), then padded to the minimum integer digits.
    fn round(&self, value: &Decimal) -> Decimal {
        let magnitude = if value.is_zero() {
            0
        } else {
            value.nonzero_magnitude_start()
        };
        let mut rounded = match self.kind {
            1 => self.to_precision(value, magnitude),
            2 | 3 => {
                let by_sig = self.to_precision(value, magnitude);
                let by_frac = self.to_fixed(value);
                // Their rounding magnitudes (section 15.5.3 steps 6-7).
                let sig_at = magnitude - self.max_sig + 1;
                let frac_at = -self.max_frac;
                let sig_wins = if self.kind == 2 {
                    sig_at <= frac_at
                } else {
                    sig_at > frac_at
                };
                if sig_wins { by_sig } else { by_frac }
            }
            _ => self.to_fixed(value),
        };
        if self.strip_integer {
            rounded.trim_end_if_integer();
        }
        rounded.pad_start(self.min_int);
        rounded
    }

    /// Section 15.5.9, ToRawPrecision.
    fn to_precision(&self, value: &Decimal, magnitude: i16) -> Decimal {
        let mut rounded = value.clone();
        rounded.round_with_mode(magnitude - self.max_sig + 1, self.mode);
        let magnitude = if rounded.is_zero() {
            0
        } else {
            rounded.nonzero_magnitude_start()
        };
        rounded.trim_end();
        rounded.pad_end((magnitude - self.min_sig + 1).min(0));
        rounded
    }

    /// Section 15.5.10, ToRawFixed.
    fn to_fixed(&self, value: &Decimal) -> Decimal {
        let mut rounded = value.clone();
        let (power, increment) = self.increment;
        rounded.round_with_mode_and_increment(-self.max_frac + power, self.mode, increment);
        rounded.trim_end();
        rounded.pad_end(-self.min_frac);
        rounded
    }
}

/// A rounding increment as a power of ten and a multiple of it; one
/// section 15.1.3 does not allow is one.
fn increment_of(mut increment: i32) -> (i16, RoundingIncrement) {
    let mut power = 0;
    while increment >= 10 && increment % 10 == 0 {
        increment /= 10;
        power += 1;
    }
    match increment {
        2 => (power, RoundingIncrement::MultiplesOf2),
        5 => (power, RoundingIncrement::MultiplesOf5),
        25 => (power, RoundingIncrement::MultiplesOf25),
        1 => (power, RoundingIncrement::MultiplesOf1),
        _ => (0, RoundingIncrement::MultiplesOf1),
    }
}

/// A script's number, string or BigInt's digits as a number to format.
fn number_of(value: &Value<'_>) -> Num {
    if let Some(n) = value.as_number() {
        return finite(n);
    }
    let Some(text) = value.as_string().and_then(|s| s.to_string().ok()) else {
        return Num::NaN;
    };
    let text = text.trim();
    if text.len() > MAX_NUMBER_STRING {
        return finite(text.parse::<f64>().unwrap_or(f64::NAN));
    }
    let unsigned = text.trim_start_matches(['+', '-']);
    if unsigned == "Infinity" {
        return Num::Infinite(text.starts_with('-'));
    }
    let plain = !text.contains(['e', 'E', 'x', 'X', 'o', 'O', 'b', 'B']);
    if text.is_empty() {
        return finite(0.0);
    }
    if plain && let Ok(decimal) = Decimal::try_from_str(text) {
        return Num::Finite(decimal, None);
    }
    finite(text.parse::<f64>().unwrap_or(f64::NAN))
}

fn finite(n: f64) -> Num {
    if n.is_nan() {
        return Num::NaN;
    }
    if n.is_infinite() {
        return Num::Infinite(n < 0.0);
    }
    // Rust writes a double as its shortest round-trip digits, in full.
    match Decimal::try_from_str(&format!("{n}")) {
        Ok(decimal) => Num::Finite(decimal, Some(n)),
        Err(_) => Num::NaN,
    }
}

/// The double's digits to round: its shortest round-trip digits, or,
/// when the options ask for digits past those, its exact value, as
/// ICU rounds a double (`shift` the power of ten it is scaled by).
fn digits_to_round(value: Decimal, source: Option<f64>, digits: &Digits, shift: i16) -> Decimal {
    let Some(n) = source else {
        return value;
    };
    if value.is_zero() {
        return value;
    }
    let finest = match digits.kind {
        0 => -digits.max_frac,
        1 => value.nonzero_magnitude_start() - digits.max_sig + 1,
        _ => (-digits.max_frac).min(value.nonzero_magnitude_start() - digits.max_sig + 1),
    };
    if finest >= value.nonzero_magnitude_end() {
        return value;
    }
    // Every double's exact value ends within 1074 places of the point.
    match Decimal::try_from_str(&format!("{:.1100}", n.abs())) {
        Ok(mut exact) => {
            exact.trim_end();
            exact.multiply_pow10(shift);
            exact
        }
        Err(_) => value,
    }
}

/// The separators of a locale's numbers, read from its formatter's
/// output, which tell `formatToParts` what each run is.
struct Symbols {
    decimal: String,
    group: String,
    zero: char,
    /// The size of the integer's last group.
    primary: usize,
}

/// The pattern a percentage takes (CLDR's percentFormats): before and
/// after the number. Most locales write `12%`; these, a space between,
/// and Turkish, the sign first.
fn percent_affixes(locale: &Locale) -> (&'static str, &'static str) {
    match locale.id.language.as_str() {
        "fr" => ("", "\u{202f}%"),
        "de" | "es" | "ru" | "sv" | "nb" | "no" | "nn" | "da" | "fi" | "cs" | "sk" | "bg"
        | "ca" | "et" | "lt" | "lv" | "hr" | "sl" | "uk" | "be" | "ro" | "is" => ("", "\u{a0}%"),
        "tr" => ("%", ""),
        _ => ("", "%"),
    }
}

/// The numbering system a locale's digits are in, by its zero.
fn numbering_system(zero: char) -> &'static str {
    match zero as u32 {
        0x660 => "arab",
        0x6f0 => "arabext",
        0x966 => "deva",
        0x9e6 => "beng",
        0xa66 => "guru",
        0xae6 => "gujr",
        0xb66 => "orya",
        0xbe6 => "tamldec",
        0xc66 => "telu",
        0xce6 => "knda",
        0xd66 => "mlym",
        0xe50 => "thai",
        0xed0 => "laoo",
        0xf20 => "tibt",
        0x1040 => "mymr",
        0x17e0 => "khmr",
        0x1810 => "mong",
        0xff10 => "fullwide",
        _ => "latn",
    }
}

/// The parts a formatted number is written in (ECMA-402 section
/// 15.5.4, PartitionNumberPattern), as pairs of type and text.
struct Parts(Vec<String>);

impl Parts {
    fn push(&mut self, kind: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        self.0.push(kind.to_owned());
        self.0.push(text.to_owned());
    }
}

/// What `intlNumber` needs of a locale: its formatter for the grouping
/// asked for and its separators.
struct NumberContext {
    formatter: Rc<DecimalFormatter>,
    symbols: Rc<Symbols>,
}

fn grouping_of(code: i32) -> (u8, GroupingStrategy) {
    match code {
        0 => (0, GroupingStrategy::Never),
        2 => (2, GroupingStrategy::Always),
        3 => (3, GroupingStrategy::Min2),
        _ => (1, GroupingStrategy::Auto),
    }
}

fn decimal_formatter(cache: &mut Cache, tag: &str, grouping: i32) -> Option<Rc<DecimalFormatter>> {
    let (key, strategy) = grouping_of(grouping);
    cached(&mut cache.decimal, (tag.to_owned(), key), || {
        let locale = locale_of(tag);
        let mut options = DecimalFormatterOptions::default();
        options.grouping_strategy = Some(strategy);
        DecimalFormatter::try_new(DecimalFormatterPreferences::from(&locale), options).ok()
    })
}

fn number_context(cache: &mut Cache, tag: &str, grouping: i32) -> Option<NumberContext> {
    let formatter = decimal_formatter(cache, tag, grouping)?;
    let probe = decimal_formatter(cache, tag, 2)?;
    let symbols = cached(&mut cache.symbols, tag.to_owned(), || {
        let sample = Decimal::try_from_str("1234567.5").ok()?;
        let text = probe.format(&sample).to_string();
        let runs = runs(&text);
        let zero = probe
            .format(&Decimal::from(0))
            .to_string()
            .chars()
            .find(|c| c.is_numeric())
            .unwrap_or('0');
        // The separators: of the runs between the digits of 1,234,567.5
        // grouped, the first is the grouping separator, the last the
        // decimal one.
        let inner: Vec<String> = runs
            .iter()
            .enumerate()
            .filter(|(at, (digits, _))| !digits && *at > 0 && at + 1 < runs.len())
            .map(|(_, (_, text))| text.clone())
            .collect();
        // The last group's size: the digits of 1,234,567's 567.
        let groups: Vec<usize> = runs
            .iter()
            .filter(|(digits, _)| *digits)
            .map(|(_, run)| run.chars().count())
            .collect();
        let primary = groups.iter().rev().nth(1).copied().unwrap_or(3);
        Some(Symbols {
            primary,
            decimal: inner.last().cloned().unwrap_or_else(|| ".".to_owned()),
            group: if inner.len() > 1 {
                inner[0].clone()
            } else {
                ",".to_owned()
            },
            zero,
        })
    })?;
    Some(NumberContext { formatter, symbols })
}

/// `text` with its integer grouped whatever its length, as `useGrouping:
/// "always"` asks: ICU4X's `Always` keeps the locale's minimum grouping
/// digits (`es` writes 1234 but 12.345), so a short integer is grouped
/// here, its last group cut off (a locale's minimum is at most two, so
/// an integer left whole is too short for a second group).
fn group_always(text: &str, symbols: &Symbols) -> String {
    let chars: Vec<char> = text.chars().collect();
    let Some(start) = chars.iter().position(|c| c.is_numeric()) else {
        return text.to_owned();
    };
    let end = chars[start..]
        .iter()
        .position(|c| !c.is_numeric())
        .map_or(chars.len(), |n| start + n);
    let rest: String = chars[end..].iter().collect();
    let primary = symbols.primary;
    if primary == 0 || end - start <= primary || rest.starts_with(&symbols.group) {
        return text.to_owned();
    }
    let mut out: String = chars[..end - primary].iter().collect();
    out.push_str(&symbols.group);
    out.extend(&chars[end - primary..]);
    out
}

/// The text in runs of digits and of other characters.
fn runs(text: &str) -> Vec<(bool, String)> {
    let mut runs: Vec<(bool, String)> = Vec::new();
    for c in text.chars() {
        let digit = c.is_numeric();
        match runs.last_mut() {
            Some((kind, run)) if *kind == digit => run.push(c),
            _ => runs.push((digit, c.to_string())),
        }
    }
    runs
}

/// A formatted decimal's parts: its sign, its integer's digits and
/// groups, its decimal separator and its fraction.
fn decimal_parts(parts: &mut Parts, text: &str, symbols: &Symbols, negative: bool) {
    let runs = runs(text);
    let first_digit = runs.iter().position(|(digit, _)| *digit);
    let last_digit = runs.iter().rposition(|(digit, _)| *digit);
    let sign = if negative { "minusSign" } else { "plusSign" };
    let (Some(first), Some(last)) = (first_digit, last_digit) else {
        parts.push("literal", text);
        return;
    };
    let mut in_fraction = false;
    for (i, (digit, run)) in runs.iter().enumerate() {
        if i < first || i > last {
            parts.push(sign, run);
        } else if *digit {
            parts.push(if in_fraction { "fraction" } else { "integer" }, run);
        } else if !in_fraction && *run == symbols.decimal && i + 1 == last {
            in_fraction = true;
            parts.push("decimal", run);
        } else if *run == symbols.group {
            parts.push("group", run);
        } else {
            parts.push("literal", run);
        }
    }
}

/// The sign a rounded number shows (ECMA-402 section 15.5.4's
/// signDisplay table): 0 auto, 1 never, 2 always, 3 exceptZero, 4
/// negative.
fn sign_for(display: i32, negative: bool, zero: bool) -> Sign {
    match display {
        1 => Sign::None,
        2 if negative => Sign::Negative,
        2 => Sign::Positive,
        3 if zero => Sign::None,
        3 if negative => Sign::Negative,
        3 => Sign::Positive,
        4 if negative && !zero => Sign::Negative,
        4 => Sign::None,
        _ if negative => Sign::Negative,
        _ => Sign::None,
    }
}

/// `intlNumber(locale, codes, unit, x)`: ECMA-402 section 15.5.4's
/// parts of `x` formatted, as pairs of type and text. The codes are
/// [`Digits`]' and the style (0 decimal, 1 percent, 2 currency, 3
/// unit), the grouping (0 never, 1 auto, 2 always, 3 min2), the sign
/// display and the notation (0 standard, 1 scientific, 2 engineering,
/// 3 compact, written as standard). A currency is written by its code
/// and a unit by its name, after or before the number as the locale
/// writes a percentage.
fn format_number(
    cache: &mut Cache,
    tag: &str,
    codes: &[i32],
    unit: &str,
    x: Num,
) -> Option<Vec<String>> {
    let context = number_context(cache, tag, code(codes, GROUPING))?;
    let locale = locale_of(tag);
    let style = code(codes, STYLE);
    let display = code(codes, SIGN_DISPLAY);
    let mut parts = Parts(Vec::new());
    let (before, after) = match style {
        1 => percent_affixes(&locale),
        _ => ("", ""),
    };
    let unit_text = unit.chars().take(64).collect::<String>();
    // A currency's code goes where the locale puts a percentage's sign,
    // before the number unless a space parts them; a unit's name after.
    let spaced = percent_affixes(&locale).1.len() > 1;
    let (unit_before, unit_after) = match style {
        2 => (!spaced, spaced),
        3 => (false, true),
        _ => (false, false),
    };
    let unit_kind = if style == 2 { "currency" } else { "unit" };
    if unit_before {
        parts.push(unit_kind, &unit_text);
        parts.push("literal", "\u{a0}");
    }
    parts.push("percentSign", before.trim_matches('\u{a0}'));
    if before.ends_with('\u{a0}') {
        parts.push("literal", "\u{a0}");
    }
    match x {
        Num::NaN => parts.push("nan", "NaN"),
        Num::Infinite(negative) => {
            match sign_for(display, negative, false) {
                Sign::Negative => parts.push("minusSign", "-"),
                Sign::Positive => parts.push("plusSign", "+"),
                Sign::None => {}
            }
            parts.push("infinity", "\u{221e}");
        }
        Num::Finite(mut value, source) => {
            let negative = value.sign == Sign::Negative;
            value.sign = Sign::None;
            let shift = if style == 1 { 2 } else { 0 };
            value.multiply_pow10(shift);
            let digits = Digits::of(codes);
            let notation = code(codes, NOTATION);
            let (mut rounded, exponent) = if notation == 1 || notation == 2 {
                scientific(&value, &digits, notation == 2)
            } else {
                let value = digits_to_round(value, source, &digits, shift);
                (digits.round(&value), None)
            };
            rounded.sign = sign_for(display, negative, rounded.is_zero());
            let mut text = context.formatter.format(&rounded).to_string();
            if code(codes, GROUPING) == 2 {
                text = group_always(&text, &context.symbols);
            }
            decimal_parts(
                &mut parts,
                &text,
                &context.symbols,
                rounded.sign == Sign::Negative,
            );
            if let Some(exponent) = exponent {
                parts.push("exponentSeparator", "E");
                if exponent < 0 {
                    parts.push("exponentMinusSign", "-");
                }
                let power = Decimal::from(exponent.unsigned_abs());
                parts.push(
                    "exponentInteger",
                    &context.formatter.format(&power).to_string(),
                );
            }
        }
    }
    let after_text = after.trim_start_matches(['\u{a0}', '\u{202f}']);
    if after_text.len() != after.len() {
        parts.push("literal", &after[..after.len() - after_text.len()]);
    }
    parts.push("percentSign", after_text);
    if unit_after {
        parts.push("literal", if style == 3 { " " } else { "\u{a0}" });
        parts.push(unit_kind, &unit_text);
    }
    Some(parts.0)
}

/// Section 15.5.6's scientific and engineering notations: the value as
/// a rounded mantissa and its power of ten, a multiple of three for
/// engineering.
fn scientific(value: &Decimal, digits: &Digits, engineering: bool) -> (Decimal, Option<i16>) {
    let magnitude = if value.is_zero() {
        0
    } else {
        value.nonzero_magnitude_start()
    };
    let step = |m: i16| if engineering { m.div_euclid(3) * 3 } else { m };
    let mut exponent = step(magnitude);
    let mut mantissa = value.clone();
    mantissa.multiply_pow10(-exponent);
    let mut rounded = digits.round(&mantissa);
    // Rounding up may carry into the next power (9.9995 to 10.000).
    let limit = if engineering { 3 } else { 1 };
    if !rounded.is_zero() && rounded.nonzero_magnitude_start() >= limit {
        exponent = step(magnitude + 1);
        let mut mantissa = value.clone();
        mantissa.multiply_pow10(-exponent);
        rounded = digits.round(&mantissa);
    }
    (rounded, Some(exponent))
}

/// `intlNumberingSystem(locale)`: the numbering system the locale's
/// numbers are written in.
fn numbering_system_of(cache: &mut Cache, tag: &str) -> String {
    number_context(cache, tag, 1)
        .map(|context| numbering_system(context.symbols.zero).to_owned())
        .unwrap_or_else(|| "latn".to_owned())
}

// ---- Plural rules (ECMA-402 section 17) -------------------------------

fn plural_rules(cache: &mut Cache, tag: &str, ordinal: bool) -> Option<Rc<PluralRules>> {
    cached(&mut cache.plurals, (tag.to_owned(), ordinal), || {
        let locale = locale_of(tag);
        let kind = if ordinal {
            PluralRuleType::Ordinal
        } else {
            PluralRuleType::Cardinal
        };
        PluralRules::try_new(
            (&locale).into(),
            PluralRulesOptions::default().with_type(kind),
        )
        .ok()
    })
}

/// The categories in the order ECMA-402 section 17.3.4 lists them.
const CATEGORIES: [(PluralCategory, &str); 6] = [
    (PluralCategory::Zero, "zero"),
    (PluralCategory::One, "one"),
    (PluralCategory::Two, "two"),
    (PluralCategory::Few, "few"),
    (PluralCategory::Many, "many"),
    (PluralCategory::Other, "other"),
];

fn category_name(category: PluralCategory) -> &'static str {
    CATEGORIES
        .iter()
        .find(|(c, _)| *c == category)
        .map(|(_, name)| *name)
        .unwrap_or("other")
}

/// `intlPlural(locale, ordinal, codes, n)`: section 17.5.2's
/// ResolvePlural, the number rounded as its digit options say first.
fn plural(cache: &mut Cache, tag: &str, ordinal: bool, codes: &[i32], n: f64) -> String {
    let Some(rules) = plural_rules(cache, tag, ordinal) else {
        return "other".to_owned();
    };
    let Num::Finite(mut value, source) = finite(n) else {
        return "other".to_owned();
    };
    value.sign = Sign::None;
    let digits = Digits::of(codes);
    let rounded = digits.round(&digits_to_round(value, source, &digits, 0));
    category_name(rules.category_for(&rounded)).to_owned()
}

/// `intlPluralCategories(locale, ordinal)`: the categories the rules
/// have, in section 17.3.4's order.
fn plural_categories(cache: &mut Cache, tag: &str, ordinal: bool) -> Vec<String> {
    let Some(rules) = plural_rules(cache, tag, ordinal) else {
        return vec!["other".to_owned()];
    };
    let has: Vec<PluralCategory> = rules.categories().collect();
    CATEGORIES
        .iter()
        .filter(|(category, _)| has.contains(category))
        .map(|(_, name)| (*name).to_owned())
        .collect()
}

// ---- Collation (ECMA-402 section 10) ----------------------------------

/// `intlCompare(locale, codes, x, y)`: section 10.3.3.1's
/// CompareStrings. The codes are the sensitivity (0 base, 1 accent, 2
/// case, 3 variant), whether punctuation is ignored and the usage (0
/// sort, 1 search); the numeric order, the case first and the
/// collation are the locale's keywords.
fn compare(cache: &mut Cache, tag: &str, codes: &[i32], x: &str, y: &str) -> i32 {
    let Some(collator) = cached(
        &mut cache.collators,
        (tag.to_owned(), codes.to_vec()),
        || {
            let mut locale = locale_of(tag);
            if code(codes, 2) == 1
                && let (Ok(key), Ok(value)) = (
                    Key::try_from_str("co"),
                    KeywordValue::try_from_str("search"),
                )
            {
                locale.extensions.unicode.keywords.set(key, value);
            }
            let mut options = CollatorOptions::default();
            let sensitivity = code(codes, 0);
            options.strength = Some(match sensitivity {
                0 | 2 => Strength::Primary,
                1 => Strength::Secondary,
                _ => Strength::Tertiary,
            });
            if sensitivity == 2 {
                options.case_level = Some(CaseLevel::On);
            }
            if code(codes, 1) == 1 {
                options.alternate_handling = Some(AlternateHandling::Shifted);
            }
            CollatorBorrowed::try_new(CollatorPreferences::from(&locale), options).ok()
        },
    ) else {
        return x.cmp(y) as i32;
    };
    collator.compare(x, y) as i32
}

// ---- Dates (ECMA-402 section 11) --------------------------------------

/// The codes' places for `intlDateTime`: each option's value, 0 if it is
/// not asked for. Weekday and era 1 narrow, 2 short, 3 long; year, day,
/// hour, minute and second 1 numeric, 2 2-digit; month 1 numeric, 2
/// 2-digit, 3 narrow, 4 short, 5 long; fractional second digits 1 to 3;
/// the time zone's name 1 short, 2 long, 3 shortOffset, 4 longOffset, 5
/// shortGeneric, 6 longGeneric; the date and time styles 1 full, 2 long,
/// 3 medium, 4 short. The hour cycle is the locale's `hc` keyword.
const WEEKDAY: usize = 0;
const ERA: usize = 1;
const YEAR: usize = 2;
const MONTH: usize = 3;
const DAY: usize = 4;
const HOUR: usize = 5;
const MINUTE: usize = 6;
const SECOND: usize = 7;
const FRACTION: usize = 8;
const ZONE_NAME: usize = 9;
const DATE_STYLE: usize = 10;
const TIME_STYLE: usize = 11;

/// The field set ICU4X's semantic skeletons offer nearest the options
/// asked for (ECMA-402 section 11.1.2's best fit): the date fields the
/// components name, a length by the month's or the weekday's width, the
/// time to its finest field, and the zone. The styles map to CLDR's
/// date and time formats of the same lengths.
fn field_set(codes: &[i32], widen: bool) -> Option<CompositeFieldSet> {
    let at = |i: usize| code(codes, i);
    let mut builder = FieldSetBuilder::new();
    let (date_style, time_style) = (at(DATE_STYLE), at(TIME_STYLE));
    if date_style != 0 || time_style != 0 {
        let length = |style| match style {
            1 | 2 => Length::Long,
            3 => Length::Medium,
            _ => Length::Short,
        };
        builder.length = Some(length(if date_style != 0 {
            date_style
        } else {
            time_style
        }));
        if date_style != 0 {
            builder.date_fields = Some(if date_style == 1 {
                DateFields::YMDE
            } else {
                DateFields::YMD
            });
            builder.year_style = Some(if date_style == 4 {
                YearStyle::Auto
            } else {
                YearStyle::Full
            });
        }
        if time_style != 0 {
            builder.time_precision = Some(if time_style == 4 {
                TimePrecision::Minute
            } else {
                TimePrecision::Second
            });
            builder.zone_style = match time_style {
                1 => Some(ZoneStyle::SpecificLong),
                2 => Some(ZoneStyle::SpecificShort),
                _ => None,
            };
        }
        return builder.build_composite().ok();
    }
    let (y, m, d, e) = (
        at(YEAR) != 0 || at(ERA) != 0,
        at(MONTH) != 0,
        at(DAY) != 0,
        at(WEEKDAY) != 0,
    );
    let fields = match (y, m, d, e) {
        (false, false, false, false) => None,
        (true, _, _, true) => Some(DateFields::YMDE),
        (true, true, false, false) if !widen => Some(DateFields::YM),
        (true, false, false, false) if !widen => Some(DateFields::Y),
        (true, _, _, false) => Some(DateFields::YMD),
        (false, true, _, true) => Some(DateFields::MDE),
        (false, true, true, false) => Some(DateFields::MD),
        (false, true, false, false) if !widen => Some(DateFields::M),
        (false, true, false, false) => Some(DateFields::MD),
        (false, false, true, true) => Some(DateFields::DE),
        (false, false, true, false) => Some(DateFields::D),
        (false, false, false, true) => Some(DateFields::E),
    };
    builder.date_fields = fields;
    let width = match at(MONTH) {
        1 | 2 => Some(Length::Short),
        3 | 4 => Some(Length::Medium),
        5 => Some(Length::Long),
        _ => match at(WEEKDAY) {
            1 => Some(Length::Short),
            2 => Some(Length::Medium),
            3 => Some(Length::Long),
            _ => None,
        },
    };
    builder.length = Some(width.unwrap_or(Length::Short));
    if fields.is_some() && y {
        builder.year_style = Some(if at(ERA) != 0 {
            YearStyle::WithEra
        } else {
            YearStyle::Full
        });
    }
    builder.time_precision = if at(FRACTION) != 0 {
        Some(TimePrecision::Subsecond(match at(FRACTION) {
            1 => SubsecondDigits::S1,
            2 => SubsecondDigits::S2,
            _ => SubsecondDigits::S3,
        }))
    } else if at(SECOND) != 0 {
        Some(TimePrecision::Second)
    } else if at(MINUTE) != 0 {
        Some(TimePrecision::Minute)
    } else if at(HOUR) != 0 {
        Some(TimePrecision::Hour)
    } else {
        None
    };
    builder.zone_style = match at(ZONE_NAME) {
        1 => Some(ZoneStyle::SpecificShort),
        2 => Some(ZoneStyle::SpecificLong),
        3 => Some(ZoneStyle::LocalizedOffsetShort),
        4 => Some(ZoneStyle::LocalizedOffsetLong),
        5 => Some(ZoneStyle::GenericShort),
        6 => Some(ZoneStyle::GenericLong),
        _ => None,
    };
    builder.build_composite().ok()
}

fn date_time_formatter(
    cache: &mut Cache,
    tag: &str,
    codes: &[i32],
) -> Option<Rc<DateTimeFormatter<CompositeFieldSet>>> {
    cached(
        &mut cache.datetimes,
        (tag.to_owned(), codes.to_vec()),
        || {
            let locale = locale_of(tag);
            // A year or month alone with a time ICU4X has no skeleton for:
            // the nearest that has the day too.
            let set = field_set(codes, false).or_else(|| field_set(codes, true))?;
            DateTimeFormatter::try_new(DateTimeFormatterPreferences::from(&locale), set).ok()
        },
    )
}

/// The days since 1970-01-01 as a proleptic Gregorian year, month and
/// day (ECMAScript section 21.4.1's YearFromTime and kin).
fn civil(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// A piece of a date pattern (UTS #35 section 4.8.1): a field, its
/// letter repeated, or literal text.
enum Item {
    Field(char, usize),
    Literal(String),
}

/// A pattern's pieces, its quoted text unquoted.
fn pattern_items(pattern: &str) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut chars = pattern.chars().peekable();
    let mut quoted = false;
    let literal = |items: &mut Vec<Item>, c: char| match items.last_mut() {
        Some(Item::Literal(text)) => text.push(c),
        _ => items.push(Item::Literal(c.to_string())),
    };
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                literal(&mut items, '\'');
            } else {
                quoted = !quoted;
            }
        } else if !quoted && c.is_ascii_alphabetic() {
            match items.last_mut() {
                Some(Item::Field(letter, count)) if *letter == c => *count += 1,
                _ => items.push(Item::Field(c, 1)),
            }
        } else {
            literal(&mut items, c);
        }
    }
    items
}

/// The part type of a pattern's field (ECMA-402 section 11.5.6's
/// table).
fn field_type(letter: char) -> &'static str {
    match letter {
        'G' => "era",
        'y' | 'Y' | 'u' => "year",
        'r' => "relatedYear",
        'U' => "yearName",
        'M' | 'L' => "month",
        'd' => "day",
        'E' | 'c' | 'e' => "weekday",
        'a' | 'b' | 'B' => "dayPeriod",
        'h' | 'H' | 'K' | 'k' => "hour",
        'm' => "minute",
        's' => "second",
        'S' => "fractionalSecond",
        'z' | 'Z' | 'O' | 'v' | 'V' | 'X' | 'x' => "timeZoneName",
        _ => "literal",
    }
}

/// Whether a field is written in digits.
fn numeric_field(letter: char, count: usize) -> bool {
    match letter {
        'y' | 'Y' | 'u' | 'r' | 'd' | 'h' | 'H' | 'K' | 'k' | 'm' | 's' | 'S' | 'D' | 'F' => true,
        'M' | 'L' => count <= 2,
        _ => false,
    }
}

/// The formatted text cut into the pattern's pieces: each literal
/// where the pattern has it, each field the text between. `None` if the
/// text does not follow the pattern.
fn align(items: &[Item], text: &str) -> Option<Vec<(String, String)>> {
    let mut parts = Vec::new();
    let mut steps = 0;
    walk(items, text, 0, &mut parts, &mut steps).then_some(parts)
}

fn walk(
    items: &[Item],
    text: &str,
    at: usize,
    parts: &mut Vec<(String, String)>,
    steps: &mut u32,
) -> bool {
    *steps += 1;
    if *steps > 10_000 {
        return false;
    }
    let Some((item, rest)) = items.split_first() else {
        return at == text.len();
    };
    let tail = &text[at..];
    match item {
        Item::Literal(literal) => {
            if !tail.starts_with(literal.as_str()) {
                return false;
            }
            parts.push(("literal".to_owned(), literal.clone()));
            if walk(rest, text, at + literal.len(), parts, steps) {
                return true;
            }
            parts.pop();
            false
        }
        Item::Field(letter, count) => {
            let mut ends: Vec<usize> = Vec::new();
            if numeric_field(*letter, *count) {
                let digits: usize = tail
                    .chars()
                    .take_while(|c| c.is_numeric())
                    .map(char::len_utf8)
                    .sum();
                if digits > 0 {
                    ends.push(at + digits);
                }
            } else {
                match rest.first() {
                    None => ends.push(text.len()),
                    Some(Item::Literal(next)) => {
                        for (i, _) in tail.char_indices().skip(1) {
                            if tail[i..].starts_with(next.as_str()) {
                                ends.push(at + i);
                            }
                        }
                    }
                    Some(Item::Field(..)) => {
                        let end = tail
                            .char_indices()
                            .skip(1)
                            .find(|(_, c)| c.is_numeric())
                            .map_or(text.len(), |(i, _)| at + i);
                        ends.push(end);
                    }
                }
            }
            for end in ends {
                parts.push((field_type(*letter).to_owned(), text[at..end].to_owned()));
                if walk(rest, text, end, parts, steps) {
                    return true;
                }
                parts.pop();
            }
            false
        }
    }
}

/// The calendar's name as BCP 47 writes it (ECMA-402 section 11.1.2).
fn calendar_name(kind: AnyCalendarKind) -> &'static str {
    match kind {
        AnyCalendarKind::Buddhist => "buddhist",
        AnyCalendarKind::Chinese => "chinese",
        AnyCalendarKind::Coptic => "coptic",
        AnyCalendarKind::Dangi => "dangi",
        AnyCalendarKind::Ethiopian => "ethiopic",
        AnyCalendarKind::EthiopianAmeteAlem => "ethioaa",
        AnyCalendarKind::Hebrew => "hebrew",
        AnyCalendarKind::Indian => "indian",
        AnyCalendarKind::HijriTabularTypeIIFriday => "islamic-civil",
        AnyCalendarKind::HijriSimulatedMecca => "islamic",
        AnyCalendarKind::HijriTabularTypeIIThursday => "islamic-tbla",
        AnyCalendarKind::HijriUmmAlQura => "islamic-umalqura",
        AnyCalendarKind::Iso => "iso8601",
        AnyCalendarKind::Japanese => "japanese",
        AnyCalendarKind::Persian => "persian",
        AnyCalendarKind::Roc => "roc",
        _ => "gregory",
    }
}

const MS_PER_DAY: i64 = 86_400_000;

/// `intlDateTime(locale, codes, zone, t, offset)`: section 11.5.6's
/// FormatDateTimePattern of time value `t` in a zone at `offset` minutes
/// from UTC, as pairs of part type and text, and the pattern ICU4X
/// chose after them. The zone is "UTC", an IANA name, or "" for one
/// known only by its offset.
fn format_date_time(
    cache: &mut Cache,
    tag: &str,
    codes: &[i32],
    zone: &str,
    t: f64,
    offset_minutes: f64,
) -> Option<Vec<String>> {
    if !t.is_finite() || !offset_minutes.is_finite() || offset_minutes.abs() > 24.0 * 60.0 {
        return None;
    }
    let formatter = date_time_formatter(cache, tag, codes)?;
    let offset_seconds = (offset_minutes * 60.0).round() as i32;
    let local = t.floor() as i64 + i64::from(offset_seconds) * 1000;
    let (year, month, day) = civil(local.div_euclid(MS_PER_DAY));
    let in_day = local.rem_euclid(MS_PER_DAY);
    let date = Date::try_new_iso(i32::try_from(year).ok()?, month, day).ok()?;
    let time = Time::try_new(
        (in_day / 3_600_000) as u8,
        (in_day / 60_000 % 60) as u8,
        (in_day / 1000 % 60) as u8,
        (in_day % 1000) as u32 * 1_000_000,
    )
    .ok()?;
    let at = DateTime { date, time };
    let zone = if zone == "UTC" {
        TimeZoneInfo::utc().at_date_time(at)
    } else {
        let id = if zone.is_empty() {
            TimeZone::UNKNOWN
        } else {
            TimeZone::from_iana_id(zone)
        };
        id.with_offset(UtcOffset::try_from_seconds(offset_seconds).ok())
            .at_date_time(at)
    };
    let zoned = ZonedDateTime { date, time, zone };
    let formatted = formatter.format(&zoned);
    let text = formatted.to_string();
    let pattern = formatted.pattern().to_string();
    let pieces = align(&pattern_items(&pattern), &text)
        .unwrap_or_else(|| vec![("literal".to_owned(), text.clone())]);
    let mut answer = Vec::with_capacity(pieces.len() * 2 + 1);
    for (kind, mut value) in pieces {
        let two_digit = match kind.as_str() {
            "month" => code(codes, MONTH) == 2,
            "day" => code(codes, DAY) == 2,
            "hour" => code(codes, HOUR) == 2,
            "minute" => code(codes, MINUTE) == 2,
            "second" => code(codes, SECOND) == 2,
            _ => false,
        };
        if two_digit && value.len() == 1 && value.as_bytes()[0].is_ascii_digit() {
            value.insert(0, '0');
        }
        if kind == "year"
            && code(codes, YEAR) == 2
            && value.len() > 2
            && value.bytes().all(|b| b.is_ascii_digit())
        {
            value = value[value.len() - 2..].to_owned();
        }
        answer.push(kind);
        answer.push(value);
    }
    answer.push(pattern);
    Some(answer)
}

/// `intlDateTimeInfo(locale, codes)`: the calendar the formatter uses
/// and the hour cycle its pattern writes hours in ("" if it writes
/// none), for `resolvedOptions()` (section 11.3.5).
fn date_time_info(cache: &mut Cache, tag: &str, codes: &[i32]) -> Vec<String> {
    let calendar = date_time_formatter(cache, tag, codes)
        .map(|formatter| calendar_name(formatter.calendar().kind()))
        .unwrap_or("gregory");
    // An afternoon, which every hour cycle writes apart.
    let parts = format_date_time(cache, tag, codes, "UTC", 46_800_000.0, 0.0).unwrap_or_default();
    let pattern = parts.last().cloned().unwrap_or_default();
    let cycle = pattern_items(&pattern)
        .iter()
        .find_map(|item| match item {
            Item::Field('h', _) => Some("h12"),
            Item::Field('H', _) => Some("h23"),
            Item::Field('K', _) => Some("h11"),
            Item::Field('k', _) => Some("h24"),
            _ => None,
        })
        .unwrap_or("");
    vec![calendar.to_owned(), cycle.to_owned()]
}

/// `intlSystemZone()`: the IANA name of the system's time zone, as the
/// `TZ` variable or `/etc/localtime` names it, if ICU4X knows it; ""
/// if it is not known, when the bootstrap names the zone by its offset
/// (ECMA-402 section 6.4.3's DefaultTimeZone).
fn system_zone() -> &'static str {
    static ZONE: OnceLock<String> = OnceLock::new();
    ZONE.get_or_init(|| {
        let known = |name: &str| {
            !name.is_empty() && name.len() <= 64 && !TimeZone::from_iana_id(name).is_unknown()
        };
        if let Ok(tz) = std::env::var("TZ") {
            let tz = tz.trim_start_matches(':');
            if known(tz) {
                return tz.to_owned();
            }
        }
        if let Ok(path) = std::fs::read_link("/etc/localtime")
            && let Some(path) = path.to_str()
            && let Some(at) = path.find("zoneinfo/")
        {
            let name = &path[at + "zoneinfo/".len()..];
            if known(name) {
                return name.to_owned();
            }
        }
        String::new()
    })
}

// ---- The natives -------------------------------------------------------

/// Installs the natives on the bootstrap's `native` object, each named
/// `intl...` as its function above says, with a cache of formatters
/// for the realm.
pub(super) fn install<'js>(ctx: &Ctx<'js>, natives: &Object<'js>) -> rquickjs::Result<()> {
    let cache: Shared = Rc::default();
    natives.set(
        "intlCanonical",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, tags: Vec<String>| {
            canonical(&ctx, tags)
        })?,
    )?;
    natives.set("intlResolve", Function::new(ctx.clone(), resolve)?)?;
    natives.set("intlSupported", Function::new(ctx.clone(), supported)?)?;
    natives.set(
        "intlLocale",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, tag: String, options: Vec<String>| locale_with(&ctx, &tag, &options),
        )?,
    )?;
    natives.set(
        "intlLikely",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, tag: String, maximize: bool| {
            likely(&ctx, &tag, maximize)
        })?,
    )?;
    let shared = cache.clone();
    natives.set(
        "intlNumber",
        Function::new(
            ctx.clone(),
            move |tag: String, codes: Vec<i32>, unit: String, x: Value<'js>| -> Vec<String> {
                let x = number_of(&x);
                format_number(&mut shared.borrow_mut(), &tag, &codes, &unit, x).unwrap_or_default()
            },
        )?,
    )?;
    let shared = cache.clone();
    natives.set(
        "intlNumberingSystem",
        Function::new(ctx.clone(), move |tag: String| {
            numbering_system_of(&mut shared.borrow_mut(), &tag)
        })?,
    )?;
    let shared = cache.clone();
    natives.set(
        "intlPlural",
        Function::new(
            ctx.clone(),
            move |tag: String, ordinal: bool, codes: Vec<i32>, n: f64| {
                plural(&mut shared.borrow_mut(), &tag, ordinal, &codes, n)
            },
        )?,
    )?;
    let shared = cache.clone();
    natives.set(
        "intlPluralCategories",
        Function::new(ctx.clone(), move |tag: String, ordinal: bool| {
            plural_categories(&mut shared.borrow_mut(), &tag, ordinal)
        })?,
    )?;
    let shared = cache.clone();
    natives.set(
        "intlCompare",
        Function::new(
            ctx.clone(),
            move |tag: String, codes: Vec<i32>, x: String, y: String| {
                compare(&mut shared.borrow_mut(), &tag, &codes, &x, &y)
            },
        )?,
    )?;
    let shared = cache.clone();
    natives.set(
        "intlDateTime",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  tag: String,
                  codes: Vec<i32>,
                  zone: String,
                  t: f64,
                  offset: f64|
                  -> rquickjs::Result<Vec<String>> {
                format_date_time(&mut shared.borrow_mut(), &tag, &codes, &zone, t, offset)
                    .ok_or_else(|| range_error(&ctx, "Invalid time value"))
            },
        )?,
    )?;
    let shared = cache;
    natives.set(
        "intlDateTimeInfo",
        Function::new(ctx.clone(), move |tag: String, codes: Vec<i32>| {
            date_time_info(&mut shared.borrow_mut(), &tag, &codes)
        })?,
    )?;
    natives.set(
        "intlSystemZone",
        Function::new(ctx.clone(), || system_zone().to_owned())?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_become_gregorian_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(18_263), (2020, 1, 2));
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn a_pattern_cuts_its_text_into_parts() {
        let parts = align(&pattern_items("h:mm a"), "4:09 p. m.").expect("it aligns");
        assert_eq!(
            parts,
            [
                ("hour".to_owned(), "4".to_owned()),
                ("literal".to_owned(), ":".to_owned()),
                ("minute".to_owned(), "09".to_owned()),
                ("literal".to_owned(), " ".to_owned()),
                ("dayPeriod".to_owned(), "p. m.".to_owned()),
            ]
        );
        let parts =
            align(&pattern_items("d 'de' MMMM 'de' y"), "2 de enero de 2020").expect("it aligns");
        assert_eq!(parts[2], ("month".to_owned(), "enero".to_owned()));
        assert!(align(&pattern_items("y"), "x").is_none());
    }
}
