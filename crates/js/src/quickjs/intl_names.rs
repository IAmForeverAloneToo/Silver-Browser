//! `Intl.DisplayNames` (ECMA-402 section 12) as the reference browser's
//! ICU names things: languages, regions, scripts and variants by
//! ICU4X's display names, composed as ICU's LocaleDisplayNames composes
//! a locale's name (a dialect's own name tried for the whole of it, then
//! the language's, its script, region and variants in the locale's
//! pattern); currencies by `icu_experimental`'s names; calendars and
//! date fields from `intl_names.txt`, generated from Chrome 156.

use std::collections::HashMap;
use std::sync::OnceLock;

use icu_experimental::dimension::provider::currency::displayname::CurrencyDisplaynameV1;
use icu_experimental::displaynames::multi::{
    LanguageDisplayNames, RegionDisplayNames, ScriptDisplayNames, VariantDisplayNames,
};
use icu_experimental::displaynames::provider::LocaleDisplayNamesV1;
use icu_experimental::displaynames::{DisplayNamesOptions, DisplayNamesPreferences, Style};
use icu_experimental::provider::Baked;
use icu_locale::subtags::{Language, Region, Script, Variant};
use icu_locale::{LanguageIdentifier, Locale};
use icu_provider::prelude::*;

/// The data: what ICU4X has no names of, and the patterns names are
/// composed by.
const DATA: &str = include_str!("intl_names.txt");

/// The calendars and the date fields, in the data's order.
const CALENDARS: [&str; 18] = [
    "buddhist",
    "chinese",
    "coptic",
    "dangi",
    "ethioaa",
    "ethiopic",
    "gregory",
    "hebrew",
    "indian",
    "islamic",
    "islamic-civil",
    "islamic-rgsa",
    "islamic-tbla",
    "islamic-umalqura",
    "iso8601",
    "japanese",
    "persian",
    "roc",
];
const FIELDS: [&str; 12] = [
    "era",
    "year",
    "quarter",
    "month",
    "weekOfYear",
    "weekday",
    "day",
    "dayPeriod",
    "hour",
    "minute",
    "second",
    "timeZoneName",
];

/// The languages whose names the reference browser's ICU has none of:
/// its own name alone, regions and currencies in English, no scripts.
const NO_NAMES: [&str; 8] = ["ak", "az", "bs", "kk", "kok", "pa", "sd", "uz"];

