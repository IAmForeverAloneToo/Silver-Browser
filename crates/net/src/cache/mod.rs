//! The HTTP cache (M28.4, docs/design/storage.md; M38,
//! docs/design/http.md): RFC 9111's private cache, of a client and the
//! clients that share it. It keeps the responses to `GET`s, a
//! document's too, and gives one back while it is fresh as
//! `Cache-Control`, `Expires`, `Age`, `Date` and `Last-Modified` say,
//! and the request's own directives allow; one that is not is
//! revalidated with its validators (`ETag`, `Last-Modified`), a `304`
//! freshening it (RFC 9111 4.3). Fetch's cache modes decide how a
//! request uses it ([`decide`]). It is partitioned by the site a request
//! is made for, so that no site can learn from the cache what another's
//! pages fetched.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use crate::url::Site;
use crate::{AddressSpace, HeaderList, Url};

#[cfg(test)]
mod revalidation_tests;
#[cfg(test)]
mod tests;

/// The most the cache's bodies hold in all: past it, the least
/// recently used entries go first.
pub const MAX_CACHE_BYTES: usize = 64 << 20;

/// The largest body the cache keeps.
pub const MAX_CACHE_ENTRY_BYTES: usize = 8 << 20;

/// The most entries the cache keeps.
pub const MAX_CACHE_ENTRIES: usize = 10_000;

/// A request's cache mode (Fetch 2.2.5): how it uses the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheMode {
    /// A fresh response stored is used; what comes is stored.
    #[default]
    Default,
    /// The cache is neither read nor written.
    NoStore,
    /// The cache is not read; what comes is stored.
    Reload,
    /// A stored response is revalidated before it is used (M38); what
    /// comes is stored.
    NoCache,
    /// A response stored is used, fresh or stale; what comes is stored.
    ForceCache,
    /// A response stored is used, fresh or stale; with none, the fetch
    /// is a network error.
    OnlyIfCached,
}

impl CacheMode {
    /// The mode by its name in Fetch (`default`, `no-store`, ...).
    pub fn parse(name: &str) -> Option<CacheMode> {
        Some(match name {
            "default" => CacheMode::Default,
            "no-store" => CacheMode::NoStore,
            "reload" => CacheMode::Reload,
            "no-cache" => CacheMode::NoCache,
            "force-cache" => CacheMode::ForceCache,
            "only-if-cached" => CacheMode::OnlyIfCached,
            _ => return None,
        })
    }

    /// Whether the cache is read at all.
    pub(crate) fn reads(self) -> bool {
        !matches!(self, CacheMode::NoStore | CacheMode::Reload)
    }
}

/// What a request does with what the cache holds for it (Fetch's
/// HTTP-network-or-cache fetch, M38).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The stored response answers as it is.
    Use(Stored),
    /// The request goes out conditional on these validators; a `304`
    /// lets the stored response answer.
    Validate {
        etag: Option<Vec<u8>>,
        last_modified: Option<Vec<u8>>,
    },
    /// The request goes out as it is.
    Network,
    /// An `only-if-cached` request with nothing stored.
    NotCached,
}

/// What a request in `mode` does with `hit`, what the cache found: a
/// fresh response answers; a stale one the request accepts answers;
/// else it is revalidated if it can be. `no-cache` revalidates even a
/// fresh one but one `immutable` (RFC 8246: a reload need not ask about
/// it); `force-cache` and `only-if-cached` take anything.
pub fn decide(mode: CacheMode, hit: Option<Hit>) -> Decision {
    let Some(hit) = hit else {
        return match mode {
            CacheMode::OnlyIfCached => Decision::NotCached,
            _ => Decision::Network,
        };
    };
    let validate = |hit: Hit| match (hit.etag, hit.last_modified) {
        (None, None) => Decision::Network,
        (etag, last_modified) => Decision::Validate {
            etag,
            last_modified,
        },
    };
    match mode {
        CacheMode::NoStore | CacheMode::Reload => Decision::Network,
        CacheMode::ForceCache | CacheMode::OnlyIfCached => Decision::Use(hit.stored),
        CacheMode::Default if hit.fresh || hit.stale_ok => Decision::Use(hit.stored),
        CacheMode::NoCache if hit.fresh && hit.immutable => Decision::Use(hit.stored),
        CacheMode::Default | CacheMode::NoCache => validate(hit),
    }
}

