//! Cookies (M27.4, docs/design/fetch.md): RFC 6265bis-22's user agent.
//! A `Set-Cookie` value is parsed (section 5.6) and stored by the
//! storage model (5.7), within the jar's limits; the `Cookie` header a
//! request sends, and what `document.cookie` reads, are retrieved by
//! section 5.8. The jar lives in memory, one to a client; the cookies
//! with an expiry outlive the session in the profile (M28.3,
//! docs/design/storage.md), as records [`Jar::export`] makes and
//! [`Jar::restore`] reads.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{Host, Url};

/// The most bytes a cookie's name and value may hold together.
pub const MAX_COOKIE_SIZE: usize = 4096;

/// The most bytes a cookie attribute's value may hold; a longer one is
/// ignored.
pub const MAX_ATTRIBUTE_SIZE: usize = 1024;

/// The longest a cookie lives: `Expires` and `Max-Age` past it are
/// brought back to it.
pub const MAX_COOKIE_AGE: Duration = Duration::from_secs(400 * 24 * 60 * 60);

/// The most cookies a domain keeps (the reference browser's; the RFC
/// asks for at least 50).
pub const MAX_COOKIES_PER_DOMAIN: usize = 180;

/// The most cookies the jar keeps.
pub const MAX_COOKIES: usize = 3000;

/// A cookie's `SameSite` (RFC 6265bis 4.1.2.7): with no attribute, or
/// one of another value, it is `Lax` (the "default" enforcement).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

/// A cookie as the jar keeps it (5.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie {
    pub name: Vec<u8>,
    pub value: Vec<u8>,
    /// The host it goes to (host-only), or the domain it goes to with
    /// its subdomains; lower case.
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    /// When it expires; `None` for a session cookie, which lives as
    /// long as the jar.
    pub expires: Option<SystemTime>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
    /// For a `Partitioned` cookie (CHIPS, M33.2), the top-level site it
    /// is kept under; it goes to no context of another.
    pub partition: Option<String>,
    /// The order of its making, kept when it is replaced.
    created: u64,
    /// The order of its last sending, for eviction.
    accessed: u64,
}

/// Where a cookie came from or goes to: HTTP, or a script's
/// `document.cookie` (the RFC's "non-HTTP" API).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    Http,
    Script,
}

/// What a request is, to the cookies' rules (5.2): whether it is
/// same-site with the site for cookies, and whether it is a top-level
/// navigation, and by a safe method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub same_site: bool,
    pub top_level: bool,
    pub safe_method: bool,
    /// The top-level site's key (M33.2): which `Partitioned` cookies the
    /// context has; none has none.
    pub partition: Option<String>,
}

impl Context {
    /// A same-site context: `document.cookie`'s, the document being its
    /// own top level.
    pub const SAME_SITE: Context = Context {
        same_site: true,
        top_level: true,
        safe_method: true,
        partition: None,
    };
}

/// The cookies of a client (5.7), in the order they were stored.
#[derive(Debug, Default)]
pub struct Jar {
    cookies: Vec<Cookie>,
    /// The next number for `created` and `accessed`.
    next: u64,
    /// How many times the cookies with an expiry have changed.
    kept: u64,
}

/// The version of the records [`Jar::export`] makes.
const RECORD: u8 = 2;

/// M28's records, which have no partition.
const RECORD_1: u8 = 1;

/// A `Set-Cookie` value parsed (5.6): its name and value and the
/// attributes the storage model reads, each the last of its kind.
#[derive(Debug, Default)]
struct Parsed {
    name: Vec<u8>,
    value: Vec<u8>,
    expires: Option<SystemTime>,
    max_age: Option<SystemTime>,
    domain: Option<String>,
    path: Option<String>,
    secure: bool,
    http_only: bool,
    same_site: Option<SameSite>,
    partitioned: bool,
}

/// A control character a cookie may not hold: any but the tab.
fn is_control(b: u8) -> bool {
    (b < 0x20 && b != b'\t') || b == 0x7F
}

/// `bytes` without leading or trailing spaces and tabs.
fn trim(bytes: &[u8]) -> &[u8] {
    let wsp = |b: &u8| matches!(b, b' ' | b'\t');
    let start = bytes.iter().position(|b| !wsp(b)).unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !wsp(b))
        .map_or(start, |end| end + 1);
    &bytes[start..end]
}

/// The `Set-Cookie` value `line` parsed (5.6), at `now`; `None` for one
/// the algorithm ignores whole.
fn parse(line: &[u8], now: SystemTime) -> Option<Parsed> {
    if line.iter().any(|&b| is_control(b)) {
        return None;
    }
    let (pair, mut rest) = match line.iter().position(|&b| b == b';') {
        Some(semicolon) => (&line[..semicolon], &line[semicolon..]),
        None => (line, &[][..]),
    };
    let (name, value) = match pair.iter().position(|&b| b == b'=') {
        Some(equals) => (trim(&pair[..equals]), trim(&pair[equals + 1..])),
        None => (&[][..], trim(pair)),
    };
    if name.len() + value.len() > MAX_COOKIE_SIZE {
        return None;
    }
    let mut parsed = Parsed {
        name: name.to_vec(),
        value: value.to_vec(),
        ..Parsed::default()
    };
    while let Some(after) = rest.strip_prefix(b";") {
        let end = after.iter().position(|&b| b == b';').unwrap_or(after.len());
        let av = &after[..end];
        rest = &after[end..];
        let (name, value) = match av.iter().position(|&b| b == b'=') {
            Some(equals) => (trim(&av[..equals]), trim(&av[equals + 1..])),
            None => (trim(av), &[][..]),
        };
        if value.len() > MAX_ATTRIBUTE_SIZE {
            continue;
        }
        let latest = now + MAX_COOKIE_AGE;
        match name.to_ascii_lowercase().as_slice() {
            b"expires" => {
                if let Some(date) = parse_date(value) {
                    parsed.expires = Some(date.min(latest));
                }
            }
            b"max-age" => {
                let digits = value.strip_prefix(b"-").unwrap_or(value);
                if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
                    continue;
                }
                // Too many digits for a number is a very long time.
                let seconds = std::str::from_utf8(digits)
                    .ok()
                    .and_then(|digits| digits.parse::<u64>().ok())
                    .unwrap_or(u64::MAX);
                parsed.max_age = Some(if value.starts_with(b"-") || seconds == 0 {
                    UNIX_EPOCH
                } else {
                    now.checked_add(Duration::from_secs(seconds))
                        .map_or(latest, |date| date.min(latest))
                });
            }
            b"domain" => {
                if value.is_empty() {
                    continue;
                }
                let value = value.strip_prefix(b".").unwrap_or(value);
                parsed.domain = Some(String::from_utf8_lossy(value).to_ascii_lowercase());
            }
            b"path" => {
                parsed.path = (value.first() == Some(&b'/'))
                    .then(|| String::from_utf8_lossy(value).into_owned());
            }
            b"secure" => parsed.secure = true,
            b"httponly" => parsed.http_only = true,
            b"partitioned" => parsed.partitioned = true,
            b"samesite" => {
                parsed.same_site = Some(match value.to_ascii_lowercase().as_slice() {
                    b"strict" => SameSite::Strict,
                    b"none" => SameSite::None,
                    _ => SameSite::Lax,
                });
            }
            _ => {}
        }
    }
    Some(parsed)
}

