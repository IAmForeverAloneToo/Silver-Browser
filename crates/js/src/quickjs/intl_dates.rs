//! Date patterns for `Intl.DateTimeFormat` (D63) as the reference
//! browser picks them: the patterns of its ICU's data (`intl_dates.txt`,
//! the gregorian calendar's, and the Buddhist and Persian ones of the
//! languages that keep them), a pattern for options found as ICU's
//! DateTimePatternGenerator finds one (a skeleton of the options matched
//! to the locale's available formats by ICU's distance, its fields'
//! widths adjusted, a date and a time joined by the locale's glue), the
//! patterns of a range as ICU's DateIntervalFormat picks them (the
//! locale's interval formats by the greatest field that differs), and
//! the zone data ICU names zones by.

use std::collections::HashMap;
use std::sync::OnceLock;

use icu_locale::{Locale, LocaleExpander};

/// The data: the date patterns and zone data of the reference browser's
/// ICU.
const DATA: &str = include_str!("intl_dates.txt");

/// [`DATA`] read: entries by locale, kind and key; aliases and parents
/// by locale; regions' hour formats; languages' day periods; and the
/// languages whose zone names are English and locales whose are none;
/// and zones' regions, metazones and metazones' reference zones.
struct Data {
    entries: HashMap<(&'static str, &'static str, &'static str), &'static str>,
    aliases: HashMap<&'static str, &'static str>,
    parents: HashMap<&'static str, &'static str>,
    hours: HashMap<&'static str, (&'static str, &'static str)>,
    periods: HashMap<&'static str, Vec<(&'static str, i32, i32)>>,
    english_zones: std::collections::HashSet<&'static str>,
    root_zones: std::collections::HashSet<&'static str>,
    locations: HashMap<&'static str, (&'static str, bool)>,
    metazones: HashMap<&'static str, Vec<(&'static str, i64, i64)>>,
    golden: HashMap<(&'static str, &'static str), &'static str>,
}

fn data() -> &'static Data {
    static DATA_READ: OnceLock<Data> = OnceLock::new();
    DATA_READ.get_or_init(|| {
        let mut data = Data {
            entries: HashMap::new(),
            aliases: HashMap::new(),
            parents: HashMap::new(),
            hours: HashMap::new(),
            periods: HashMap::new(),
            english_zones: std::collections::HashSet::new(),
            root_zones: std::collections::HashSet::new(),
            locations: HashMap::new(),
            metazones: HashMap::new(),
            golden: HashMap::new(),
        };
        for line in DATA.lines().filter(|line| !line.starts_with('#')) {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                [locale, "=", target] => {
                    data.aliases.insert(locale, target);
                }
                [locale, "^", parent] => {
                    data.parents.insert(locale, parent);
                }
                ["*", "H", region, preferred, allowed] => {
                    data.hours.insert(region, (preferred, allowed));
                }
                ["*", "Z", language, "en"] => {
                    data.english_zones.insert(language);
                }
                ["*", "Z", locale, "root"] => {
                    data.root_zones.insert(locale);
                }
                ["*", "L", zone, region, primary] => {
                    data.locations.insert(zone, (region, *primary == "1"));
                }
                ["*", "M", zone, metazone, from, to] => {
                    data.metazones.entry(zone).or_default().push((
                        metazone,
                        utc_of(from),
                        utc_of(to),
                    ));
                }
                ["*", "G", metazone, region, zone] => {
                    data.golden.insert((metazone, region), zone);
                }
                ["*", "R", language, period, from, before] => {
                    let minutes = |time: &str| -> i32 {
                        let (h, m) = time.split_once(':').unwrap_or(("0", "0"));
                        h.parse::<i32>().unwrap_or(0) * 60 + m.parse::<i32>().unwrap_or(0)
                    };
                    // An exact time ("at") has no end: -1.
                    let end = if before.is_empty() {
                        -1
                    } else {
                        minutes(before)
                    };
                    data.periods
                        .entry(language)
                        .or_default()
                        .push((period, minutes(from), end));
                }
                [locale, "I", skeleton, field, pattern] => {
                    // An interval's key: its skeleton and field apart by a space.
                    let key: &'static str =
                        Box::leak(format!("{skeleton} {field}").into_boxed_str());
                    data.entries.insert((locale, "I", key), pattern);
                }
                [locale, kind, key, pattern] => {
                    data.entries.insert((locale, kind, key), pattern);
                }
                _ => {}
            }
        }
        data
    })
}

/// The time value of a UTC date and time as the data writes it
/// (`1991-10-27 07:00`).
fn utc_of(text: &str) -> i64 {
    let number = |range: std::ops::Range<usize>| -> i64 {
        text.get(range).and_then(|n| n.parse().ok()).unwrap_or(0)
    };
    let (year, month, day) = (number(0..4), number(5..7), number(8..10));
    // Days from civil (Howard Hinnant's), 1970-01-01 day 0.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    days * 86_400_000 + number(11..13) * 3_600_000 + number(14..16) * 60_000
}

/// A zone's region and whether it is the region's primary zone (the
/// only one, or CLDR's primary); `None` for a zone of no region.
pub(super) fn location_of(zone: &str) -> Option<(&'static str, bool)> {
    data().locations.get(zone).copied()
}

/// A zone's metazone at time value `t`.
pub(super) fn metazone_of(zone: &str, t: i64) -> Option<&'static str> {
    data()
        .metazones
        .get(zone)?
        .iter()
        .find(|(_, from, to)| (*from..*to).contains(&t))
        .map(|(metazone, _, _)| *metazone)
}

/// A metazone's reference zone for a region, else its world one.
pub(super) fn golden_zone(metazone: &str, region: &str) -> Option<&'static str> {
    let golden = &data().golden;
    golden
        .get(&(metazone, region))
        .or_else(|| golden.get(&(metazone, "001")))
        .copied()
}

/// A data value's escapes undone.
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

/// The ICU locale names a locale's data is looked up by, nearest first,
/// as ICU's resource fallback walks them: its own (an alias followed),
/// then each parent, to the root.
fn chain(locale: &Locale) -> Vec<String> {
    let data = data();
    let mut name = locale.id.language.as_str().to_owned();
    if let Some(script) = locale.id.script {
        name = format!("{name}_{script}");
    }
    if let Some(region) = locale.id.region {
        name = format!("{name}_{region}");
    }
    let mut out = Vec::new();
    let mut next = Some(name);
    while let Some(mut name) = next {
        if out.len() > 8 {
            break;
        }
        if let Some(target) = data.aliases.get(name.as_str()) {
            name = (*target).to_owned();
        }
        next = if name == "root" {
            None
        } else if let Some(parent) = data.parents.get(name.as_str()) {
            Some((*parent).to_owned())
        } else {
            Some(
                name.rsplit_once('_')
                    .map_or("root".to_owned(), |(head, _)| head.to_owned()),
            )
        };
        out.push(name);
    }
    out
}

/// The value of a key of a kind in the locale's data, by its chain; an
/// empty one (a calendar's pattern its chain has and it has not) is none.
fn lookup(chain: &[String], kind: &str, key: &str) -> Option<String> {
    let data = data();
    chain
        .iter()
        .find_map(|name| data.entries.get(&(name.as_str(), kind, key)))
        .filter(|value| !value.is_empty())
        .map(|value| unescape(value))
}

/// All the keys and values of a kind along the chain, the nearest's
/// kept, in the order of their keys.
fn all(chain: &[String], kind: &str) -> Vec<(String, String)> {
    let data = data();
    let mut found: HashMap<&str, &str> = HashMap::new();
    for name in chain.iter().rev() {
        for ((locale, k, key), value) in &data.entries {
            if *k == kind && *locale == name.as_str() {
                found.insert(key, value);
            }
        }
    }
    let mut out: Vec<(String, String)> = found
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| (key.to_owned(), unescape(value)))
        .collect();
    out.sort();
    out
}

