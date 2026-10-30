//! The time zones of `Intl` (D63, `docs/design/text-dom-scripts.md`):
//! IANA names canonicalized as the reference browser's ICU does, the
//! system's zone (on Windows its zone's name mapped by CLDR's
//! windowsZones), and a zone's offset from UTC at an instant from 1970
//! on as the reference browser's ICU has it (`intl_zones.txt`, from its
//! zoneinfo64): each zone's changes, then its final rule.

use std::collections::HashMap;
use std::sync::OnceLock;

use icu_locale::subtags::Region;
use icu_time::TimeZone;
use icu_time::zone::WindowsParser;
use icu_time::zone::iana::IanaParserExtended;

const MS_PER_DAY: i64 = 86_400_000;

/// The names ICU keeps as canonical where ICU4X keeps the IANA
/// database's newer ones, which the reference browser reports.
const LEGACY_NAMES: [(&str, &str); 20] = [
    ("Africa/Asmara", "Africa/Asmera"),
    ("America/Argentina/Buenos_Aires", "America/Buenos_Aires"),
    ("America/Argentina/Catamarca", "America/Catamarca"),
    ("America/Argentina/Cordoba", "America/Cordoba"),
    ("America/Argentina/Jujuy", "America/Jujuy"),
    ("America/Argentina/Mendoza", "America/Mendoza"),
    ("America/Atikokan", "America/Coral_Harbour"),
    ("America/Indiana/Indianapolis", "America/Indianapolis"),
    ("America/Kentucky/Louisville", "America/Louisville"),
    ("America/Nuuk", "America/Godthab"),
    ("Asia/Ho_Chi_Minh", "Asia/Saigon"),
    ("Asia/Kathmandu", "Asia/Katmandu"),
    ("Asia/Kolkata", "Asia/Calcutta"),
    ("Asia/Yangon", "Asia/Rangoon"),
    ("Atlantic/Faroe", "Atlantic/Faeroe"),
    ("Europe/Kyiv", "Europe/Kiev"),
    ("Pacific/Chuuk", "Pacific/Truk"),
    ("Pacific/Kanton", "Pacific/Enderbury"),
    ("Pacific/Pohnpei", "Pacific/Ponape"),
    ("Etc/UTC", "UTC"),
];

/// The name a zone is reported by: its canonical IANA name, the older
/// one where ICU keeps that.
fn reported(canonical: &str) -> &str {
    LEGACY_NAMES
        .iter()
        .find(|(new, _)| *new == canonical)
        .map_or(canonical, |(_, old)| old)
}

/// Each zone's reported name, by its BCP 47 identifier.
fn names() -> &'static HashMap<TimeZone, &'static str> {
    static NAMES: OnceLock<HashMap<TimeZone, &'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        IanaParserExtended::new()
            .iter()
            .map(|zone| (zone.time_zone, reported(zone.canonical)))
            .collect()
    })
}

/// A zone by its IANA name, any case, and the name it is reported by;
/// `None` if ICU4X does not know it.
pub(super) fn zone_named(name: &str) -> Option<(TimeZone, &'static str)> {
    if name.is_empty() || name.len() > 64 || !name.is_ascii() {
        return None;
    }
    let zone = IanaParserExtended::new().parse(name).time_zone;
    if zone.is_unknown() {
        return None;
    }
    names().get(&zone).map(|name| (zone, *name))
}

