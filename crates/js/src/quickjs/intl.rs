//! ECMA-402's `Intl` (gap batch D, D63, `docs/design/text-dom-scripts.md`):
//! what the bootstrap's `Intl` classes compute, by ICU4X's formatters
//! with their compiled data, as natives of the bootstrap's `native`
//! object. Pure computation, which never reaches the host: locales
//! parsed and canonicalized (ECMA-402 section 6.2), negotiated against
//! the languages ICU4X knows (section 9.2), numbers rounded and
//! formatted (section 15.5), currencies, units and compact forms by
//! `icu_experimental`, plural categories (section 17), strings compared
//! (section 10), dates formatted (section 11) in the zones of
//! `intl_zone`, ranges of each, and what a locale's week, calendars and
//! zones are (the Intl Locale Info proposal). The
//! bootstrap resolves the options, as section 15.1.2 and its kin say;
//! the natives take them as small integer codes. Formatters are kept
//! per locale and options, a bounded few of each kind.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;
use std::sync::OnceLock;

use fixed_decimal::{Decimal, RoundingIncrement, Sign, SignedRoundingMode, UnsignedRoundingMode};
#[allow(deprecated)]
use icu_calendar::cal::hijri::AstronomicalSimulation;
use icu_calendar::cal::hijri::{
    TabularAlgorithm, TabularAlgorithmEpoch, TabularAlgorithmLeapYears, UmmAlQura,
};
use icu_calendar::cal::{
    Buddhist, ChineseTraditional, Coptic, Ethiopian, EthiopianEraStyle, Hebrew, Hijri, Indian,
    Japanese, KoreanTraditional, Persian, Roc,
};
use icu_calendar::{AnyCalendarKind, AsCalendar, Gregorian, Iso};
use icu_collator::options::{AlternateHandling, CaseLevel, CollatorOptions, Strength};
use icu_collator::{CollatorBorrowed, CollatorPreferences};
use icu_datetime::fieldsets::builder::{DateFields, FieldSetBuilder, ZoneStyle};
use icu_datetime::fieldsets::enums::CompositeFieldSet;
use icu_datetime::input::{Date, DateTime, Time, TimeZone, TimeZoneInfo, UtcOffset, ZonedDateTime};
use icu_datetime::options::{Length, SubsecondDigits, TimePrecision, YearStyle};
use icu_datetime::pattern::{DateTimePattern, FixedCalendarDateTimeNames};
use icu_datetime::{DateTimeFormatter, DateTimeFormatterPreferences};
use icu_decimal::options::{DecimalFormatterOptions, GroupingStrategy};
use icu_decimal::{CompactDecimalFormatter, DecimalFormatter, DecimalFormatterPreferences};
use icu_experimental::dimension::currency::CurrencyType;
use icu_experimental::dimension::currency::formatter::CurrencyFormatter;
use icu_experimental::dimension::currency::options::{CurrencyFormatterOptions, CurrencyUsage};
use icu_experimental::dimension::provider::currency::extended::CurrencyExtendedDataV1;
use icu_experimental::dimension::provider::currency::patterns::CurrencyPatternsDataV1;
use icu_experimental::dimension::provider::units::categorized_display_names::{
    UnitsNamesAreaCoreV1, UnitsNamesAreaExtendedV1, UnitsNamesAreaOutlierV1,
    UnitsNamesDurationCoreV1, UnitsNamesDurationExtendedV1, UnitsNamesDurationOutlierV1,
    UnitsNamesLengthCoreV1, UnitsNamesLengthExtendedV1, UnitsNamesLengthOutlierV1,
    UnitsNamesMassCoreV1, UnitsNamesMassExtendedV1, UnitsNamesMassOutlierV1,
    UnitsNamesVolumeCoreV1, UnitsNamesVolumeExtendedV1, UnitsNamesVolumeOutlierV1,
};
use icu_experimental::dimension::provider::units::display_names::UnitsDisplayNames;
use icu_experimental::displaynames::multi::RegionDisplayNames;
use icu_experimental::displaynames::{DisplayNamesOptions, DisplayNamesPreferences};
use icu_experimental::provider::Baked as UnitData;
use icu_locale::extensions::unicode::{Key, Value as KeywordValue};
use icu_locale::subtags::{Language, Region, Script};
use icu_locale::{LanguageIdentifier, Locale, LocaleCanonicalizer, LocaleExpander};
use icu_plurals::{
    PluralCategory, PluralOperands, PluralRuleType, PluralRules, PluralRulesOptions,
};
use icu_provider::marker::ErasedMarker;
use icu_provider::prelude::*;
use icu_time::zone::models::AtTime;
use rquickjs::{Ctx, Exception, Function, Object, Value};
use writeable::TryWriteable;

use super::{intl_dates, intl_zone};

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
    compacts: HashMap<(String, bool), Rc<CompactDecimalFormatter>>,
    currencies: HashMap<(String, String, i32, i32), Rc<AnyCurrencyFormatter>>,
    units: HashMap<(String, String, i32), Rc<UnitNames>>,
    generators: HashMap<String, Rc<intl_dates::Generator>>,
    zone_fields: HashMap<(String, String, String, i64, i32), Option<String>>,
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

/// The languages the reference browser's ICU has data for, which a
/// negotiation may pick (ECMA-402 section 9.1's [[AvailableLocales]]);
/// any other falls back to the default locale there, as here.
const AVAILABLE_LANGUAGES: [&str; 60] = [
    "af", "ak", "am", "ar", "az", "bg", "bn", "bs", "ca", "cs", "da", "de", "el", "en", "es", "et",
    "fa", "fi", "fil", "fr", "gu", "he", "hi", "hr", "hu", "id", "it", "ja", "kk", "kn", "ko",
    "kok", "lt", "lv", "ml", "mr", "ms", "nb", "nl", "no", "pa", "pl", "pt", "ro", "ru", "sd",
    "sk", "sl", "sr", "sv", "sw", "ta", "te", "th", "tr", "uk", "ur", "uz", "vi", "zh",
];

/// Whether the locale's language is one of [`AVAILABLE_LANGUAGES`]. Its
/// script, region and variants fall back as ICU4X's data does.
fn available(locale: &Locale) -> bool {
    AVAILABLE_LANGUAGES.contains(&locale.id.language.as_str())
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
            return Err(range_error(ctx, &format!("Invalid language tag: {tag}")));
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
/// The languages the reference browser's ICU has no number, currency or
/// date data of, having only their names, plural rules and collation
/// (not so the scripts it has data of, `pa-Arab`): their formats fall
/// back to the root locale's, their currencies' names to English's, as
/// ICU's default locale's.
const ROOT_FORMAT_LANGUAGES: [&str; 7] = ["az", "bs", "kk", "kok", "pa", "sd", "uz"];

fn without_format_data(locale: &Locale) -> bool {
    locale.id.script.is_none() && ROOT_FORMAT_LANGUAGES.contains(&locale.id.language.as_str())
}

/// The locale whose data numbers, currencies and units are formatted
/// by: the tag's, the root's for [`ROOT_FORMAT_LANGUAGES`] (the tag's
/// keywords kept), and Punjabi in Arabic script with its Persian digits
/// as the reference browser writes it.
fn data_locale(tag: &str) -> Locale {
    let mut locale = locale_of(tag);
    if without_format_data(&locale) {
        locale.id = LanguageIdentifier::UNKNOWN;
    }
    if locale.id.language.as_str() == "pa"
        && locale
            .id
            .script
            .is_some_and(|script| script.as_str() == "Arab")
        && let (Ok(key), Ok(value)) = (
            Key::try_from_str("nu"),
            KeywordValue::try_from_str("arabext"),
        )
        && locale.extensions.unicode.keywords.get(&key).is_none()
    {
        locale.extensions.unicode.keywords.set(key, value);
    }
    locale
}

/// The locale whose data dates are formatted by: the tag's, the root's
/// for [`ROOT_FORMAT_LANGUAGES`] (the tag's keywords kept).
fn date_locale(tag: &str) -> Locale {
    let mut locale = data_locale(tag);
    if without_format_data(&locale_of(tag)) {
        locale.id = LanguageIdentifier::UNKNOWN;
    }
    locale
}

/// The locale plural rules are read for: the tag's without its script,
/// rules being a language's (and some regions'), where a script's
/// locale would fall back to the root's (`sr-Latn`).
fn plural_locale(tag: &str) -> Locale {
    let mut locale = locale_of(tag);
    locale.id.script = None;
    locale
}

fn locale_of(tag: &str) -> Locale {
    parse_tag(tag)
        .unwrap_or_else(|| Locale::try_from_str(DEFAULT_LOCALE).unwrap_or(Locale::UNKNOWN))
}

// ---- Numbers (ECMA-402 section 15.5) -----------------------------------

/// A number to format: ECMA-402 section 15.5.16's ToIntlMathematicalValue,
/// a double's finite value its shortest round-trip digits, as ICU rounds
/// it.
enum Num {
    NaN,
    Infinite(bool),
    Finite(Decimal),
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
const CURRENCY_DISPLAY: usize = 13;
const CURRENCY_SIGN: usize = 14;
const UNIT_DISPLAY: usize = 15;
const COMPACT_DISPLAY: usize = 16;

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
        return Num::Finite(decimal);
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
        Ok(decimal) => Num::Finite(decimal),
        Err(_) => Num::NaN,
    }
}

/// The separators of a locale's numbers, read from its formatter's
/// output, which tell `formatToParts` what each run is.
#[derive(Clone)]
struct Symbols {
    decimal: String,
    group: String,
    zero: char,
    /// The size of the integer's last group.
    primary: usize,
}

/// The minus and plus signs the reference browser writes where ICU4X's
/// data has others.
const SIGNS: [(&str, &str, &str); 1] = [("pa-Arab", "\u{200e}-\u{200e}", "\u{200e}+\u{200e}")];

/// The signs of [`SIGNS`] for a locale.
fn signs_of(locale: &Locale) -> Option<(&'static str, &'static str)> {
    let rows: Vec<(&str, (&str, &str))> = SIGNS
        .iter()
        .map(|(name, minus, plus)| (*name, (*minus, *plus)))
        .collect();
    nearest(&rows, locale)
}

/// The grouping and decimal separators the reference browser writes
/// where ICU4X's data has others (in numbers, or in currencies' alone).
const SEPARATORS: [(&str, bool, &str, &str); 5] = [
    ("fr-CH", false, "\u{202f}", ","),
    ("fr-CH", true, "\u{202f}", "."),
    ("de-AT", true, ".", ","),
    ("pa-Arab", false, "\u{66c}", "\u{66b}"),
    ("pa-Arab", true, "\u{66c}", "\u{66b}"),
];

/// The separators of [`SEPARATORS`] for a locale's numbers or currencies.
fn separators_of(locale: &Locale, currency: bool) -> Option<(&'static str, &'static str)> {
    let rows: Vec<(&str, (&str, &str))> = SEPARATORS
        .iter()
        .filter(|(_, of_currency, _, _)| *of_currency == currency)
        .map(|(name, _, group, decimal)| (*name, (*group, *decimal)))
        .collect();
    let keys = locale_keys(locale);
    // Only the locale's own entry, not its language's.
    rows.iter()
        .find(|(name, _)| keys.first().is_some_and(|key| key == name))
        .map(|(_, value)| *value)
}

/// The patterns a percentage takes, positive and negative, "{0}" its
/// digits, as the reference browser writes them, where they are not
/// `{0}%` and `-{0}%`; a locale left out takes its parent's.
const PERCENT_PATTERNS: [(&str, &str, &str); 28] = [
    (
        "ar",
        "{0}\u{200e}%\u{200e}",
        "\u{200e}-{0}\u{200e}%\u{200e}",
    ),
    ("ca", "{0}\u{a0}%", "-{0}\u{a0}%"),
    ("cs", "{0}\u{a0}%", "-{0}\u{a0}%"),
    ("da", "{0}\u{a0}%", "-{0}\u{a0}%"),
    ("de", "{0}\u{a0}%", "-{0}\u{a0}%"),
    ("es", "{0}\u{a0}%", "-{0}\u{a0}%"),
    ("et", "{0}%", "\u{2212}{0}%"),
    ("fa", "{0}\u{66a}", "\u{200e}\u{2212}{0}\u{66a}"),
    ("fi", "{0}\u{a0}%", "\u{2212}{0}\u{a0}%"),
    ("fr", "{0}\u{a0}%", "-{0}\u{a0}%"),
    ("he", "{0}%", "\u{200e}-{0}%"),
    ("hr", "{0}\u{a0}%", "\u{2212}{0}\u{a0}%"),
    ("lt", "{0}\u{a0}%", "\u{2212}{0}\u{a0}%"),
    ("nb", "{0}\u{a0}%", "\u{2212}{0}\u{a0}%"),
    ("no", "{0}\u{a0}%", "\u{2212}{0}\u{a0}%"),
    ("ro", "{0}\u{a0}%", "-{0}\u{a0}%"),
    ("ru", "{0}\u{a0}%", "-{0}\u{a0}%"),
    ("sk", "{0}\u{a0}%", "-{0}\u{a0}%"),
    ("sl", "{0}\u{a0}%", "\u{2212}{0}\u{a0}%"),
    ("sv", "{0}\u{a0}%", "\u{2212}{0}\u{a0}%"),
    ("tr", "%{0}", "-%{0}"),
    ("ur", "{0}%", "\u{200e}-{0}%"),
    ("es-419", "{0}%", "-{0}%"),
    ("fr-CH", "{0}%", "-{0}%"),
    ("de-CH", "{0}%", "-{0}%"),
    ("ar-EG", "{0}\u{66a}\u{61c}", "\u{61c}-{0}\u{66a}\u{61c}"),
    ("ar-SA", "{0}\u{66a}\u{61c}", "\u{61c}-{0}\u{66a}\u{61c}"),
    ("pa-Arab", "{0}\u{66a}", "\u{200e}-\u{200e}{0}\u{66a}"),
];

/// The separator of a number's exponent where it is not `E`, as the
/// reference browser writes it.
const EXPONENT_SEPARATORS: [(&str, &str); 12] = [
    ("el", "e"),
    ("et", "\u{d7}10^"),
    ("fa", "\u{d7}\u{6f1}\u{6f0}^"),
    ("lt", "\u{d7}10^"),
    ("sk", "e"),
    ("sl", "e"),
    ("sv", "\u{d7}10^"),
    ("uk", "\u{415}"),
    ("en-AU", "e"),
    ("ar-EG", "\u{623}\u{633}"),
    ("ar-SA", "\u{623}\u{633}"),
    ("pa-Arab", "\u{d7}\u{6f1}\u{6f0}^"),
];

