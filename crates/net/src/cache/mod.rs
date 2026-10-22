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
//! pages fetched. Given a profile it keeps its entries there too (M38):
//! their records read when it is given, their bodies at first use.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use crate::url::Site;
use crate::{AddressSpace, HeaderList, Url};

#[cfg(test)]
mod persistence_tests;
mod record;
#[cfg(test)]
mod revalidation_tests;
#[cfg(test)]
mod tests;

/// The most the cache's bodies hold in memory: past it, the least
/// recently used go first, to the profile if it keeps them.
pub const MAX_CACHE_BYTES: usize = 64 << 20;

/// The most the bodies the profile keeps hold (M38): past it, the least
/// recently used entries go first.
pub const MAX_PROFILE_CACHE_BYTES: usize = 256 << 20;

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
    /// Its number, the profile's key of it (M38); its body's length;
    /// whether the body is in memory; whether the profile keeps it.
    id: u64,
    body_len: usize,
    in_memory: bool,
    persisted: bool,
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

    /// Its body's bytes in memory.
    fn bytes(&self) -> usize {
        if self.in_memory { self.body_len } else { 0 }
    }

    /// Its body's bytes in the profile.
    fn profile_bytes(&self) -> usize {
        if self.persisted { self.body_len } else { 0 }
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

/// How the cache reads the body the profile keeps of the entry it
/// numbers so (M38), if it is there.
pub type BodyReader = Arc<dyn Fn(u64) -> Option<Vec<u8>> + Send + Sync>;

/// A change the profile is to be given (M38), by the entry's number: its
/// record, with its body when that is new; or the entry gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Put {
        id: u64,
        record: Vec<u8>,
        body: Option<Vec<u8>>,
    },
    Delete {
        id: u64,
    },
}

/// The cache: entries by partition and URL, each URL's variants.
#[derive(Debug, Default)]
pub struct Cache {
    inner: Mutex<Inner>,
}

/// What of an entry the profile has not been given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dirty {
    /// Its record.
    Record,
    /// Its record and its body.
    Body,
    /// That it is gone.
    Gone,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<(Partition, String), Vec<Entry>>,
    /// The bytes of the bodies in memory, of those the profile keeps,
    /// and the entries, in all.
    bytes: usize,
    profile_bytes: usize,
    count: usize,
    /// The count of uses, for which was used last.
    uses: u64,
    /// The next entry's number.
    next_id: u64,
    /// How bodies the profile keeps are read, once the cache keeps its
    /// entries in one (M38).
    reader: Option<BodyReader>,
    /// What changed that the profile has not been given.
    dirty: BTreeMap<u64, Dirty>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("count", &self.count)
            .field("bytes", &self.bytes)
            .field("profile_bytes", &self.profile_bytes)
            .finish_non_exhaustive()
    }
}

/// The cache's key of `url`: without its fragment.
fn key(site: &Partition, url: &Url) -> (Partition, String) {
    let text = url.as_str();
    let text = text.split_once('#').map_or(text, |(before, _)| before);
    (site.clone(), text.to_owned())
}

/// Whether `entry`, kept under `key`, is worth keeping in the profile:
/// not an opaque site's, and of use to a later browsing without asking
/// the network, fresh for a while or with a validator; one that is
/// neither is used again only by a traversal, and kept in memory.
fn keepable(key: &(Partition, String), entry: &Entry) -> bool {
    !matches!(key.0.top, Site::Opaque(_))
        && !matches!(key.0.frame, Site::Opaque(_))
        && (!entry.lifetime.is_zero()
            || entry.stored.headers.contains(b"etag")
            || entry.stored.headers.contains(b"last-modified"))
}

impl Cache {
    pub fn new() -> Cache {
        Cache::default()
    }

    /// How many entries it keeps, and their bodies' bytes in memory.
    pub fn size(&self) -> (usize, usize) {
        let inner = self.lock();
        (inner.count, inner.bytes)
    }