// ---- Skeletons (ICU's DateTimePatternGenerator) -------------------------

const ERA: usize = 0;
const YEAR: usize = 1;
const MONTH: usize = 3;
const WEEKDAY: usize = 6;
const DAY: usize = 9;
const DAYPERIOD: usize = 10;
const HOUR: usize = 11;
const MINUTE: usize = 12;
const SECOND: usize = 13;
const FRACTION: usize = 14;
const ZONE: usize = 15;
const FIELDS: usize = 16;

const NUMERIC: i32 = 0x100;
const NARROW: i32 = -0x101;
const SHORTER: i32 = -0x102;
const SHORT: i32 = -0x103;
const LONG: i32 = -0x104;
const DELTA: i32 = 0x10;

/// ICU's dtTypes: a pattern letter, its field, its type and the least
/// count it takes.
const TYPES: [(char, usize, i32, usize); 79] = [
    ('G', ERA, SHORT, 1),
    ('G', ERA, LONG, 4),
    ('G', ERA, NARROW, 5),
    ('y', YEAR, NUMERIC, 1),
    ('Y', YEAR, NUMERIC + DELTA, 1),
    ('u', YEAR, NUMERIC + 2 * DELTA, 1),
    ('r', YEAR, NUMERIC + 3 * DELTA, 1),
    ('U', YEAR, SHORT, 1),
    ('U', YEAR, LONG, 4),
    ('U', YEAR, NARROW, 5),
    ('Q', 2, NUMERIC, 1),
    ('Q', 2, SHORT, 3),
    ('Q', 2, LONG, 4),
    ('Q', 2, NARROW, 5),
    ('q', 2, NUMERIC + DELTA, 1),
    ('q', 2, SHORT - DELTA, 3),
    ('q', 2, LONG - DELTA, 4),
    ('q', 2, NARROW - DELTA, 5),
    ('M', MONTH, NUMERIC, 1),
    ('M', MONTH, SHORT, 3),
    ('M', MONTH, LONG, 4),
    ('M', MONTH, NARROW, 5),
    ('L', MONTH, NUMERIC + DELTA, 1),
    ('L', MONTH, SHORT - DELTA, 3),
    ('L', MONTH, LONG - DELTA, 4),
    ('L', MONTH, NARROW - DELTA, 5),
    ('l', MONTH, NUMERIC + DELTA, 1),
    ('w', 4, NUMERIC, 1),
    ('W', 5, NUMERIC, 1),
    ('E', WEEKDAY, SHORT, 1),
    ('E', WEEKDAY, LONG, 4),
    ('E', WEEKDAY, NARROW, 5),
    ('E', WEEKDAY, SHORTER, 6),
    ('c', WEEKDAY, NUMERIC + 2 * DELTA, 1),
    ('c', WEEKDAY, SHORT - 2 * DELTA, 3),
    ('c', WEEKDAY, LONG - 2 * DELTA, 4),
    ('c', WEEKDAY, NARROW - 2 * DELTA, 5),
    ('c', WEEKDAY, SHORTER - 2 * DELTA, 6),
    ('e', WEEKDAY, NUMERIC + DELTA, 1),
    ('e', WEEKDAY, SHORT - DELTA, 3),
    ('e', WEEKDAY, LONG - DELTA, 4),
    ('e', WEEKDAY, NARROW - DELTA, 5),
    ('e', WEEKDAY, SHORTER - DELTA, 6),
    ('d', DAY, NUMERIC, 1),
    ('g', DAY, NUMERIC + DELTA, 1),
    ('D', 7, NUMERIC, 1),
    ('F', 8, NUMERIC, 1),
    ('a', DAYPERIOD, SHORT, 1),
    ('a', DAYPERIOD, LONG, 4),
    ('a', DAYPERIOD, NARROW, 5),
    ('b', DAYPERIOD, SHORT - DELTA, 1),
    ('b', DAYPERIOD, LONG - DELTA, 4),
    ('b', DAYPERIOD, NARROW - DELTA, 5),
    ('B', DAYPERIOD, SHORT - 3 * DELTA, 1),
    ('B', DAYPERIOD, LONG - 3 * DELTA, 4),
    ('B', DAYPERIOD, NARROW - 3 * DELTA, 5),
    ('H', HOUR, NUMERIC + 10 * DELTA, 1),
    ('k', HOUR, NUMERIC + 11 * DELTA, 1),
    ('h', HOUR, NUMERIC, 1),
    ('K', HOUR, NUMERIC + DELTA, 1),
    ('m', MINUTE, NUMERIC, 1),
    ('s', SECOND, NUMERIC, 1),
    ('A', SECOND, NUMERIC + DELTA, 1),
    ('S', FRACTION, NUMERIC, 1),
    ('v', ZONE, SHORT - 2 * DELTA, 1),
    ('v', ZONE, LONG - 2 * DELTA, 4),
    ('z', ZONE, SHORT, 1),
    ('z', ZONE, LONG, 4),
    ('Z', ZONE, NARROW - DELTA, 1),
    ('Z', ZONE, LONG - DELTA, 4),
    ('Z', ZONE, SHORT - DELTA, 5),
    ('O', ZONE, SHORT - DELTA, 1),
    ('O', ZONE, LONG - DELTA, 4),
    ('V', ZONE, SHORT - DELTA, 1),
    ('V', ZONE, LONG - DELTA, 2),
    ('X', ZONE, NARROW - DELTA, 1),
    ('X', ZONE, SHORT - DELTA, 2),
    ('X', ZONE, LONG - DELTA, 4),
    ('x', ZONE, NARROW - DELTA, 1),
];

/// The row of [`TYPES`] a letter repeated `count` times reads as.
fn row_of(letter: char, count: usize) -> Option<&'static (char, usize, i32, usize)> {
    TYPES
        .iter()
        .rfind(|(c, _, _, least)| *c == letter && *least <= count)
}

/// A piece of a pattern: a field (letter, count) or literal text, its
/// quotes kept as written.
#[derive(Clone, Debug, PartialEq)]
enum Piece {
    Field(char, usize),
    Text(String),
}

/// A pattern's pieces (UTS #35's pattern syntax).
fn pieces(pattern: &str) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if c == '\'' {
            // A quoted run, its quotes kept.
            let mut text = String::from('\'');
            at += 1;
            while at < chars.len() {
                text.push(chars[at]);
                if chars[at] == '\'' {
                    if chars.get(at + 1) == Some(&'\'') {
                        text.push('\'');
                        at += 2;
                        continue;
                    }
                    at += 1;
                    break;
                }
                at += 1;
            }
            match out.last_mut() {
                Some(Piece::Text(last)) => last.push_str(&text),
                _ => out.push(Piece::Text(text)),
            }
        } else if c.is_ascii_alphabetic() {
            let mut count = 1;
            while chars.get(at + count) == Some(&c) {
                count += 1;
            }
            out.push(Piece::Field(c, count));
            at += count;
        } else {
            match out.last_mut() {
                Some(Piece::Text(last)) => last.push(c),
                _ => out.push(Piece::Text(c.to_string())),
            }
            at += 1;
        }
    }
    out
}

fn written(pieces: &[Piece]) -> String {
    let mut out = String::new();
    for piece in pieces {
        match piece {
            Piece::Field(c, n) => {
                for _ in 0..*n {
                    out.push(*c);
                }
            }
            Piece::Text(text) => out.push_str(text),
        }
    }
    out
}

/// A skeleton: each field's letter and count as written, and its type.
#[derive(Clone, Debug, Default, PartialEq)]
struct Skeleton {
    original: [Option<(char, usize)>; FIELDS],
    types: [i32; FIELDS],
}