/// The keys a locale's entries in the reference browser's tables are
/// found by, nearest first: its language and script, then its language
/// and region and their parents (CLDR's parent locales), then its
/// language.
fn locale_keys(locale: &Locale) -> Vec<String> {
    let language = match locale.id.language.as_str() {
        "no" => "nb",
        language => language,
    };
    let mut keys = Vec::new();
    if let Some(script) = locale.id.script {
        keys.push(format!("{language}-{script}"));
    }
    let mut key = Some(match locale.id.region {
        Some(region) => format!("{language}-{region}"),
        None => language.to_owned(),
    });
    while let Some(next) = key {
        key = parent_of(&next);
        keys.push(next);
    }
    keys
}

/// The entry of a table by locale nearest the locale.
fn nearest<T: Copy>(table: &[(&str, T)], locale: &Locale) -> Option<T> {
    locale_keys(locale).iter().find_map(|key| {
        table
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| *value)
    })
}

/// The locale's percent pattern for a sign: its negative one for a
/// minus, that one with a plus for a plus.
fn percent_pattern(locale: &Locale, sign: Sign) -> String {
    let patterns: Vec<(&str, (&str, &str))> = PERCENT_PATTERNS
        .iter()
        .map(|(name, positive, negative)| (*name, (*positive, *negative)))
        .collect();
    let (positive, negative) = nearest(&patterns, locale).unwrap_or(("{0}%", "-{0}%"));
    match sign {
        Sign::None => positive.to_owned(),
        Sign::Negative => negative.to_owned(),
        Sign::Positive => negative.replace(['-', '\u{2212}'], "+"),
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
        let locale = data_locale(tag);
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

/// A number rounded as its options say and written without its sign:
/// the sign it shows, the parts of its digits (and of its exponent and
/// compact form), and the value rounded (`None` for NaN and the
/// infinities), which picks the plural forms of its affixes.
struct Written {
    sign: Sign,
    parts: Parts,
    value: Option<Decimal>,
    /// For compact notation: the digits' parts alone, and the compact
    /// form's abbreviation, which a currency's compact pattern places.
    compact: Option<(Parts, String)>,
}

/// Rounds and writes `x` for `intlNumber`, unsigned: section 15.5.3's
/// rounding, then the notation, compact as ICU does (section 15.5.3's
/// ComputeExponent: the power the locale abbreviates the magnitude by,
/// one more if rounding carried over).
fn write_number(cache: &mut Cache, tag: &str, codes: &[i32], x: Num) -> Option<Written> {
    let context = number_context(cache, tag, code(codes, GROUPING))?;
    let display = code(codes, SIGN_DISPLAY);
    let mut parts = Parts(Vec::new());
    let mut value = match x {
        Num::NaN => {
            parts.push("nan", "NaN");
            return Some(Written {
                sign: Sign::None,
                parts,
                value: None,
                compact: None,
            });
        }
        Num::Infinite(negative) => {
            parts.push("infinity", "\u{221e}");
            return Some(Written {
                sign: sign_for(display, negative, false),
                parts,
                value: None,
                compact: None,
            });
        }
        Num::Finite(value) => value,
    };
    let negative = value.sign == Sign::Negative;
    value.sign = Sign::None;
    let shift = if code(codes, STYLE) == 1 { 2 } else { 0 };
    value.multiply_pow10(shift);
    let digits = Digits::of(codes);
    let notation = code(codes, NOTATION);
    let mut compact = None;
    let (rounded, exponent) = if notation == 1 || notation == 2 {
        scientific(&value, &digits, notation == 2)
    } else if notation == 3 {
        let formatter = compact_formatter(cache, tag, code(codes, COMPACT_DISPLAY) == 1)?;
        let (mantissa, power) = compact_round(&value, &digits, &formatter);
        compact = Some((formatter, power));
        (mantissa, None)
    } else {
        (digits.round(&value), None)
    };
    let sign = sign_for(display, negative, rounded.is_zero());
    let mut text = context.formatter.format(&rounded).to_string();
    if code(codes, GROUPING) == 2 {
        text = group_always(&text, &context.symbols);
    }
    let mut number = Parts(Vec::new());
    let symbols = match separators_of(&locale_of(tag), code(codes, STYLE) == 2) {
        Some((group, decimal)) => {
            let symbols = Symbols {
                group: group.to_owned(),
                decimal: decimal.to_owned(),
                ..context.symbols.as_ref().clone()
            };
            // Each separator swapped at once, the two may trade places.
            text = text
                .chars()
                .map(|c| {
                    if context.symbols.group.starts_with(c) {
                        symbols.group.clone()
                    } else if context.symbols.decimal.starts_with(c) {
                        symbols.decimal.clone()
                    } else {
                        c.to_string()
                    }
                })
                .collect();
            Rc::new(symbols)
        }
        None => context.symbols.clone(),
    };
    decimal_parts(&mut number, &text, &symbols, false);
    if let Some(exponent) = exponent {
        let separator = nearest(&EXPONENT_SEPARATORS, &locale_of(tag)).unwrap_or("E");
        number.push("exponentSeparator", separator);
        if exponent < 0 {
            number.push("exponentMinusSign", "-");
        }
        let power = Decimal::from(exponent.unsigned_abs());
        number.push(
            "exponentInteger",
            &context.formatter.format(&power).to_string(),
        );
    }
    let mut abbreviation = None;
    match &compact {
        Some((formatter, power)) if *power > 0 => {
            let template = formatter
                .format_with_exponent(&rounded, *power)
                .map(|t| t.to_string())
                .unwrap_or_default();
            match split_template(&template) {
                Some((before, after)) => {
                    affix_parts(&mut parts, &before, "compact");
                    parts.0.extend(number.0.iter().cloned());
                    affix_parts(&mut parts, &after, "compact");
                    let text = format!("{before}{after}");
                    abbreviation = Some((number, text.trim_matches(spacing).to_owned()));
                }
                None => parts.push("compact", &template),
            }
        }
        _ => parts.0.extend(number.0),
    }
    let mut value = rounded;
    if let Some((_, power)) = compact {
        value.multiply_pow10(i16::from(power));
    }
    Some(Written {
        sign,
        parts,
        value: Some(value),
        compact: abbreviation,
    })
}

/// Section 15.5.3's ComputeExponent for compact notation: the mantissa
/// rounded and the power of ten it is scaled by.
fn compact_round(
    value: &Decimal,
    digits: &Digits,
    formatter: &CompactDecimalFormatter,
) -> (Decimal, u8) {
    if value.is_zero() {
        return (digits.round(value), 0);
    }
    let magnitude = value.nonzero_magnitude_start();
    let scaled = |power: u8| {
        let mut mantissa = value.clone();
        mantissa.multiply_pow10(-i16::from(power));
        digits.round(&mantissa)
    };
    let power = formatter.compact_exponent_for_magnitude(magnitude);
    let rounded = scaled(power);
    if rounded.is_zero() || rounded.nonzero_magnitude_start() + i16::from(power) == magnitude {
        return (rounded, power);
    }
    let power = formatter.compact_exponent_for_magnitude(magnitude + 1);
    (scaled(power), power)
}

fn compact_formatter(
    cache: &mut Cache,
    tag: &str,
    long: bool,
) -> Option<Rc<CompactDecimalFormatter>> {
    cached(&mut cache.compacts, (tag.to_owned(), long), || {
        let locale = data_locale(tag);
        let mut options = DecimalFormatterOptions::default();
        options.grouping_strategy = Some(GroupingStrategy::Min2);
        if long {
            CompactDecimalFormatter::try_new_long((&locale).into(), options.into()).ok()
        } else {
            CompactDecimalFormatter::try_new_short((&locale).into(), options.into()).ok()
        }
    })
}

/// A formatted template's text before and after its number: the runs
/// before its first digit and after its last; `None` if it has none.
fn split_template(text: &str) -> Option<(String, String)> {
    let first = text.char_indices().find(|(_, c)| c.is_numeric())?.0;
    let (last, c) = text.char_indices().rfind(|(_, c)| c.is_numeric())?;
    Some((
        text[..first].to_owned(),
        text[last + c.len_utf8()..].to_owned(),
    ))
}

/// Whether a character is spacing or a direction mark, which an affix's
/// parts write as literals.
fn spacing(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{200e}' | '\u{200f}' | '\u{61c}')
}

/// An affix's parts: its signs, its spacing and brackets as literals,
/// and the rest as one part of `kind` (a currency's symbol, a unit's
/// name, a compact form's abbreviation).
fn affix_parts(parts: &mut Parts, text: &str, kind: &'static str) {
    let mut runs: Vec<(&str, String)> = Vec::new();
    for c in text.chars() {
        let mut this = match c {
            '-' | '\u{2212}' => "minusSign",
            '+' => "plusSign",
            '(' | ')' => "literal",
            c if spacing(c) => "literal",
            _ => kind,
        };
        // A direction mark is a literal of its own, as ICU's fields leave
        // it out of a sign's.
        if matches!(c, '\u{200e}' | '\u{200f}' | '\u{61c}') {
            this = "literal";
        }
        match runs.last_mut() {
            Some((kind, run)) if *kind == this => run.push(c),
            _ => runs.push((this, c.to_string())),
        }
    }
    // Spacing within a name ("US dollars") is the name's.
    let mut merged: Vec<(&str, String)> = Vec::new();
    for (at, (this, run)) in runs.iter().enumerate() {
        let inner = *this == "literal"
            && run.chars().all(char::is_whitespace)
            && at > 0
            && runs[at - 1].0 == kind
            && runs.get(at + 1).is_some_and(|(next, _)| *next == kind);
        match merged.last_mut() {
            Some((last, text)) if *last == kind && (inner || *this == kind) => text.push_str(run),
            _ => merged.push((this, run.clone())),
        }
    }
    for (this, run) in merged {
        parts.push(this, &run);
    }
}

/// A compact currency's affix's parts: its abbreviation a compact
/// part, the rest a currency's.
fn compact_affix_parts(parts: &mut Parts, text: &str, abbreviation: &str) {
    match text.find(abbreviation).filter(|_| !abbreviation.is_empty()) {
        Some(at) => {
            affix_parts(parts, &text[..at], "currency");
            parts.push("compact", abbreviation);
            affix_parts(parts, &text[at + abbreviation.len()..], "currency");
        }
        None => affix_parts(parts, text, "currency"),
    }
}

/// The sign's text in the locale, as its formatter writes it.
fn sign_text(cache: &mut Cache, tag: &str, sign: Sign) -> String {
    if sign == Sign::None {
        return String::new();
    }
    if let Some((minus, plus)) = signs_of(&locale_of(tag)) {
        return if sign == Sign::Negative { minus } else { plus }.to_owned();
    }
    let Some(formatter) = decimal_formatter(cache, tag, 1) else {
        return String::new();
    };
    let mut one = Decimal::from(1);
    one.sign = sign;
    let text = formatter.format(&one).to_string();
    text.chars().filter(|c| !c.is_numeric()).collect()
}

fn sign_kind(sign: Sign) -> &'static str {
    if sign == Sign::Positive {
        "plusSign"
    } else {
        "minusSign"
    }
}

/// The sign's parts: its direction marks literals of their own.
fn sign_parts(parts: &mut Parts, sign: &str, kind: &'static str) {
    let mut run = String::new();
    let mut marks = false;
    for c in sign.chars() {
        let mark = matches!(c, '\u{200e}' | '\u{200f}' | '\u{61c}');
        if mark != marks && !run.is_empty() {
            parts.push(if marks { "literal" } else { kind }, &run);
            run.clear();
        }
        marks = mark;
        run.push(c);
    }
    parts.push(if marks { "literal" } else { kind }, &run);
}

/// `intlNumber(locale, codes, unit, x)`: ECMA-402 section 15.5.4's
/// parts of `x` formatted, as pairs of type and text. The codes are
/// [`Digits`]' and the style (0 decimal, 1 percent, 2 currency, 3
/// unit), the grouping (0 never, 1 auto, 2 always, 3 min2), the sign
/// display, the notation (0 standard, 1 scientific, 2 engineering, 3
/// compact), the currency display (0 symbol, 1 narrowSymbol, 2 code, 3
/// name) and sign (0 standard, 1 accounting), the unit display (0
/// short, 1 narrow, 2 long) and the compact display (0 short, 1 long).
/// A currency is written by ICU4X's currency patterns, a unit by its
/// unit patterns or, for the units it has none of, CLDR's English ones.
fn format_number(
    cache: &mut Cache,
    tag: &str,
    codes: &[i32],
    unit: &str,
    x: Num,
) -> Option<Vec<String>> {
    let written = write_number(cache, tag, codes, x)?;
    let style = code(codes, STYLE);
    let mut parts = Parts(Vec::new());
    match style {
        2 => {
            let template = currency_template(cache, tag, codes, unit, &written)?;
            let (before, after) = split_template(&template)?;
            match &written.compact {
                // The currency's compact pattern places the abbreviation.
                Some((digits, abbreviation)) => {
                    compact_affix_parts(&mut parts, &before, abbreviation);
                    parts.0.extend(digits.0.iter().cloned());
                    compact_affix_parts(&mut parts, &after, abbreviation);
                }
                None => {
                    affix_parts(&mut parts, &before, "currency");
                    parts.0.extend(written.parts.0);
                    affix_parts(&mut parts, &after, "currency");
                }
            }
        }
        3 => {
            let value = written.value.clone().unwrap_or_default();
            let (before, after) = unit_affixes(cache, tag, unit, code(codes, UNIT_DISPLAY), &value);
            // A locale that writes the amount in words (Arabic's two
            // hours) leaves the number, and its sign, out.
            if after.is_some() {
                let sign = sign_text(cache, tag, written.sign);
                sign_parts(&mut parts, &sign, sign_kind(written.sign));
            }
            affix_parts(&mut parts, &before, "unit");
            if let Some(after) = after {
                parts.0.extend(written.parts.0);
                affix_parts(&mut parts, &after, "unit");
            }
        }
        _ => {
            if style == 1 {
                let pattern = percent_pattern(&locale_of(tag), written.sign);
                let (before, after) = pattern.split_once("{0}").unwrap_or(("", "%"));
                affix_parts(&mut parts, before, "percentSign");
                parts.0.extend(written.parts.0);
                affix_parts(&mut parts, after, "percentSign");
            } else {
                let sign = sign_text(cache, tag, written.sign);
                sign_parts(&mut parts, &sign, sign_kind(written.sign));
                parts.0.extend(written.parts.0);
            }
        }
    }
    Some(parts.0)
}

/// The currencies the reference browser writes by their ISO codes in a
/// locale, as symbol or as narrow symbol, where ICU4X's data has a
/// symbol (`USD` in Italian, where ICU4X writes `US$`); a locale's list
/// replaces its parent's.
const CURRENCY_CODES: [(&str, &str, &str); 66] = [
    ("af", "symbol", "CAD MXN PHP USD"),
    ("ak", "symbol", "IQD"),
    ("am", "symbol", "PHP"),
    ("ar", "symbol", "PHP"),
    (
        "bg",
        "narrow",
        "ARS AUD BDT BRL CAD CLP CNY COP HKD ILS INR KRW MXN NGN NZD PHP RON SGD TRY TWD UAH VND",
    ),
    (
        "bg",
        "symbol",
        "AUD BRL CAD CNY GBP HKD ILS INR JPY KRW MXN NZD PHP TWD VND",
    ),
    ("bn", "symbol", "PHP"),
    ("bs-Cyrl", "symbol", "CZK PLN RSD TRY"),
    ("ca", "symbol", "BRL CAD CNY MXN PHP USD"),
    ("cs", "symbol", "ILS INR PHP VND"),
    ("da", "symbol", "PHP"),
    ("de", "symbol", "PHP"),
    ("de-CH", "narrow", "EUR"),
    ("de-CH", "symbol", "EUR PHP"),
    ("el", "symbol", "PHP"),
    (
        "en-AU",
        "symbol",
        "BRL CAD CNY EUR GBP HKD ILS INR JPY KRW MXN NZD PHP TWD USD VND",
    ),
    ("es", "narrow", "EGP"),
    (
        "es",
        "symbol",
        "AUD BRL CAD CNY GBP HKD ILS INR JPY KRW MXN NZD PHP TWD",
    ),
    (
        "es-419",
        "symbol",
        "AUD BRL CAD CNY EUR GBP HKD ILS INR JPY KRW MXN NZD PHP THB TWD USD VND",
    ),
    (
        "es-AR",
        "symbol",
        "AUD BRL CAD CNY EUR GBP HKD ILS INR JPY KRW MXN NZD PHP THB TWD VND",
    ),
    (
        "es-CO",
        "symbol",
        "AUD BRL CAD CNY EUR GBP HKD ILS INR JPY KRW MXN NZD PHP THB TWD VND",
    ),
    (
        "es-MX",
        "symbol",
        "AUD BRL CAD CNY EUR GBP HKD ILS INR JPY KRW NZD PHP THB TWD USD VND",
    ),
    (
        "es-US",
        "symbol",
        "AUD BRL CAD CNY EUR GBP HKD ILS INR KRW MXN NZD PHP THB TWD VND",
    ),
    ("et", "symbol", "PHP"),
    ("fa", "symbol", "PHP"),
    (
        "fi",
        "narrow",
        "ARS AUD BDT BRL CAD CLP CNY COP CZK DKK EGP HKD HUF IDR ILS INR ISK KRW MXN MYR NGN NOK NZD PHP PKR PLN RON SEK SGD THB TRY TWD UAH VND ZAR",
    ),
    (
        "fi",
        "symbol",
        "AUD BRL CAD CNY HKD ILS INR KRW MXN NZD PHP TWD VND",
    ),
    ("fr", "symbol", "CNY HKD JPY PHP TWD"),
    ("fr-CA", "symbol", "ARS CLP COP ILS INR KRW MXN PHP TWD VND"),
    ("gu", "symbol", "PHP"),
    ("he", "symbol", "PHP"),
    ("hi", "symbol", "PHP"),
    (
        "hr",
        "symbol",
        "AUD BRL CAD CNY GBP HKD ILS INR JPY KRW MXN NZD PHP TWD USD VND",
    ),
    (
        "hu",
        "symbol",
        "AUD BRL CAD CNY EUR GBP HKD ILS INR KRW MXN NZD PHP TWD USD VND",
    ),
    ("id", "symbol", "PHP"),
    ("it", "symbol", "BRL HKD INR JPY KRW MXN TWD USD VND"),
    ("ja", "symbol", "PHP"),
    ("kn", "symbol", "PHP"),
    ("ko", "symbol", "PHP"),
    ("lt", "narrow", "BDT ILS INR VND"),
    (
        "lt",
        "symbol",
        "AUD BRL CAD CNY GBP HKD ILS INR JPY KRW MXN NZD PHP TWD USD VND",
    ),
    ("lv", "symbol", "PHP"),
    ("ml", "symbol", "PHP"),
    ("mr", "symbol", "PHP"),
    ("ms", "symbol", "CAD MXN PHP USD"),
    (
        "nb",
        "symbol",
        "AUD BRL CAD CNY HKD ILS INR JPY KRW MXN NZD PHP TWD USD VND",
    ),
    ("nl", "symbol", "PHP"),
    (
        "no",
        "symbol",
        "AUD BRL CAD CNY HKD ILS INR JPY KRW MXN NZD PHP TWD USD VND",
    ),
    (
        "pl",
        "symbol",
        "AUD CAD CNY GBP HKD ILS INR JPY KRW MXN NZD PHP TWD USD VND",
    ),
    ("pt", "symbol", "PHP"),
    (
        "ro",
        "symbol",
        "AUD BRL CAD CNY EUR GBP HKD ILS INR JPY KRW MXN NZD PHP TWD USD VND",
    ),
    ("ru", "symbol", "PHP"),
    (
        "sk",
        "symbol",
        "AUD BRL CAD CNY GBP HKD INR JPY KRW NZD PHP TWD USD VND",
    ),
    ("sl", "symbol", "AUD BRL CAD GBP MXN NZD PHP TWD"),
    ("sr", "symbol", "AUD KRW NZD PHP VND"),
    (
        "sv",
        "symbol",
        "AUD CNY GBP HKD INR JPY KRW NZD PHP TWD VND",
    ),
    ("sw", "symbol", "PHP"),
    ("ta", "symbol", "PHP"),
    ("te", "symbol", "PHP"),
    ("th", "symbol", "PHP"),
    ("tr", "symbol", "PHP"),
    (
        "uk",
        "symbol",
        "AUD BRL CAD CNY EUR GBP HKD ILS INR KRW MXN NZD PHP TWD USD VND",
    ),
    ("ur", "symbol", "PHP"),
    ("uz-Cyrl", "symbol", "THB"),
    ("vi", "symbol", "PHP"),
    ("zh", "symbol", "PHP"),
];

/// The currency symbols the reference browser writes where ICU4X's data
/// has others: locale, display, currency, ICU4X's text and the
/// reference's (a language with no currency data of its own its default
/// locale's, English's).
const CURRENCY_SYMBOLS: [(&str, &str, &str, &str, &str); 34] = [
    ("ak", "symbol", "USD", "US$", "$"),
    ("ak", "symbol", "JPY", "JP\u{a5}", "\u{a5}"),
    ("az", "symbol", "USD", "US$", "$"),
    ("az", "symbol", "JPY", "JP\u{a5}", "\u{a5}"),
    ("bs", "symbol", "USD", "US$", "$"),
    ("bs", "symbol", "JPY", "JP\u{a5}", "\u{a5}"),
    ("kk", "symbol", "USD", "US$", "$"),
    ("kk", "symbol", "JPY", "JP\u{a5}", "\u{a5}"),
    ("kok", "symbol", "USD", "US$", "$"),
    ("kok", "symbol", "JPY", "JP\u{a5}", "\u{a5}"),
    ("pa", "symbol", "USD", "US$", "$"),
    ("pa", "symbol", "JPY", "JP\u{a5}", "\u{a5}"),
    ("sd", "symbol", "USD", "US$", "$"),
    ("sd", "symbol", "JPY", "JP\u{a5}", "\u{a5}"),
    ("uz", "symbol", "USD", "US$", "$"),
    ("uz", "symbol", "JPY", "JP\u{a5}", "\u{a5}"),
    ("ar", "narrow", "AED", "AED", "\u{62f}.\u{625}.\u{200f}"),
    ("ar", "narrow", "BHD", "BHD", "\u{62f}.\u{628}.\u{200f}"),
    ("ar", "narrow", "IQD", "IQD", "\u{62f}.\u{639}.\u{200f}"),
    ("ar", "narrow", "JOD", "JOD", "\u{62f}.\u{623}.\u{200f}"),
    ("ar", "narrow", "KWD", "KWD", "\u{62f}.\u{643}.\u{200f}"),
    ("ar", "narrow", "MAD", "MAD", "\u{62f}.\u{645}.\u{200f}"),
    ("ar", "narrow", "OMR", "OMR", "\u{631}.\u{639}.\u{200f}"),
    ("ar", "narrow", "QAR", "QAR", "\u{631}.\u{642}.\u{200f}"),
    ("ar", "narrow", "SAR", "SAR", "\u{631}.\u{633}.\u{200f}"),
    ("ar", "narrow", "TND", "TND", "\u{62f}.\u{62a}.\u{200f}"),
    ("bg", "narrow", "BGN", "BGN", "\u{43b}\u{432}."),
    ("bs-Cyrl", "symbol", "JPY", "\u{a5}", "JP\u{a5}"),
    ("es-419", "narrow", "EGP", "EGP\u{a0}", "E\u{a3}"),
    ("es-AR", "narrow", "EGP", "EGP", "E\u{a3}"),
    ("es-CO", "narrow", "EGP", "EGP", "E\u{a3}"),
    ("ru", "symbol", "UAH", "\u{20b4}", "\u{433}\u{440}\u{43d}."),
    ("sw", "narrow", "KES", "KES", "Ksh"),
    ("uk", "symbol", "UAH", "\u{20b4}", "\u{433}\u{440}\u{43d}"),
];

/// The compact patterns of a currency the reference browser writes
/// where ICU4X's differ: `{0}` the compact number, `{s}` the symbol.
const COMPACT_CURRENCY_PATTERNS: [(&str, &str); 5] = [
    ("bn", "{0}{s}"),
    ("fa", "\u{200e}{s}\u{a0}{0}"),
    ("es-AR", "{s}{0}"),
    ("es-CO", "{s}{0}"),
    ("it-CH", "{0}\u{a0}{s}"),
];

/// The languages whose units the reference browser writes in English.
const UNITS_IN_ENGLISH: [&str; 7] = ["ak", "az", "kk", "kok", "pa", "sd", "uz"];

/// The locales whose currencies the reference browser names by English
/// names (its ICU having none of theirs), or by their codes.
const CURRENCY_NAMES_IN_ENGLISH: [&str; 1] = ["ak"];
const CURRENCY_NAMES_BY_CODE: [&str; 2] = ["bs-Cyrl", "uz-Cyrl"];

/// Whether the reference browser writes the currency by its code at the
/// display (0 symbol, 1 narrow symbol) in the locale.
fn written_by_code(locale: &Locale, currency: &str, display: i32) -> bool {
    let width = if display == 1 { "narrow" } else { "symbol" };
    let rows: Vec<(&str, &str)> = CURRENCY_CODES
        .iter()
        .filter(|(_, kind, _)| *kind == width)
        .map(|(name, _, list)| (*name, *list))
        .collect();
    nearest(&rows, locale).is_some_and(|list| list.split(' ').any(|code| code == currency))
}

/// A currency formatter of ICU4X's, of the plain or the compact kind.
enum AnyCurrencyFormatter {
    Plain(CurrencyFormatter<DecimalFormatter>),
    Compact(CurrencyFormatter<CompactDecimalFormatter>),
}

impl AnyCurrencyFormatter {
    fn format(&self, value: &Decimal) -> String {
        match self {
            AnyCurrencyFormatter::Plain(f) => f.format_fixed_decimal(value).to_string(),
            AnyCurrencyFormatter::Compact(f) => f.format_fixed_decimal(value).to_string(),
        }
    }
}

/// A currency's amount as ICU4X writes it for the number's sign, whose
/// digits the number's own replace: the currency's symbol, narrow
/// symbol or code where the locale's pattern (or compact pattern) puts
/// it, the code where the reference browser writes that, or its plural
/// name. A plus sign takes the minus sign's place in the negative
/// pattern (`US$ +5,00` in Dutch); a letter next to a digit is spaced
/// from it (CLDR's currency spacing).
fn currency_template(
    cache: &mut Cache,
    tag: &str,
    codes: &[i32],
    currency: &str,
    written: &Written,
) -> Option<String> {
    let mut display = code(codes, CURRENCY_DISPLAY);
    let accounting = code(codes, CURRENCY_SIGN);
    if display == 3 && written.compact.is_none() {
        return currency_name_template(cache, tag, currency, written);
    }
    if display < 2 && written_by_code(&locale_of(tag), currency, display) {
        display = 2;
    }
    let compact = match (written.compact.is_some(), code(codes, COMPACT_DISPLAY)) {
        (false, _) => 0,
        (true, 1) => 2,
        (true, _) => 1,
    };
    let formatter = cached(
        &mut cache.currencies,
        (
            tag.to_owned(),
            currency.to_owned(),
            display + 4 * compact,
            accounting,
        ),
        || {
            let locale = data_locale(tag);
            let prefs = (&locale).into();
            let kind = CurrencyType::try_from_str(currency).ok()?;
            let options = CurrencyFormatterOptions::from(if accounting == 1 {
                CurrencyUsage::Accounting
            } else {
                CurrencyUsage::Standard
            });
            Some(match (compact, display) {
                (0, 1) => AnyCurrencyFormatter::Plain(
                    CurrencyFormatter::try_new_symbol_narrow(prefs, kind, options).ok()?,
                ),
                (0, 2) => AnyCurrencyFormatter::Plain(
                    CurrencyFormatter::try_new_code(prefs, kind, options).ok()?,
                ),
                (0, _) => AnyCurrencyFormatter::Plain(
                    CurrencyFormatter::try_new_symbol(prefs, kind, options).ok()?,
                ),
                (1, 1) => AnyCurrencyFormatter::Compact(
                    CurrencyFormatter::try_new_compact_symbol_narrow(prefs, kind, options).ok()?,
                ),
                (1, 2) => AnyCurrencyFormatter::Compact(
                    CurrencyFormatter::try_new_compact_code(prefs, kind, options).ok()?,
                ),
                (1, 3) => AnyCurrencyFormatter::Compact(
                    CurrencyFormatter::try_new_compact_name(prefs, kind).ok()?,
                ),
                (1, _) => AnyCurrencyFormatter::Compact(
                    CurrencyFormatter::try_new_compact_symbol(prefs, kind, options).ok()?,
                ),
                (_, 1) => AnyCurrencyFormatter::Compact(
                    CurrencyFormatter::try_new_compact_long_symbol_narrow(prefs, kind, options)
                        .ok()?,
                ),
                (_, 2) => AnyCurrencyFormatter::Compact(
                    CurrencyFormatter::try_new_compact_long_code(prefs, kind, options).ok()?,
                ),
                (_, 3) => AnyCurrencyFormatter::Compact(
                    CurrencyFormatter::try_new_compact_long_name(prefs, kind).ok()?,
                ),
                (_, _) => AnyCurrencyFormatter::Compact(
                    CurrencyFormatter::try_new_compact_long_symbol(prefs, kind, options).ok()?,
                ),
            })
        },
    )?;
    let mut probe = written.value.clone().unwrap_or_else(|| Decimal::from(1));
    probe.sign = if written.sign == Sign::Positive {
        Sign::Negative
    } else {
        written.sign
    };
    let mut text = formatter.format(&probe);
    if written.sign == Sign::Positive {
        text = text.replace(['-', '\u{2212}'], "+");
    }
    let locale = locale_of(tag);
    let width = if code(codes, CURRENCY_DISPLAY) == 1 {
        "narrow"
    } else {
        "symbol"
    };
    if let Some((_, _, _, from, to)) = CURRENCY_SYMBOLS
        .iter()
        .filter(|(_, kind, code, _, _)| *kind == width && *code == currency)
        .find(|(name, _, _, _, _)| locale_keys(&locale).iter().any(|key| key == name))
    {
        text = text.replacen(from, to, 1);
    }
    if let Some((minus, plus)) = signs_of(&locale) {
        text = text.replace('-', minus).replace('+', plus);
    }
    let Some((_, abbreviation)) = &written.compact else {
        return Some(currency_spaced(&text));
    };
    let patterns: Vec<(&str, &str)> = COMPACT_CURRENCY_PATTERNS.to_vec();
    if let Some(pattern) = nearest(&patterns, &locale)
        && let Some((before, after)) = split_template(&text)
    {
        // The symbol: what is left of the affixes but the abbreviation.
        let symbol = format!("{before}{after}").replacen(abbreviation.as_str(), "", 1);
        let symbol = symbol.trim_matches(spacing);
        let end = after
            .find(abbreviation.as_str())
            .map_or(0, |at| at + abbreviation.len());
        let number = format!("1{}", &after[..end]);
        // A letter of the symbol next to the digits is spaced from them.
        let symbol_first = pattern.find("{s}") < pattern.find("{0}");
        let spaced = if symbol_first {
            symbol.chars().last()
        } else {
            symbol.chars().next()
        }
        .is_some_and(|c| c.is_alphabetic())
            && pattern.contains("{s}{0}");
        let symbol = if spaced {
            format!("{symbol}\u{a0}")
        } else {
            symbol.to_owned()
        };
        text = pattern.replace("{0}", &number).replace("{s}", &symbol);
    }
    Some(text)
}

/// Whether a character is a currency sign, which CLDR's currency
/// spacing leaves next to a digit.
fn currency_sign(c: char) -> bool {
    matches!(c, '$' | '\u{a2}'..='\u{a5}' | '\u{58f}' | '\u{60b}' | '\u{9f2}' | '\u{9f3}'
        | '\u{e3f}' | '\u{17db}' | '\u{20a0}'..='\u{20cf}' | '\u{fdfc}' | '\u{fe69}'
        | '\u{ff04}' | '\u{ffe0}' | '\u{ffe1}' | '\u{ffe5}' | '\u{ffe6}')
}

/// A currency's amount with CLDR's currency spacing: a no-break space
/// between a digit and a currency's letter or mark next to it.
fn currency_spaced(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let first = chars.iter().position(|c| c.is_numeric());
    let last = chars.iter().rposition(|c| c.is_numeric());
    let (Some(first), Some(last)) = (first, last) else {
        return text.to_owned();
    };
    let spaceable = |c: char| {
        !c.is_whitespace()
            && !currency_sign(c)
            && !matches!(
                c,
                '-' | '+' | '(' | ')' | '\u{2212}' | '\u{200e}' | '\u{200f}' | '\u{61c}'
            )
    };
    let mut out = String::new();
    for (at, c) in chars.iter().enumerate() {
        if at == first && at > 0 && spaceable(chars[at - 1]) {
            out.push('\u{a0}');
        }
        out.push(*c);
        if at == last && chars.get(at + 1).is_some_and(|next| spaceable(*next)) {
            out.push('\u{a0}');
        }
    }
    out
}

/// A currency's amount by its plural name: the locale's currency
/// pattern (`{0} {1}`, `{0}{1}` in Japanese) and name for the plural
/// form of the number as written, the sign before; the code where
/// ICU4X has no name.
fn currency_name_template(
    cache: &mut Cache,
    tag: &str,
    currency: &str,
    written: &Written,
) -> Option<String> {
    let mut locale = data_locale(tag);
    let keys = locale_keys(&locale_of(tag));
    if keys
        .iter()
        .any(|key| CURRENCY_NAMES_BY_CODE.contains(&key.as_str()))
    {
        let sign = sign_text(cache, tag, written.sign);
        let code = currency.to_ascii_uppercase();
        return Some(format!("{sign}1 {code}"));
    }
    if without_format_data(&locale_of(tag))
        || keys
            .iter()
            .any(|key| CURRENCY_NAMES_IN_ENGLISH.contains(&key.as_str()))
    {
        locale = Locale::try_from_str("en").unwrap_or(Locale::UNKNOWN);
    }
    let locale = DataLocale::from(&locale);
    let rules = plural_rules(cache, tag, false)?;
    let value = written.value.clone().unwrap_or_else(|| Decimal::from(2));
    let operands: PluralOperands = (&value).into();
    let code = currency.to_ascii_uppercase();
    let attributes = DataMarkerAttributes::try_from_str(&code).ok()?;
    let mut metadata = DataRequestMetadata::default();
    metadata.silent = true;
    let name: Option<DataResponse<CurrencyExtendedDataV1>> = UnitData
        .load(DataRequest {
            id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, &locale),
            metadata,
        })
        .ok();
    let name = name.as_ref().map_or(code.as_str(), |name| {
        name.payload.get().get(operands, &rules)
    });
    let patterns: DataResponse<CurrencyPatternsDataV1> = UnitData
        .load(DataRequest {
            id: DataIdentifierBorrowed::for_locale(&locale),
            metadata,
        })
        .ok()?;
    let pattern = patterns.payload.get().get(operands, &rules);
    let text = pattern.interpolate(("1", name)).to_string();
    let sign = sign_text(cache, tag, written.sign);
    Some(format!("{sign}{text}"))
}