    /// The bytes of the bodies it keeps in the profile.
    pub fn profile_bytes(&self) -> usize {
        self.lock().profile_bytes
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps the entries in a profile from now on (M38): takes in those
    /// it kept, `records` by their numbers, their bodies to be read by
    /// `reader` at their first use, and gives it those stored so far.
    /// A record that does not read is deleted.
    pub fn load(&self, records: Vec<(u64, Vec<u8>)>, reader: BodyReader) {
        let mut inner = self.lock();
        inner.dirty.clear();
        inner.reader = Some(reader);
        let top = records.iter().map(|(id, _)| id + 1).max().unwrap_or(0);
        let mut read = Vec::new();
        for (id, bytes) in records {
            match record::decode(id, &bytes) {
                Some(record) => read.push(record),
                None => {
                    inner.dirty.insert(id, Dirty::Gone);
                }
            }
        }
        let base = read.iter().map(|r| r.entry.used).max().unwrap_or(0);
        // What this browsing stored: numbered after the profile's, used
        // after them, and written; a body another profile kept is gone.
        let mut next_id = top.max(inner.next_id);
        let mut dirty = Vec::new();
        for (key, variants) in &mut inner.entries {
            variants.retain(|entry| entry.in_memory);
            for entry in variants {
                entry.id = next_id;
                next_id += 1;
                entry.used += base;
                entry.persisted = keepable(key, entry);
                if entry.persisted {
                    dirty.push(entry.id);
                }
            }
        }
        inner.entries.retain(|_, variants| !variants.is_empty());
        inner.next_id = next_id;
        inner.uses += base;
        for id in dirty {
            inner.dirty.insert(id, Dirty::Body);
        }
        for record::Record { key, entry } in read {
            let variants = inner.entries.entry(key).or_default();
            if variants.iter().any(|other| other.vary == entry.vary) {
                inner.dirty.insert(entry.id, Dirty::Gone);
            } else {
                variants.push(entry);
            }
        }
        for variants in inner.entries.values_mut() {
            variants.sort_by_key(|entry| entry.response_time);
        }
        let all = inner.entries.values().flatten();
        let (count, bytes, profile_bytes) = all.fold((0, 0, 0), |(n, m, p), entry| {
            (n + 1, m + entry.bytes(), p + entry.profile_bytes())
        });
        inner.count = count;
        inner.bytes = bytes;
        inner.profile_bytes = profile_bytes;
        inner.evict();
    }

    /// Whether there are changes the profile has not been given.
    pub fn has_changes(&self) -> bool {
        !self.lock().dirty.is_empty()
    }

    /// The changes the profile has not been given, taken: the bodies
    /// they hold may leave memory after.
    pub fn take_changes(&self) -> Vec<Change> {
        let mut inner = self.lock();
        let dirty = std::mem::take(&mut inner.dirty);
        let mut changes = Vec::with_capacity(dirty.len());
        {
            let mut found = HashMap::new();
            for (key, variants) in &inner.entries {
                for entry in variants {
                    if dirty.contains_key(&entry.id) {
                        found.insert(entry.id, (key, entry));
                    }
                }
            }
            for (id, what) in dirty {
                let put =
                    found
                        .get(&id)
                        .filter(|_| what != Dirty::Gone)
                        .and_then(|(key, entry)| {
                            let record = record::encode(key, entry)?;
                            let body = (what == Dirty::Body && entry.in_memory)
                                .then(|| entry.stored.body.clone());
                            Some(Change::Put { id, record, body })
                        });
                changes.push(put.unwrap_or(Change::Delete { id }));
            }
        }
        inner.evict();
        changes
    }

    /// The cache locked, and the number of the entry a request with the
    /// fields `request` finds under `key`, its body in memory: read from
    /// the profile, the cache not locked meanwhile, if it was not. `None`
    /// if none is found, or its body does not read (the entry then
    /// gone).
    fn loaded(
        &self,
        key: &(Partition, String),
        request: &HeaderList,
    ) -> Option<(std::sync::MutexGuard<'_, Inner>, u64)> {
        let inner = self.lock();
        let index = inner.find(key, request)?;
        let entry = &inner.entries.get(key)?[index];
        let id = entry.id;
        if entry.in_memory {
            return Some((inner, id));
        }
        let reader = inner.reader.clone();
        drop(inner);
        let body = reader.and_then(|read| read(id));
        let mut inner = self.lock();
        let index = inner.position(key, id)?;
        inner.uses += 1;
        let uses = inner.uses;
        let entry = &mut inner.entries.get_mut(key)?[index];
        if entry.in_memory {
            return Some((inner, id));
        }
        match body.filter(|body| body.len() == entry.body_len) {
            Some(body) => {
                entry.stored.body = body;
                entry.in_memory = true;
                entry.used = uses;
                inner.bytes += entry.body_len;
                inner.evict();
                inner.position(key, id)?;
                Some((inner, id))
            }
            None => {
                inner.remove_at(key, index);
                None
            }
        }
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
        let key = key(site, url);
        let (mut inner, id) = self.loaded(&key, request)?;
        inner.uses += 1;
        let uses = inner.uses;
        let index = inner.position(&key, id)?;
        let entry = &mut inner.entries.get_mut(&key)?[index];
        entry.used = uses;
        Some(entry.hit(request, now))
    }

    /// Whether a response stored for `url` for `site`, any of its
    /// variants, is fresh at `now` by its own lifetime (M38): what a
    /// preload need not ask the network for. Nothing is read or cloned.
    pub fn fresh(&self, site: &Partition, url: &Url, now: SystemTime) -> bool {
        let inner = self.lock();
        inner.entries.get(&key(site, url)).is_some_and(|variants| {
            variants
                .iter()
                .any(|entry| !entry.no_cache && entry.lifetime > entry.current_age(now))
        })
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
        let key = key(site, url);
        let (mut inner, id) = self.loaded(&key, request)?;
        inner.uses += 1;
        let uses = inner.uses;
        let index = inner.position(&key, id)?;
        let entry = &mut inner.entries.get_mut(&key)?[index];
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
        let stored = entry.hit(request, response_time).stored;
        let persisted = entry.persisted;
        if persisted {
            inner.mark(id, Dirty::Record);
        }
        Some(stored)
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
        let key = key(site, url);
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
            id: 0,
            body_len: stored.body.len(),
            in_memory: true,
            persisted: false,
            stored,
        };
        entry.set_timing(timing);
        let mut inner = self.lock();
        inner.uses += 1;
        entry.used = inner.uses;
        entry.id = inner.next_id;
        inner.next_id += 1;
        entry.persisted = inner.reader.is_some() && keepable(&key, &entry);
        let same = inner
            .entries
            .get(&key)
            .and_then(|variants| variants.iter().position(|old| old.vary == entry.vary));
        if let Some(index) = same {
            inner.remove_at(&key, index);
        }
        inner.add(key, entry);
        inner.evict();
    }