impl Skeleton {
    /// ICU's DateTimeMatcher::set of a skeleton or a pattern's fields.
    fn of(text: &str) -> Skeleton {
        let mut skeleton = Skeleton::default();
        for piece in pieces(text) {
            let Piece::Field(letter, count) = piece else {
                continue;
            };
            let Some(&(_, field, kind, _)) = row_of(letter, count) else {
                continue;
            };
            skeleton.original[field] = Some((letter, count));
            skeleton.types[field] = if kind > 0 { kind + count as i32 } else { kind };
        }
        // Minutes and fractions without seconds take seconds.
        if skeleton.original[MINUTE].is_some()
            && skeleton.original[FRACTION].is_some()
            && skeleton.original[SECOND].is_none()
        {
            skeleton.original[SECOND] = Some(('s', 1));
            skeleton.types[SECOND] = NUMERIC + 1;
        }
        // A 12-hour hour takes a day period; a 24-hour one loses it.
        if let Some((hour, _)) = skeleton.original[HOUR] {
            if hour == 'h' || hour == 'K' {
                if skeleton.original[DAYPERIOD].is_none() {
                    skeleton.original[DAYPERIOD] = Some(('a', 1));
                    skeleton.types[DAYPERIOD] = SHORT;
                }
            } else {
                skeleton.original[DAYPERIOD] = None;
                skeleton.types[DAYPERIOD] = 0;
            }
        }
        skeleton
    }

    fn mask(&self) -> u32 {
        (0..FIELDS)
            .filter(|f| self.types[*f] != 0)
            .fold(0, |mask, f| mask | 1 << f)
    }

    /// ICU's base skeleton, which its patterns are kept by: each field's
    /// letter, a numeric one's count dropped.
    fn base(&self) -> String {
        let mut out = String::new();
        for (f, field) in self.original.iter().enumerate() {
            if let Some((letter, _)) = field {
                out.push(*letter);
                if self.types[f] <= 0 {
                    out.push_str(&self.types[f].to_string());
                }
            }
        }
        out
    }

    fn key(&self) -> String {
        let mut out = String::new();
        for (letter, count) in self.original.iter().flatten() {
            for _ in 0..*count {
                out.push(*letter);
            }
        }
        out
    }
}

/// ICU's distance from a requested skeleton (its fields of `mask`) to
/// another: its sum and the requested fields the other lacks.
fn distance(want: &Skeleton, other: &Skeleton, mask: u32) -> (i32, u32) {
    let mut sum = 0;
    let mut missing = 0;
    for field in 0..FIELDS {
        let mine = if mask & (1 << field) == 0 {
            0
        } else {
            want.types[field]
        };
        let theirs = other.types[field];
        if mine == theirs {
            continue;
        }
        if mine == 0 {
            sum += 0x10000;
        } else if theirs == 0 {
            sum += 0x1000;
            missing |= 1 << field;
        } else {
            sum += (mine - theirs).abs();
        }
    }
    (sum, missing)
}

/// A locale's pattern generator: its patterns with their skeletons
/// (`specified` where the data names it), its glue, its append items
/// and its default hour letter.
pub(super) struct Generator {
    chain: Vec<String>,
    /// Where eras, months and weekdays are named.
    names_chain: Vec<String>,
    patterns: Vec<(Skeleton, String, bool)>,
    hour: char,
    language: String,
    zones: ZoneNames,
    zone_chain: Vec<String>,
    decimal: String,
}

/// Whose zone names a locale's are, as the reference browser's ICU finds
/// its zone data: its own, English (a language with none, whose lookup
/// ends at the default locale), or none (a locale under root with none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ZoneNames {
    Own,
    English,
    Root,
}

/// A time as the fields [`Generator::with_names`] writes need it: its
/// era's index, its month (from 0) and weekday (from Sunday, 0) in the
/// calendar, its minutes and milliseconds into the day, its offset
/// from UTC, the zero of the digits it is written in, and the year in
/// its cycle and the related gregorian year for a calendar of cycles
/// (ICU's `y` and `r` of the Chinese one).
pub(super) struct Instant {
    pub(super) cyclic: Option<(i64, i64)>,
    /// Whether its day and weekday are the caller's to write.
    pub(super) own_day: bool,
    pub(super) era: i32,
    pub(super) month: usize,
    pub(super) weekday: usize,
    pub(super) minutes: i32,
    pub(super) millis: u32,
    pub(super) offset_seconds: i32,
    pub(super) zero: char,
}

/// Whether the data has a calendar's patterns for the locale's language
/// (the gregorian calendar's for every locale, the generic one's for
/// each language over them).
pub(super) fn has_calendar(locale: &Locale, calendar: &str) -> bool {
    calendar == "gregorian" || lookup(&[calendar_name(locale, "generic")], "P", "4").is_some()
}

/// The data's name of a language's calendar not the gregorian one.
fn calendar_name(locale: &Locale, calendar: &str) -> String {
    format!("{}@{calendar}", locale.id.language)
}

/// The calendars a calendar's patterns fall back to in the data, after
/// its own and before the gregorian ones (ICU's aliases).
fn calendar_parents(calendar: &str) -> &'static [&'static str] {
    match calendar {
        "dangi" => &["chinese", "generic"],
        "islamic-civil" | "islamic-rgsa" | "islamic-tbla" | "islamic-umalqura" => {
            &["islamic", "generic"]
        }
        "ethiopic-amete-alem" => &["ethiopic", "generic"],
        "generic" => &[],
        _ => &["generic"],
    }
}

impl Generator {
    #[cfg(test)]
    pub(super) fn new(locale: &Locale, hours_of: &Locale) -> Generator {
        Generator::in_calendar(locale, hours_of, "gregorian")
    }

    /// The generator of a calendar's patterns: one not the gregorian one
    /// by its own, its parents' and then the gregorian ones; its names
    /// (eras, months, weekdays) only its own, the rest ICU4X's.
    pub(super) fn in_calendar(locale: &Locale, hours_of: &Locale, calendar: &str) -> Generator {
        let (chain, names_chain) = if calendar == "gregorian" {
            (chain(locale), chain(locale))
        } else {
            let own: Vec<String> = std::iter::once(calendar)
                .chain(calendar_parents(calendar).iter().copied())
                .map(|calendar| calendar_name(locale, calendar))
                .collect();
            let mut chain = own.clone();
            chain.extend(self::chain(locale));
            (chain, own)
        };
        let mut patterns: Vec<(Skeleton, String, bool)> = Vec::new();
        // ICU's canonical items first: a pattern of each field alone.
        for letter in [
            'G', 'y', 'Q', 'M', 'w', 'W', 'E', 'D', 'F', 'd', 'a', 'H', 'm', 's', 'S', 'v',
        ] {
            let pattern = letter.to_string();
            patterns.push((Skeleton::of(&pattern), pattern, false));
        }
        // ICU's order: date then time format of each style, each kept
        // unless a pattern of its base skeleton is there already; then the
        // available formats, each replacing the pattern of its skeleton.
        for style in 0..4 {
            for index in [4 + style, style] {
                if let Some(pattern) = lookup(&chain, "P", &index.to_string()) {
                    let skeleton = Skeleton::of(&pattern);
                    if !patterns.iter().any(|(s, _, _)| s.base() == skeleton.base()) {
                        patterns.push((skeleton, pattern, false));
                    }
                }
            }
        }
        for (key, pattern) in all(&chain, "A") {
            let skeleton = Skeleton::of(&key);
            if let Some(entry) = patterns.iter_mut().find(|(s, _, _)| *s == skeleton) {
                *entry = (skeleton, pattern, true);
            } else {
                patterns.push((skeleton, pattern, true));
            }
        }
        // ICU's map is walked by the skeleton's first letter's bucket.
        patterns.sort_by_key(|(skeleton, _, _)| bucket(&skeleton.key()));
        let language = hours_of.id.language.as_str();
        let zones = match hours_of.id.script {
            Some(script)
                if data()
                    .root_zones
                    .contains(format!("{language}_{script}").as_str()) =>
            {
                ZoneNames::Root
            }
            None if data().english_zones.contains(language) => ZoneNames::English,
            _ => ZoneNames::Own,
        };
        // Zones' data is the locale's whatever the calendar.
        let zone_chain = match zones {
            ZoneNames::English => vec!["en".to_owned(), "root".to_owned()],
            _ => self::chain(locale),
        };
        Generator {
            hour: hour_letter(hours_of),
            language: language.to_owned(),
            zones,
            zone_chain,
            decimal: ".".to_owned(),
            chain,
            names_chain,
            patterns,
        }
    }