/// A cookie date (5.1.1): its tokens read for a time, a day of the
/// month, a month and a year, in any order, the first of each found;
/// `None` when one is missing or out of range.
pub(crate) fn parse_date(value: &[u8]) -> Option<SystemTime> {
    let delimiter =
        |b: u8| matches!(b, 0x09 | 0x20..=0x2F | 0x3B..=0x40 | 0x5B..=0x60 | 0x7B..=0x7E);
    let (mut time, mut day, mut month, mut year) = (None, None, None, None);
    for token in value.split(|&b| delimiter(b)).filter(|t| !t.is_empty()) {
        if time.is_none()
            && let Some(found) = parse_time(token)
        {
            time = Some(found);
        } else if day.is_none()
            && let Some(found) = leading_digits(token, 1, 2)
        {
            day = Some(found);
        } else if month.is_none()
            && let Some(found) = parse_month(token)
        {
            month = Some(found);
        } else if year.is_none()
            && let Some(found) = leading_digits(token, 2, 4)
        {
            year = Some(found);
        }
    }
    let (hour, minute, second) = time?;
    let (day, month) = (day?, month?);
    let year = match year? {
        year @ 70..=99 => year + 1900,
        year @ 0..=69 => year + 2000,
        year => year,
    };
    let days_in_month = match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day < 1 || day > days_in_month || year < 1601 || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + i64::from(hour * 3600 + minute * 60 + second);
    match u64::try_from(seconds) {
        Ok(after) => UNIX_EPOCH.checked_add(Duration::from_secs(after)),
        Err(_) => UNIX_EPOCH.checked_sub(Duration::from_secs(seconds.unsigned_abs())),
    }
}

/// A number of `min` to `max` digits starting `token`, whatever follows
/// it that is not a digit.
fn leading_digits(token: &[u8], min: usize, max: usize) -> Option<u32> {
    let count = token.iter().take_while(|b| b.is_ascii_digit()).count();
    if count < min || count > max {
        return None;
    }
    std::str::from_utf8(&token[..count]).ok()?.parse().ok()
}

/// `hms-time` (5.1.1): one or two digits, `:`, twice more, whatever
/// follows the seconds that is not a digit.
fn parse_time(token: &[u8]) -> Option<(u32, u32, u32)> {
    let mut parts = token.splitn(3, |&b| b == b':');
    let (hour, minute, second) = (parts.next()?, parts.next()?, parts.next()?);
    let exact = |part: &[u8]| (1..=2).contains(&part.len()) && part.iter().all(u8::is_ascii_digit);
    if !exact(hour) || !exact(minute) {
        return None;
    }
    Some((
        leading_digits(hour, 1, 2)?,
        leading_digits(minute, 1, 2)?,
        leading_digits(second, 1, 2)?,
    ))
}

/// A month by its first three letters, in any case.
fn parse_month(token: &[u8]) -> Option<u32> {
    let prefix = token.get(..3)?.to_ascii_lowercase();
    [
        b"jan", b"feb", b"mar", b"apr", b"may", b"jun", b"jul", b"aug", b"sep", b"oct", b"nov",
        b"dec",
    ]
    .iter()
    .position(|month| month.as_slice() == prefix.as_slice())
    .map(|index| index as u32 + 1)
}

/// Days from 1970-01-01 to a date of the proleptic Gregorian calendar.
fn days_from_civil(year: u32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let month = i64::from(month);
    let of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let of_era_days = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    era * 146_097 + of_era_days - 719_468
}

/// The canonicalized host of `url` (5.1.2): its host serialized, lower
/// case already.
fn canonical_host(url: &Url) -> Option<String> {
    url.host().map(ToString::to_string)
}

/// Whether `host` domain-matches `domain` (5.1.3): the same, or a name
/// ending in `.` and `domain` that is not an IP address.
fn domain_match(host: &str, domain: &str) -> bool {
    host == domain
        || (host.len() > domain.len()
            && host.ends_with(domain)
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
            && !is_ip(host))
}

fn is_ip(host: &str) -> bool {
    host.starts_with('[') || host.parse::<std::net::Ipv4Addr>().is_ok()
}

/// The default path of `url` (5.1.4): its path up to its last `/`, or
/// `/` when that is all there is.
fn default_path(url: &Url) -> String {
    let path = url.path_str();
    match path.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(last) if path.starts_with('/') => path[..last].to_owned(),
        Some(_) => "/".to_owned(),
    }
}

/// Whether the request path `path` path-matches a cookie's `cookie`
/// path (5.1.4).
fn path_match(path: &str, cookie: &str) -> bool {
    path == cookie
        || (path.starts_with(cookie)
            && (cookie.ends_with('/') || path.as_bytes().get(cookie.len()) == Some(&b'/')))
}

/// Whether `domain`, a cookie's `Domain`, is a public suffix: what the
/// list says for it as a host.
fn is_public_suffix(domain: &str) -> bool {
    let host = Host::Domain(domain.to_owned());
    host.public_suffix() == Some(domain)
}

impl Jar {
    /// An empty jar.
    pub fn new() -> Jar {
        Jar::default()
    }

    /// The cookies kept, in the order they were stored.
    pub fn cookies(&self) -> &[Cookie] {
        &self.cookies
    }

    /// Stores the cookie of the `Set-Cookie` value `line` received from
    /// `url` through `api`, for a request in `context`, at `now` (5.7);
    /// whether one was stored (a cookie that expires on arrival deletes
    /// its namesake and counts as stored).
    pub fn store(
        &mut self,
        line: &[u8],
        url: &Url,
        api: Api,
        context: Context,
        now: SystemTime,
    ) -> bool {
        self.remove_expired(now);
        let Some(parsed) = parse(line, now) else {
            return false;
        };
        let Some(host) = canonical_host(url) else {
            return false;
        };
        if parsed.name.is_empty() && parsed.value.is_empty() {
            return false;
        }
        let prefixed = |bytes: &[u8], prefix: &[u8]| {
            bytes.len() >= prefix.len() && bytes[..prefix.len()].eq_ignore_ascii_case(prefix)
        };
        if parsed.name.is_empty()
            && (prefixed(&parsed.value, b"__Secure-") || prefixed(&parsed.value, b"__Host-"))
        {
            return false;
        }
        let mut domain = parsed.domain.clone().unwrap_or_default();
        if !domain.is_empty() && is_public_suffix(&domain) {
            if domain != host {
                return false;
            }
            domain.clear();
        }
        if !domain.is_empty() && !domain_match(&host, &domain) {
            return false;
        }
        let host_only = domain.is_empty();
        let secure_url = url.is_potentially_trustworthy();
        if parsed.secure && !secure_url {
            return false;
        }
        if parsed.http_only && api == Api::Script {
            return false;
        }
        // A partitioned cookie is a secure one kept under the context's
        // top-level site (CHIPS); a context with none keeps none.
        if parsed.partitioned && (!parsed.secure || context.partition.is_none()) {
            return false;
        }
        let cookie = Cookie {
            domain: if host_only { host.clone() } else { domain },
            host_only,
            path: parsed.path.clone().unwrap_or_else(|| default_path(url)),
            expires: parsed.max_age.or(parsed.expires),
            secure: parsed.secure,
            http_only: parsed.http_only,
            same_site: parsed.same_site.unwrap_or(SameSite::Lax),
            partition: parsed
                .partitioned
                .then(|| context.partition.clone())
                .flatten(),
            name: parsed.name,
            value: parsed.value,
            created: 0,
            accessed: 0,
        };
        // An insecure origin leaves secure cookies alone.
        if !cookie.secure
            && !secure_url
            && self.cookies.iter().any(|old| {
                old.secure
                    && old.name == cookie.name
                    && (domain_match(&cookie.domain, &old.domain)
                        || domain_match(&old.domain, &cookie.domain))
                    && path_match(&cookie.path, &old.path)
            })
        {
            return false;
        }
        if cookie.same_site != SameSite::None && !context.same_site && !context.top_level {
            return false;
        }
        if cookie.same_site == SameSite::None && !cookie.secure {
            return false;
        }
        if prefixed(&cookie.name, b"__Secure-") && !cookie.secure {
            return false;
        }
        if prefixed(&cookie.name, b"__Host-")
            && !(cookie.secure && cookie.host_only && cookie.path == "/")
        {
            return false;
        }
        let mut cookie = cookie;
        let same = self.cookies.iter().position(|old| {
            old.name == cookie.name
                && old.domain == cookie.domain
                && old.host_only == cookie.host_only
                && old.path == cookie.path
                && old.partition == cookie.partition
        });
        cookie.created = self.next;
        if let Some(index) = same {
            let old = &self.cookies[index];
            if api == Api::Script && old.http_only {
                return false;
            }
            cookie.created = old.created;
            self.take(index);
        }
        self.next += 1;
        cookie.accessed = self.next;
        if cookie.expires.is_some_and(|expires| expires <= now) {
            return true;
        }
        if cookie.expires.is_some() {
            self.kept += 1;
        }
        let domain = (cookie.domain.clone(), cookie.partition.clone());
        self.cookies.push(cookie);
        self.evict(&domain);
        true
    }