/// The zones of the world region (`001`) as the reference browser's ICU
/// lists them: those of no region, legacy ones included.
const WORLD_ZONES: [&str; 46] = [
    "CST6CDT",
    "EST5EDT",
    "Etc/GMT",
    "Etc/GMT+1",
    "Etc/GMT+10",
    "Etc/GMT+11",
    "Etc/GMT+12",
    "Etc/GMT+2",
    "Etc/GMT+3",
    "Etc/GMT+4",
    "Etc/GMT+5",
    "Etc/GMT+6",
    "Etc/GMT+7",
    "Etc/GMT+8",
    "Etc/GMT+9",
    "Etc/GMT-1",
    "Etc/GMT-10",
    "Etc/GMT-11",
    "Etc/GMT-12",
    "Etc/GMT-13",
    "Etc/GMT-14",
    "Etc/GMT-2",
    "Etc/GMT-3",
    "Etc/GMT-4",
    "Etc/GMT-5",
    "Etc/GMT-6",
    "Etc/GMT-7",
    "Etc/GMT-8",
    "Etc/GMT-9",
    "Etc/UTC",
    "Factory",
    "MST7MDT",
    "PST8PDT",
    "SystemV/AST4",
    "SystemV/AST4ADT",
    "SystemV/CST6",
    "SystemV/CST6CDT",
    "SystemV/EST5",
    "SystemV/EST5EDT",
    "SystemV/HST10",
    "SystemV/MST7",
    "SystemV/MST7MDT",
    "SystemV/PST8",
    "SystemV/PST8PDT",
    "SystemV/YST9",
    "SystemV/YST9YDT",
];

/// The zones a region keeps (`Intl.Locale`'s `getTimeZones()`), or all
/// for none, by their reported names in order; UTC, the `Etc` zones
/// and the legacy ones without a region (`EST5EDT`) left out.
pub(super) fn zones_of(region: &str) -> Vec<String> {
    if region == "001" {
        return WORLD_ZONES.iter().map(|zone| (*zone).to_owned()).collect();
    }
    let prefix = region.to_ascii_lowercase();
    let mut zones: Vec<String> = names()
        .iter()
        .filter(|(zone, name)| {
            if region.is_empty() {
                name.contains('/') && !name.starts_with("Etc/")
            } else {
                zone.as_str().len() > 2
                    && zone.as_str().starts_with(&prefix)
                    && !name.starts_with("Etc/")
            }
        })
        .map(|(_, name)| (*name).to_owned())
        .collect();
    zones.sort();
    zones.dedup();
    zones
}

/// Whether a zone keeps daylight time at all (since 1970).
pub(super) fn keeps_daylight(name: &str) -> bool {
    zone_named(name)
        .and_then(|(_, reported)| read().zones.get(reported))
        .is_some_and(|offsets| {
            offsets.rule.is_some() || offsets.changes.iter().any(|(_, _, daylight)| *daylight)
        })
}

/// Whether a zone keeps daylight time at time value `t` or within 184
/// days of it (ICU's check before it names a zone's generic time by its
/// standard one).
pub(super) fn daylight_near(name: &str, t: f64) -> bool {
    let Some(offsets) = zone_named(name).and_then(|(_, reported)| read().zones.get(reported))
    else {
        return false;
    };
    let second = (t.floor() as i64).div_euclid(1000);
    let (from, to) = (second - 184 * 86_400, second + 184 * 86_400);
    // The state at the window's start and each change within it.
    let first = offsets
        .changes
        .partition_point(|(when, _, _)| *when <= from)
        .saturating_sub(1);
    if offsets.changes[first..]
        .iter()
        .take_while(|(when, _, _)| *when < to)
        .any(|(_, _, daylight)| *daylight)
    {
        return true;
    }
    // A final rule keeps daylight time some months of every year.
    offsets.rule.is_some_and(|(_, raw, year)| {
        let last = offsets.changes.last().map_or(0, |(when, _, _)| *when);
        to > last && civil_year((to + i64::from(raw)) * 1000) >= year
    })
}

// ---- The system's zone (ECMA-402 section 6.4.3) ----------------------

/// The Windows zone's key name and the user's region, as the shell read
/// them before any page ran.
static WINDOWS_ZONE: OnceLock<(String, String)> = OnceLock::new();

/// Tells `Intl` the system's time zone as Windows names it (its key
/// name, `W. Europe Standard Time`) and the user's region (`DE`, or ""),
/// which CLDR's windowsZones map to an IANA name as ICU does. The first
/// call holds.
pub fn set_windows_time_zone(name: &str, region: &str) {
    let _ = WINDOWS_ZONE.set((name.to_owned(), region.to_owned()));
}