    /// The generator with the locale's decimal separator, which ICU
    /// writes between a pattern's seconds and their fraction.
    pub(super) fn with_decimal(mut self, decimal: &str) -> Generator {
        self.decimal = decimal.to_owned();
        self
    }

    /// A pattern with the fields ICU4X's names write otherwise than the
    /// reference browser's data written by that data, quoted: eras, day
    /// periods (flexible ones too), the 24-hour cycle's hours, fractions
    /// of a second, offsets from GMT, and zones' names by `zone_name`
    /// (`None` to leave the field to ICU4X).
    pub(super) fn with_names(
        &self,
        pattern: &str,
        at: &Instant,
        outside: &mut dyn FnMut(char, usize) -> Option<String>,
    ) -> String {
        let width = |n: usize| match n {
            1..=3 => "abbreviated",
            4 => "wide",
            _ => "narrow",
        };
        let digits = |value: i64, count: usize| -> String {
            let text = format!("{value:0count$}");
            text.chars()
                .map(|c| match c.to_digit(10) {
                    Some(d) => char::from_u32(at.zero as u32 + d).unwrap_or(c),
                    None => c,
                })
                .collect()
        };
        let mut out: Vec<Piece> = Vec::new();
        for piece in pieces(pattern) {
            let name = match piece {
                // A day and a weekday the caller writes itself (a calendar
                // ICU4X has another day of).
                Piece::Field(c @ ('d' | 'E' | 'c'), n) if at.own_day => outside(c, n),
                Piece::Field('y', n) if at.cyclic.is_some() => at
                    .cyclic
                    .map(|(year, _)| digits(year, if n == 2 { 2 } else { 1 })),
                Piece::Field('r', n) if at.cyclic.is_some() => {
                    at.cyclic.map(|(_, related)| digits(related, n))
                }
                Piece::Field('G', n) => {
                    lookup(&self.names_chain, "E", &format!("{}/{}", width(n), at.era))
                }
                // Weekdays' and months' names where the data has them.
                Piece::Field(c @ ('E' | 'c'), n) if c == 'E' || n >= 3 => {
                    let context = if c == 'c' { "stand-alone" } else { "format" };
                    let width = if n == 6 { "short" } else { width(n) };
                    lookup(
                        &self.names_chain,
                        "W",
                        &format!("{context}/{width}/{}", at.weekday),
                    )
                    .or_else(|| {
                        lookup(
                            &self.names_chain,
                            "W",
                            &format!("format/{width}/{}", at.weekday),
                        )
                    })
                }
                Piece::Field(c @ ('M' | 'L'), n) if n >= 3 => {
                    let context = if c == 'L' { "stand-alone" } else { "format" };
                    lookup(
                        &self.names_chain,
                        "O",
                        &format!("{context}/{}/{}", width(n), at.month),
                    )
                    .or_else(|| {
                        lookup(
                            &self.names_chain,
                            "O",
                            &format!("format/{}/{}", width(n), at.month),
                        )
                    })
                }
                Piece::Field('B', n) => self.day_period(at.minutes).and_then(|period| {
                    lookup(&self.chain, "D", &format!("{}/{period}", width(n)))
                        .or_else(|| lookup(&self.chain, "D", &format!("abbreviated/{period}")))
                }),
                Piece::Field('a', n) => {
                    let pm = i32::from(at.minutes >= 12 * 60);
                    lookup(&self.chain, "M", &format!("{}/{pm}", width(n)))
                        .or_else(|| lookup(&self.chain, "M", &format!("abbreviated/{pm}")))
                }
                Piece::Field('k', n) => {
                    let hour = i64::from(at.minutes / 60);
                    Some(digits(if hour == 0 { 24 } else { hour }, n))
                }
                Piece::Field('S', n) => {
                    let mut text = digits(i64::from(at.millis), 3);
                    while text.chars().count() < n {
                        text.push(at.zero);
                    }
                    Some(text.chars().take(n).collect())
                }
                Piece::Field('O', n) => Some(self.gmt(at.offset_seconds, n >= 4, at.zero)),
                Piece::Field(c @ ('z' | 'v'), n) => outside(c, n),
                _ => None,
            };
            match name {
                Some(name) => out.push(Piece::Text(format!("'{}'", name.replace('\'', "''")))),
                None => out.push(piece),
            }
        }
        written(&out)
    }

    /// Whose zone names the locale's are.
    pub(super) fn zone_names(&self) -> ZoneNames {
        self.zones
    }

    /// The data's name where ICU4X's differs from the reference's: a
    /// metazone's or a zone's by type (`Turkey/ls`), a zone's city
    /// (`.../ec`) or a region's (`C/HK`); `Some(None)` where the reference
    /// has none.
    pub(super) fn zone_override(&self, key: &str) -> Option<Option<String>> {
        lookup(&self.zone_chain, "Y", key).map(|name| (!name.is_empty()).then_some(name))
    }

    /// The locale's location format of a zone (ICU's regionFormat): a
    /// place's name in it.
    pub(super) fn in_region_format(&self, place: &str) -> String {
        lookup(&self.zone_chain, "Z", "regionFormat")
            .unwrap_or_else(|| "{0}".to_owned())
            .replace("{0}", place)
    }

    /// The locale's partial location format of a zone (ICU's
    /// fallbackFormat): a metazone's name and the zone's place.
    pub(super) fn in_fallback_format(&self, place: &str, name: &str) -> String {
        lookup(&self.zone_chain, "Z", "fallbackFormat")
            .unwrap_or_else(|| "{1} ({0})".to_owned())
            .replace("{0}", place)
            .replace("{1}", name)
    }

    /// An offset from GMT in the locale's localized GMT format (ICU's
    /// formatOffsetLocalizedGMT), its digits from `zero`: long, its hours
    /// and minutes in full; short, the pattern up to the hours alone
    /// where the minutes are none. (V8's GMT+0 for no offset, not the
    /// zero format.)
    pub(super) fn gmt(&self, seconds: i32, long: bool, zero: char) -> String {
        let digits = |value: i64, count: usize| -> String {
            format!("{value:0count$}")
                .chars()
                .map(|c| match c.to_digit(10) {
                    Some(d) => char::from_u32(zero as u32 + d).unwrap_or(c),
                    None => c,
                })
                .collect()
        };
        let format =
            lookup(&self.zone_chain, "Z", "gmtFormat").unwrap_or_else(|| "GMT{0}".to_owned());
        let hours = lookup(&self.zone_chain, "Z", "hourFormat")
            .unwrap_or_else(|| "+HH:mm;-HH:mm".to_owned());
        let (positive, negative) = hours
            .split_once(';')
            .unwrap_or((hours.as_str(), hours.as_str()));
        let pattern = if seconds < 0 { negative } else { positive };
        let total = seconds.unsigned_abs() / 60;
        let (h, m) = (i64::from(total / 60), i64::from(total % 60));
        let s = i64::from(seconds.unsigned_abs() % 60);
        let mut text = String::new();
        // ICU's short pattern of whole hours: the pattern up to its hours.
        let hours_only = !long && m == 0 && s == 0;
        // The separator before the minutes, which seconds take too (ICU's
        // pattern of hours, minutes and seconds).
        let mut separator = String::new();
        for piece in pieces(pattern) {
            match piece {
                Piece::Field('H', _) => {
                    text.push_str(&digits(h, if long { 2 } else { 1 }));
                    if hours_only {
                        break;
                    }
                }
                Piece::Field('m', _) => {
                    text.push_str(&digits(m, 2));
                    if s != 0 {
                        text.push_str(&separator);
                        text.push_str(&digits(s, 2));
                    }
                }
                Piece::Field(c, n) => text.extend(std::iter::repeat_n(c, n)),
                Piece::Text(literal) => {
                    separator.clone_from(&literal);
                    text.push_str(&literal);
                }
            }
        }
        format.replace("{0}", &text)
    }