/// A stored response a request found, and what may be done with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The response, its `Age` what it is now (RFC 9111 5.1).
    pub stored: Stored,
    /// Whether it is fresh, by its own lifetime and the request's
    /// directives.
    pub fresh: bool,
    /// Whether, stale, the request takes it all the same (its
    /// `max-stale`), its server not having asked otherwise
    /// (`must-revalidate`).
    pub stale_ok: bool,
    /// Its validators.
    pub etag: Option<Vec<u8>>,
    pub last_modified: Option<Vec<u8>>,
    /// `immutable` (RFC 8246).
    pub immutable: bool,
}

/// A field's cache directives (RFC 9111 5.2), their names lowercased,
/// a quoted value unquoted.
#[derive(Debug, Default)]
struct Directives(Vec<(String, Option<String>)>);

impl Directives {
    fn of(headers: &HeaderList, name: &[u8]) -> Directives {
        let Some(value) = headers.get(name) else {
            return Directives::default();
        };
        let text = String::from_utf8_lossy(&value).into_owned();
        let mut out = Vec::new();
        let mut rest = text.as_str();
        while !rest.is_empty() {
            // A directive runs to the next comma outside quotes.
            let mut quoted = false;
            let mut escaped = false;
            let mut end = rest.len();
            for (at, c) in rest.char_indices() {
                if escaped {
                    escaped = false;
                } else if quoted && c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    quoted = !quoted;
                } else if c == ',' && !quoted {
                    end = at;
                    break;
                }
            }
            let (one, left) = rest.split_at(end);
            rest = left.strip_prefix(',').unwrap_or(left);
            let one = one.trim();
            if one.is_empty() {
                continue;
            }
            let (name, value) = match one.split_once('=') {
                Some((name, value)) => {
                    let value = value.trim();
                    let value = match value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                        Some(inner) => inner.replace('\\', ""),
                        None => value.to_owned(),
                    };
                    (name.trim(), Some(value))
                }
                None => (one, None),
            };
            out.push((name.to_ascii_lowercase(), value));
        }
        Directives(out)
    }

    fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(directive, _)| directive == name)
    }

    /// A directive's delta-seconds (RFC 9111 1.2.2): its first, digits
    /// alone, past what a `u32` holds taken as the most it holds.
    fn seconds(&self, name: &str) -> Option<u64> {
        let (_, value) = self.0.iter().find(|(directive, _)| directive == name)?;
        delta_seconds(value.as_deref()?)
    }
}

/// Delta-seconds: digits alone, at most 2^31, as RFC 9111 1.2.2 has a
/// larger number taken.
fn delta_seconds(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(text.parse::<u64>().unwrap_or(u64::MAX).min(1 << 31))
}

/// A field's HTTP-date (RFC 9110 5.6.7), read as the cookie date
/// algorithm reads one, which takes every form it allows.
fn http_date(headers: &HeaderList, name: &[u8]) -> Option<SystemTime> {
    crate::cookies::parse_date(&headers.get(name)?)
}

/// The statuses whose responses are heuristically cacheable (RFC 9110
/// 15.1).
fn heuristic(status: u16) -> bool {
    matches!(
        status,
        200 | 203 | 204 | 206 | 300 | 301 | 308 | 404 | 405 | 410 | 414 | 501
    )
}

/// The part of a response the cache keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub status: u16,
    pub status_message: Vec<u8>,
    pub headers: HeaderList,
    pub body: Vec<u8>,
    pub address_space: Option<AddressSpace>,
}

