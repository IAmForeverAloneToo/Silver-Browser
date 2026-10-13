//! The HTTP cache, version 0 (M28.4, docs/design/storage.md): RFC
//! 9111's private cache, in memory, of a client and the clients that
//! share it. It keeps the responses to the `GET`s fetched whole, not a
//! document's, which streams, and gives one back while it is fresh as
//! `Cache-Control`, `Expires`, `Age`, `Date` and `Last-Modified` say,
//! or, for a request's `force-cache` and `only-if-cached` modes, fresh
//! or not. It does not revalidate: what is stale is fetched again
//! (M38). It is partitioned by the site a request is made for, so that
//! no site can learn from the cache what another's pages fetched.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use crate::url::Site;
use crate::{AddressSpace, HeaderList, Url};

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
    /// The cache is not read, a stored response wanting revalidation,
    /// which version 0 does not do; what comes is stored.
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

    /// Whether a stored response may answer, and whether a stale one
    /// may: `None` when the cache is not read.
    pub(crate) fn reads(self) -> Option<bool> {
        match self {
            CacheMode::Default => Some(false),
            CacheMode::ForceCache | CacheMode::OnlyIfCached => Some(true),
            CacheMode::NoStore | CacheMode::Reload | CacheMode::NoCache => None,
        }
    }
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
    /// `no-cache`: never used fresh without revalidating.
    no_cache: bool,
    /// When it was last used, by the cache's count.
    used: u64,
}

impl Entry {
    fn current_age(&self, now: SystemTime) -> Duration {
        self.initial_age + now.duration_since(self.response_time).unwrap_or_default()
    }

    fn fresh(&self, now: SystemTime) -> bool {
        !self.no_cache && self.lifetime > self.current_age(now)
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
    /// fields `request` (RFC 9111 4): one whose `Vary` fields the
    /// request has the same, fresh at `now` or, if `stale`, any; the
    /// latest stored of those.
    pub fn lookup(
        &self,
        site: &Partition,
        url: &Url,
        request: &HeaderList,
        now: SystemTime,
        stale: bool,
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
                && (stale || entry.fresh(now))
        })?;
        entry.used = uses;
        Some(entry.stored.clone())
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
        let entry = Entry {
            vary: vary
                .into_iter()
                .map(|name| {
                    let value = request.get(&name);
                    (name, value)
                })
                .collect(),
            response_time,
            lifetime,
            initial_age: apparent.max(age + delay),
            no_cache: said.has("no-cache"),
            used: 0,
            stored,
        };
        let mut inner = self.lock();
        inner.uses += 1;
        let mut entry = entry;
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