    /// The language's flexible day period at a time of day in minutes:
    /// noon or midnight at their minute, else the span it falls in.
    fn day_period(&self, minutes: i32) -> Option<&'static str> {
        let rules = data().periods.get(self.language.as_str())?;
        if let Some((period, _, _)) = rules
            .iter()
            .find(|(period, from, end)| *end < 0 && *from == minutes && *period != "midnight")
        {
            return Some(period);
        }
        rules
            .iter()
            .find(|(_, from, end)| {
                *end >= 0
                    && if from < end {
                        (*from..*end).contains(&minutes)
                    } else {
                        minutes >= *from || minutes < *end
                    }
            })
            .map(|(period, _, _)| *period)
    }

    /// ICU's getBestPattern of a skeleton, with the hour's length
    /// matched.
    pub(super) fn best(&self, skeleton_text: &str) -> String {
        let mapped = self.map_hours(skeleton_text);
        let want = Skeleton::of(&mapped);
        let (best, missing, extra, specified) = self.best_raw(&want, u32::MAX);
        if missing == 0 && extra == 0 {
            return self.adjust(&best, &want, specified.as_ref(), false);
        }
        let needed = want.mask();
        let date = self.best_appending(&want, needed & 0x3ff);
        let time = self.best_appending(&want, needed & !0x3ff);
        if date.is_empty() {
            return time;
        }
        if time.is_empty() {
            return date;
        }
        let glue = self.glue(&want);
        glue.replace("{1}", &date).replace("{0}", &time)
    }

    /// A style's time pattern in the locale's preferred hour cycle
    /// where its own is the other (ICU's DateFormat for a region that
    /// prefers 12-hour times over a 24-hour pattern, and the reverse):
    /// the pattern found for its skeleton with the preferred hour.
    pub(super) fn in_preferred_cycle(&self, pattern: &str) -> String {
        let twelve = |c: char| c == 'h' || c == 'K';
        let Some((hour, count)) = pieces(pattern).iter().find_map(|piece| match piece {
            Piece::Field(c @ ('h' | 'H' | 'k' | 'K'), n) => Some((*c, *n)),
            _ => None,
        }) else {
            return pattern.to_owned();
        };
        if twelve(hour) == twelve(self.hour) {
            return pattern.to_owned();
        }
        let skeleton: String = pieces(pattern)
            .iter()
            .filter_map(|piece| match piece {
                Piece::Field('h' | 'H' | 'k' | 'K', _) => {
                    Some(std::iter::repeat_n(self.hour, count).collect::<String>())
                }
                Piece::Field('a' | 'b' | 'B', _) => None,
                Piece::Field(c, n) => Some(std::iter::repeat_n(*c, *n).collect()),
                Piece::Text(_) => None,
            })
            .collect();
        self.best(&skeleton)
    }

    /// The glue of a date and a time for a skeleton: by its month's
    /// width (and weekday), the locale's at-time glue where it has one.
    fn glue(&self, want: &Skeleton) -> String {
        let style = match want.original[MONTH] {
            Some((_, 4)) if want.original[WEEKDAY].is_some() => 0,
            Some((_, 4)) => 1,
            Some((_, 3)) => 2,
            _ => 3,
        };
        self.style_glue(style)
    }

    /// The glue of a date style's date and a time (`{1}` the date).
    pub(super) fn style_glue(&self, style: usize) -> String {
        lookup(&self.chain, "T", &style.to_string())
            .or_else(|| lookup(&self.chain, "P", &(9 + style).to_string()))
            .or_else(|| lookup(&self.chain, "P", "8"))
            .unwrap_or_else(|| "{1} {0}".to_owned())
    }

    /// A style's date (`date`) or time pattern.
    pub(super) fn style_pattern(&self, style: usize, date: bool) -> Option<String> {
        let index = if date { 4 + style } else { style };
        lookup(&self.chain, "P", &index.to_string())
    }

    /// The interval data of the locale.
    pub(super) fn chain(&self) -> &[String] {
        &self.chain
    }

    /// ICU's mapping of the hour's `j` to the locale's hour letter, with
    /// a day period for a 12-hour one.
    fn map_hours(&self, skeleton: &str) -> String {
        let mut out = String::new();
        for piece in pieces(skeleton) {
            match piece {
                Piece::Field('j', count) => {
                    let hour = self.hour;
                    let extra = count - 1;
                    if hour == 'h' || hour == 'K' {
                        let period = if extra < 2 { 1 } else { 3 + (extra >> 1) };
                        for _ in 0..period {
                            out.push('a');
                        }
                    }
                    for _ in 0..(1 + (extra & 1)) {
                        out.push(hour);
                    }
                }
                Piece::Field(c, n) => {
                    for _ in 0..n {
                        out.push(c);
                    }
                }
                Piece::Text(text) => out.push_str(&text),
            }
        }
        out
    }

    /// ICU's getBestRaw: the nearest pattern to the skeleton's fields of
    /// `mask`, the fields it lacks and has over, and its skeleton where
    /// the data names it.
    fn best_raw(&self, want: &Skeleton, mask: u32) -> (String, u32, u32, Option<Skeleton>) {
        let mut best: Option<(i32, u32, usize)> = None;
        for (at, (skeleton, _, _)) in self.patterns.iter().enumerate() {
            let (sum, missing) = distance(want, skeleton, mask);
            let better = match best {
                None => true,
                Some((best_sum, best_missing, _)) => {
                    sum < best_sum || (sum == best_sum && best_missing < missing)
                }
            };
            if better {
                best = Some((sum, missing, at));
                if sum == 0 {
                    break;
                }
            }
        }
        let Some((sum, missing, at)) = best else {
            return (String::new(), 0, 0, None);
        };
        let (skeleton, pattern, specified) = &self.patterns[at];
        let extra = if sum >= 0x10000 { 1 } else { 0 };
        (
            pattern.clone(),
            missing,
            extra,
            specified.then(|| skeleton.clone()),
        )
    }

    /// ICU's getBestAppending: a pattern for the fields of `mask`, the
    /// ones its nearest lacks appended by the locale's append items.
    fn best_appending(&self, want: &Skeleton, mask: u32) -> String {
        if mask == 0 {
            return String::new();
        }
        let (pattern, mut missing, _, specified) = self.best_raw(want, mask);
        let mut result = self.adjust(&pattern, want, specified.as_ref(), false);
        let mut guard = 0;
        while missing != 0 && guard < FIELDS {
            guard += 1;
            // Fractions missing where seconds were found: written after them.
            if missing & (1 << FRACTION) != 0
                && missing & (1 << SECOND) == 0
                && mask & (1 << SECOND) != 0
            {
                result = self.adjust(&result, want, specified.as_ref(), true);
                missing &= !(1 << FRACTION);
                continue;
            }
            let start = missing;
            let (pattern, still, _, specified) = self.best_raw(want, missing);
            let found = self.adjust(&pattern, want, specified.as_ref(), false);
            let top = 31 - (start & !still).leading_zeros() as usize;
            let item = match top {
                ERA => "Era",
                YEAR => "Year",
                2 => "Quarter",
                MONTH => "Month",
                4 | 5 => "Week",
                WEEKDAY => "Day-Of-Week",
                DAY => "Day",
                HOUR => "Hour",
                MINUTE => "Minute",
                SECOND => "Second",
                ZONE => "Timezone",
                _ => "",
            };
            let format = lookup(&self.chain, "N", item).unwrap_or_else(|| "{0} {1}".to_owned());
            result = format
                .replace("{0}", &result)
                .replace("{1}", &found)
                .replace("{2}", "");
            if still == start {
                break;
            }
            missing = still;
        }
        result
    }

    /// ICU's adjustFieldTypes: the pattern's fields' widths (and some
    /// letters) taken from the skeleton.
    fn adjust(
        &self,
        pattern: &str,
        want: &Skeleton,
        specified: Option<&Skeleton>,
        fix_fraction: bool,
    ) -> String {
        let mut out: Vec<Piece> = Vec::new();
        for piece in pieces(pattern) {
            let Piece::Field(letter, count) = piece else {
                out.push(piece);
                continue;
            };
            let Some(&(_, field, kind, _)) = row_of(letter, count) else {
                out.push(Piece::Field(letter, count));
                continue;
            };
            if fix_fraction && field == SECOND {
                out.push(Piece::Field(letter, count));
                out.push(Piece::Text(self.decimal.clone()));
                if let Some((s, n)) = want.original[FRACTION] {
                    out.push(Piece::Field(s, n));
                }
                continue;
            }
            let Some((req_letter, mut req_count)) = want.original[field] else {
                out.push(Piece::Field(letter, count));
                continue;
            };
            if req_letter == 'E' && req_count < 3 {
                req_count = 3;
            }
            let mut adjusted = req_count;
            if field == MINUTE || field == SECOND {
                adjusted = count;
            } else if let Some(specified) = specified
                && req_letter != 'c'
                && req_letter != 'e'
            {
                let skeleton_count = specified.original[field].map_or(0, |(_, n)| n);
                let pattern_numeric = kind > 0;
                let skeleton_numeric = specified.types[field] > 0;
                if skeleton_count == req_count || pattern_numeric != skeleton_numeric {
                    adjusted = count;
                }
            }
            let mut c = if field != HOUR
                && field != MONTH
                && field != WEEKDAY
                && (field != YEAR || req_letter == 'Y')
            {
                req_letter
            } else {
                letter
            };
            if c == 'E' && adjusted < 3 {
                c = 'e';
            }
            if field == HOUR {
                c = match (req_letter, self.hour) {
                    (r, h) if r == h => h,
                    ('h', 'K') => 'K',
                    ('H', 'k') => 'k',
                    ('k', 'H') => 'H',
                    ('K', 'h') => 'h',
                    _ => c,
                };
            }
            out.push(Piece::Field(c, adjusted));
        }
        written(&out)
    }
}