#[derive(Debug)]
struct Entry {
    stored: Stored,
    /// The values the request had of the fields its `Vary` names.
    vary: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    /// When it came, by the cache's clock.
    response_time: SystemTime,
    /// How long it is fresh, and how old it was when it came (RFC 9111
    /// 4.2.1 and 4.2.3).
    lifetime: Duration,
    initial_age: Duration,
    /// `no-cache`: never used fresh without revalidating;
    /// `must-revalidate`: never used stale without; `immutable`.
    no_cache: bool,
    must_revalidate: bool,
    immutable: bool,
    /// When it was last used, by the cache's count.
    used: u64,
}

/// A stored response's times and its server's directives, from its
/// header fields: what [`Entry`] keeps besides the response.
struct Timing {
    response_time: SystemTime,
    lifetime: Duration,
    initial_age: Duration,
    no_cache: bool,
    must_revalidate: bool,
    immutable: bool,
}

/// `stored`'s timing, sent at `request_time` and come at
/// `response_time` (RFC 9111 4.2).
fn timing(stored: &Stored, request_time: SystemTime, response_time: SystemTime) -> Timing {
    let said = Directives::of(&stored.headers, b"cache-control");
    let headers = &stored.headers;
    let date = http_date(headers, b"date").unwrap_or(response_time);
    let lifetime = if let Some(seconds) = said.seconds("max-age") {
        Duration::from_secs(seconds)
    } else if headers.contains(b"expires") {
        http_date(headers, b"expires")
            .and_then(|expires| expires.duration_since(date).ok())
            .unwrap_or_default()
    } else if heuristic(stored.status) || said.has("public") {
        // 10% of the time since it was last modified, as the
        // reference browser's heuristic.
        http_date(headers, b"last-modified")
            .and_then(|modified| date.duration_since(modified).ok())
            .map_or(Duration::ZERO, |since| since / 10)
    } else {
        Duration::ZERO
    };
    let age = headers
        .get(b"age")
        .and_then(|value| delta_seconds(String::from_utf8_lossy(&value).trim()))
        .map_or(Duration::ZERO, Duration::from_secs);
    let apparent = response_time.duration_since(date).unwrap_or_default();
    let delay = response_time
        .duration_since(request_time)
        .unwrap_or_default();
    Timing {
        response_time,
        lifetime,
        initial_age: apparent.max(age + delay),
        no_cache: said.has("no-cache"),
        must_revalidate: said.has("must-revalidate") || said.has("proxy-revalidate"),
        immutable: said.has("immutable"),
    }
}

/// The fields a `304` does not change in what is stored (RFC 9111 3.2):
/// those that frame the stored body.
const KEPT_BY_304: [&[u8]; 4] = [
    b"content-length",
    b"content-encoding",
    b"transfer-encoding",
    b"content-range",
];

impl Entry {
    fn current_age(&self, now: SystemTime) -> Duration {
        self.initial_age + now.duration_since(self.response_time).unwrap_or_default()
    }

    fn set_timing(&mut self, timing: Timing) {
        self.response_time = timing.response_time;
        self.lifetime = timing.lifetime;
        self.initial_age = timing.initial_age;
        self.no_cache = timing.no_cache;
        self.must_revalidate = timing.must_revalidate;
        self.immutable = timing.immutable;
    }

    /// What a request with the fields `request` may do with the entry at
    /// `now`: whether it is fresh, by its lifetime and the request's
    /// `max-age`, `min-fresh` and `no-cache` (RFC 9111 5.2.1), and
    /// whether, stale, the request's `max-stale` takes it.
    fn hit(&self, request: &HeaderList, now: SystemTime) -> Hit {
        let asked = Directives::of(request, b"cache-control");
        let age = self.current_age(now);
        let left = self.lifetime.saturating_sub(age);
        let fresh = !self.no_cache
            && !asked.has("no-cache")
            && self.lifetime > age
            && asked
                .seconds("max-age")
                .is_none_or(|most| age <= Duration::from_secs(most))
            && asked
                .seconds("min-fresh")
                .is_none_or(|least| left >= Duration::from_secs(least));
        let stale_ok = !fresh
            && !self.no_cache
            && !self.must_revalidate
            && asked.has("max-stale")
            && asked
                .seconds("max-stale")
                .is_none_or(|most| age.saturating_sub(self.lifetime) <= Duration::from_secs(most));
        let mut stored = self.stored.clone();
        stored
            .headers
            .set(b"age", age.as_secs().to_string().as_bytes());
        Hit {
            stored,
            fresh,
            stale_ok,
            etag: self.stored.headers.get(b"etag"),
            last_modified: self.stored.headers.get(b"last-modified"),
            immutable: self.immutable,
        }
    }