    /// Removes what is stored for `url` for `site` (RFC 9111 4.4): an
    /// unsafe method's response that is not an error came.
    pub fn invalidate(&self, site: &Partition, url: &Url) {
        let mut inner = self.lock();
        let key = key(site, url);
        while inner.entries.contains_key(&key) {
            inner.remove_at(&key, 0);
        }
    }

    /// Removes what is stored for `site`, at the top or in a frame
    /// (M38): the user cleared its data.
    pub fn clear_site(&self, site: &Site) {
        let mut inner = self.lock();
        let keys: Vec<_> = inner
            .entries
            .keys()
            .filter(|(partition, _)| partition.top == *site || partition.frame == *site)
            .cloned()
            .collect();
        for key in keys {
            while inner.entries.contains_key(&key) {
                inner.remove_at(&key, 0);
            }
        }
    }
}

/// For fuzzing (M38): reads `data` as a record, which, if it reads,
/// writes back as it was.
#[doc(hidden)]
pub fn fuzz_record(data: &[u8]) {
    if let Some(read) = record::decode(1, data) {
        assert_eq!(
            record::encode(&read.key, &read.entry).as_deref(),
            Some(data)
        );
    }
}

impl Inner {
    /// The index of the latest variant under `key` whose `Vary` fields
    /// `request` has the same.
    fn find(&self, key: &(Partition, String), request: &HeaderList) -> Option<usize> {
        self.entries.get(key)?.iter().rposition(|entry| {
            entry
                .vary
                .iter()
                .all(|(name, value)| request.get(name) == *value)
        })
    }