/// The bucket ICU's pattern map keeps a skeleton in: its first letter's,
/// capitals first.
fn bucket(key: &str) -> usize {
    match key.chars().next() {
        Some(c @ 'A'..='Z') => c as usize - 'A' as usize,
        Some(c @ 'a'..='z') => 26 + c as usize - 'a' as usize,
        _ => 52,
    }
}

/// The locale's preferred hour letter (CLDR's timeData, by its region or
/// its likely one, the world's else).
pub(super) fn hour_letter(locale: &Locale) -> char {
    let data = data();
    let mut id = locale.id.clone();
    LocaleExpander::new_extended().maximize(&mut id);
    let region = id.region.map(|r| r.to_string()).unwrap_or_default();
    let language_region = format!("{}_{}", locale.id.language, region);
    let (preferred, _) = data
        .hours
        .get(language_region.as_str())
        .or_else(|| data.hours.get(region.as_str()))
        .or_else(|| data.hours.get("001"))
        .copied()
        .unwrap_or(("H", "H"));
    preferred.chars().next().unwrap_or('H')
}

// ---- Options as a skeleton (V8's) ---------------------------------------

/// The skeleton V8 asks ICU for of a component bag: each field's letter
/// by its option (codes as `intlDateTime` takes them), the hour by the
/// hour cycle asked for (`j` the locale's).
pub(super) fn skeleton_of(codes: &[i32], hour: char) -> String {
    let at = |i: usize| codes.get(i).copied().unwrap_or(0);
    let mut out = String::new();
    let mut push = |letter: char, count: usize| {
        for _ in 0..count {
            out.push(letter);
        }
    };
    // weekday, era, year, month, day, (day period), hour, minute,
    // second, fraction, zone name (`intlDateTime`'s order).
    match at(0) {
        1 => push('E', 5),
        2 => push('E', 3),
        3 => push('E', 4),
        _ => {}
    }
    match at(1) {
        1 => push('G', 5),
        2 => push('G', 3),
        3 => push('G', 4),
        _ => {}
    }
    match at(2) {
        1 => push('y', 1),
        2 => push('y', 2),
        _ => {}
    }
    match at(3) {
        1 => push('M', 1),
        2 => push('M', 2),
        3 => push('M', 5),
        4 => push('M', 3),
        5 => push('M', 4),
        _ => {}
    }
    match at(4) {
        1 => push('d', 1),
        2 => push('d', 2),
        _ => {}
    }
    match at(12) {
        1 => push('B', 5),
        2 => push('B', 1),
        3 => push('B', 4),
        _ => {}
    }
    match at(5) {
        1 => push(hour, 1),
        2 => push(hour, 2),
        _ => {}
    }
    match at(6) {
        1 => push('m', 1),
        2 => push('m', 2),
        _ => {}
    }
    match at(7) {
        1 => push('s', 1),
        2 => push('s', 2),
        _ => {}
    }
    if at(8) > 0 {
        push('S', at(8).clamp(1, 3) as usize);
    }
    match at(9) {
        1 => push('z', 1),
        2 => push('z', 4),
        3 => push('O', 1),
        4 => push('O', 4),
        5 => push('v', 1),
        6 => push('v', 4),
        _ => {}
    }
    out
}

/// A pattern with its hours written in an hour cycle's letter, as V8
/// does after ICU picks it.
pub(super) fn with_hour_letter(pattern: &str, letter: char) -> String {
    let mut out: Vec<Piece> = Vec::new();
    for piece in pieces(pattern) {
        match piece {
            Piece::Field('h' | 'H' | 'k' | 'K', n) => out.push(Piece::Field(letter, n)),
            other => out.push(other),
        }
    }
    written(&out)
}

/// The skeleton of a pattern (ICU's staticGetSkeleton): its fields'
/// letters and counts in their order.
pub(super) fn skeleton_of_pattern(pattern: &str) -> String {
    Skeleton::of(pattern).key()
}

// ---- Intervals (ICU's DateIntervalFormat) --------------------------------

/// The calendar fields a range differs in, largest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Difference {
    Era,
    Year,
    Month,
    Day,
    DayPeriod,
    Hour,
    Minute,
    Second,
    Millisecond,
}

impl Difference {
    fn letter(self) -> char {
        match self {
            Difference::Era => 'G',
            Difference::Year => 'y',
            Difference::Month => 'M',
            Difference::Day => 'd',
            Difference::DayPeriod => 'a',
            Difference::Hour => 'h',
            Difference::Minute => 'm',
            Difference::Second => 's',
            Difference::Millisecond => 'S',
        }
    }

    fn level(self) -> i32 {
        match self {
            Difference::Era => 0,
            Difference::Year => 10,
            Difference::Month => 20,
            Difference::Day => 30,
            Difference::DayPeriod => 40,
            Difference::Hour => 50,
            Difference::Minute => 60,
            Difference::Second => 70,
            Difference::Millisecond => 80,
        }
    }
}