/// A unit's names at a width in a locale: a pattern ("{0} km") for
/// each plural category, from ICU4X's unit data, which
/// `icu_experimental` keeps for the area, duration, length, mass and
/// volume units, each locale's own units in its core set, the rest in
/// its extended and outlier ones.
type UnitNames = DataPayload<ErasedMarker<UnitsDisplayNames<'static>>>;

/// Loads the names of one of `icu_experimental`'s unit sets into
/// `found`: the first a locale has of its own, or, if none has, the
/// root's, which the compiled data falls back to.
fn load_unit_names<M>(id: DataIdentifierBorrowed<'_>, found: &mut Option<(bool, UnitNames)>)
where
    M: DataMarker<DataStruct = UnitsDisplayNames<'static>>,
    UnitData: DataProvider<M>,
{
    if found.as_ref().is_some_and(|(own, _)| *own) {
        return;
    }
    // Asked of each set in turn, a miss is no error to log.
    let mut metadata = DataRequestMetadata::default();
    metadata.silent = true;
    if let Ok(response) = UnitData.load(DataRequest { id, metadata }) {
        let own = !response
            .metadata
            .locale
            .is_some_and(|locale| locale.is_unknown());
        if own || found.is_none() {
            *found = Some((own, response.payload.cast()));
        }
    }
}

/// ICU4X's names of `unit` at `width` (short, narrow or long) in the
/// locale, or in the nearest it falls back to (Norwegian's under `no`).
fn icu_unit_names(locale: &Locale, unit: &str, width: &str) -> Option<UnitNames> {
    let attributes = format!("{width}-{unit}");
    let attributes = DataMarkerAttributes::try_from_str(&attributes).ok()?;
    let mut locale = locale.clone();
    if locale.id.language.as_str() == "nb" {
        locale.id.language = Language::try_from_str("no").ok()?;
    }
    let locale = DataLocale::from(&locale);
    let id = DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, &locale);
    let mut found = None;
    load_unit_names::<UnitsNamesLengthCoreV1>(id, &mut found);
    load_unit_names::<UnitsNamesLengthExtendedV1>(id, &mut found);
    load_unit_names::<UnitsNamesLengthOutlierV1>(id, &mut found);
    load_unit_names::<UnitsNamesDurationCoreV1>(id, &mut found);
    load_unit_names::<UnitsNamesDurationExtendedV1>(id, &mut found);
    load_unit_names::<UnitsNamesDurationOutlierV1>(id, &mut found);
    load_unit_names::<UnitsNamesMassCoreV1>(id, &mut found);
    load_unit_names::<UnitsNamesMassExtendedV1>(id, &mut found);
    load_unit_names::<UnitsNamesMassOutlierV1>(id, &mut found);
    load_unit_names::<UnitsNamesVolumeCoreV1>(id, &mut found);
    load_unit_names::<UnitsNamesVolumeExtendedV1>(id, &mut found);
    load_unit_names::<UnitsNamesVolumeOutlierV1>(id, &mut found);
    load_unit_names::<UnitsNamesAreaCoreV1>(id, &mut found);
    load_unit_names::<UnitsNamesAreaExtendedV1>(id, &mut found);
    load_unit_names::<UnitsNamesAreaOutlierV1>(id, &mut found);
    found.map(|(_, names)| names)
}