/// [`DATA`] read: each language's pattern and separator, its names by
/// kind (C calendars, F fields) and style, and the names that differ
/// from ICU4X's by language, type (`standard` a language's standard
/// name), style (long, or short for short and narrow) and code, empty
/// where there is none.
struct Data {
    patterns: HashMap<&'static str, (String, String)>,
    names: HashMap<(&'static str, &'static str, &'static str), Vec<String>>,
    overrides: HashMap<(&'static str, &'static str, &'static str, &'static str), String>,
    /// The codes of each type that have names at all.
    universe: HashMap<&'static str, std::collections::HashSet<&'static str>>,
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("\\u") {
        out.push_str(&rest[..at]);
        match rest
            .get(at + 2..at + 6)
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .and_then(char::from_u32)
        {
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

fn data() -> &'static Data {
    static READ: OnceLock<Data> = OnceLock::new();
    READ.get_or_init(|| {
        let mut data = Data {
            patterns: HashMap::new(),
            names: HashMap::new(),
            overrides: HashMap::new(),
            universe: HashMap::new(),
        };
        for line in DATA.lines().filter(|line| !line.starts_with('#')) {
            let mut fields = line.splitn(4, '\t');
            let (Some(language), Some(kind), Some(a), rest) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            match kind {
                "U" => {
                    let codes = rest.unwrap_or("").split(' ').collect();
                    data.universe.insert(a, codes);
                }
                "N" => {
                    // A name of another type: type, style, code, name.
                    let mut more = rest.unwrap_or("").splitn(3, '\t');
                    let (Some(style), Some(code)) = (more.next(), more.next()) else {
                        continue;
                    };
                    let name = unescape(more.next().unwrap_or(""));
                    if style == "*" {
                        data.overrides
                            .insert((language, a, "long", code), name.clone());
                        data.overrides.insert((language, a, "short", code), name);
                    } else {
                        data.overrides.insert((language, a, style, code), name);
                    }
                }
                "P" => {
                    let separator = rest.unwrap_or("{0}, {1}");
                    data.patterns
                        .insert(language, (unescape(a), unescape(separator)));
                }
                _ => {
                    let rest = rest.unwrap_or("");
                    let names = if rest == "=" {
                        data.names
                            .get(&(language, kind, "long"))
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        rest.split('\t').map(unescape).collect()
                    };
                    data.names.insert((language, kind, a), names);
                }
            }
        }
        data
    })
}

/// The options ICU4X's names are looked up with: ICU's short length for
/// the short and narrow styles.
fn options(style: &str) -> DisplayNamesOptions {
    let mut options = DisplayNamesOptions::default();
    options.style = Some(if style == "long" {
        Style::Long
    } else {
        Style::Short
    });
    options
}

/// The locale the data has for `locale`: itself, else without its
/// region, else its language where it names no script.
fn data_key(locale: &Locale) -> String {
    let id = &locale.id;
    let mut tries = Vec::new();
    let mut base = id.language.as_str().to_owned();
    if let Some(script) = id.script {
        base = format!("{base}-{script}");
    }
    if let Some(region) = id.region {
        tries.push(format!("{base}-{region}"));
    }
    tries.push(base);
    if id.script.is_none() {
        tries.push(id.language.as_str().to_owned());
    }
    tries
        .into_iter()
        .find(|key| data().patterns.contains_key(key.as_str()))
        .unwrap_or_else(|| id.language.as_str().to_owned())
}

/// The data's name of a code of a type in a locale, where it differs
/// from ICU4X's: `Some("")` for none.
fn overridden(locale: &Locale, kind: &str, style: &str, code: &str) -> Option<String> {
    let style = if style == "long" { "long" } else { "short" };
    data()
        .overrides
        .get(&(data_key(locale).as_str(), kind, style, code))
        .cloned()
}

/// The name of a code of a type, the data's where it differs, else
/// `found`'s.
fn named(
    locale: &Locale,
    kind: &str,
    style: &str,
    code: &str,
    found: impl FnOnce() -> Option<String>,
) -> Option<String> {
    match overridden(locale, kind, style, code) {
        Some(name) if name.is_empty() => None,
        Some(name) => Some(name),
        None => found(),
    }
}

/// Whether a code of a type has a name in any language.
fn in_universe(kind: &str, code: &str) -> bool {
    data()
        .universe
        .get(kind)
        .is_some_and(|codes| codes.contains(code))
}

/// Whether the locale is of a language the reference browser has no
/// names in.
fn without_names(locale: &Locale) -> bool {
    NO_NAMES.contains(&locale.id.language.as_str())
}

fn english() -> Locale {
    Locale::try_from_str("en").unwrap_or(Locale::UNKNOWN)
}

/// A region's name, the code exactly as ICU keys it (two capitals or
/// three digits).
fn region_name(locale: &Locale, style: &str, code: &str) -> Option<String> {
    if without_names(locale) {
        return region_name(&english(), style, code);
    }
    let keyed = (code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase()))
        || (code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit()));
    if !keyed || !in_universe("region", code) {
        return None;
    }
    named(locale, "region", style, code, || {
        icu_region_name(locale, style, code)
    })
}

fn icu_region_name(locale: &Locale, style: &str, code: &str) -> Option<String> {
    let region = Region::try_from_str(code).ok()?;
    let names =
        RegionDisplayNames::try_new(DisplayNamesPreferences::from(locale), options(style)).ok()?;
    names.of(region).map(str::to_owned)
}

/// A script's name, the code exactly as ICU keys it (a capital and
/// three small letters).
fn script_name(locale: &Locale, style: &str, code: &str) -> Option<String> {
    let mut bytes = code.bytes();
    let keyed = code.len() == 4
        && bytes.next().is_some_and(|b| b.is_ascii_uppercase())
        && bytes.all(|b| b.is_ascii_lowercase());
    if !keyed || without_names(locale) || !in_universe("script", code) {
        return None;
    }
    named(locale, "script", style, code, || {
        icu_script_name(locale, style, code)
    })
}

fn icu_script_name(locale: &Locale, style: &str, code: &str) -> Option<String> {
    let script = Script::try_from_str(code).ok()?;
    let names =
        ScriptDisplayNames::try_new(DisplayNamesPreferences::from(locale), options(style)).ok()?;
    names.of(script).map(str::to_owned)
}

/// A dialect's own name for a whole language identifier (`en-GB`,
/// `zh-Hans`), the short one first for a short style.
fn dialect_name(locale: &Locale, style: &str, id: &LanguageIdentifier) -> Option<String> {
    let code = id.to_string();
    if !in_universe("language", &code) || without_names(locale) {
        return None;
    }
    if let Some(name) = overridden(locale, "language", style, &code) {
        return (!name.is_empty()).then_some(name);
    }
    let data_locale = DataLocale::from(locale);
    let response: DataResponse<LocaleDisplayNamesV1> = Baked
        .load(DataRequest {
            id: DataIdentifierBorrowed::for_locale(&data_locale),
            ..Default::default()
        })
        .ok()?;
    let data = response.payload.get();
    if style != "long"
        && let Some(name) = data.short_names.get_by(|key| id.strict_cmp(key).reverse())
    {
        return Some(name.to_owned());
    }
    data.names
        .get_by(|key| id.strict_cmp(key).reverse())
        .map(str::to_owned)
}

/// A language's own name; in a language without names, its own alone.
fn language_only(locale: &Locale, style: &str, language: Language) -> Option<String> {
    if !in_universe("language", language.as_str())
        || (without_names(locale) && language != locale.id.language)
    {
        return None;
    }
    if let Some(name) = overridden(locale, "language", style, language.as_str()) {
        return (!name.is_empty()).then_some(name);
    }
    let names =
        LanguageDisplayNames::try_new(DisplayNamesPreferences::from(locale), options(style))
            .ok()?;
    names.of(language).map(str::to_owned)
}

/// A locale's name (ICU's LocaleDisplayNamesImpl::localeDisplayName):
/// with dialect names, the dialect's own name for the language and all
/// of its script and region together, else for the language and the one
/// it has; then the language's name and the rest qualifying it in the
/// locale's pattern. A part with no name is its code with `substitute`,
/// else the whole has none.
fn language_name(
    locale: &Locale,
    style: &str,
    dialect: bool,
    substitute: bool,
    code: &str,
) -> Option<String> {
    // The reference's name for the whole where it differs; one it has
    // none of is composed, as the reference composes it.
    let kind = if dialect { "language" } else { "standard" };
    if let Some(name) = overridden(locale, kind, style, code).filter(|name| !name.is_empty()) {
        return Some(name);
    }
    let parsed = Locale::try_from_str(code).ok()?;
    let id = &parsed.id;
    let mut script = id.script;
    let mut region = id.region;
    let mut name = None;
    if dialect && id.language != Language::UNKNOWN {
        // ICU appends each try to the last: with both a script and a
        // region, only the three together are found.
        let tried = match (script, region) {
            (Some(_), Some(_)) => LanguageIdentifier::from((id.language, script, region)),
            (Some(_), None) => LanguageIdentifier::from((id.language, script, None)),
            (None, Some(_)) => LanguageIdentifier::from((id.language, None, region)),
            (None, None) => LanguageIdentifier::from((id.language, None, None)),
        };
        if script.is_some() || region.is_some() {
            name = dialect_name(locale, style, &tried);
            if name.is_some() {
                script = None;
                region = None;
            }
        }
    }
    let name = match name {
        Some(name) => name,
        None if id.language == Language::UNKNOWN => {
            if !substitute {
                return None;
            }
            "root".to_owned()
        }
        None => match language_only(locale, style, id.language) {
            Some(name) => name,
            None if substitute => id.language.as_str().to_owned(),
            None => return None,
        },
    };
    let mut qualifiers = Vec::new();
    if let Some(script) = script {
        match script_name(locale, style, script.as_str()) {
            Some(text) => qualifiers.push(text),
            None if substitute => qualifiers.push(script.as_str().to_owned()),
            None => return None,
        }
    }
    if let Some(region) = region {
        match region_name(locale, style, region.as_str()) {
            Some(text) => qualifiers.push(text),
            None if substitute => qualifiers.push(region.as_str().to_owned()),
            None => return None,
        }
    }
    if !id.variants.is_empty() {
        let joined: Vec<String> = id
            .variants
            .iter()
            .map(|variant: &Variant| variant.as_str().to_ascii_uppercase())
            .collect();
        let named =
            id.variants.len() == 1 && in_universe("variant", &joined[0]) && !without_names(locale);
        let found = named
            .then(|| {
                VariantDisplayNames::try_new(DisplayNamesPreferences::from(locale), options(style))
                    .ok()
                    .and_then(|names| names.of(id.variants[0]).map(str::to_owned))
            })
            .flatten();
        match found {
            Some(text) => qualifiers.push(text),
            None if substitute => qualifiers.push(joined.join("_")),
            None => return None,
        }
    }
    if qualifiers.is_empty() {
        return Some(name);
    }
    let (pattern, separator) = data()
        .patterns
        .get(data_key(locale).as_str())
        .cloned()
        .unwrap_or_else(|| ("{0} ({1})".to_owned(), "{0}, {1}".to_owned()));
    let mut joined = qualifiers[0].clone();
    for qualifier in &qualifiers[1..] {
        joined = separator.replace("{0}", &joined).replace("{1}", qualifier);
    }
    Some(pattern.replace("{0}", &name).replace("{1}", &joined))
}

/// A currency's name, its code taken whatever its case.
fn currency_name(locale: &Locale, code: &str) -> Option<String> {
    if without_names(locale) {
        return currency_name(&english(), code);
    }
    let code = code.to_ascii_uppercase();
    if !in_universe("currency", &code) {
        return None;
    }
    if let Some(name) = overridden(locale, "currency", "long", &code) {
        return (!name.is_empty()).then_some(name);
    }
    let attributes = DataMarkerAttributes::try_from_str(&code).ok()?;
    let data_locale = DataLocale::from(locale);
    let mut metadata = DataRequestMetadata::default();
    metadata.silent = true;
    let response: DataResponse<CurrencyDisplaynameV1> = Baked
        .load(DataRequest {
            id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, &data_locale),
            metadata,
        })
        .ok()?;
    Some(response.payload.get().to_string())
}

/// `intlDisplayName(locale, type, style, standard, substitute, code)`:
/// the name of `code`, a code of the type the bootstrap checked and made
/// canonical; empty for none.
pub(super) fn display_name(
    tag: &str,
    kind: &str,
    style: &str,
    standard: bool,
    substitute: bool,
    code: &str,
) -> String {
    let locale = Locale::try_from_str(tag).unwrap_or(Locale::UNKNOWN);
    let data_key = data_key(&locale);
    let table = |key: &'static str, codes: &[&str]| -> Option<String> {
        let at = codes.iter().position(|c| *c == code)?;
        let names = data()
            .names
            .get(&(data_key.as_str(), key, style))
            .or_else(|| data().names.get(&("en", key, style)))?;
        names.get(at).filter(|name| !name.is_empty()).cloned()
    };
    let name = match kind {
        "language" => language_name(&locale, style, !standard, substitute, code),
        "region" => region_name(&locale, style, code),
        "script" => script_name(&locale, style, code),
        "currency" => currency_name(&locale, code),
        "calendar" => table("C", &CALENDARS),
        "dateTimeField" => table("F", &FIELDS),
        _ => None,
    };
    name.unwrap_or_default()
}