    /// The cookies for a request to `url` through `api` in `context` at
    /// `now`, as a `Cookie` header's value (5.8.3): those whose domain
    /// and path match it, secure ones only to a potentially trustworthy
    /// URL, `HttpOnly` ones only to HTTP, and those `SameSite` lets go
    /// with a cross-site request; longer paths first, then older ones;
    /// a nameless cookie its value alone. `None` when there is none.
    pub fn header(
        &mut self,
        url: &Url,
        api: Api,
        context: Context,
        now: SystemTime,
    ) -> Option<Vec<u8>> {
        self.remove_expired(now);
        let host = canonical_host(url)?;
        let path = url.path_str();
        let secure_url = url.is_potentially_trustworthy();
        let mut chosen: Vec<usize> = (0..self.cookies.len())
            .filter(|&index| {
                let cookie = &self.cookies[index];
                let domain = if cookie.host_only {
                    host == cookie.domain
                } else {
                    domain_match(&host, &cookie.domain)
                };
                domain
                    && path_match(&path, &cookie.path)
                    && (cookie.partition.is_none() || cookie.partition == context.partition)
                    && (!cookie.secure || secure_url)
                    && (!cookie.http_only || api == Api::Http)
                    && match cookie.same_site {
                        SameSite::None => true,
                        _ if context.same_site => true,
                        SameSite::Strict => false,
                        SameSite::Lax => context.top_level && context.safe_method,
                    }
            })
            .collect();
        if chosen.is_empty() {
            return None;
        }
        chosen.sort_by(|&a, &b| {
            let (a, b) = (&self.cookies[a], &self.cookies[b]);
            b.path
                .len()
                .cmp(&a.path.len())
                .then(a.created.cmp(&b.created))
        });
        let mut out = Vec::new();
        for index in chosen {
            self.next += 1;
            let cookie = &mut self.cookies[index];
            cookie.accessed = self.next;
            if !out.is_empty() {
                out.extend_from_slice(b"; ");
            }
            if !cookie.name.is_empty() {
                out.extend_from_slice(&cookie.name);
                out.push(b'=');
            }
            out.extend_from_slice(&cookie.value);
        }
        Some(out)
    }

    /// Takes out the cookies expired at `now`.
    fn remove_expired(&mut self, now: SystemTime) {
        let before = self.cookies.len();
        self.cookies
            .retain(|cookie| cookie.expires.is_none_or(|expires| expires > now));
        if self.cookies.len() != before {
            self.kept += 1;
        }
    }

    /// Takes out the cookie at `index`.
    fn take(&mut self, index: usize) {
        if self.cookies.remove(index).expires.is_some() {
            self.kept += 1;
        }
    }

    /// How many times the cookies with an expiry have changed: one
    /// stored, replaced, expired or evicted (M28.3). The engine writes
    /// them to the profile when this has moved since it last did; a
    /// cookie sent moves it not.
    pub fn kept_changes(&self) -> u64 {
        self.kept
    }

    /// The cookies with an expiry, those that outlive the session (the
    /// RFC's persistent-flag), as records for the profile, in the jar's
    /// order (M28.3).
    pub fn export(&self) -> Vec<Vec<u8>> {
        self.cookies.iter().filter_map(Cookie::encode).collect()
    }

    /// Takes back the cookies `records` hold, as [`Jar::export`] made
    /// them, at `now` (M28.3): a record that does not read as one, or
    /// a cookie expired, is dropped; one with the name, domain and path
    /// of a cookie in the jar replaces it; and the jar keeps within its
    /// limits. Their orders of making and of use are kept, after the
    /// jar's own. Whether the cookies with an expiry the jar now has
    /// are other than the records: some dropped, or the jar had its own.
    pub fn restore(&mut self, records: &[Vec<u8>], now: SystemTime) -> bool {
        self.remove_expired(now);
        let mut other = self.cookies.iter().any(|cookie| cookie.expires.is_some());
        let mut restored: Vec<Cookie> = records
            .iter()
            .filter_map(|record| Cookie::decode(record))
            .filter(|cookie| cookie.expires.is_some_and(|expires| expires > now))
            .map(|cookie| Cookie {
                // No longer than a cookie stored now may live.
                expires: cookie
                    .expires
                    .map(|expires| expires.min(now + MAX_COOKIE_AGE)),
                ..cookie
            })
            .collect();
        other |= restored.len() != records.len();
        let mut order: Vec<usize> = (0..restored.len()).collect();
        order.sort_by_key(|&index| restored[index].created);
        for (rank, index) in (self.next..).zip(order.iter().copied()) {
            restored[index].created = rank;
        }
        order.sort_by_key(|&index| restored[index].accessed);
        for (rank, index) in (self.next..).zip(order.iter().copied()) {
            restored[index].accessed = rank;
        }
        self.next += restored.len() as u64;
        let kept = self.kept;
        // The names, domains and paths kept, so that a namesake is found
        // without a search of the jar for each of thousands.
        let identity = |c: &Cookie| {
            (
                c.name.clone(),
                c.domain.clone(),
                c.host_only,
                c.path.clone(),
                c.partition.clone(),
            )
        };
        let mut held: HashSet<_> = self.cookies.iter().map(identity).collect();
        for cookie in restored {
            if !held.insert(identity(&cookie))
                && let Some(index) = self
                    .cookies
                    .iter()
                    .position(|old| identity(old) == identity(&cookie))
            {
                self.take(index);
            }
            self.cookies.push(cookie);
        }
        self.trim();
        other || self.kept != kept
    }