/// The reference browser's unit patterns for what ICU4X has no names of
/// (the digital, temperature, angle and percent units, CLDR's compound
/// units of speed and consumption), the per patterns compound units are
/// written by and their exceptions (`{0}/s`), and the few names where
/// ICU4X's differ, generated from Chrome 156 for each of its locales
/// (`intl_units.txt` says how).
const UNIT_DATA: &str = include_str!("intl_units.txt");

/// [`UNIT_DATA`] by locale, unit and width.
fn unit_data() -> &'static HashMap<(&'static str, &'static str, &'static str), &'static str> {
    static DATA: OnceLock<HashMap<(&'static str, &'static str, &'static str), &'static str>> =
        OnceLock::new();
    DATA.get_or_init(|| {
        UNIT_DATA
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| {
                let mut fields = line.split('\t');
                Some((
                    (fields.next()?, fields.next()?, fields.next()?),
                    fields.next()?,
                ))
            })
            .collect()
    })
}

/// The locale a table's locales fall back to (CLDR's parent locales, as
/// [`UNIT_DATA`] was written with).
fn parent_of(locale: &str) -> Option<String> {
    let (language, rest) = locale.split_once('-')?;
    Some(match (language, rest) {
        ("en", region) if region != "001" && region != "US" => "en-001".to_owned(),
        ("es", region) if region != "419" && region != "ES" => "es-419".to_owned(),
        ("zh", "TW" | "HK" | "MO") => "zh-Hant".to_owned(),
        ("pt", region) if region != "BR" && region != "PT" => "pt-PT".to_owned(),
        _ => language.to_owned(),
    })
}

/// The patterns [`UNIT_DATA`] has for a unit (or `per`, or `per-` and a
/// unit) at a width in the locale or its nearest parent, as written.
fn table_spec(locale: &Locale, unit: &str, width: &str) -> Option<&'static str> {
    let data = unit_data();
    locale_keys(locale)
        .iter()
        .find_map(|key| data.get(&(key.as_str(), unit, width)).copied())
}