    /// The index of the entry numbered `id` under `key`.
    fn position(&self, key: &(Partition, String), id: u64) -> Option<usize> {
        self.entries
            .get(key)?
            .iter()
            .position(|entry| entry.id == id)
    }

    /// Notes that the profile is to be given `what` of entry `id`.
    fn mark(&mut self, id: u64, what: Dirty) {
        let now = self.dirty.entry(id).or_insert(what);
        if what != Dirty::Record {
            *now = what;
        }
    }

    fn add(&mut self, key: (Partition, String), entry: Entry) {
        self.bytes += entry.bytes();
        self.profile_bytes += entry.profile_bytes();
        self.count += 1;
        if entry.persisted {
            self.mark(entry.id, Dirty::Body);
        }
        self.entries.entry(key).or_default().push(entry);
    }

    fn remove_at(&mut self, key: &(Partition, String), index: usize) {
        let Some(variants) = self.entries.get_mut(key) else {
            return;
        };
        let entry = variants.remove(index);
        if variants.is_empty() {
            self.entries.remove(key);
        }
        self.bytes -= entry.bytes();
        self.profile_bytes -= entry.profile_bytes();
        self.count -= 1;
        if entry.persisted {
            self.mark(entry.id, Dirty::Gone);
        }
    }

    /// The key and index of the least recently used entry `which` takes.
    fn least_used(&self, which: impl Fn(&Entry) -> bool) -> Option<((Partition, String), usize)> {
        self.entries
            .iter()
            .flat_map(|(key, variants)| {
                variants
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| which(entry))
                    .map(move |(index, entry)| (entry.used, key, index))
            })
            .min_by_key(|(used, _, _)| *used)
            .map(|(_, key, index)| (key.clone(), index))
    }

    /// Takes out the least recently used entries until the cache is
    /// within [`MAX_CACHE_ENTRIES`] and [`MAX_PROFILE_CACHE_BYTES`], and
    /// their bodies from memory until it is within [`MAX_CACHE_BYTES`]:
    /// an entry the profile keeps keeps its record, its body read again
    /// at its next use, unless the profile has not had it yet.
    fn evict(&mut self) {
        while self.count > MAX_CACHE_ENTRIES {
            let Some((key, index)) = self.least_used(|_| true) else {
                return;
            };
            self.remove_at(&key, index);
        }
        while self.profile_bytes > MAX_PROFILE_CACHE_BYTES {
            let Some((key, index)) = self.least_used(|entry| entry.persisted) else {
                return;
            };
            self.remove_at(&key, index);
        }
        while self.bytes > MAX_CACHE_BYTES {
            let dirty = &self.dirty;
            let Some((key, index)) = self.least_used(|entry| {
                entry.in_memory
                    && entry.body_len > 0
                    && !(entry.persisted && dirty.get(&entry.id) == Some(&Dirty::Body))
            }) else {
                return;
            };
            let Some(entry) = self
                .entries
                .get_mut(&key)
                .and_then(|variants| variants.get_mut(index))
            else {
                return;
            };
            if entry.persisted {
                entry.stored.body = Vec::new();
                entry.in_memory = false;
                self.bytes -= entry.body_len;
            } else {
                self.remove_at(&key, index);
            }
        }
    }
}