    /// Keeps the jar within its limits all at once, the least recently
    /// used going first, as [`Jar::evict`] would after each of many
    /// cookies came: a domain's past [`MAX_COOKIES_PER_DOMAIN`], then
    /// the jar's past [`MAX_COOKIES`]. A sort, not a search for each,
    /// however many a profile held. The orders of use are distinct.
    fn trim(&mut self) {
        // A partition's cookies count apart (CHIPS).
        let mut by_domain: HashMap<(&str, Option<&str>), Vec<u64>> = HashMap::new();
        for cookie in &self.cookies {
            by_domain
                .entry((&cookie.domain, cookie.partition.as_deref()))
                .or_default()
                .push(cookie.accessed);
        }
        let mut gone = HashSet::new();
        for (_, mut used) in by_domain {
            if used.len() > MAX_COOKIES_PER_DOMAIN {
                used.sort_unstable();
                gone.extend(used[..used.len() - MAX_COOKIES_PER_DOMAIN].iter().copied());
            }
        }
        let left = self.cookies.len() - gone.len();
        if left > MAX_COOKIES {
            let mut used: Vec<u64> = self
                .cookies
                .iter()
                .map(|cookie| cookie.accessed)
                .filter(|accessed| !gone.contains(accessed))
                .collect();
            used.sort_unstable();
            gone.extend(used[..left - MAX_COOKIES].iter().copied());
        }
        if gone.is_empty() {
            return;
        }
        let kept = self
            .cookies
            .iter()
            .any(|cookie| gone.contains(&cookie.accessed) && cookie.expires.is_some());
        self.cookies
            .retain(|cookie| !gone.contains(&cookie.accessed));
        if kept {
            self.kept += 1;
        }
    }

    /// Keeps the jar within its limits (5.7) once a cookie for `domain`
    /// came: past [`MAX_COOKIES_PER_DOMAIN`] for the domain, or
    /// [`MAX_COOKIES`] in all, the least recently used go first.
    fn evict(&mut self, domain: &(String, Option<String>)) {
        let of = |cookie: &Cookie| cookie.domain == domain.0 && cookie.partition == domain.1;
        let oldest = |cookies: &[Cookie], domain: bool| {
            (0..cookies.len())
                .filter(|&index| !domain || of(&cookies[index]))
                .min_by_key(|&index| cookies[index].accessed)
                .expect("a cookie to evict")
        };
        while self.cookies.iter().filter(|cookie| of(cookie)).count() > MAX_COOKIES_PER_DOMAIN {
            let index = oldest(&self.cookies, true);
            self.take(index);
        }
        while self.cookies.len() > MAX_COOKIES {
            let index = oldest(&self.cookies, false);
            self.take(index);
        }
    }
}

impl Cookie {
    /// The cookie as a record for the profile (M28.3): the version, the
    /// name, value, domain and path each after its length, the flags,
    /// `SameSite`, the expiry since the Unix epoch, and the orders of
    /// making and of use; little-endian. `None` for a session cookie,
    /// which the profile does not keep.
    fn encode(&self) -> Option<Vec<u8>> {
        let expires = self.expires?.duration_since(UNIX_EPOCH).ok()?;
        let mut record = vec![RECORD];
        for field in [
            &self.name[..],
            &self.value,
            self.domain.as_bytes(),
            self.path.as_bytes(),
            self.partition.as_deref().unwrap_or_default().as_bytes(),
        ] {
            record.extend_from_slice(&u32::try_from(field.len()).ok()?.to_le_bytes());
            record.extend_from_slice(field);
        }
        record.push(
            u8::from(self.host_only) | u8::from(self.secure) << 1 | u8::from(self.http_only) << 2,
        );
        record.push(match self.same_site {
            SameSite::Strict => 0,
            SameSite::Lax => 1,
            SameSite::None => 2,
        });
        record.extend_from_slice(&expires.as_secs().to_le_bytes());
        record.extend_from_slice(&expires.subsec_nanos().to_le_bytes());
        record.extend_from_slice(&self.created.to_le_bytes());
        record.extend_from_slice(&self.accessed.to_le_bytes());
        Some(record)
    }