/// The IANA name of the system's zone: the `TZ` variable's, the zone
/// `/etc/localtime` links to, or the Windows zone's mapped, if ICU4X
/// knows it; "" if none, when the bootstrap names the zone by its
/// offset.
pub(super) fn system_zone() -> &'static str {
    static ZONE: OnceLock<String> = OnceLock::new();
    ZONE.get_or_init(|| {
        if let Ok(tz) = std::env::var("TZ")
            && let Some((_, name)) = zone_named(tz.trim_start_matches(':'))
        {
            return name.to_owned();
        }
        if let Ok(path) = std::fs::read_link("/etc/localtime")
            && let Some(path) = path.to_str()
            && let Some(at) = path.find("zoneinfo/")
            && let Some((_, name)) = zone_named(&path[at + "zoneinfo/".len()..])
        {
            return name.to_owned();
        }
        WINDOWS_ZONE
            .get()
            .and_then(|(name, region)| windows_zone(name, region))
            .unwrap_or_default()
            .to_owned()
    })
}

/// A Windows zone's IANA name: CLDR's mapping for the region, else its
/// world one (`001`).
fn windows_zone(name: &str, region: &str) -> Option<&'static str> {
    let parser = WindowsParser::new();
    let region = Region::try_from_str(region).ok();
    let zone = parser
        .parse(name, region)
        .or_else(|| parser.parse(name, None))?;
    names().get(&zone).copied()
}

// ---- Offsets -------------------------------------------------------------

/// The zones' offsets from 1970 on, as the reference browser's ICU has
/// them (`intl_zones.txt`).
const OFFSETS: &str = include_str!("intl_zones.txt");

/// A zone's offsets read: at 1970, and each change after as the UTC
/// second it takes effect, the offset from then in seconds and whether
/// it is daylight time; then its final rule, the offset it keeps that
/// rule from and the year it does from.
struct Offsets {
    changes: Vec<(i64, i32, bool)>,
    rule: Option<(&'static str, i32, i64)>,
}

/// ICU's SimpleTimeZone rule: when daylight time starts and ends, each a
/// month (from 0), a day, a weekday, seconds into the day and how they
/// are told; and its saving in seconds.
#[derive(Clone, Copy)]
struct Rule {
    start: (i64, i64, i64, i64, i64),
    end: (i64, i64, i64, i64, i64),
    saving: i32,
}

/// [`OFFSETS`] read: each zone's offsets and each rule.
struct Read {
    zones: HashMap<&'static str, Offsets>,
    rules: HashMap<&'static str, Rule>,
}

fn read() -> &'static Read {
    static READ: OnceLock<Read> = OnceLock::new();
    READ.get_or_init(|| {
        let base36 = |text: &str| -> i64 {
            let (sign, digits) = match text.strip_prefix('-') {
                Some(digits) => (-1, digits),
                None => (1, text),
            };
            sign * i64::from_str_radix(digits, 36).unwrap_or(0)
        };
        let mut read = Read {
            zones: HashMap::new(),
            rules: HashMap::new(),
        };
        for line in OFFSETS.lines().filter(|line| !line.starts_with('#')) {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["*", name, numbers] => {
                    let n: Vec<i64> = numbers.split(' ').filter_map(|n| n.parse().ok()).collect();
                    if let [a, b, c, d, e, f, g, h, i, j, saving] = n.as_slice() {
                        read.rules.insert(
                            name,
                            Rule {
                                start: (*a, *b, *c, *d, *e),
                                end: (*f, *g, *h, *i, *j),
                                saving: *saving as i32,
                            },
                        );
                    }
                }
                [zone, items, rest @ ..] => {
                    let mut changes = Vec::new();
                    let mut at = 0;
                    for item in items.split(' ') {
                        let daylight = item.ends_with('+');
                        let item = item.trim_end_matches('+');
                        let offset = match item.split_once('/') {
                            Some((since, offset)) => {
                                at += base36(since);
                                base36(offset)
                            }
                            None => base36(item),
                        };
                        changes.push((at, offset as i32, daylight));
                    }
                    let rule = rest.first().and_then(|rule| {
                        let parts: Vec<&str> = rule.split(' ').collect();
                        match parts.as_slice() {
                            [name, raw, year] => {
                                Some((*name, base36(raw) as i32, year.parse().ok()?))
                            }
                            _ => None,
                        }
                    });
                    read.zones.insert(zone, Offsets { changes, rule });
                }
                _ => {}
            }
        }
        read
    })
}