/// The level of a pattern letter (ICU's fgPatternCharToLevel).
fn letter_level(c: char) -> i32 {
    match c {
        'G' => 0,
        'y' | 'Y' | 'u' | 'U' | 'r' => 10,
        'Q' | 'q' | 'M' | 'L' | 'l' => 20,
        'w' | 'W' | 'd' | 'D' | 'F' | 'g' | 'E' | 'e' | 'c' => 30,
        'a' | 'b' | 'B' => 40,
        'h' | 'H' | 'k' | 'K' => 50,
        'm' => 60,
        's' => 70,
        'S' | 'A' => 80,
        _ => -1,
    }
}

/// Whether a pattern shows nothing as fine as a field (ICU's
/// isFieldUnitIgnored).
fn field_ignored(pattern: &str, field: Difference) -> bool {
    !pieces(pattern).iter().any(|piece| match piece {
        Piece::Field(c, _) => letter_level(*c) >= field.level(),
        Piece::Text(_) => false,
    })
}

/// What a range is written by: one date's pattern, two patterns (the
/// first for the start, the second for the end), or the fallback (the
/// locale's `{0} - {1}` of a pattern; for times of one day, the date
/// once by its pattern, the times' by theirs, joined by the glue).
#[derive(Debug, PartialEq)]
pub(super) enum Range {
    Single(String),
    Split(String, String),
    Fallback {
        pattern: String,
        separator: String,
        date: Option<(String, String, String)>,
    },
}

/// A letter's counts in a skeleton, by letter (ICU's parseSkeleton).
fn counts(skeleton: &str) -> HashMap<char, usize> {
    let mut out = HashMap::new();
    for c in skeleton.chars() {
        *out.entry(c).or_insert(0) += 1;
    }
    out
}

/// A skeleton's date and time parts as ICU's getDateTimeSkeleton
/// splits them: each as written, and normalized as the interval data is
/// keyed (its year, month, weekday and day once each in that order
/// after its other date fields, `MM` as `M`; its hour, minute and zone
/// once each after its other time fields).
struct Parts {
    date: String,
    normal_date: String,
    time: String,
    normal_time: String,
}

fn split_skeleton(skeleton: &str) -> Parts {
    let mut parts = Parts {
        date: String::new(),
        normal_date: String::new(),
        time: String::new(),
        normal_time: String::new(),
    };
    let (mut e, mut d, mut m_upper, mut y, mut m_lower, mut v, mut z) = (0, 0, 0, 0, 0, 0, 0);
    let mut hour: Option<char> = None;
    for c in skeleton.chars() {
        match c {
            'E' => {
                parts.date.push(c);
                e += 1;
            }
            'd' => {
                parts.date.push(c);
                d += 1;
            }
            'M' => {
                parts.date.push(c);
                m_upper += 1;
            }
            'y' => {
                parts.date.push(c);
                y += 1;
            }
            'G' | 'Y' | 'u' | 'Q' | 'q' | 'L' | 'l' | 'W' | 'w' | 'D' | 'F' | 'g' | 'e' | 'c'
            | 'U' | 'r' => {
                parts.normal_date.push(c);
                parts.date.push(c);
            }
            'a' => parts.time.push(c),
            'h' | 'H' | 'k' | 'K' => {
                parts.time.push(c);
                hour.get_or_insert(c);
            }
            'm' => {
                parts.time.push(c);
                m_lower += 1;
            }
            'z' => {
                parts.time.push(c);
                z += 1;
            }
            'v' => {
                parts.time.push(c);
                v += 1;
            }
            'V' | 'Z' | 'j' | 's' | 'S' | 'A' | 'b' | 'B' => {
                parts.time.push(c);
                parts.normal_time.push(c);
            }
            _ => {}
        }
    }
    let repeat = |c: char, n: usize| std::iter::repeat_n(c, n).collect::<String>();
    parts.normal_date.push_str(&repeat('y', y));
    if m_upper > 0 {
        parts
            .normal_date
            .push_str(&repeat('M', if m_upper < 3 { 1 } else { m_upper.min(5) }));
    }
    if e > 0 {
        parts
            .normal_date
            .push_str(&repeat('E', if e <= 3 { 1 } else { e.min(5) }));
    }
    if d > 0 {
        parts.normal_date.push('d');
    }
    if let Some(hour) = hour {
        parts.normal_time.push(hour);
    }
    for (c, n) in [('m', m_lower), ('z', z), ('v', v)] {
        if n > 0 {
            parts.normal_time.push(c);
        }
    }
    parts
}

/// ICU's DateIntervalInfo::getBestSkeleton: the interval skeleton
/// nearest a skeleton, letter by letter, and how it differs (0 alike, 1
/// by width, -1 by fields, 2 by a zone written alike).
fn best_interval_skeleton(chain: &[String], skeleton: &str) -> Option<(String, i32)> {
    let input = skeleton.replace('z', "v");
    let replaced_zone = skeleton.contains('z');
    let want = counts(&input);
    let mut skeletons: Vec<String> = all(chain, "I")
        .into_iter()
        .filter_map(|(key, _)| key.split_once(' ').map(|(s, _)| s.to_owned()))
        .filter(|s| s != "fallback")
        .collect();
    skeletons.dedup();
    let mut best: Option<(String, i32, i32)> = None;
    for candidate in skeletons {
        let have = counts(&candidate);
        let mut sum = 0;
        let mut difference = 1;
        let letters: std::collections::BTreeSet<char> =
            want.keys().chain(have.keys()).copied().collect();
        for c in letters {
            let a = want.get(&c).copied().unwrap_or(0);
            let b = have.get(&c).copied().unwrap_or(0);
            if a == b {
                continue;
            }
            if a == 0 || b == 0 {
                difference = -1;
                sum += 0x1000;
            } else if c == 'M' && ((a <= 2 && b > 2) || (a > 2 && b <= 2)) {
                sum += 0x100;
            } else {
                sum += (a as i32 - b as i32).abs();
            }
        }
        if best.as_ref().is_none_or(|(_, best_sum, _)| sum < *best_sum) {
            best = Some((candidate, sum, if sum == 0 { 0 } else { difference }));
            if sum == 0 {
                break;
            }
        }
    }
    best.map(|(s, _, d)| (s, if replaced_zone && d != -1 { 2 } else { d }))
}

/// ICU's adjustFieldWidth: an interval pattern's fields widened to the
/// skeleton's where the data's skeleton writes them narrower (a
/// pattern's `L` counted as the skeletons' `M`).
fn adjust_interval(skeleton: &str, best: &str, pattern: &str, difference: i32) -> String {
    let want = counts(skeleton);
    let have = counts(best);
    let mut out: Vec<Piece> = Vec::new();
    for piece in pieces(pattern) {
        match piece {
            Piece::Field(c, n) => {
                let key = if c == 'L' { 'M' } else { c };
                let c = if difference == 2 && c == 'v' { 'z' } else { c };
                let a = want.get(&key).copied().unwrap_or(0);
                let b = have.get(&key).copied().unwrap_or(0);
                if b == n && a > b {
                    out.push(Piece::Field(c, a));
                } else {
                    out.push(Piece::Field(c, n));
                }
            }
            text => out.push(text),
        }
    }
    written(&out)
}

/// An interval pattern cut where its second date begins: at the first
/// letter written a second time, letter for letter (ICU's
/// splitPatternInto2Part); all of it and nothing when none is.
fn split_interval(pattern: &str) -> (String, String) {
    let items = pieces(pattern);
    let mut seen: Vec<char> = Vec::new();
    for (at, piece) in items.iter().enumerate() {
        if let Piece::Field(c, _) = piece {
            if seen.contains(c) {
                return (written(&items[..at]), written(&items[at..]));
            }
            seen.push(*c);
        }
    }
    (pattern.to_owned(), String::new())
}