/// The pattern of a [`UNIT_DATA`] entry for a plural category, its
/// escapes undone.
fn spec_pattern(spec: &str, category: &str) -> String {
    let pairs: Vec<(&str, &str)> = spec
        .split('|')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(name, _)| CATEGORIES.iter().any(|(_, known)| known == name))
        .collect();
    let pattern = if pairs.is_empty() {
        spec
    } else {
        pairs
            .iter()
            .find(|(name, _)| *name == category)
            .or_else(|| pairs.iter().find(|(name, _)| *name == "other"))
            .map_or(spec, |(_, pattern)| pattern)
    };
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(at) = rest.find("\\u") {
        out.push_str(&rest[..at]);
        let code = rest
            .get(at + 2..at + 6)
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .and_then(char::from_u32);
        match code {
            Some(c) => {
                out.push(c);
                rest = &rest[at + 6..];
            }
            None => {
                out.push_str("\\u");
                rest = &rest[at + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A unit's amount as written for `value`, "{0}" where its digits go
/// (none where the locale writes the amount in words), at a width (0
/// short, 1 narrow, 2 long): [`UNIT_DATA`]'s, else ICU4X's names, else,
/// for a compound unit, the first unit's amount per the second.
fn unit_template(
    cache: &mut Cache,
    tag: &str,
    unit: &str,
    display: i32,
    value: &Decimal,
) -> Option<String> {
    let width = match display {
        1 => "narrow",
        2 => "long",
        _ => "short",
    };
    let rules = plural_rules(cache, tag, false)?;
    let category = category_name(rules.category_for(value));
    // A language the reference browser's ICU has no units of: its
    // default locale's, English.
    let own = locale_of(tag);
    let tag = if UNITS_IN_ENGLISH.contains(&own.id.language.as_str()) && own.id.script.is_none() {
        "en"
    } else {
        tag
    };
    let locale = locale_of(tag);
    if let Some(spec) = table_spec(&locale, unit, width) {
        return Some(spec_pattern(spec, category));
    }
    if let Some(names) = cached(
        &mut cache.units,
        (tag.to_owned(), unit.to_owned(), display),
        || icu_unit_names(&data_locale(tag), unit, width),
    ) {
        let pattern = names.get().get(value.into(), &rules);
        return Some(pattern.interpolate(("{0}",)).to_string());
    }
    // A compound unit: the first unit's amount per the second, by the
    // second's own per pattern or the locale's with its singular name.
    let (numerator, denominator) = unit.split_once("-per-")?;
    let amount = unit_template(cache, tag, numerator, display, value)?;
    if let Some(spec) = table_spec(&locale, &format!("per-{denominator}"), width) {
        return Some(spec_pattern(spec, category).replace("{0}", &amount));
    }
    let per = spec_pattern(table_spec(&locale, "per", width)?, "other");
    let name = unit_template(cache, tag, denominator, display, &Decimal::from(1))?;
    let name = name.replace("{0}", "");
    Some(
        per.replace("{0}", &amount)
            .replace("{1}", name.trim_matches(spacing)),
    )
}

/// A unit's text before and after the number, or, where the locale
/// writes the amount in words, that text alone.
fn unit_affixes(
    cache: &mut Cache,
    tag: &str,
    unit: &str,
    display: i32,
    value: &Decimal,
) -> (String, Option<String>) {
    let unit = unit.chars().take(64).collect::<String>();
    match unit_template(cache, tag, &unit, display, value) {
        Some(template) => match template.split_once("{0}") {
            Some((before, after)) => (before.to_owned(), Some(after.to_owned())),
            None => (template, None),
        },
        None => (String::new(), Some(format!(" {unit}"))),
    }
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
        let locale = plural_locale(tag);
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
    let Num::Finite(mut value) = finite(n) else {
        return "other".to_owned();
    };
    value.sign = Sign::None;
    let digits = Digits::of(codes);
    let rounded = digits.round(&value);
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

/// The languages the reference browser's ICU has no collation data of.
const ROOT_COLLATION_LANGUAGES: [&str; 8] = ["ak", "az", "bs", "kk", "kok", "pa", "sd", "uz"];

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
            // A language the reference browser's ICU has no collation of
            // sorts by the root order.
            if ROOT_COLLATION_LANGUAGES.contains(&locale.id.language.as_str()) {
                locale.id = LanguageIdentifier::UNKNOWN;
            }
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
const DAY_PERIOD: usize = 12;

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
            let locale = date_locale(tag);
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

/// A time value in a zone at `offset_minutes` from UTC as ICU4X's zoned
/// date and time: its local ISO date and time, and the zone ("UTC", an
/// IANA name, or "" for one known only by its offset).
fn zoned_at(zone: &str, t: f64, offset_minutes: f64) -> Option<Zoned> {
    if !t.is_finite() || !offset_minutes.is_finite() || offset_minutes.abs() > 24.0 * 60.0 {
        return None;
    }
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
    let name = zone.to_owned();
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
    Some(Zoned {
        zoned: ZonedDateTime { date, time, zone },
        local,
        name,
    })
}

/// A zoned date and time, its local time value and its zone's name.
struct Zoned {
    zoned: ZonedDateTime<Iso, TimeZoneInfo<AtTime>>,
    local: i64,
    name: String,
}

/// `intlDateTime(locale, codes, zone, t, offset)`: section 11.5.6's
/// FormatDateTimePattern of time value `t` in a zone at `offset` minutes
/// from UTC, as pairs of part type and text, and the pattern after them:
/// the reference browser's pattern for the codes, formatted with
/// ICU4X's names, where the calendar is the gregorian one (or ISO's);
/// ICU4X's own formatter's for another calendar.
fn format_date_time(
    cache: &mut Cache,
    tag: &str,
    codes: &[i32],
    zone: &str,
    t: f64,
    offset_minutes: f64,
) -> Option<Vec<String>> {
    let zoned = zoned_at(zone, t, offset_minutes)?;
    if let Some(pattern) = reference_pattern(cache, tag, codes)
        && let Some(text) = format_with_pattern(cache, tag, &pattern, &zoned)
    {
        return Some(pattern_parts(&pattern, &text));
    }
    let zoned = zoned.zoned;
    let formatter = date_time_formatter(cache, tag, codes)?;
    let formatted = formatter.format(&zoned);
    let text = formatted.to_string();
    let pattern = formatted.pattern().to_string();
    let pieces = align(&pattern_items(&pattern), &text)
        .unwrap_or_else(|| vec![("literal".to_owned(), text.clone())]);
    let mut answer = Vec::with_capacity(pieces.len() * 2 + 1);
    let (day_alone, month_alone) = unpadded_numeric(&locale_of(tag));
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
        let one_digit = match kind.as_str() {
            "month" => code(codes, MONTH) == 1 && month_alone,
            "day" => code(codes, DAY) == 1 && day_alone,
            _ => false,
        };
        if one_digit && value.len() == 2 && value.starts_with('0') {
            value.remove(0);
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
    let calendar = match calendar_of(cache, tag) {
        // ICU's astronomical islamic calendar, which ICU4X takes as another.
        Some("islamic") => asked_calendar(tag).unwrap_or_else(|| "islamic".to_owned()),
        _ => date_time_formatter(cache, tag, codes)
            .map(|formatter| calendar_name(formatter.calendar().kind()))
            .unwrap_or("gregory")
            .to_owned(),
    };
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
    let mut answer = vec![calendar, cycle.to_owned()];
    if code(codes, DATE_STYLE) == 0
        && code(codes, TIME_STYLE) == 0
        && generator_of(cache, tag).is_some()
    {
        answer.extend(components_of(&pattern));
    }
    answer
}

// ---- The reference browser's patterns ----------------------------------

/// The hour letter an `hc` keyword asks for (`c12` and `c24` the
/// bootstrap's for `hour12`).
fn hour_cycle_letter(locale: &Locale) -> Option<char> {
    let key = Key::try_from_str("hc").ok()?;
    match locale
        .extensions
        .unicode
        .keywords
        .get(&key)?
        .to_string()
        .as_str()
    {
        "h11" => Some('K'),
        "h12" | "c12" => Some('h'),
        "h23" | "c24" => Some('H'),
        "h24" => Some('k'),
        _ => None,
    }
}

/// The locale's pattern generator over the reference browser's data.
/// `None` for a calendar the reference browser's patterns in the data
/// are not of.
fn generator_of(cache: &mut Cache, tag: &str) -> Option<Rc<intl_dates::Generator>> {
    let calendar = calendar_of(cache, tag)?;
    let locale = date_locale(tag);
    if !intl_dates::has_calendar(&locale, calendar) {
        return None;
    }
    let own = locale_of(tag);
    let key = format!("{} {} {calendar}", locale.id, own.id);
    let decimal = match separators_of(&locale_of(tag), false) {
        Some((_, decimal)) => decimal.to_owned(),
        None => number_context(cache, tag, 1)
            .map_or_else(|| ".".to_owned(), |context| context.symbols.decimal.clone()),
    };
    cached(&mut cache.generators, key, || {
        Some(intl_dates::Generator::in_calendar(&locale, &own, calendar).with_decimal(&decimal))
    })
}

/// The calendar a tag's `-u-ca-` asks for.
fn asked_calendar(tag: &str) -> Option<String> {
    let key = Key::try_from_str("ca").ok()?;
    locale_of(tag)
        .extensions
        .unicode
        .keywords
        .get(&key)
        .map(|value| value.to_string())
}

/// The calendar the locale formats dates in, by the data's name of its
/// patterns: the gregorian one (or the ISO one, written alike), or
/// another a `-u-ca-` asks for.
fn calendar_of(cache: &mut Cache, tag: &str) -> Option<&'static str> {
    // ICU's astronomical islamic calendar, which ICU4X takes as another.
    if matches!(
        asked_calendar(tag).as_deref(),
        Some("islamic" | "islamic-rgsa")
    ) {
        return Some("islamic");
    }
    // A day's formatter has the locale's calendar.
    let formatter = date_time_formatter(cache, tag, &[0, 0, 0, 0, 1])?;
    Some(match formatter.calendar().kind() {
        AnyCalendarKind::Gregorian | AnyCalendarKind::Iso => "gregorian",
        AnyCalendarKind::Buddhist => "buddhist",
        AnyCalendarKind::Persian => "persian",
        AnyCalendarKind::Japanese => "japanese",
        AnyCalendarKind::Chinese => "chinese",
        AnyCalendarKind::Dangi => "dangi",
        AnyCalendarKind::Coptic => "coptic",
        AnyCalendarKind::Ethiopian => "ethiopic",
        AnyCalendarKind::EthiopianAmeteAlem => "ethiopic-amete-alem",
        AnyCalendarKind::Hebrew => "hebrew",
        AnyCalendarKind::Indian => "indian",
        AnyCalendarKind::Roc => "roc",
        AnyCalendarKind::HijriSimulatedMecca => "islamic",
        AnyCalendarKind::HijriTabularTypeIIFriday => "islamic-civil",
        AnyCalendarKind::HijriTabularTypeIIThursday => "islamic-tbla",
        AnyCalendarKind::HijriUmmAlQura => "islamic-umalqura",
        _ => return None,
    })
}

/// A date in a calendar: its era's index, extended year, month from 0
/// and day of the month.
fn calendar_fields(date: &Date<Iso>, calendar: &str) -> (i32, i64, usize, u8) {
    fn fields<A: AsCalendar>(date: Date<A>) -> (i64, usize, u8) {
        (
            i64::from(date.year().extended_year()),
            usize::from(date.month().ordinal.saturating_sub(1)),
            date.day_of_month().0,
        )
    }
    match calendar {
        "buddhist" => {
            let (year, month, day) = fields(date.to_calendar(Buddhist));
            (0, year, month, day)
        }
        "persian" => {
            let (year, month, day) = fields(date.to_calendar(Persian));
            (0, year, month, day)
        }
        _ => {
            let (year, month, day) = fields(date.to_calendar(Gregorian));
            (i32::from(year > 0), year, month, day)
        }
    }
}

/// The reference browser's pattern for a format's codes: a style's (a
/// date's and a time's joined by the date style's glue), or the pattern
/// ICU's generator finds for V8's skeleton of the components, its hours
/// in the hour cycle asked for; `None` for a calendar not the gregorian
/// one.
fn reference_pattern(cache: &mut Cache, tag: &str, codes: &[i32]) -> Option<String> {
    let generator = generator_of(cache, tag)?;
    let letter = hour_cycle_letter(&locale_of(tag));
    let (date_style, time_style) = (code(codes, DATE_STYLE), code(codes, TIME_STYLE));
    let pattern = if date_style != 0 || time_style != 0 {
        let style = |s: i32| (s - 1).clamp(0, 3) as usize;
        let date = (date_style != 0)
            .then(|| generator.style_pattern(style(date_style), true))
            .flatten();
        let time = (time_style != 0)
            .then(|| generator.style_pattern(style(time_style), false))
            .flatten()
            .map(|time| generator.in_preferred_cycle(&time));
        match (date, time) {
            (Some(date), Some(time)) => generator
                .style_glue(style(date_style))
                .replace("{1}", &date)
                .replace("{0}", &time),
            (Some(date), None) => date,
            (None, Some(time)) => time,
            (None, None) => return None,
        }
    } else {
        // A flexible day period asks for 12-hour hours.
        let hour = match letter {
            Some(letter) => letter,
            None if code(codes, DAY_PERIOD) != 0 => 'h',
            None => 'j',
        };
        generator.best(&intl_dates::skeleton_of(codes, hour))
    };
    Some(match letter {
        Some(letter) => intl_dates::with_hour_letter(&pattern, letter),
        None => pattern,
    })
}

/// A zoned time formatted by a pattern, with ICU4X's names for the
/// date's locale (its numbering system kept).
fn format_with_pattern(
    cache: &mut Cache,
    tag: &str,
    pattern: &str,
    zoned: &Zoned,
) -> Option<String> {
    let generator = generator_of(cache, tag)?;
    let date = &zoned.zoned.date;
    let time = &zoned.zoned.time;
    let zero = number_context(cache, tag, 1).map_or('0', |context| context.symbols.zero);
    let calendar = calendar_of(cache, tag)?;
    let (era, _, month, _) = calendar_fields(date, calendar);
    // The year in its cycle, which ICU's Chinese calendars write as `y`.
    let cyclic = match calendar {
        "chinese" => date.to_calendar(ChineseTraditional::new()).year().cyclic(),
        "dangi" => date.to_calendar(KoreanTraditional::new()).year().cyclic(),
        _ => None,
    };
    // ICU's islamic calendar where ICU4X's simulated one is a day off: the
    // month by its first day, written by ICU4X, the day and the weekday
    // written here.
    let local_day = zoned.local.div_euclid(MS_PER_DAY);
    let islamic = (calendar == "islamic")
        .then(|| islamic_date(local_day))
        .flatten();
    let at = intl_dates::Instant {
        own_day: islamic.is_some(),
        cyclic: cyclic.map(|cyclic| (i64::from(cyclic.year), i64::from(cyclic.related_iso))),
        era,
        month,
        weekday: (zoned.local.div_euclid(MS_PER_DAY) + 4).rem_euclid(7) as usize,
        minutes: i32::from(time.hour.number()) * 60 + i32::from(time.minute.number()),
        millis: time.subsecond.number() / 1_000_000,
        offset_seconds: zoned
            .zoned
            .zone
            .offset()
            .map_or(0, |offset| offset.to_seconds()),
        zero,
    };
    let zone_locale = match generator.zone_names() {
        intl_dates::ZoneNames::English => Locale::try_from_str("en").ok()?,
        _ => date_locale(tag),
    };
    let digits = |value: u8, count: usize| -> String {
        format!("{value:0count$}")
            .chars()
            .map(|c| {
                c.to_digit(10)
                    .map_or(c, |d| char::from_u32(zero as u32 + d).unwrap_or(c))
            })
            .collect()
    };
    let mut outside = |letter: char, count: usize| -> Option<String> {
        match (letter, islamic) {
            ('d', Some((_, _, day))) => Some(digits(day, count.min(2))),
            ('E' | 'c', Some(_)) => weekday_text(tag, &zoned.zoned, letter, count),
            _ => zone_name(cache, &generator, &zone_locale, zoned, letter, count, zero),
        }
    };
    let named = generator.with_names(pattern, &at, &mut outside);
    let parsed = DateTimePattern::try_from_pattern_str(&named).ok()?;
    let preferences = DateTimeFormatterPreferences::from(&date_locale(tag));
    // The pattern's fields in the calendar, by ICU4X's names for it.
    macro_rules! written_in {
        ($calendar:expr, $kind:ty) => {{
            let mut names =
                FixedCalendarDateTimeNames::<$kind, CompositeFieldSet>::try_new(preferences)
                    .ok()?;
            let formatter = names.include_for_pattern(&parsed).ok()?;
            let input = ZonedDateTime {
                date: zoned.zoned.date.to_calendar($calendar),
                time: zoned.zoned.time,
                zone: zoned.zoned.zone,
            };
            let formatted = formatter.format(&input);
            formatted.try_write_to_string().ok()?.into_owned()
        }};
    }
    let tabular = |epoch| Hijri::new_tabular(TabularAlgorithmLeapYears::TypeII, epoch);
    if let Some((year, month, _)) = islamic {
        // The month's first day in a calendar of the same months.
        let calendar = tabular(TabularAlgorithmEpoch::Friday);
        let first = Date::try_new(
            year.into(),
            icu_calendar::types::Month::new(month),
            1,
            calendar,
        )
        .ok()?;
        let mut names =
            FixedCalendarDateTimeNames::<Hijri<TabularAlgorithm>, CompositeFieldSet>::try_new(
                preferences,
            )
            .ok()?;
        let formatter = names.include_for_pattern(&parsed).ok()?;
        let input = ZonedDateTime {
            date: first,
            time: zoned.zoned.time,
            zone: zoned.zoned.zone,
        };
        return formatter
            .format(&input)
            .try_write_to_string()
            .ok()
            .map(|text| text.into_owned());
    }
    Some(match calendar {
        "buddhist" => written_in!(Buddhist, Buddhist),
        "persian" => written_in!(Persian, Persian),
        "japanese" => written_in!(Japanese::new(), Japanese),
        "chinese" => written_in!(ChineseTraditional::new(), ChineseTraditional),
        "dangi" => written_in!(KoreanTraditional::new(), KoreanTraditional),
        "coptic" => written_in!(Coptic, Coptic),
        "ethiopic" => written_in!(Ethiopian::new(), Ethiopian),
        "ethiopic-amete-alem" => written_in!(
            Ethiopian::new_with_era_style(EthiopianEraStyle::AmeteAlem),
            Ethiopian
        ),
        "hebrew" => written_in!(Hebrew, Hebrew),
        "indian" => written_in!(Indian, Indian),
        "roc" => written_in!(Roc, Roc),
        #[allow(deprecated)]
        "islamic" => written_in!(Hijri::new_simulated_mecca(), Hijri<AstronomicalSimulation>),
        "islamic-civil" => written_in!(
            tabular(TabularAlgorithmEpoch::Friday),
            Hijri<TabularAlgorithm>
        ),
        "islamic-tbla" => written_in!(
            tabular(TabularAlgorithmEpoch::Thursday),
            Hijri<TabularAlgorithm>
        ),
        "islamic-umalqura" => written_in!(Hijri::new_umm_al_qura(), Hijri<UmmAlQura>),
        _ => written_in!(Gregorian, Gregorian),
    })
}

/// The reference browser's islamic (astronomical) calendar's months,
/// ICU's (`intl_islamic.txt`, from Chrome 156): the first's year, month
/// and first day, and each one's length.
const ISLAMIC: &str = include_str!("intl_islamic.txt");

/// [`ISLAMIC`] read: the first month's year, month and first day, and
/// whether each month has 30 days.
type IslamicMonths = (i32, u8, i64, Vec<bool>);

/// The islamic (ICU's astronomical) year, month and day of a day (days
/// since 1970-01-01), where the table has it.
fn islamic_date(day: i64) -> Option<(i32, u8, u8)> {
    static READ: OnceLock<Option<IslamicMonths>> = OnceLock::new();
    let (year, month, first, lengths) = READ
        .get_or_init(|| {
            let mut lines = ISLAMIC.lines().filter(|line| !line.starts_with('#'));
            let numbers: Vec<i64> = lines
                .next()?
                .split(' ')
                .filter_map(|n| n.parse().ok())
                .collect();
            let [year, month, first, count] = numbers.as_slice() else {
                return None;
            };
            let lengths: Vec<bool> = lines
                .next()?
                .chars()
                .filter_map(|c| c.to_digit(16))
                .flat_map(|nibble| (0..4).rev().map(move |bit| nibble >> bit & 1 == 1))
                .take(*count as usize)
                .collect();
            Some((*year as i32, *month as u8, *first, lengths))
        })
        .as_ref()?;
    if day < *first {
        return None;
    }
    let (mut year, mut month, mut start) = (*year, *month, *first);
    for thirty in lengths {
        let length = if *thirty { 30 } else { 29 };
        if day < start + length {
            return Some((year, month, (day - start + 1) as u8));
        }
        start += length;
        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
    }
    None
}

/// A weekday's name in a locale as a pattern's field writes it (`EEEE`),
/// of a date, by ICU4X's names, which are any calendar's.
fn weekday_text(
    tag: &str,
    zoned: &ZonedDateTime<Iso, TimeZoneInfo<AtTime>>,
    letter: char,
    count: usize,
) -> Option<String> {
    let pattern: String = std::iter::repeat_n(letter, count).collect();
    let parsed = DateTimePattern::try_from_pattern_str(&pattern).ok()?;
    let preferences = DateTimeFormatterPreferences::from(&date_locale(tag));
    let mut names =
        FixedCalendarDateTimeNames::<Gregorian, CompositeFieldSet>::try_new(preferences).ok()?;
    let formatter = names.include_for_pattern(&parsed).ok()?;
    let input = ZonedDateTime {
        date: zoned.date.to_calendar(Gregorian),
        time: zoned.time,
        zone: zoned.zone,
    };
    formatter
        .format(&input)
        .try_write_to_string()
        .ok()
        .map(|text| text.into_owned())
}

/// A zone field alone as ICU4X writes it in a locale for a zone at time
/// value `t` (the zone's offset then from its rules), kept by locale,
/// zone, field, day and offset.
fn zone_field(
    cache: &mut Cache,
    locale: &Locale,
    zone: &str,
    t: i64,
    field: &str,
) -> Option<String> {
    let offset = if zone == "UTC" || zone.is_empty() {
        0
    } else {
        intl_zone::offset_at(zone, t as f64)?
    };
    let key = (
        locale.to_string(),
        zone.to_owned(),
        field.to_owned(),
        t.div_euclid(MS_PER_DAY),
        offset,
    );
    if let Some(text) = cache.zone_fields.get(&key) {
        return text.clone();
    }
    let text = (|| {
        let at = zoned_at(zone, t as f64, f64::from(offset) / 60.0)?;
        let parsed = DateTimePattern::try_from_pattern_str(field).ok()?;
        let mut names = FixedCalendarDateTimeNames::<Gregorian, CompositeFieldSet>::try_new(
            DateTimeFormatterPreferences::from(locale),
        )
        .ok()?;
        let formatter = names.include_for_pattern(&parsed).ok()?;
        let input = ZonedDateTime {
            date: at.zoned.date.to_calendar(Gregorian),
            time: at.zoned.time,
            zone: at.zoned.zone,
        };
        let formatted = formatter.format(&input);
        formatted
            .try_write_to_string()
            .ok()
            .map(|text| text.into_owned())
    })();
    cache.zone_fields.insert(key, text.clone());
    text
}

/// A zone field written as the reference browser's ICU writes it
/// (TimeZoneFormat and TimeZoneGenericNames), ICU4X's names for the
/// parts. A specific name (`z`): the zone's, else its offset in the
/// locale's GMT format. A generic one (`v`): its metazone's standard
/// name where the zone keeps no daylight time near the instant, else
/// its metazone's generic name (with the zone's place where the zone
/// keeps another time than the metazone's reference zone for the
/// locale's region), else the location format of its region (its own
/// or primary zone) or city, and for a zone of no region its offset.
/// A locale whose zone data is root's has no names.
fn zone_name(
    cache: &mut Cache,
    generator: &intl_dates::Generator,
    locale: &Locale,
    zoned: &Zoned,
    letter: char,
    count: usize,
    zero: char,
) -> Option<String> {
    let long = count >= 4;
    let offset = zoned
        .zoned
        .zone
        .offset()
        .map_or(0, |offset| offset.to_seconds());
    let t = zoned.local - i64::from(offset) * 1000;
    let gmt = generator.gmt(offset, long, zero);
    let root = generator.zone_names() == intl_dates::ZoneNames::Root;
    let (specific, offset_field) = if long { ("zzzz", "OOOO") } else { ("z", "O") };
    // ICU4X's specific name for a zone, `None` where it falls back to
    // the offset.
    // ICU4X's specific name for a zone, `None` where it falls back to the
    // offset; the data's where ICU4X's differs (the zone's own name, else
    // its metazone's).
    let named = |cache: &mut Cache, zone: &str| -> Option<String> {
        let reported = intl_zone::zone_named(zone).map_or(zone, |(_, reported)| reported);
        let kind = format!(
            "{}{}",
            if long { 'l' } else { 's' },
            if intl_zone::daylight_at(zone, t as f64) {
                'd'
            } else {
                's'
            }
        );
        let overridden = generator
            .zone_override(&format!("{reported}/{kind}"))
            .or_else(|| {
                let metazone = intl_dates::metazone_of(reported, t)?;
                generator.zone_override(&format!("{metazone}/{kind}"))
            });
        if let Some(name) = overridden {
            return name;
        }
        if root {
            return None;
        }
        // No metazone then (ICU's mappings start in 1970): the offset,
        // but for the zones with names of their own (UTC's, and London's
        // and Dublin's daylight time), as ICU4X names them later.
        let at = if intl_dates::metazone_of(reported, t).is_some() {
            t
        } else {
            match reported {
                "UTC" | "Etc/UTC" => t,
                "Europe/London" | "Europe/Dublin" if kind.ends_with('d') => 962_409_600_000,
                _ => return None,
            }
        };
        let name = zone_field(cache, locale, zone, at, specific)?;
        (name != zone_field(cache, locale, zone, at, offset_field)?).then_some(name)
    };
    if letter == 'z' {
        return Some(named(cache, &zoned.name).unwrap_or(gmt));
    }
    if letter != 'v' {
        return None;
    }
    let Some((_, zone)) = intl_zone::zone_named(&zoned.name) else {
        return Some(gmt);
    };
    let location = intl_dates::location_of(zone);
    let region_name = |region: &str| -> String {
        if !root && let Some(Some(name)) = generator.zone_override(&format!("C/{region}")) {
            return name;
        }
        Region::try_from_str(region)
            .ok()
            .filter(|_| !root)
            .and_then(|code| {
                let preferences = DisplayNamesPreferences::from(locale);
                let names =
                    RegionDisplayNames::try_new(preferences, DisplayNamesOptions::default())
                        .ok()?;
                names.of(code).map(str::to_owned)
            })
            .unwrap_or_else(|| region.to_owned())
    };
    // The city a zone's name ends in (the reference browser's data has
    // few cities' names).
    let city = generator
        .zone_override(&format!("{zone}/ec"))
        .flatten()
        .unwrap_or_else(|| zone.rsplit('/').next().unwrap_or(zone).replace('_', " "));
    if !root && let Some(metazone) = intl_dates::metazone_of(zone, t) {
        // The metazone's generic name: ICU4X's for its world reference
        // zone, none where ICU4X writes that zone's location or (a
        // metazone of no daylight time) its standard name instead.
        let generic_field = if long { "vvvv" } else { "v" };
        // ICU4X writes a metazone of no daylight time with no generic
        // name by its standard one.
        let generic_kind = format!("{metazone}/{}g", if long { 'l' } else { 's' });
        let generic = match generator.zone_override(&generic_kind) {
            Some(name) => name,
            None => intl_dates::golden_zone(metazone, "001").and_then(|golden| {
                let generic = zone_field(cache, locale, golden, t, generic_field)?;
                let location = zone_field(cache, locale, golden, t, "VVVV")?;
                let standard = (!intl_zone::keeps_daylight(golden))
                    .then(|| zone_field(cache, locale, golden, t, specific))
                    .flatten();
                (generic != location && Some(&generic) != standard.as_ref()).then_some(generic)
            }),
        };
        if !intl_zone::daylight_near(zone, t as f64)
            && let Some(standard) = named(cache, zone)
            && Some(&standard) != generic.as_ref()
        {
            return Some(standard);
        }
        if let Some(generic) = generic {
            // The locale's region, the reference zone's ICU picks.
            let mut id = locale.id.clone();
            LocaleExpander::new_common().maximize(&mut id);
            let target = id
                .region
                .map_or_else(|| "001".to_owned(), |region| region.to_string());
            if let Some(reference) = intl_dates::golden_zone(metazone, &target)
                && reference != zone
                && intl_zone::offset_at(reference, t as f64) != intl_zone::offset_at(zone, t as f64)
            {
                let place = match location {
                    Some((region, _))
                        if intl_dates::golden_zone(metazone, region) == Some(zone) =>
                    {
                        region_name(region)
                    }
                    _ => city,
                };
                return Some(generator.in_fallback_format(&place, &generic));
            }
            return Some(generic);
        }
    }
    match location {
        Some((region, true)) => Some(generator.in_region_format(&region_name(region))),
        Some((_, false)) => Some(generator.in_region_format(&city)),
        None => Some(gmt),
    }
}

/// V8's spacing of a date: the narrow no-break and thin spaces of ICU's
/// patterns written as spaces.
fn v8_spaced(text: &str) -> String {
    text.replace(['\u{202f}', '\u{2009}'], " ")
}

/// A formatted date's parts by its pattern, as `intlDateTime` gives
/// them, the pattern after.
fn pattern_parts(pattern: &str, text: &str) -> Vec<String> {
    let pattern = v8_spaced(pattern);
    let text = v8_spaced(text);
    let pieces = align(&pattern_items(&pattern), &text)
        .unwrap_or_else(|| vec![("literal".to_owned(), text.clone())]);
    let mut answer = Vec::with_capacity(pieces.len() * 2 + 1);
    for (kind, value) in pieces {
        answer.push(kind);
        answer.push(value);
    }
    answer.push(pattern);
    answer
}

/// The options a pattern's fields resolve to (ECMA-402 section 11.3.5's
/// table, as V8 reads them off its pattern), as pairs of name and value.
fn components_of(pattern: &str) -> Vec<String> {
    let mut out: Vec<(usize, &str, String)> = Vec::new();
    for item in pattern_items(pattern) {
        let Item::Field(letter, count) = item else {
            continue;
        };
        let text = |names: [&'static str; 3]| -> &'static str {
            match count {
                1..=3 => names[0],
                4 => names[1],
                _ => names[2],
            }
        };
        let numeric = if count == 2 { "2-digit" } else { "numeric" };
        let found: Option<(usize, &str, String)> = match letter {
            'E' | 'c' | 'e' => Some((0, "weekday", text(["short", "long", "narrow"]).to_owned())),
            'G' => Some((1, "era", text(["short", "long", "narrow"]).to_owned())),
            'y' => Some((2, "year", numeric.to_owned())),
            'M' | 'L' => Some((
                3,
                "month",
                match count {
                    1 => "numeric",
                    2 => "2-digit",
                    3 => "short",
                    4 => "long",
                    _ => "narrow",
                }
                .to_owned(),
            )),
            'd' => Some((4, "day", numeric.to_owned())),
            'B' => Some((5, "dayPeriod", text(["short", "long", "narrow"]).to_owned())),
            'h' | 'H' | 'k' | 'K' => Some((6, "hour", numeric.to_owned())),
            'm' => Some((7, "minute", numeric.to_owned())),
            's' => Some((8, "second", numeric.to_owned())),
            'S' => Some((9, "fractionalSecondDigits", count.min(3).to_string())),
            'z' => Some((
                10,
                "timeZoneName",
                if count >= 4 { "long" } else { "short" }.to_owned(),
            )),
            'O' => Some((
                10,
                "timeZoneName",
                if count >= 4 {
                    "longOffset"
                } else {
                    "shortOffset"
                }
                .to_owned(),
            )),
            'v' => Some((
                10,
                "timeZoneName",
                if count >= 4 {
                    "longGeneric"
                } else {
                    "shortGeneric"
                }
                .to_owned(),
            )),
            _ => None,
        };
        if let Some(found) = found
            && !out.iter().any(|(at, _, _)| *at == found.0)
        {
            out.push(found);
        }
    }
    out.sort_by_key(|(at, _, _)| *at);
    out.into_iter()
        .flat_map(|(_, name, value)| [name.to_owned(), value])
        .collect()
}

/// The fields of a zoned time in a calendar ICU compares a range's
/// dates by: era, year, month, day, day period, hour, minute, second and
/// millisecond.
fn range_fields(at: &Zoned, calendar: &str) -> [i64; 9] {
    let (era, year, month, day) = calendar_fields(&at.zoned.date, calendar);
    let in_day = at.local.rem_euclid(MS_PER_DAY);
    let hour = in_day / 3_600_000;
    [
        i64::from(era),
        year,
        month as i64,
        i64::from(day),
        i64::from(hour >= 12),
        hour,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000,
    ]
}

/// Whether the reference browser writes a numeric day and month of one
/// digit unpadded in the locale: ICU matches the options to CLDR's
/// `availableFormats` (`yMd` is `d.M.y` in German), where ICU4X's field
/// sets take the short date format (`dd.MM.yy`), which pads them.
fn unpadded_numeric(locale: &Locale) -> (bool, bool) {
    let region = locale.id.region.map(|region| region.to_string());
    match (locale.id.language.as_str(), region.as_deref()) {
        ("en", Some("IN")) | ("es", Some("MX" | "US")) | ("nl", Some("BE")) => (true, true),
        ("de" | "it" | "nl" | "ja" | "da" | "nb" | "no" | "nn" | "cs" | "id" | "vi", _) => {
            (true, true)
        }
        ("pl", _) => (true, false),
        _ => (false, false),
    }
}

// ---- Ranges (ECMA-402 section 15.5.21, section 11.5.9, section 17.5.4) --

/// Parts with the range part each came from (`startRange`, `endRange`
/// or `shared`), as triples of type, text and source.
struct RangeParts(Vec<String>);

impl RangeParts {
    fn extend(&mut self, pairs: &[String], source: &str) {
        for pair in pairs.chunks(2) {
            if let [kind, text] = pair {
                self.0.push(kind.clone());
                self.0.push(text.clone());
                self.0.push(source.to_owned());
            }
        }
    }
}

/// The parts of a number that are its digits, signs and notation, not
/// an affix of its style or its compact form.
fn numeric_part(kind: &str) -> bool {
    !matches!(
        kind,
        "currency" | "unit" | "percentSign" | "compact" | "literal"
    )
}

/// The pairs before a number's first numeric part and after its last.
fn affix_split(pairs: &[String]) -> (usize, usize) {
    let kinds: Vec<&str> = pairs.iter().step_by(2).map(String::as_str).collect();
    let first = kinds.iter().position(|k| numeric_part(k)).unwrap_or(0);
    let last = kinds
        .iter()
        .rposition(|k| numeric_part(k))
        .map_or(kinds.len(), |at| at + 1);
    (first * 2, last * 2)
}

/// The text of pairs.
fn pairs_text(pairs: &[String]) -> String {
    pairs
        .iter()
        .skip(1)
        .step_by(2)
        .map(String::as_str)
        .collect()
}

/// The locale's range separator (CLDR's `miscPatterns` range), as the
/// reference browser writes it.
fn range_separator(locale: &Locale) -> &'static str {
    let region = locale.id.region.map(|region| region.to_string());
    match (locale.id.language.as_str(), region.as_deref()) {
        ("pt", Some(region)) if region != "BR" => " - ",
        ("ca" | "da" | "es" | "fil" | "gu" | "it" | "ml" | "nl" | "th" | "vi" | "zh", _) => "-",
        ("bg" | "hr" | "sk", _) => " \u{2013} ",
        ("ro", _) => " - ",
        ("et", _) => "\u{2012}",
        ("ja", _) => "\u{ff5e}",
        ("ko", _) => "~",
        _ => "\u{2013}",
    }
}

/// The locale's approximately sign (CLDR's `approximatelySign`), as the
/// reference browser writes it.
fn approximately_sign(locale: &Locale) -> &'static str {
    let region = locale.id.region.map(|region| region.to_string());
    match (locale.id.language.as_str(), region.as_deref()) {
        ("fr", Some("CA")) => "\u{2248}",
        ("de" | "et" | "ru", _) => "\u{2248}",
        ("fr", _) => "\u{2243}",
        ("ja", _) => "\u{7d04}",
        ("lt", _) => "\u{223c}",
        ("nb" | "no", _) => "ca.",
        _ => "~",
    }
}

/// `intlNumberRange(locale, codes, unit, x, y)`: section 15.5.21's
/// PartitionNumberRangePattern as ICU does it: one number, marked
/// approximate, if both format alike; else both, the style's affix
/// written once where both share it and it is longer than one
/// character (ICU's heuristic), the locale's range separator between,
/// spaced when an affix or a sign is repeated.
fn format_number_range(
    cache: &mut Cache,
    tag: &str,
    codes: &[i32],
    unit: &str,
    x: Num,
    y: Num,
) -> Option<Vec<String>> {
    let first = format_number(cache, tag, codes, unit, x)?;
    let second = format_number(cache, tag, codes, unit, y)?;
    let mut out = RangeParts(Vec::new());
    if first == second {
        let at = first
            .iter()
            .step_by(2)
            .position(|kind| kind == "minusSign" || kind == "plusSign")
            .map_or(0, |at| at * 2);
        out.extend(&first[..at], "shared");
        let sign = approximately_sign(&locale_of(tag)).to_owned();
        out.extend(&["approximatelySign".to_owned(), sign], "shared");
        out.extend(&first[at..], "shared");
        return Some(out.0);
    }
    let (x_start, x_end) = affix_split(&first);
    let (y_start, y_end) = affix_split(&second);
    // The style's affixes, before and after the number together, are
    // written once if both numbers share them and they are longer than
    // one character (ICU's heuristic for its middle modifier).
    let affixes = |pairs: &[String], start: usize, end: usize| {
        (pairs[..start].to_vec(), pairs[end..].to_vec())
    };
    let shared = affixes(&first, x_start, x_end) == affixes(&second, y_start, y_end)
        && first[..x_start]
            .iter()
            .chain(&first[x_end..])
            .step_by(2)
            .any(|kind| kind != "literal")
        && pairs_text(&first[..x_start]).chars().count()
            + pairs_text(&first[x_end..]).chars().count()
            > 1;
    let (prefix, suffix) = (shared, shared);
    let first_kept =
        &first[if prefix { x_start } else { 0 }..if suffix { x_end } else { first.len() }];
    let second_kept =
        &second[if prefix { y_start } else { 0 }..if suffix { y_end } else { second.len() }];
    let spaced = first_kept
        .iter()
        .chain(second_kept)
        .step_by(2)
        .any(|kind| !matches!(kind.as_str(), "integer" | "group" | "decimal" | "fraction"));
    let separator = range_separator(&locale_of(tag));
    let separator = if spaced && !separator.starts_with(' ') {
        format!(" {separator} ")
    } else {
        separator.to_owned()
    };
    if prefix {
        out.extend(&first[..x_start], "shared");
    }
    out.extend(first_kept, "startRange");
    out.extend(&["literal".to_owned(), separator], "shared");
    out.extend(second_kept, "endRange");
    if suffix {
        out.extend(&first[x_end..], "shared");
    }
    Some(out.0)
}

/// The locale's interval fallback separator (CLDR's
/// `intervalFormatFallback`).
fn interval_separator(locale: &Locale) -> &'static str {
    match locale.id.language.as_str() {
        "en" => "\u{2009}\u{2013}\u{2009}",
        "ja" => "\u{ff5e}",
        "ko" => " ~ ",
        "zh" => " - ",
        _ => " \u{2013} ",
    }
}

/// `intlDateTimeRange(locale, codes, zone, x, xOffset, y, yOffset)`:
/// section 11.5.9's PartitionDateTimeRangePattern: one date if the two
/// format alike, else both apart by the locale's interval fallback, as
/// triples of type, text and source.
#[allow(clippy::too_many_arguments)]
fn format_date_time_range(
    cache: &mut Cache,
    tag: &str,
    codes: &[i32],
    zone: &str,
    x: f64,
    x_offset: f64,
    y: f64,
    y_offset: f64,
) -> Option<Vec<String>> {
    if let Some(pattern) = reference_pattern(cache, tag, codes)
        && let Some(generator) = generator_of(cache, tag)
        && let (Some(from), Some(to)) = (zoned_at(zone, x, x_offset), zoned_at(zone, y, y_offset))
    {
        let calendar = calendar_of(cache, tag).unwrap_or("gregorian");
        let (a, b) = (range_fields(&from, calendar), range_fields(&to, calendar));
        let fields = [
            intl_dates::Difference::Era,
            intl_dates::Difference::Year,
            intl_dates::Difference::Month,
            intl_dates::Difference::Day,
            intl_dates::Difference::DayPeriod,
            intl_dates::Difference::Hour,
            intl_dates::Difference::Minute,
            intl_dates::Difference::Second,
            intl_dates::Difference::Millisecond,
        ];
        let difference = (0..9).find(|at| a[*at] != b[*at]).map(|at| fields[at]);
        let same_day = a[..4] == b[..4];
        let skeleton = intl_dates::skeleton_of_pattern(&pattern);
        let plan = intl_dates::range_plan(&generator, &skeleton, &pattern, difference, same_day);
        if let Some(parts) = render_range(cache, tag, &plan, &from, &to) {
            return Some(parts);
        }
    }
    let mut first = format_date_time(cache, tag, codes, zone, x, x_offset)?;
    let mut second = format_date_time(cache, tag, codes, zone, y, y_offset)?;
    first.pop();
    second.pop();
    let mut out = RangeParts(Vec::new());
    if first == second {
        out.extend(&first, "shared");
        return Some(out.0);
    }
    out.extend(&first, "startRange");
    out.extend(
        &[
            "literal".to_owned(),
            interval_separator(&locale_of(tag)).to_owned(),
        ],
        "shared",
    );
    out.extend(&second, "endRange");
    Some(out.0)
}

/// A range written by its plan: triples of type, text and source. Of
/// two patterns, the fields both write are each date's (and the text
/// between them), the rest shared; a fallback's two dates are each one
/// whole.
fn render_range(
    cache: &mut Cache,
    tag: &str,
    plan: &intl_dates::Range,
    from: &Zoned,
    to: &Zoned,
) -> Option<Vec<String>> {
    let mut parts = |pattern: &str, at: &Zoned| -> Option<Vec<String>> {
        let text = format_with_pattern(cache, tag, pattern, at)?;
        let mut flat = pattern_parts(pattern, &text);
        flat.pop();
        Some(flat)
    };
    let mut out = RangeParts(Vec::new());
    match plan {
        intl_dates::Range::Single(pattern) => out.extend(&parts(pattern, from)?, "shared"),
        intl_dates::Range::Split(first, second) => {
            let letters = |pattern: &str| -> Vec<char> {
                pattern_items(pattern)
                    .iter()
                    .filter_map(|item| match item {
                        Item::Field(c, _) => Some(if *c == 'L' { 'M' } else { *c }),
                        Item::Literal(_) => None,
                    })
                    .collect()
            };
            let (in_first, in_second) = (letters(first), letters(second));
            let both = |c: char| in_first.contains(&c) && in_second.contains(&c);
            let first_items = pattern_items(first);
            let second_items = pattern_items(second);
            let start = parts(first, from)?;
            let end = parts(second, to)?;
            out.0.extend(sourced(&start, &first_items, &both, true));
            out.0.extend(sourced(&end, &second_items, &both, false));
        }
        intl_dates::Range::Fallback {
            pattern,
            separator,
            date,
        } => {
            let separator = v8_spaced(separator);
            let (before, rest) = separator.split_once("{0}").unwrap_or(("", " - {1}"));
            let (middle, after) = rest.split_once("{1}").unwrap_or((rest, ""));
            let mut times = RangeParts(Vec::new());
            times.extend(&["literal".to_owned(), before.to_owned()], "shared");
            match date {
                Some((date_pattern, time_pattern, _)) => {
                    times.extend(&parts(time_pattern, from)?, "startRange");
                    times.extend(&["literal".to_owned(), middle.to_owned()], "shared");
                    times.extend(&parts(time_pattern, to)?, "endRange");
                    times.extend(&["literal".to_owned(), after.to_owned()], "shared");
                    let _ = date_pattern;
                }
                None => {
                    times.extend(&parts(pattern, from)?, "startRange");
                    times.extend(&["literal".to_owned(), middle.to_owned()], "shared");
                    times.extend(&parts(pattern, to)?, "endRange");
                    times.extend(&["literal".to_owned(), after.to_owned()], "shared");
                }
            }
            match date {
                Some((date_pattern, _, glue)) => {
                    // The date once, the times' range by the glue.
                    let glue = v8_spaced(glue);
                    let (head, tail) = glue.split_once("{1}").unwrap_or(("", glue.as_str()));
                    let (between, last) = tail.split_once("{0}").unwrap_or((tail, ""));
                    out.extend(&["literal".to_owned(), unquote(head)], "shared");
                    out.extend(&parts(date_pattern, from)?, "shared");
                    out.extend(&["literal".to_owned(), unquote(between)], "shared");
                    out.0.extend(times.0);
                    out.extend(&["literal".to_owned(), unquote(last)], "shared");
                }
                None => out.0.extend(times.0),
            }
        }
    }
    // Empty literals dropped, neighbours of a source joined.
    let mut joined: Vec<String> = Vec::new();
    for triple in out.0.chunks(3) {
        if let [kind, text, source] = triple {
            if text.is_empty() {
                continue;
            }
            let n = joined.len();
            if n >= 3 && kind == "literal" && joined[n - 3] == "literal" && joined[n - 1] == *source
            {
                joined[n - 2].push_str(text);
                continue;
            }
            joined.extend([kind.clone(), text.clone(), source.clone()]);
        }
    }
    Some(joined)
}

/// A pattern's literal text, its quotes undone.
fn unquote(text: &str) -> String {
    pattern_items(text)
        .into_iter()
        .map(|item| match item {
            Item::Literal(text) => text,
            Item::Field(c, n) => std::iter::repeat_n(c, n).collect(),
        })
        .collect()
}

/// One date's parts in a split range, each with its source: the span
/// from its first field the other date also writes to its last field
/// (the first date), or from its start to its last field both write
/// (the second), is the date's own, the rest shared.
fn sourced(
    flat: &[String],
    items: &[Item],
    both: &dyn Fn(char) -> bool,
    first: bool,
) -> Vec<String> {
    // The fields' letters in their order, to find the span by.
    let letters: Vec<char> = items
        .iter()
        .filter_map(|item| match item {
            Item::Field(c, _) => Some(if *c == 'L' { 'M' } else { *c }),
            Item::Literal(_) => None,
        })
        .collect();
    let kinds: Vec<&str> = flat.iter().step_by(2).map(String::as_str).collect();
    let field_at: Vec<usize> = kinds
        .iter()
        .enumerate()
        .filter(|(_, kind)| **kind != "literal")
        .map(|(at, _)| at)
        .collect();
    let (start, end) = if first {
        let first_both = letters.iter().position(|c| both(*c));
        let last_field = field_at.last().copied();
        (
            first_both
                .and_then(|n| field_at.get(n).copied())
                .unwrap_or(kinds.len()),
            last_field.unwrap_or(0),
        )
    } else {
        let last_both = letters.iter().rposition(|c| both(*c));
        (
            0,
            last_both
                .and_then(|n| field_at.get(n).copied())
                .unwrap_or(0),
        )
    };
    let source = if first { "startRange" } else { "endRange" };
    let mut out = Vec::new();
    for (at, pair) in flat.chunks(2).enumerate() {
        if let [kind, text] = pair {
            let own = at >= start && at <= end && start <= end;
            out.extend([
                kind.clone(),
                text.clone(),
                if own { source } else { "shared" }.to_owned(),
            ]);
        }
    }
    out
}

/// CLDR's plural ranges as the reference browser's ICU has them: each
/// set's start, end and result categories, and each locale's set.
const PLURAL_RANGE_SETS: [(&str, &str); 17] = [
    ("set00", "other other other"),
    ("set01", "one one one,one other other,other other other"),
    ("set03", "one other other,other one one,other other other"),
    ("set04", "one other other,other one other,other other other"),
    (
        "set05",
        "one one one,one other other,other one one,other other other",
    ),
    (
        "set07",
        "one one other,one other other,other one one,other other other",
    ),
    (
        "set09",
        "zero zero other,zero one one,zero other other,one zero other,one one one,one other other,other zero other,other one one,other other other",
    ),
    (
        "set10",
        "one two other,one other other,two other other,other one other,other two other,other other other",
    ),
    (
        "set11",
        "one few few,one other other,few one few,few few few,few other other,other few few,other other other",
    ),
    (
        "set12",
        "one one one,one few few,one other other,few one one,few few few,few other other,other one one,other few few,other other other",
    ),
    ("set13", "one one one,one other other,other other other"),
    ("set14", "one other other,other one one,other other other"),
    ("set15", "one other other,other one other,other other other"),
    (
        "set16",
        "one one few,one two two,one few few,one other other,two one few,two two two,two few few,two other other,few one few,few two two,few few few,few other other,other one few,other two two,other few few,other other other",
    ),
    (
        "set17",
        "one few few,one many many,one other other,few few few,few many many,few other other,many one one,many few few,many many many,many other other,other one one,other few few,other many many,other other other",
    ),
    (
        "set18",
        "one one one,one few few,one many many,one other other,few one one,few few few,few many many,few other other,many one one,many few few,many many many,many other other,other one one,other few few,other many many,other other other",
    ),
    (
        "set21",
        "zero one zero,zero two zero,zero few few,zero many many,zero other other,one two other,one few few,one many many,one other other,two few few,two many many,two other other,few few few,few many many,few other other,many few few,many many many,many other other,other one other,other two other,other few few,other many many,other other other",
    ),
];
const PLURAL_RANGE_LOCALES: [(&str, &str); 59] = [
    ("af", "set04"),
    ("ak", "set07"),
    ("am", "set01"),
    ("ar", "set21"),
    ("az", "set03"),
    ("bg", "set04"),
    ("bn", "set01"),
    ("bs", "set12"),
    ("ca", "set15"),
    ("cs", "set17"),
    ("da", "set05"),
    ("de", "set03"),
    ("el", "set03"),
    ("en", "set04"),
    ("es", "set15"),
    ("et", "set04"),
    ("fa", "set07"),
    ("fi", "set04"),
    ("fil", "set05"),
    ("fr", "set13"),
    ("gu", "set01"),
    ("he", "set10"),
    ("hi", "set01"),
    ("hr", "set12"),
    ("hu", "set03"),
    ("id", "set00"),
    ("it", "set14"),
    ("ja", "set00"),
    ("kk", "set03"),
    ("kn", "set01"),
    ("ko", "set00"),
    ("lt", "set18"),
    ("lv", "set09"),
    ("ml", "set03"),
    ("mr", "set01"),
    ("ms", "set00"),
    ("nb", "set04"),
    ("nl", "set03"),
    ("no", "set04"),
    ("pa", "set05"),
    ("pl", "set17"),
    ("pt", "set13"),
    ("ro", "set11"),
    ("ru", "set18"),
    ("sd", "set07"),
    ("sk", "set17"),
    ("sl", "set16"),
    ("sr", "set12"),
    ("sv", "set04"),
    ("sw", "set03"),
    ("ta", "set03"),
    ("te", "set03"),
    ("th", "set00"),
    ("tr", "set03"),
    ("uk", "set18"),
    ("ur", "set04"),
    ("uz", "set03"),
    ("vi", "set00"),
    ("zh", "set00"),
];

/// `intlPluralRange(locale, ordinal, codes, x, y)`: section 17.5.4's
/// ResolvePluralRange, of the numbers rounded: the range of their
/// categories by the locale's set of plural ranges, or `other` where it
/// has none (ICU's StandardPluralRanges).
fn plural_range(
    cache: &mut Cache,
    tag: &str,
    ordinal: bool,
    codes: &[i32],
    x: f64,
    y: f64,
) -> String {
    let Some(rules) = plural_rules(cache, tag, ordinal) else {
        return "other".to_owned();
    };
    let digits = Digits::of(codes);
    let round = |n: f64| match finite(n) {
        Num::Finite(mut value) => {
            value.sign = Sign::None;
            Some(digits.round(&value))
        }
        _ => None,
    };
    let (Some(x), Some(y)) = (round(x), round(y)) else {
        return "other".to_owned();
    };
    let (start, end) = (
        category_name(rules.category_for(&x)),
        category_name(rules.category_for(&y)),
    );
    let set = locale_keys(&plural_locale(tag)).iter().find_map(|key| {
        PLURAL_RANGE_LOCALES
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, set)| *set)
    });
    let ranges = set.and_then(|set| PLURAL_RANGE_SETS.iter().find(|(name, _)| *name == set));
    ranges
        .and_then(|(_, ranges)| {
            ranges.split(',').find_map(|range| {
                let mut parts = range.split(' ');
                let (from, to, result) = (parts.next()?, parts.next()?, parts.next()?);
                (from == start && to == end).then_some(result)
            })
        })
        .unwrap_or("other")
        .to_owned()
}

// ---- Locale information (Intl Locale Info) -----------------------------

/// The locale's region, or its likely one.
fn region_of(locale: &Locale) -> String {
    let mut id = locale.id.clone();
    LocaleExpander::new_extended().maximize(&mut id);
    id.region.map(|r| r.to_string()).unwrap_or_default()
}

fn weekday_number(day: icu_calendar::types::Weekday) -> i32 {
    use icu_calendar::types::Weekday as W;
    match day {
        W::Monday => 1,
        W::Tuesday => 2,
        W::Wednesday => 3,
        W::Thursday => 4,
        W::Friday => 5,
        W::Saturday => 6,
        W::Sunday => 7,
    }
}

/// `intlWeekInfo(locale)`: the Locale Info proposal's week of the
/// locale, as the reference browser has it: its first day, then its
/// weekend's days, Monday 1 to Sunday 7.
fn week_info(tag: &str) -> Vec<i32> {
    let locale = locale_of(tag);
    let Ok(info) = icu_calendar::week::WeekInformation::try_new((&locale).into()) else {
        return vec![1, 6, 7];
    };
    let mut weekend: Vec<i32> = info.weekend().map(weekday_number).collect();
    weekend.sort_unstable();
    let mut answer = vec![weekday_number(info.first_weekday)];
    answer.extend(weekend);
    answer
}

/// `intlTextDirection(locale)`: "rtl" for a locale written in a script
/// that runs right to left, else "ltr".
fn text_direction(tag: &str) -> &'static str {
    let mut id = locale_of(tag).id;
    LocaleExpander::new_extended().maximize(&mut id);
    match id.script.as_ref().map(|s| s.as_str()) {
        Some(
            "Adlm" | "Arab" | "Hebr" | "Mand" | "Mend" | "Nkoo" | "Rohg" | "Samr" | "Syrc" | "Thaa"
            | "Yezi",
        ) => "rtl",
        _ => "ltr",
    }
}