    fn bytes(&self) -> usize {
        self.stored.body.len()
    }
}

/// What the cache keeps a response under besides its URL (M33.2): the
/// top-level site a request is made for and the site of the document
/// asking (the reference browser's "triple key"), the same site twice
/// for a top-level document's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Partition {
    pub top: Site,
    pub frame: Site,
}

impl From<Site> for Partition {
    fn from(site: Site) -> Partition {
        Partition {
            top: site.clone(),
            frame: site,
        }
    }
}

/// The cache: entries by partition and URL, each URL's variants.
#[derive(Debug, Default)]
pub struct Cache {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    entries: HashMap<(Partition, String), Vec<Entry>>,
    /// The bytes of the bodies, and the entries, in all.
    bytes: usize,
    count: usize,
    /// The count of uses, for which was used last.
    uses: u64,
}

/// The cache's key of `url`: without its fragment.
fn key(site: &Partition, url: &Url) -> (Partition, String) {
    let text = url.as_str();
    let text = text.split_once('#').map_or(text, |(before, _)| before);
    (site.clone(), text.to_owned())
}

impl Cache {
    pub fn new() -> Cache {
        Cache::default()
    }

    /// How many entries it keeps, and their bodies' bytes.
    pub fn size(&self) -> (usize, usize) {
        let inner = self.lock();
        (inner.count, inner.bytes)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The response stored for a `GET` of `url` for `site` with the
    /// fields `request` (RFC 9111 4): the latest stored whose `Vary`
    /// fields the request has the same, and what the request may do with
    /// it at `now`.
    pub fn lookup(
        &self,
        site: &Partition,
        url: &Url,
        request: &HeaderList,
        now: SystemTime,
    ) -> Option<Hit> {
        let mut inner = self.lock();
        inner.uses += 1;
        let uses = inner.uses;
        let variants = inner.entries.get_mut(&key(site, url))?;
        let entry = variants.iter_mut().rev().find(|entry| {
            entry
                .vary
                .iter()
                .all(|(name, value)| request.get(name) == *value)
        })?;
        entry.used = uses;
        Some(entry.hit(request, now))
    }

    /// Freshens what is stored for a `GET` of `url` for `site` with the
    /// fields `request` by `not_modified`'s fields, a `304`'s, sent at
    /// `request_time` and come at `response_time` (RFC 9111 4.3.4): its
    /// fields replaced by the `304`'s but those that frame its body, its
    /// times taken anew. The response it now is, if one was stored.
    pub fn freshen(
        &self,
        site: &Partition,
        url: &Url,
        request: &HeaderList,
        not_modified: &HeaderList,
        request_time: SystemTime,
        response_time: SystemTime,
    ) -> Option<Stored> {
        let mut inner = self.lock();
        inner.uses += 1;
        let uses = inner.uses;
        let variants = inner.entries.get_mut(&key(site, url))?;
        let entry = variants.iter_mut().rev().find(|entry| {
            entry
                .vary
                .iter()
                .all(|(name, value)| request.get(name) == *value)
        })?;
        let mut names: Vec<Vec<u8>> = Vec::new();
        for (name, _) in not_modified.iter() {
            let name = name.to_ascii_lowercase();
            if !KEPT_BY_304.contains(&name.as_slice()) && !names.contains(&name) {
                names.push(name);
            }
        }
        for name in names {
            entry.stored.headers.delete(&name);
            for (other, value) in not_modified.iter() {
                if other.eq_ignore_ascii_case(&name) {
                    entry.stored.headers.append(&name, value);
                }
            }
        }
        let timing = timing(&entry.stored, request_time, response_time);
        entry.set_timing(timing);
        entry.used = uses;
        Some(entry.hit(request, response_time).stored)
    }

    /// Stores the response `stored` to a `GET` of `url` for `site` with
    /// the fields `request`, sent at `request_time` and come at
    /// `response_time`, if it may be (RFC 9111 3): a final status but
    /// 206, no `no-store` from either side, no `Vary: *`, something
    /// that makes it storable, and a body within
    /// [`MAX_CACHE_ENTRY_BYTES`]. It replaces a variant with the same
    /// `Vary` values; the least recently used go when the cache is
    /// past its bounds.
    pub fn store(
        &self,
        site: &Partition,
        url: &Url,
        request: &HeaderList,
        stored: Stored,
        request_time: SystemTime,
        response_time: SystemTime,
    ) {
        let asked = Directives::of(request, b"cache-control");
        let said = Directives::of(&stored.headers, b"cache-control");
        let vary: Vec<Vec<u8>> = stored
            .headers
            .get_decode_split(b"vary")
            .unwrap_or_default()
            .into_iter()
            .map(|name| name.trim().to_ascii_lowercase().into_bytes())
            .filter(|name| !name.is_empty())
            .collect();
        let storable = said.has("public")
            || said.has("private")
            || said.has("max-age")
            || stored.headers.contains(b"expires")
            || heuristic(stored.status);
        if stored.status < 200
            || stored.status == 206
            || asked.has("no-store")
            || said.has("no-store")
            || vary.iter().any(|name| name == b"*")
            || !storable
            || stored.body.len() > MAX_CACHE_ENTRY_BYTES
        {
            return;
        }
        let timing = timing(&stored, request_time, response_time);
        let mut entry = Entry {
            vary: vary
                .into_iter()
                .map(|name| {
                    let value = request.get(&name);
                    (name, value)
                })
                .collect(),
            response_time,
            lifetime: Duration::ZERO,
            initial_age: Duration::ZERO,
            no_cache: false,
            must_revalidate: false,
            immutable: false,
            used: 0,
            stored,
        };
        entry.set_timing(timing);
        let mut inner = self.lock();
        inner.uses += 1;
        entry.used = inner.uses;
        let bytes = entry.bytes();
        let variants = inner.entries.entry(key(site, url)).or_default();
        let same = variants.iter().position(|old| old.vary == entry.vary);
        let replaced = same.map(|index| variants.remove(index));
        variants.push(entry);
        if let Some(old) = replaced {
            inner.bytes -= old.bytes();
            inner.count -= 1;
        }
        inner.bytes += bytes;
        inner.count += 1;
        inner.evict();
    }

    /// Removes what is stored for `url` for `site` (RFC 9111 4.4): an
    /// unsafe method's response that is not an error came.
    pub fn invalidate(&self, site: &Partition, url: &Url) {
        let mut inner = self.lock();
        if let Some(variants) = inner.entries.remove(&key(site, url)) {
            inner.count -= variants.len();
            inner.bytes -= variants.iter().map(Entry::bytes).sum::<usize>();
        }
    }
}

impl Inner {
    /// Takes out the least recently used entries until the cache is
    /// within [`MAX_CACHE_BYTES`] and [`MAX_CACHE_ENTRIES`].
    fn evict(&mut self) {
        while self.bytes > MAX_CACHE_BYTES || self.count > MAX_CACHE_ENTRIES {
            let Some((key, index)) = self
                .entries
                .iter()
                .flat_map(|(key, variants)| {
                    variants
                        .iter()
                        .enumerate()
                        .map(move |(index, entry)| (entry.used, key, index))
                })
                .min_by_key(|(used, _, _)| *used)
                .map(|(_, key, index)| (key.clone(), index))
            else {
                return;
            };
            let Some(variants) = self.entries.get_mut(&key) else {
                return;
            };
            let entry = variants.remove(index);
            if variants.is_empty() {
                self.entries.remove(&key);
            }
            self.bytes -= entry.bytes();
            self.count -= 1;
        }
    }
}