    /// The cookie a record holds, if it reads as one [`Cookie::encode`]
    /// makes, its sizes within the jar's limits: the profile is a file
    /// anything may have written.
    fn decode(record: &[u8]) -> Option<Cookie> {
        fn split<'a>(rest: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
            if rest.len() < n {
                return None;
            }
            let (taken, left) = rest.split_at(n);
            *rest = left;
            Some(taken)
        }
        fn field<'a>(rest: &mut &'a [u8]) -> Option<&'a [u8]> {
            let length = u32::from_le_bytes(split(rest, 4)?.try_into().ok()?);
            split(rest, usize::try_from(length).ok()?)
        }
        fn number(rest: &mut &[u8]) -> Option<u64> {
            Some(u64::from_le_bytes(split(rest, 8)?.try_into().ok()?))
        }
        let mut rest = record;
        let version = split(&mut rest, 1)?[0];
        if version != RECORD && version != RECORD_1 {
            return None;
        }
        let name = field(&mut rest)?.to_vec();
        let value = field(&mut rest)?.to_vec();
        let domain = String::from_utf8(field(&mut rest)?.to_vec()).ok()?;
        let path = String::from_utf8(field(&mut rest)?.to_vec()).ok()?;
        let partition = match version {
            RECORD_1 => None,
            _ => Some(String::from_utf8(field(&mut rest)?.to_vec()).ok()?)
                .filter(|partition| !partition.is_empty()),
        };
        let flags = split(&mut rest, 1)?[0];
        let same_site = match split(&mut rest, 1)?[0] {
            0 => SameSite::Strict,
            1 => SameSite::Lax,
            2 => SameSite::None,
            _ => return None,
        };
        let seconds = number(&mut rest)?;
        let nanos = u32::from_le_bytes(split(&mut rest, 4)?.try_into().ok()?);
        let created = number(&mut rest)?;
        let accessed = number(&mut rest)?;
        if !rest.is_empty() || flags > 7 || nanos >= 1_000_000_000 {
            return None;
        }
        let cookie = Cookie {
            name,
            value,
            domain,
            host_only: flags & 1 != 0,
            path,
            expires: Some(UNIX_EPOCH.checked_add(Duration::new(seconds, nanos))?),
            secure: flags & 2 != 0,
            http_only: flags & 4 != 0,
            same_site,
            partition,
            created,
            accessed,
        };
        cookie.sound().then_some(cookie)
    }

    /// Whether the cookie is one the storage model could have kept: its
    /// size within the limit, a name or a value, neither with a control
    /// character, a `;` or a space at an edge, no `=` in its name, a
    /// domain, a path from the root, and what `SameSite=None` and the
    /// prefixes ask of it. What a record holds is held to this, the
    /// profile being a file anything may have written.
    fn sound(&self) -> bool {
        let control = |b: &u8| (*b < 0x20 && *b != b'\t') || *b == 0x7F || *b == b';';
        let edge = |bytes: &[u8]| {
            bytes.first().is_some_and(|b| matches!(b, b' ' | b'\t'))
                || bytes.last().is_some_and(|b| matches!(b, b' ' | b'\t'))
        };
        let prefixed = |bytes: &[u8], prefix: &[u8]| {
            bytes.len() >= prefix.len() && bytes[..prefix.len()].eq_ignore_ascii_case(prefix)
        };
        let secure_prefix =
            |bytes: &[u8]| prefixed(bytes, b"__Secure-") || prefixed(bytes, b"__Host-");
        self.name.len() + self.value.len() <= MAX_COOKIE_SIZE
            && !(self.name.is_empty() && self.value.is_empty())
            && [&self.name, &self.value]
                .iter()
                .all(|part| !part.iter().any(control) && !edge(part))
            && !self.name.contains(&b'=')
            && !self.domain.is_empty()
            && self.path.starts_with('/')
            && (self.same_site != SameSite::None || self.secure)
            && (self.partition.is_none() || self.secure)
            && (!prefixed(&self.name, b"__Secure-") || self.secure)
            && (!prefixed(&self.name, b"__Host-")
                || (self.secure && self.host_only && self.path == "/"))
            && (!self.name.is_empty() || !secure_prefix(&self.value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(input: &str) -> Url {
        Url::parse(input).expect(input)
    }

    /// A time well inside the representable range.
    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    const SAME: Context = Context::SAME_SITE;
    const CROSS: Context = Context {
        same_site: false,
        top_level: false,
        safe_method: true,
        partition: None,
    };

    /// The jar after storing each line from `at` over HTTP, same-site.
    fn jar(at: &str, lines: &[&str]) -> Jar {
        let mut jar = Jar::new();
        for line in lines {
            jar.store(line.as_bytes(), &url(at), Api::Http, SAME, now());
        }
        jar
    }

    /// What `jar` sends to `to` over HTTP in `context`.
    fn sent(jar: &mut Jar, to: &str, context: Context) -> String {
        jar.header(&url(to), Api::Http, context, now())
            .map(|bytes| String::from_utf8(bytes).unwrap())
            .unwrap_or_default()
    }

    #[test]
    fn a_set_cookie_value_is_split_trimmed_and_bounded() {
        let mut jar = jar(
            "http://a.example/",
            &[
                " a = 1 ; Path=/",
                "novalue",
                "q=\"quoted\"",
                "sp=v w",
                "tab=a\tb",
                "ctl=a\u{1}b",
                "=",
                "",
            ],
        );
        assert_eq!(
            sent(&mut jar, "http://a.example/", SAME),
            "a=1; novalue; q=\"quoted\"; sp=v w; tab=a\tb"
        );
        // A nameless cookie is replaced by the next.
        let mut jar = jar_with("http://a.example/", &["novalue", "=v2"]);
        assert_eq!(sent(&mut jar, "http://a.example/", SAME), "v2");
        let big = |n: usize| format!("big={}", "x".repeat(n));
        // Name and value of 4,096 bytes together are kept; one more is not.
        let mut jar = jar_with("http://a.example/", &[&big(4092), &big(4093)]);
        assert_eq!(sent(&mut jar, "http://a.example/", SAME).len(), 4097);
        let mut too_big = jar_with("http://a.example/", &[&big(4094)]);
        assert_eq!(sent(&mut too_big, "http://a.example/", SAME), "");
    }

    fn jar_with(at: &str, lines: &[&str]) -> Jar {
        jar(at, lines)
    }

    #[test]
    fn dates_are_read_as_the_cookie_date_algorithm_reads_them() {
        let day = |input: &str| {
            parse_date(input.as_bytes())
                .map(|date| date.duration_since(UNIX_EPOCH).unwrap().as_secs())
        };
        let nov_6_1994 = 784_111_777;
        assert_eq!(day("Sun, 06 Nov 1994 08:49:37 GMT"), Some(nov_6_1994));
        assert_eq!(day("Sunday, 06-Nov-94 08:49:37 GMT"), Some(nov_6_1994));
        assert_eq!(day("Sun Nov  6 08:49:37 1994"), Some(nov_6_1994));
        assert_eq!(day("6 nov 1994 8:49:37"), Some(nov_6_1994));
        assert_eq!(day("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(day("Wed, 01 Jan 2070 00:00:00 GMT"), Some(3_155_760_000));
        assert_eq!(day("01 Jan 69 00:00:00"), day("01 Jan 2069 00:00:00"));
        for bad in [
            "Sun, 30 Feb 2000 08:49:37 GMT",
            "Sun, 06 Nov 1600 08:49:37 GMT",
            "Sun, 06 Nov 1994 24:00:00 GMT",
            "Sun, 06 Nov 1994",
            "06 1994 08:49:37",
            "",
        ] {
            assert_eq!(day(bad), None, "{bad}");
        }
        assert!(parse_date(b"29 Feb 2000 00:00:00").is_some());
        assert!(parse_date(b"29 Feb 1900 00:00:00").is_none());
    }

    #[test]
    fn expires_and_max_age_set_a_life_max_age_winning_and_capped() {
        let at = "http://a.example/";
        let mut jar = jar(
            at,
            &[
                "s=1",
                "e=1; Expires=Wed, 01 Jan 2070 00:00:00 GMT",
                "m=1; Max-Age=60; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
                "far=1; Max-Age=999999999999999999999",
            ],
        );
        let life = |name: &[u8]| {
            jar.cookies()
                .iter()
                .find(|cookie| cookie.name == name)
                .and_then(|cookie| cookie.expires)
                .map(|expires| expires.duration_since(now()).unwrap())
        };
        assert_eq!(life(b"s"), None);
        assert_eq!(life(b"e"), Some(MAX_COOKIE_AGE));
        assert_eq!(life(b"m"), Some(Duration::from_secs(60)));
        assert_eq!(life(b"far"), Some(MAX_COOKIE_AGE));
        // Expired on arrival, a cookie deletes its namesake.
        for gone in [
            "m=; Max-Age=0",
            "e=; Max-Age=-1",
            "s=; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
        ] {
            jar.store(gone.as_bytes(), &url(at), Api::Http, SAME, now());
        }
        assert_eq!(sent(&mut jar, at, SAME), "far=1");
        // Bad attributes are ignored, not the cookie.
        let mut jar = jar_with(at, &["x=1; Max-Age=1.5; Max-Age=; Expires=never; Domain="]);
        assert!(jar.cookies()[0].expires.is_none());
        assert_eq!(sent(&mut jar, at, SAME), "x=1");
    }

    #[test]
    fn domain_and_path_choose_where_a_cookie_goes() {
        let mut jar = jar(
            "http://www.a.example/dir/page",
            &[
                "host=1",
                "dom=1; Domain=.A.example",
                "other=1; Domain=b.example",
                "suffix=1; Domain=example",
                "root=1; Path=/",
                "sub=1; Path=/dir/sub",
                "bad=1; Path=nope",
            ],
        );
        assert_eq!(
            sent(&mut jar, "http://www.a.example/dir/x", SAME),
            "host=1; dom=1; bad=1; root=1"
        );
        assert_eq!(sent(&mut jar, "http://a.example/dir/x", SAME), "dom=1");
        assert_eq!(sent(&mut jar, "http://x.www.a.example/dir", SAME), "dom=1");
        assert_eq!(sent(&mut jar, "http://www.a.example/dirx", SAME), "root=1");
        assert_eq!(
            sent(&mut jar, "http://www.a.example/dir/sub/x", SAME),
            "sub=1; host=1; dom=1; bad=1; root=1"
        );
        assert_eq!(sent(&mut jar, "http://b.example/dir/x", SAME), "");
        // A public suffix as Domain is the host itself, or nothing.
        let mut jar = jar_with("http://co.uk/", &["a=1; Domain=co.uk"]);
        assert!(jar.cookies()[0].host_only);
        assert_eq!(sent(&mut jar, "http://x.co.uk/", SAME), "");
        let mut jar = jar_with("http://x.co.uk/", &["a=1; Domain=co.uk"]);
        assert_eq!(sent(&mut jar, "http://x.co.uk/", SAME), "");
        // An address takes a Domain only equal to itself.
        let mut jar = jar_with(
            "http://127.0.0.1/",
            &["a=1; Domain=127.0.0.1", "b=1; Domain=0.1"],
        );
        assert_eq!(sent(&mut jar, "http://127.0.0.1:8080/", SAME), "a=1");
        assert!(
            domain_match("a.b.example", "b.example") && !domain_match("ab.example", "b.example")
        );
        assert!(!domain_match("1.2.3.4", "2.3.4"));
        assert_eq!(default_path(&url("http://a/b/c/d")), "/b/c");
        assert_eq!(default_path(&url("http://a/b")), "/");
        assert_eq!(default_path(&url("http://a/")), "/");
        assert!(path_match("/a/b", "/a/") && path_match("/a/b", "/a") && !path_match("/ab", "/a"));
    }

    #[test]
    fn the_header_orders_longer_paths_then_older_and_keeps_replacements_in_place() {
        let at = "http://a.example/p/q";
        let mut jar = jar(
            at,
            &["a=1; Path=/", "b=2; Path=/p", "c=3; Path=/", "d=4; Path=/p"],
        );
        assert_eq!(sent(&mut jar, at, SAME), "b=2; d=4; a=1; c=3");
        jar.store(b"a=new; Path=/", &url(at), Api::Http, SAME, now());
        assert_eq!(sent(&mut jar, at, SAME), "b=2; d=4; a=new; c=3");
    }

    #[test]
    fn secure_cookies_need_a_trustworthy_url_and_insecure_ones_leave_them_alone() {
        let mut jar = Jar::new();
        let http = url("http://a.example/");
        let https = url("https://a.example/");
        assert!(!jar.store(b"s=1; Secure", &http, Api::Http, SAME, now()));
        assert!(jar.store(b"s=1; Secure", &https, Api::Http, SAME, now()));
        assert!(jar.store(
            b"s=2; Secure",
            &url("http://localhost/"),
            Api::Http,
            SAME,
            now()
        ));
        assert_eq!(sent(&mut jar, "http://a.example/", SAME), "");
        assert_eq!(sent(&mut jar, "https://a.example/", SAME), "s=1");
        // From http:, no shadowing a secure cookie of the same name.
        assert!(!jar.store(b"s=evil", &http, Api::Http, SAME, now()));
        assert!(!jar.store(
            b"s=evil; Path=/",
            &url("http://x.a.example/"),
            Api::Http,
            SAME,
            now()
        ));
        assert!(jar.store(b"s=ok", &https, Api::Http, SAME, now()));
        assert!(jar.store(b"t=ok", &http, Api::Http, SAME, now()));
    }

    #[test]
    fn prefixes_require_what_they_promise() {
        let at = url("https://a.example/dir/x");
        let mut jar = Jar::new();
        let mut stored = |line: &str| jar.store(line.as_bytes(), &at, Api::Http, SAME, now());
        assert!(stored("__Secure-a=1; Secure"));
        assert!(!stored("__Secure-b=1"));
        assert!(!stored("__secure-c=1"));
        assert!(stored("__Host-d=1; Secure; Path=/"));
        assert!(!stored("__Host-e=1; Secure"));
        assert!(!stored("__Host-f=1; Secure; Path=/; Domain=a.example"));
        assert!(!stored("__HOST-g=1; Path=/"));
        assert!(!stored("=__Host-h"));
        assert!(!stored("=__secure-i"));
    }

    #[test]
    fn same_site_decides_what_goes_and_what_is_kept_across_sites() {
        let at = url("https://b.example/");
        let mut jar = Jar::new();
        for line in [
            "dflt=1",
            "lax=1; SameSite=Lax",
            "strict=1; SameSite=Strict",
            "none=1; SameSite=None; Secure",
            "odd=1; SameSite=odd",
        ] {
            jar.store(line.as_bytes(), &at, Api::Http, SAME, now());
        }
        assert_eq!(
            sent(&mut jar, "https://b.example/", SAME),
            "dflt=1; lax=1; strict=1; none=1; odd=1"
        );
        assert_eq!(sent(&mut jar, "https://b.example/", CROSS), "none=1");
        let navigation = Context {
            same_site: false,
            top_level: true,
            safe_method: true,
            partition: None,
        };
        assert_eq!(
            sent(&mut jar, "https://b.example/", navigation.clone()),
            "dflt=1; lax=1; none=1; odd=1"
        );
        let post = Context {
            safe_method: false,
            ..navigation.clone()
        };
        assert_eq!(sent(&mut jar, "https://b.example/", post), "none=1");
        // None without Secure is not kept; a cross-site response keeps
        // only None, a cross-site navigation's all.
        let mut jar = Jar::new();
        assert!(!jar.store(b"n=1; SameSite=None", &at, Api::Http, SAME, now()));
        assert!(!jar.store(b"c=1", &at, Api::Http, CROSS, now()));
        assert!(jar.store(b"c=1; SameSite=None; Secure", &at, Api::Http, CROSS, now()));
        assert!(jar.store(b"d=1", &at, Api::Http, navigation, now()));
    }

    #[test]
    fn a_partitioned_cookie_is_kept_and_sent_under_its_top_level_site() {
        // CHIPS (M33.2): secure, kept under the context's top-level site,
        // a namesake under another site another cookie.
        let at = url("https://b.example/");
        let under = |site: &str| Context {
            partition: Some(site.to_owned()),
            ..SAME
        };
        let a = under("https://a.example");
        let c = under("https://c.example");
        let mut jar = Jar::new();
        let p1 = b"p=1; Secure; Partitioned; SameSite=None; Max-Age=60";
        assert!(jar.store(p1, &at, Api::Http, a.clone(), now()));
        assert!(!jar.store(b"q=1; Partitioned", &at, Api::Http, a.clone(), now()));
        assert!(!jar.store(b"r=1; Secure; Partitioned", &at, Api::Http, SAME, now()));
        assert!(jar.store(
            b"u=1; Secure; SameSite=None",
            &at,
            Api::Http,
            c.clone(),
            now()
        ));
        let p2 = b"p=2; Secure; Partitioned; SameSite=None";
        assert!(jar.store(p2, &at, Api::Http, c.clone(), now()));
        assert_eq!(sent(&mut jar, "https://b.example/", a.clone()), "p=1; u=1");
        assert_eq!(sent(&mut jar, "https://b.example/", c), "u=1; p=2");
        assert_eq!(sent(&mut jar, "https://b.example/", SAME), "u=1");
        // Its record keeps its partition; one of M28's has none.
        let mut restored = Jar::new();
        restored.restore(&jar.export(), now());
        assert_eq!(sent(&mut restored, "https://b.example/", a.clone()), "p=1");
        assert_eq!(sent(&mut restored, "https://b.example/", SAME), "");
        let plain = jar_with("https://a.example/", &["n=v; Max-Age=60"]).export();
        let mut old = plain[0].clone();
        old[0] = RECORD_1;
        // The fifth field, the partition, empty: its length alone.
        let mut at = 1;
        for _ in 0..4 {
            let length = u32::from_le_bytes(old[at..at + 4].try_into().unwrap()) as usize;
            at += 4 + length;
        }
        old.drain(at..at + 4);
        let decoded = Cookie::decode(&old).expect("M28's record reads");
        assert_eq!(
            (decoded.name.as_slice(), decoded.partition),
            (&b"n"[..], None)
        );
    }

    #[test]
    fn scripts_neither_read_nor_write_http_only_cookies() {
        let at = url("http://a.example/");
        let mut jar = jar("http://a.example/", &["h=1; HttpOnly", "v=1"]);
        let script = |jar: &mut Jar| {
            jar.header(&at, Api::Script, SAME, now())
                .map(|bytes| String::from_utf8(bytes).unwrap())
        };
        assert_eq!(script(&mut jar).as_deref(), Some("v=1"));
        assert!(!jar.store(b"h=2", &at, Api::Script, SAME, now()));
        assert!(!jar.store(b"x=1; HttpOnly", &at, Api::Script, SAME, now()));
        assert!(jar.store(b"v=2", &at, Api::Script, SAME, now()));
        assert_eq!(sent(&mut jar, "http://a.example/", SAME), "h=1; v=2");
    }

    #[test]
    fn the_jar_keeps_within_its_limits_dropping_the_least_used() {
        let at = url("http://a.example/");
        let mut jar = Jar::new();
        for i in 0..MAX_COOKIES_PER_DOMAIN {
            jar.store(format!("c{i}=1").as_bytes(), &at, Api::Http, SAME, now());
        }
        // The first, stored again, outlives the second.
        jar.store(b"c0=2", &at, Api::Http, SAME, now());
        jar.store(b"last=1", &at, Api::Http, SAME, now());
        assert_eq!(jar.cookies().len(), MAX_COOKIES_PER_DOMAIN);
        let names: Vec<&[u8]> = jar
            .cookies()
            .iter()
            .map(|cookie| cookie.name.as_slice())
            .collect();
        assert!(names.contains(&b"c0".as_slice()) && !names.contains(&b"c1".as_slice()));
        let mut jar = Jar::new();
        for host in 0..=MAX_COOKIES / 100 {
            let at = url(&format!("http://h{host}.example/"));
            for i in 0..100 {
                jar.store(format!("c{i}=1").as_bytes(), &at, Api::Http, SAME, now());
            }
        }
        assert_eq!(jar.cookies().len(), MAX_COOKIES);
        assert!(
            !jar.cookies()
                .iter()
                .any(|cookie| cookie.domain == "h0.example")
        );
    }

    #[test]
    fn a_long_attribute_and_a_later_empty_domain_are_ignored() {
        let at = "http://www.a.example/dir/page";
        let long = format!("p=1; Path=/{}", "x".repeat(MAX_ATTRIBUTE_SIZE));
        let edge = format!("q=1; Path=/{}", "x".repeat(MAX_ATTRIBUTE_SIZE - 1));
        let jar = jar(at, &[&long, &edge, "d=1; Domain=a.example; Domain="]);
        let find = |name: &[u8]| {
            jar.cookies()
                .iter()
                .find(|c| c.name == name)
                .cloned()
                .unwrap()
        };
        assert_eq!(find(b"p").path, "/dir");
        assert_eq!(find(b"q").path.len(), MAX_ATTRIBUTE_SIZE);
        let d = find(b"d");
        assert_eq!((d.domain.as_str(), d.host_only), ("a.example", false));
    }

    #[test]
    fn cookies_expire_with_time_and_an_expired_one_evicts_nothing() {
        let at = url("http://a.example/");
        let mut jar = Jar::new();
        jar.store(b"m=1; Max-Age=60", &at, Api::Http, SAME, now());
        jar.store(b"s=1", &at, Api::Http, SAME, now());
        let later = now() + Duration::from_secs(61);
        let header = jar.header(&at, Api::Http, SAME, later);
        assert_eq!(header.as_deref(), Some(&b"s=1"[..]));
        assert_eq!(jar.cookies().len(), 1);
        let mut jar = Jar::new();
        for i in 0..MAX_COOKIES_PER_DOMAIN {
            jar.store(format!("c{i}=1").as_bytes(), &at, Api::Http, SAME, now());
        }
        assert!(jar.store(b"gone=1; Max-Age=0", &at, Api::Http, SAME, now()));
        assert_eq!(jar.cookies().len(), MAX_COOKIES_PER_DOMAIN);
        assert!(jar.cookies().iter().any(|cookie| cookie.name == b"c0"));
    }

    #[test]
    fn a_cookie_sent_is_used_and_outlives_one_that_was_not() {
        let mut jar = Jar::new();
        jar.store(
            b"c0=1; Path=/only",
            &url("http://a.example/only/x"),
            Api::Http,
            SAME,
            now(),
        );
        for i in 1..MAX_COOKIES_PER_DOMAIN {
            jar.store(
                format!("c{i}=1; Path=/other").as_bytes(),
                &url("http://a.example/"),
                Api::Http,
                SAME,
                now(),
            );
        }
        assert_eq!(sent(&mut jar, "http://a.example/only/y", SAME), "c0=1");
        jar.store(
            b"last=1; Path=/other",
            &url("http://a.example/"),
            Api::Http,
            SAME,
            now(),
        );
        let names: Vec<&[u8]> = jar
            .cookies()
            .iter()
            .map(|cookie| cookie.name.as_slice())
            .collect();
        assert!(names.contains(&b"c0".as_slice()) && !names.contains(&b"c1".as_slice()));
    }

    /// What of a cookie the profile keeps, its orders aside.
    fn kept(jar: &Jar) -> Vec<Cookie> {
        jar.cookies()
            .iter()
            .filter(|cookie| cookie.expires.is_some())
            .map(|cookie| Cookie {
                created: 0,
                accessed: 0,
                ..cookie.clone()
            })
            .collect()
    }

    #[test]
    fn the_cookies_with_an_expiry_are_exported_and_restored_in_their_order() {
        let mut jar = jar(
            "http://a.example/p/q",
            &[
                "session=1",
                "late=1; Max-Age=3600",
                "early=1; Expires=Wed, 01 Jan 2031 00:00:00 GMT; HttpOnly",
                "dom=1; Domain=a.example; Max-Age=3600; Path=/p",
                "strict=1; Max-Age=3600; SameSite=Strict",
            ],
        );
        jar.store(
            b"sec=1; Secure; SameSite=None; Max-Age=3600",
            &url("https://a.example/"),
            Api::Http,
            SAME,
            now(),
        );
        let records = jar.export();
        assert_eq!(records.len(), 5);
        let mut restored = Jar::new();
        assert!(!restored.restore(&records, now()));
        assert_eq!(kept(&restored), kept(&jar));
        // Sent in the same order: the order of making is kept.
        jar.store(
            b"session=; Max-Age=0",
            &url("http://a.example/p/q"),
            Api::Http,
            SAME,
            now(),
        );
        for to in ["https://a.example/p/x", "http://a.example/"] {
            assert_eq!(sent(&mut restored, to, CROSS), sent(&mut jar, to, CROSS));
            assert_eq!(sent(&mut restored, to, SAME), sent(&mut jar, to, SAME));
        }
    }

    #[test]
    fn a_restored_jar_drops_the_least_used_first_as_the_jar_did() {
        let mut jar = Jar::new();
        jar.store(
            b"c0=1; Path=/only; Max-Age=3600",
            &url("http://a.example/only/x"),
            Api::Http,
            SAME,
            now(),
        );
        for i in 1..MAX_COOKIES_PER_DOMAIN {
            jar.store(
                format!("c{i}=1; Path=/other; Max-Age=3600").as_bytes(),
                &url("http://a.example/"),
                Api::Http,
                SAME,
                now(),
            );
        }
        assert_eq!(sent(&mut jar, "http://a.example/only/y", SAME), "c0=1");
        let mut restored = Jar::new();
        restored.restore(&jar.export(), now());
        restored.store(b"last=1", &url("http://a.example/"), Api::Http, SAME, now());
        let names: Vec<&[u8]> = restored
            .cookies()
            .iter()
            .map(|cookie| cookie.name.as_slice())
            .collect();
        assert!(names.contains(&b"c0".as_slice()) && !names.contains(&b"c1".as_slice()));
    }

    #[test]
    fn a_restore_drops_what_has_expired_and_what_does_not_read() {
        let jar = jar(
            "http://a.example/",
            &["soon=1; Max-Age=60", "later=1; Max-Age=3600"],
        );
        let records = jar.export();
        let good = records[1].clone();
        for n in 0..good.len() {
            assert!(Cookie::decode(&good[..n]).is_none(), "{n}");
        }
        let mut longer = good.clone();
        longer.push(0);
        let mut version = good.clone();
        version[0] = RECORD + 1;
        let mut flags = good.clone();
        let at = good.len() - 8 * 3 - 4 - 2;
        flags[at] = 8;
        let mut same_site = good.clone();
        same_site[at + 1] = 3;
        for bad in [&longer, &version, &flags, &same_site] {
            assert!(Cookie::decode(bad).is_none());
        }
        let mut all = records.clone();
        all.extend([Vec::new(), longer, version, flags, same_site]);
        let mut restored = Jar::new();
        assert!(restored.restore(&all, now() + Duration::from_secs(120)));
        let names: Vec<&[u8]> = restored
            .cookies()
            .iter()
            .map(|cookie| cookie.name.as_slice())
            .collect();
        assert_eq!(names, [b"later".as_slice()]);
    }

    #[test]
    fn a_restore_holds_records_to_the_storage_models_rules() {
        let good = jar("https://a.example/", &["n=v; Max-Age=3600; Secure"]).cookies()[0].clone();
        let record = |change: fn(&mut Cookie)| {
            let mut cookie = good.clone();
            change(&mut cookie);
            cookie.encode().unwrap()
        };
        let records = vec![
            record(|c| c.value = b"a;b".to_vec()),
            record(|c| c.name = b"a=b".to_vec()),
            record(|c| c.value = b" v".to_vec()),
            record(|c| c.name = vec![b'x', 1]),
            record(|c| {
                c.name.clear();
                c.value.clear();
            }),
            record(|c| c.path = "p".to_owned()),
            record(|c| c.domain.clear()),
            record(|c| {
                c.same_site = SameSite::None;
                c.secure = false;
            }),
            record(|c| {
                c.name = b"__Host-n".to_vec();
                c.path = "/p".to_owned();
            }),
            record(|c| {
                c.name = b"__Secure-n".to_vec();
                c.secure = false;
            }),
            record(|c| {
                c.name.clear();
                c.value = b"__Host-v".to_vec();
            }),
            record(|c| c.value = vec![b'x'; MAX_COOKIE_SIZE]),
        ];
        let mut restored = Jar::new();
        assert!(restored.restore(&records, now()));
        assert!(restored.cookies().is_empty());
        assert!(!restored.restore(&[record(|_| {})], now()));
        // Far more than the limits, as a profile anything wrote may
        // hold: the most recently used of them kept, at once.
        let many: Vec<Vec<u8>> = (0..20_000u64)
            .map(|i| {
                let mut cookie = good.clone();
                cookie.name = format!("c{i}").into_bytes();
                cookie.domain = format!("d{}.example", i % 40);
                cookie.accessed = i;
                cookie.encode().unwrap()
            })
            .collect();
        let mut restored = Jar::new();
        assert!(restored.restore(&many, now()));
        assert_eq!(restored.cookies().len(), MAX_COOKIES);
        let mut kept: Vec<u64> = restored.cookies().iter().map(|c| c.accessed).collect();
        kept.sort_unstable();
        assert_eq!(kept[0], (20_000 - MAX_COOKIES) as u64);
        // One that would live past the limit lives to it.
        let mut restored = Jar::new();
        restored.restore(
            &[record(|c| c.expires = Some(now() + MAX_COOKIE_AGE * 3))],
            now(),
        );
        assert_eq!(restored.cookies()[0].expires, Some(now() + MAX_COOKIE_AGE));
    }

    #[test]
    fn a_restored_cookie_replaces_its_namesake_and_the_jar_says_it_had_its_own() {
        let records = jar("http://a.example/", &["a=old; Max-Age=3600"]).export();
        let mut own = jar("http://a.example/", &["a=new; Max-Age=3600", "s=1"]);
        assert!(own.restore(&records, now()));
        assert_eq!(sent(&mut own, "http://a.example/", SAME), "s=1; a=old");
        // A jar of session cookies alone has nothing of its own to keep.
        let mut session = jar("http://a.example/", &["s=1"]);
        assert!(!session.restore(&records, now()));
        assert_eq!(sent(&mut session, "http://a.example/", SAME), "s=1; a=old");
    }

    #[test]
    fn a_restore_numbers_its_cookies_after_the_jars_own_and_keeps_the_limits() {
        // Records of orders far past the jar's: renumbered in order.
        let mut source = jar(
            "http://a.example/",
            &["a=1; Max-Age=3600", "b=1; Max-Age=3600"],
        );
        sent(&mut source, "http://a.example/", SAME);
        for cookie in &mut source.cookies {
            cookie.created += u64::MAX / 2;
            cookie.accessed = u64::MAX - cookie.accessed;
        }
        let mut own = jar("http://a.example/", &["own=1"]);
        assert!(!own.restore(&source.export(), now()));
        for cookie in own.cookies() {
            assert!(cookie.created < own.next && cookie.accessed < own.next);
        }
        let order = |key: fn(&Cookie) -> u64| {
            let mut cookies = own.cookies().to_vec();
            cookies.sort_by_key(key);
            let names: Vec<Vec<u8>> = cookies.into_iter().map(|c| c.name).collect();
            names
        };
        assert_eq!(
            order(|c| c.created),
            [b"own".to_vec(), b"a".to_vec(), b"b".to_vec()]
        );
        assert_eq!(
            order(|c| c.accessed),
            [b"own".to_vec(), b"b".to_vec(), b"a".to_vec()]
        );
        // More for one domain than it may have: the least used go.
        let full = jar(
            "http://a.example/",
            &(0..MAX_COOKIES_PER_DOMAIN)
                .map(|i| format!("c{i}=1; Max-Age=3600"))
                .collect::<Vec<_>>()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        );
        let one = jar("http://a.example/", &["x=1; Max-Age=3600"]);
        let mut records = full.export();
        records.extend(one.export());
        let mut restored = Jar::new();
        assert!(restored.restore(&records, now()));
        assert_eq!(restored.cookies().len(), MAX_COOKIES_PER_DOMAIN);
        assert!(!restored.cookies().iter().any(|cookie| cookie.name == b"c0"));
    }

    #[test]
    fn kept_changes_moves_with_the_cookies_that_outlive_the_session() {
        let at = url("http://a.example/");
        let mut jar = Jar::new();
        let moved = |jar: &mut Jar, line: &str, when: SystemTime| {
            let before = jar.kept_changes();
            jar.store(line.as_bytes(), &at, Api::Http, SAME, when);
            jar.kept_changes() != before
        };
        assert!(!moved(&mut jar, "s=1", now()));
        assert!(moved(&mut jar, "k=1; Max-Age=60", now()));
        assert!(moved(&mut jar, "k=2; Max-Age=60", now()));
        assert!(!moved(&mut jar, "s=2", now()));
        // A session cookie replacing one with an expiry moves it.
        assert!(moved(&mut jar, "k=3", now()));
        assert!(moved(&mut jar, "e=1; Max-Age=60", now()));
        let before = jar.kept_changes();
        sent(&mut jar, "http://a.example/", SAME);
        assert_eq!(jar.kept_changes(), before);
        // Expired.
        assert!(moved(&mut jar, "s=3", now() + Duration::from_secs(61)));
        assert!(!moved(&mut jar, "s=4", now() + Duration::from_secs(62)));
    }
}