/// The days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The days since 1970-01-01 of the day a SimpleTimeZone rule names in
/// a year: a month (from 0), a day of it and a weekday (1 Sunday), as
/// ICU reads them: the day itself (no weekday), the day's nth weekday
/// (the last for -1), the weekday on or after the day (a negative
/// weekday), or on or before it (both negative).
fn rule_day(year: i64, month: i64, day: i64, weekday: i64) -> i64 {
    // 1970-01-01 was a Thursday.
    let weekday_of = |days: i64| (days + 4).rem_euclid(7);
    let month = month + 1;
    let last = |year: i64, month: i64| {
        if month == 12 {
            days_from_civil(year + 1, 1, 1) - 1
        } else {
            days_from_civil(year, month + 1, 1) - 1
        }
    };
    match (day, weekday) {
        (day, 0) => days_from_civil(year, month, day),
        (day, weekday) if day > 0 && weekday > 0 => {
            let first = days_from_civil(year, month, 1);
            first + (weekday - 1 - weekday_of(first)).rem_euclid(7) + 7 * (day - 1)
        }
        (_, weekday) if weekday > 0 => {
            let last = last(year, month);
            last - (weekday_of(last) - (weekday - 1)).rem_euclid(7)
        }
        (day, weekday) if day > 0 => {
            let from = days_from_civil(year, month, day);
            from + (-weekday - 1 - weekday_of(from)).rem_euclid(7)
        }
        (day, weekday) => {
            let from = days_from_civil(year, month, -day);
            from - (weekday_of(from) - (-weekday - 1)).rem_euclid(7)
        }
    }
}

/// Whether a rule keeps daylight time at UTC second `t` in a zone of
/// standard offset `raw`.
fn in_daylight(rule: &Rule, raw: i32, t: i64) -> bool {
    let raw = i64::from(raw);
    let year = civil_year((t + raw) * 1000);
    let instant = |(month, day, weekday, seconds, told): (i64, i64, i64, i64, i64), before: i64| {
        let local = rule_day(year, month, day, weekday) * 86_400 + seconds;
        match told {
            2 => local,
            1 => local - raw,
            _ => local - before,
        }
    };
    let start = instant(rule.start, raw);
    let end = instant(rule.end, raw + i64::from(rule.saving));
    if start < end {
        (start..end).contains(&t)
    } else {
        t >= start || t < end
    }
}

/// `intlZoneOffset(zone, t)`: the offset from UTC, in seconds, of the
/// zone named at time value `t`; `None` for a zone ICU4X does not know.
pub(super) fn offset_at(name: &str, t: f64) -> Option<i32> {
    state_at(name, t).map(|(offset, _)| offset)
}

/// Whether a zone keeps daylight time at time value `t`.
pub(super) fn daylight_at(name: &str, t: f64) -> bool {
    state_at(name, t).is_some_and(|(_, daylight)| daylight)
}