/// `intlHourCycle(locale)`: the hour cycle the locale's region prefers
/// (CLDR's timeData, as ICU's DateTimePatternGenerator reads it).
fn hour_cycle_of(tag: &str) -> &'static str {
    match intl_dates::hour_letter(&locale_of(tag)) {
        'h' => "h12",
        'K' => "h11",
        'k' => "h24",
        _ => "h23",
    }
}

/// `intlCalendars(locale)`: the calendars the locale's region prefers
/// (CLDR's `calendarPreferenceData`), first its default.
fn calendars_of(tag: &str) -> Vec<String> {
    let region = region_of(&locale_of(tag));
    let list: &[&str] = match region.as_str() {
        "CN" | "CX" | "HK" | "MO" | "SG" => &["gregory", "chinese"],
        "TW" => &["gregory", "roc", "chinese"],
        "KR" => &["gregory", "dangi"],
        "JP" => &["gregory", "japanese"],
        "TH" => &["buddhist", "gregory"],
        "IN" => &["gregory", "indian"],
        "ET" => &["gregory", "ethiopic"],
        "IL" => &[
            "gregory",
            "hebrew",
            "islamic",
            "islamic-civil",
            "islamic-tbla",
        ],
        "AF" | "IR" => &[
            "persian",
            "gregory",
            "islamic",
            "islamic-civil",
            "islamic-tbla",
        ],
        "SA" => &["gregory", "islamic-umalqura", "islamic", "islamic-rgsa"],
        "AE" | "BH" | "KW" | "QA" => &[
            "gregory",
            "islamic-umalqura",
            "islamic",
            "islamic-civil",
            "islamic-tbla",
        ],
        "EG" => &[
            "gregory",
            "coptic",
            "islamic",
            "islamic-civil",
            "islamic-tbla",
        ],
        "DJ" | "DZ" | "EH" | "ER" | "IQ" | "JO" | "KM" | "LB" | "LY" | "MA" | "MR" | "OM"
        | "PS" | "SD" | "SY" | "TD" | "TN" | "YE" | "BD" | "ID" | "MY" | "PK" => {
            &["gregory", "islamic", "islamic-civil", "islamic-tbla"]
        }
        "AL" | "AZ" | "MV" | "TJ" | "TM" | "TR" | "UZ" | "XK" => {
            &["gregory", "islamic-civil", "islamic-tbla"]
        }
        _ => &["gregory"],
    };
    list.iter().map(|name| (*name).to_owned()).collect()
}