/// The interval pattern of a field for a skeleton and its nearest in the
/// data (ICU's setIntervalPattern): the data's, its widths adjusted;
/// for a day period with none, the hour's; for a date's field with none
/// (where `extend`), the data's for the skeleton with that field first,
/// or the nearest to that. With it, the skeleton and its nearest so
/// extended, where they were.
fn interval_for(
    chain: &[String],
    field: Difference,
    skeleton: &str,
    best: &str,
    difference: i32,
    extend: bool,
) -> (Option<String>, Option<(String, String)>) {
    let find = |best: &str, field: Difference| -> Option<String> {
        let letter = field.letter();
        lookup(chain, "I", &format!("{best} {letter}")).or_else(|| {
            // The data keys hours by `h` and `H` alike.
            (letter == 'h')
                .then(|| lookup(chain, "I", &format!("{best} H")))
                .flatten()
        })
    };
    let mut best = best.to_owned();
    let mut difference = difference;
    let mut extended = None;
    let mut pattern = find(&best, field);
    if pattern.is_none() {
        if field_ignored(&best, field) {
            return (None, None);
        }
        if field == Difference::DayPeriod {
            // A flexible day period's interval, else the hour's.
            if let Some(periods) = lookup(chain, "I", &format!("{best} B")) {
                return (
                    Some(adjust_interval(skeleton, &best, &periods, difference)),
                    None,
                );
            }
            let hour = find(&best, Difference::Hour);
            return (
                hour.map(|p| adjust_interval(skeleton, &best, &p, difference)),
                None,
            );
        }
        if extend {
            let letter = field.letter();
            let extended_skeleton = format!("{letter}{skeleton}");
            let extended_best = format!("{letter}{best}");
            pattern = find(&extended_best, field);
            if pattern.is_none()
                && difference == 0
                && let Some((nearer, nearer_difference)) =
                    best_interval_skeleton(chain, &extended_best)
                && nearer_difference != -1
            {
                pattern = find(&nearer, field);
                best = nearer;
                difference = nearer_difference;
            }
            extended = Some((extended_skeleton, extended_best));
        }
    }
    let Some(pattern) = pattern else {
        return (None, None);
    };
    let adjusted = if difference != 0 {
        adjust_interval(skeleton, &best, &pattern, difference)
    } else {
        pattern
    };
    (Some(adjusted), extended)
}

/// How a range in a skeleton is written for its greatest different
/// field, `pattern` the format's own: as ICU's DateIntervalFormat of
/// V8's skeleton of the pattern writes it, and V8 then (one date by the
/// format's pattern where ICU writes no range).
pub(super) fn range_plan(
    generator: &Generator,
    skeleton: &str,
    pattern: &str,
    difference: Option<Difference>,
    same_day: bool,
) -> Range {
    let Some(difference) = difference else {
        return Range::Single(pattern.to_owned());
    };
    let chain = generator.chain();
    let separator = lookup(chain, "I", "fallback").unwrap_or_else(|| "{0} \u{2013} {1}".to_owned());
    let parts = split_skeleton(skeleton);
    let has_time = !parts.time.is_empty();
    let has_date = !parts.date.is_empty();
    let glue = lookup(chain, "P", "8").unwrap_or_else(|| "{1} {0}".to_owned());
    // The pattern of ICU's own format for the skeleton.
    let whole = generator.best(skeleton);
    let key = if has_time {
        parts.normal_time.clone()
    } else {
        parts.normal_date.clone()
    };
    let nearest = best_interval_skeleton(chain, &key);
    // A date and its times of one day: the date once (ICU's date and time
    // patterns, kept where the data has a nearest skeleton).
    let date_once = (nearest.is_some() && has_date && has_time && same_day).then(|| {
        (
            generator.best(&parts.normal_date),
            generator.best(&parts.normal_time),
            glue.clone(),
        )
    });
    let fallback = |whole: String| Range::Fallback {
        pattern: whole,
        separator: separator.clone(),
        date: date_once.clone(),
    };
    // No interval pattern for the field: one date where the format shows
    // nothing as fine, else the fallback.
    let none = || {
        if field_ignored(&whole, difference) {
            Range::Single(pattern.to_owned())
        } else {
            fallback(whole.clone())
        }
    };
    // A pattern for the field: one date where it has no second part.
    let split = |interval: String| {
        let (first, second) = split_interval(&interval);
        if second.is_empty() {
            Range::Single(pattern.to_owned())
        } else {
            Range::Split(first, second)
        }
    };
    // Times of different days: the dates written too (ICU's yMd before).
    let dated_time = || {
        let era = if difference == Difference::Era {
            "G"
        } else {
            ""
        };
        fallback(generator.best(&format!("{era}yMd{}", parts.time)))
    };
    let found = nearest.filter(|(_, info)| *info != -1);
    let Some((best, info)) = found else {
        if has_time && !has_date && difference <= Difference::Day {
            return dated_time();
        }
        return none();
    };
    if !has_time {
        // Days, then months (extending the skeleton where it lacks them
        // and the data has them), then years and eras.
        let skeleton = parts.normal_date.as_str();
        let (day, _) = interval_for(chain, Difference::Day, skeleton, &best, info, true);
        let (month, extended) = interval_for(chain, Difference::Month, skeleton, &best, info, true);
        let (skeleton, best) = match &extended {
            Some((skeleton, best)) if month.is_some() => (skeleton.as_str(), best.as_str()),
            _ => (skeleton, best.as_str()),
        };
        let interval = match difference {
            Difference::Day => day,
            Difference::Month => month,
            Difference::Year | Difference::Era => {
                interval_for(chain, difference, skeleton, best, info, true).0
            }
            _ => None,
        };
        return interval.map_or_else(none, split);
    }
    if difference <= Difference::Day {
        if !has_date {
            return dated_time();
        }
        // A date and a time of different days: the whole of each, the
        // fields the date lacks down to the one that differs first.
        let mut wider = skeleton.to_owned();
        for (field, letter) in [
            (Difference::Day, 'd'),
            (Difference::Month, 'M'),
            (Difference::Year, 'y'),
            (Difference::Era, 'G'),
        ] {
            let lacks = !parts.date.contains(letter);
            if lacks {
                wider.insert(0, letter);
            }
            if field == difference {
                return if lacks {
                    fallback(generator.best(&wider))
                } else {
                    none()
                };
            }
        }
    }
    let field = difference;
    let interval = match field {
        Difference::DayPeriod | Difference::Hour | Difference::Minute => {
            interval_for(chain, field, &parts.normal_time, &best, info, false).0
        }
        _ => None,
    };
    let Some(interval) = interval else {
        return none();
    };
    if has_date {
        // The date once, then the times' interval, by the locale's glue.
        let date_pattern = generator.best(&parts.date);
        let combined = glue.replace("{1}", &date_pattern).replace("{0}", &interval);
        return split(combined);
    }
    split(interval)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locale(tag: &str) -> Locale {
        Locale::try_from_str(tag).expect("a tag")
    }

    #[test]
    fn skeletons_find_the_patterns_icu_finds() {
        let en = Generator::new(&locale("en"), &locale("en"));
        assert_eq!(en.best("yMd"), "M/d/y");
        assert_eq!(en.best("yMMMMd"), "MMMM d, y");
        assert_eq!(en.best("yMMMMdjm"), "MMMM d, y 'at' h:mm\u{202f}a");
        assert_eq!(en.best("yMMMdjm"), "MMM d, y, h:mm\u{202f}a");
        assert_eq!(en.best("jm"), "h:mm\u{202f}a");
        let de = Generator::new(&locale("de"), &locale("de"));
        assert_eq!(de.best("yMd"), "d.M.y");
        assert_eq!(de.best("jm"), "HH:mm");
        let gb = Generator::new(&locale("en-GB"), &locale("en-GB"));
        assert_eq!(gb.best("yMd"), "dd/MM/y");
    }
}