/// A zone's offset from UTC at time value `t`, in seconds, and whether
/// it is daylight time then: its last change by then, or its final rule
/// after its changes (its state at 1970 before it).
fn state_at(name: &str, t: f64) -> Option<(i32, bool)> {
    if !t.is_finite() {
        return None;
    }
    let (_, reported) = zone_named(name)?;
    let read = read();
    let offsets = read.zones.get(reported)?;
    let second = (t.floor() as i64).div_euclid(1000);
    let at = offsets
        .changes
        .partition_point(|(when, _, _)| *when <= second);
    let (when, offset, daylight) = offsets.changes[at.saturating_sub(1)];
    if at == offsets.changes.len()
        && let Some((name, raw, from)) = offsets.rule
        && let Some(rule) = read.rules.get(name)
        && civil_year((second + i64::from(raw)) * 1000) >= from
        && second >= when
    {
        return Some(if in_daylight(rule, raw, second) {
            (raw + rule.saving, true)
        } else {
            (raw, false)
        });
    }
    Some((offset, daylight))
}

/// The proleptic Gregorian year of a time value.
fn civil_year(t: i64) -> i64 {
    let days = t.div_euclid(MS_PER_DAY);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    yoe + era * 400 + i64::from(mp >= 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> f64 {
        ((days_from_civil(year, month, day) * 24 + hour) * 3_600_000 + minute * 60_000) as f64
    }

    #[test]
    fn days_and_years_are_gregorian() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2020, 1, 2), 18_263);
        assert_eq!(days_from_civil(2000, 2, 29), 11_016);
        assert_eq!(civil_year(at(2020, 12, 31, 23, 59) as i64), 2020);
        assert_eq!(civil_year(at(1969, 12, 31, 0, 0) as i64), 1969);
        // The rules' days (ICU's months from 0, weekdays from 1 for
        // Sunday): the second Sunday of March 2024 (on or after the 8th),
        // the last Sunday of October, the Saturday on or before the 30th
        // of March, the fourth Thursday of November and a fixed day.
        assert_eq!(rule_day(2024, 2, 8, -1), days_from_civil(2024, 3, 10));
        assert_eq!(rule_day(2024, 9, -1, 1), days_from_civil(2024, 10, 27));
        assert_eq!(rule_day(2024, 2, -30, -7), days_from_civil(2024, 3, 30));
        assert_eq!(rule_day(2024, 10, 4, 5), days_from_civil(2024, 11, 28));
        assert_eq!(rule_day(2024, 2, 21, 0), days_from_civil(2024, 3, 21));
    }

    #[test]
    fn names_are_canonical_as_icu_has_them() {
        assert_eq!(
            zone_named("america/new_york").map(|z| z.1),
            Some("America/New_York")
        );
        assert_eq!(
            zone_named("US/Eastern").map(|z| z.1),
            Some("America/New_York")
        );
        assert_eq!(
            zone_named("Asia/Kolkata").map(|z| z.1),
            Some("Asia/Calcutta")
        );
        assert_eq!(zone_named("Etc/UTC").map(|z| z.1), Some("UTC"));
        assert_eq!(zone_named("Mars/Olympus"), None);
        assert_eq!(
            windows_zone("W. Europe Standard Time", ""),
            Some("Europe/Berlin")
        );
        assert_eq!(
            windows_zone("W. Europe Standard Time", "CH"),
            Some("Europe/Zurich")
        );
        assert_eq!(
            windows_zone("Pacific Standard Time", "US"),
            Some("America/Los_Angeles")
        );
        assert_eq!(
            windows_zone("India Standard Time", "IN"),
            Some("Asia/Calcutta")
        );
        assert_eq!(windows_zone("Nowhere Standard Time", ""), None);
        assert_eq!(zones_of("FR"), ["Europe/Paris"]);
    }

    #[test]
    fn offsets_follow_the_zones_rules() {
        let offset = |zone: &str, t: f64| offset_at(zone, t).map(|s| s / 60);
        // New York: EST, then EDT from 2024-03-10 07:00 UTC.
        assert_eq!(
            offset("America/New_York", at(2024, 3, 10, 6, 59)),
            Some(-300)
        );
        assert_eq!(
            offset("America/New_York", at(2024, 3, 10, 7, 0)),
            Some(-240)
        );
        assert_eq!(
            offset("America/New_York", at(2024, 11, 3, 5, 59)),
            Some(-240)
        );
        assert_eq!(
            offset("America/New_York", at(2024, 11, 3, 6, 0)),
            Some(-300)
        );
        // Berlin: CET, CEST from 01:00 UTC on the last Sunday of March.
        assert_eq!(offset("Europe/Berlin", at(2024, 3, 31, 0, 59)), Some(60));
        assert_eq!(offset("Europe/Berlin", at(2024, 3, 31, 1, 0)), Some(120));
        assert_eq!(offset("Europe/Berlin", at(2024, 10, 27, 1, 0)), Some(60));
        // Sydney, in the south: AEDT over the new year.
        assert_eq!(offset("Australia/Sydney", at(2024, 1, 15, 0, 0)), Some(660));
        assert_eq!(offset("Australia/Sydney", at(2024, 7, 15, 0, 0)), Some(600));
        assert_eq!(offset("Asia/Kolkata", at(2024, 7, 15, 0, 0)), Some(330));
        assert_eq!(offset("America/Phoenix", at(2024, 7, 15, 0, 0)), Some(-420));
        // Mexico City kept daylight time until 2022.
        assert_eq!(
            offset("America/Mexico_City", at(2020, 7, 1, 0, 0)),
            Some(-300)
        );
        assert_eq!(
            offset("America/Mexico_City", at(2024, 7, 1, 0, 0)),
            Some(-360)
        );
        assert_eq!(offset("Mars/Olympus", 0.0), None);
    }

    #[test]
    fn every_zone_has_offsets() {
        let missing: Vec<&str> = names()
            .values()
            .filter(|name| !read().zones.contains_key(*name))
            .copied()
            .collect();
        assert_eq!(missing, Vec::<&str>::new());
    }

    #[test]
    fn offsets_of_year_by_year_rules_are_the_reference_browsers() {
        let offset = |zone: &str, (y, m, d): (i64, i64, i64)| {
            offset_at(zone, at(y, m, d, 0, 0)).map(|s| s / 60)
        };
        // Chrome 156's offsets: Morocco at +00 for Ramadan and from 2027,
        // and with European summers before 2018.
        assert_eq!(offset("Africa/Casablanca", (2020, 5, 1)), Some(0));
        assert_eq!(offset("Africa/Casablanca", (2020, 7, 1)), Some(60));
        assert_eq!(offset("Africa/Casablanca", (2025, 3, 1)), Some(0));
        assert_eq!(offset("Africa/Casablanca", (2010, 7, 1)), Some(60));
        assert_eq!(offset("Africa/Casablanca", (2027, 1, 1)), Some(0));
        // Palestine's summer starts after Ramadan.
        assert_eq!(offset("Asia/Gaza", (2024, 4, 1)), Some(120));
        assert_eq!(offset("Asia/Gaza", (2024, 7, 1)), Some(180));
        assert_eq!(offset("Asia/Gaza", (2024, 12, 1)), Some(120));
        assert_eq!(offset("Asia/Hebron", (2030, 7, 1)), Some(180));
        // Jordan and Syria kept summer time for good in 2022.
        for zone in ["Asia/Amman", "Asia/Damascus"] {
            assert_eq!(offset(zone, (2021, 7, 1)), Some(180));
            assert_eq!(offset(zone, (2021, 12, 1)), Some(120));
            assert_eq!(offset(zone, (2023, 1, 1)), Some(180));
        }
        // Fiji's last summer was 2020's.
        assert_eq!(offset("Pacific/Fiji", (2020, 1, 1)), Some(780));
        assert_eq!(offset("Pacific/Fiji", (2020, 7, 1)), Some(720));
        assert_eq!(offset("Pacific/Fiji", (2024, 1, 1)), Some(720));
        // Older changes and a final rule's later years.
        assert_eq!(offset("America/Sao_Paulo", (1990, 1, 1)), Some(-120));
        assert_eq!(offset("Africa/Cairo", (1990, 7, 1)), Some(180));
        assert_eq!(offset("America/New_York", (2040, 7, 1)), Some(-240));
    }
}