/// `intlCollations(locale)`: the collations CLDR has for the locale's
/// language besides its standard one and search, in order.
fn collations_of(tag: &str) -> Vec<String> {
    let extra: &[&str] = match locale_of(tag).id.language.as_str() {
        "de" => &["phonebk"],
        "es" => &["trad"],
        "zh" => &["pinyin", "stroke", "zhuyin"],
        "ja" => &["unihan"],
        "ko" => &["searchjl", "unihan"],
        "ar" => &["compat"],
        "bn" | "fi" | "kn" | "sv" | "vi" => &["trad"],
        _ => &[],
    };
    let mut list: Vec<String> = ["emoji", "eor"]
        .iter()
        .chain(extra)
        .map(|name| (*name).to_owned())
        .collect();
    list.sort();
    list
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
    let shared = cache.clone();
    natives.set(
        "intlDateTimeInfo",
        Function::new(ctx.clone(), move |tag: String, codes: Vec<i32>| {
            date_time_info(&mut shared.borrow_mut(), &tag, &codes)
        })?,
    )?;
    let shared = cache.clone();
    natives.set(
        "intlNumberRange",
        Function::new(
            ctx.clone(),
            move |tag: String,
                  codes: Vec<i32>,
                  unit: String,
                  x: Value<'js>,
                  y: Value<'js>|
                  -> Vec<String> {
                let (x, y) = (number_of(&x), number_of(&y));
                format_number_range(&mut shared.borrow_mut(), &tag, &codes, &unit, x, y)
                    .unwrap_or_default()
            },
        )?,
    )?;
    let shared = cache.clone();
    natives.set(
        "intlDateTimeRange",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  tag: String,
                  codes: Vec<i32>,
                  zone: String,
                  x: Vec<f64>,
                  y: Vec<f64>|
                  -> rquickjs::Result<Vec<String>> {
                let at = |v: &[f64], i: usize| v.get(i).copied().unwrap_or(f64::NAN);
                format_date_time_range(
                    &mut shared.borrow_mut(),
                    &tag,
                    &codes,
                    &zone,
                    at(&x, 0),
                    at(&x, 1),
                    at(&y, 0),
                    at(&y, 1),
                )
                .ok_or_else(|| range_error(&ctx, "Invalid time value"))
            },
        )?,
    )?;
    let shared = cache;
    natives.set(
        "intlPluralRange",
        Function::new(
            ctx.clone(),
            move |tag: String, ordinal: bool, codes: Vec<i32>, x: f64, y: f64| {
                plural_range(&mut shared.borrow_mut(), &tag, ordinal, &codes, x, y)
            },
        )?,
    )?;
    natives.set(
        "intlHourCycle",
        Function::new(ctx.clone(), |tag: String| hour_cycle_of(&tag).to_owned())?,
    )?;
    natives.set(
        "intlWeekInfo",
        Function::new(ctx.clone(), |tag: String| week_info(&tag))?,
    )?;
    natives.set(
        "intlTextDirection",
        Function::new(ctx.clone(), |tag: String| text_direction(&tag))?,
    )?;
    natives.set(
        "intlCalendars",
        Function::new(ctx.clone(), |tag: String| calendars_of(&tag))?,
    )?;
    natives.set(
        "intlCollations",
        Function::new(ctx.clone(), |tag: String| collations_of(&tag))?,
    )?;
    natives.set(
        "intlSystemZone",
        Function::new(ctx.clone(), || intl_zone::system_zone().to_owned())?,
    )?;
    natives.set(
        "intlTimeZone",
        Function::new(ctx.clone(), |name: String| {
            intl_zone::zone_named(&name)
                .map(|(_, name)| name.to_owned())
                .unwrap_or_default()
        })?,
    )?;
    natives.set(
        "intlZoneOffset",
        Function::new(ctx.clone(), |name: String, t: f64| {
            intl_zone::offset_at(&name, t).map_or(f64::NAN, |seconds| f64::from(seconds) / 60.0)
        })?,
    )?;
    natives.set(
        "intlTimeZones",
        Function::new(ctx.clone(), |region: String| intl_zone::zones_of(&region))?,
    )?;
    super::intl_text::install(ctx, natives)
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
