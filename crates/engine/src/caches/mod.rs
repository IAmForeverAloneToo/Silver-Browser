//! The Cache API's store (M39, `docs/design/service-workers.md`): each
//! storage key's caches, in the order they were made, each cache's
//! entries in the order they were put, a request and its response with
//! the response's body. Kept by the context's storage beside IndexedDB's,
//! read from the profile at a key's first use, its bodies when matched,
//! and written at the storage's flush. Its queries are the standard's
//! Query Cache and its writes Batch Cache Operations, applied whole or
//! not at all, within [`CACHE_QUOTA`].

use std::collections::HashMap;
use std::sync::Arc;

mod fuzz;
mod record;
mod serve;
#[cfg(test)]
mod tests;

#[doc(hidden)]
pub use fuzz::fuzz_store;
pub(crate) use serve::serve;

/// The most a storage key's caches may hold, in bytes: their bodies, their
/// requests' and responses' heads, and [`OPAQUE_PADDING`] for each
/// opaque response.
pub const CACHE_QUOTA: u64 = 512 << 20;

/// What an opaque response counts toward the quota and `estimate()`,
/// whatever its size, so that a page cannot learn another origin's
/// response's size from them.
pub const OPAQUE_PADDING: u64 = 7 << 20;

/// The most caches a storage key keeps.
pub const MAX_CACHES: usize = 256;

/// The longest a cache's name may be, in bytes.
pub const MAX_CACHE_NAME: usize = 1024;

/// The most caches deleted while a script may still hold them that a
/// storage key keeps answering from, in memory: past it, the oldest go.
const MAX_ORPHANS: usize = 16;

/// A request as a cache keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredRequest {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
}

/// A response as a cache keeps it, but its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredResponse {
    /// `basic`, `cors`, `default` or `opaque`.
    pub(crate) kind: String,
    pub(crate) urls: Vec<String>,
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: Vec<(String, String)>,
}

/// Fields of `headers` named `name`, ASCII case ignored, joined as Fetch
/// combines them; `None` when there is none.
fn header(headers: &[(String, String)], name: &str) -> Option<String> {
    let values: Vec<&str> = headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
        .collect();
    (!values.is_empty()).then(|| values.join(", "))
}

impl StoredResponse {
    /// The field names its `Vary` names, lowercased.
    fn vary(&self) -> Vec<String> {
        header(&self.headers, "vary")
            .map(|value| {
                value
                    .split(',')
                    .map(|name| name.trim().to_ascii_lowercase())
                    .filter(|name| !name.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn opaque(&self) -> bool {
        self.kind == "opaque"
    }
}

/// How a query matches (the `CacheQueryOptions` dictionary).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct QueryOptions {
    pub(crate) ignore_search: bool,
    pub(crate) ignore_method: bool,
    pub(crate) ignore_vary: bool,
}

/// `url` as a query compares it: without its fragment, and without its
/// query when the search is ignored.
fn compared(url: &str, ignore_search: bool) -> &str {
    let url = url.split_once('#').map_or(url, |(before, _)| before);
    if ignore_search {
        url.split_once('?').map_or(url, |(before, _)| before)
    } else {
        url
    }
}

/// Whether the entry of `request` and `response` answers `query`
/// (Request Matches Cached Item).
fn matches(
    query: &StoredRequest,
    request: &StoredRequest,
    response: &StoredResponse,
    options: QueryOptions,
) -> bool {
    if !options.ignore_method && (query.method != "GET" || request.method != "GET") {
        return false;
    }
    if compared(&query.url, options.ignore_search) != compared(&request.url, options.ignore_search)
    {
        return false;
    }
    if options.ignore_vary {
        return true;
    }
    response
        .vary()
        .iter()
        .all(|name| name != "*" && header(&query.headers, name) == header(&request.headers, name))
}

/// A body: in memory, or kept in the profile alone.
#[derive(Debug, Clone)]
enum Body {
    Here(Arc<[u8]>),
    Kept(u64),
}

#[derive(Debug, Clone)]
struct Entry {
    /// Its number: its place among its cache's, and its key in the
    /// profile.
    id: u64,
    request: StoredRequest,
    response: StoredResponse,
    body: Body,
}

impl Entry {
    fn body_len(&self) -> u64 {
        match &self.body {
            Body::Here(bytes) => bytes.len() as u64,
            Body::Kept(len) => *len,
        }
    }

    /// What it counts toward the quota.
    fn cost(&self) -> u64 {
        let heads = |pairs: &[(String, String)]| {
            pairs
                .iter()
                .map(|(n, v)| (n.len() + v.len()) as u64)
                .sum::<u64>()
        };
        let head = (self.request.url.len() + self.request.method.len()) as u64
            + heads(&self.request.headers)
            + heads(&self.response.headers)
            + self
                .response
                .urls
                .iter()
                .map(|u| u.len() as u64)
                .sum::<u64>();
        if self.response.opaque() {
            OPAQUE_PADDING + head
        } else {
            self.body_len() + head
        }
    }
}

#[derive(Debug, Clone)]
struct Cache {
    id: u64,
    name: String,
    entries: Vec<Entry>,
}

/// A storage key's caches.
#[derive(Debug, Default)]
struct KeyCaches {
    caches: Vec<Cache>,
    /// Caches deleted that a script's `Cache` object may still read,
    /// in memory alone.
    orphans: Vec<Cache>,
    next_cache: u64,
    next_entry: u64,
}

impl KeyCaches {
    fn used(&self) -> u64 {
        self.caches
            .iter()
            .flat_map(|cache| &cache.entries)
            .map(Entry::cost)
            .sum()
    }

    fn find(&self, id: u64) -> Option<&Cache> {
        self.caches
            .iter()
            .chain(&self.orphans)
            .find(|cache| cache.id == id)
    }
}

/// What the store changed that the profile has not been given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Written {
    Cache {
        key: String,
        id: u64,
        record: Option<Vec<u8>>,
    },
    Entry {
        key: String,
        id: u64,
        record: Option<Vec<u8>>,
        body: Option<Vec<u8>>,
    },
}

/// Why an operation failed: the exception the script gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CacheError {
    pub(crate) name: &'static str,
    pub(crate) message: String,
}

impl CacheError {
    fn new(name: &'static str, message: impl Into<String>) -> CacheError {
        CacheError {
            name,
            message: message.into(),
        }
    }
}

/// An operation of a batch (Batch Cache Operations).
#[derive(Debug, Clone)]
pub(crate) enum Operation {
    Put {
        request: StoredRequest,
        response: StoredResponse,
        body: Vec<u8>,
    },
    Delete {
        request: StoredRequest,
        options: QueryOptions,
    },
}

/// A response matched, with its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Matched {
    pub(crate) response: StoredResponse,
    pub(crate) body: Arc<[u8]>,
}

/// An entry as the DevTools storage panel shows it: its URL, its
/// response's type and status, and its body's size.
pub(crate) type Shown = (String, String, u16, u64);

/// The store: every storage key's caches the context has read or made.
#[derive(Debug, Default)]
pub(crate) struct Store {
    keys: HashMap<String, KeyCaches>,
    /// Whether what changes is written to a profile.
    writing: bool,
    written: Vec<Written>,
}

/// Reads the body of the entry numbered so from the profile.
pub(crate) type BodyOf<'a> = &'a dyn Fn(u64) -> Option<Vec<u8>>;

impl Store {
    /// Writes what changes from now on, for the profile.
    pub(crate) fn set_writing(&mut self, writing: bool) {
        self.writing = writing;
    }

    /// Whether the key's caches were read or made.
    pub(crate) fn knows(&self, key: &str) -> bool {
        self.keys.contains_key(key)
    }

    /// The storage keys with caches.
    pub(crate) fn keys_with_caches(&self) -> Vec<String> {
        self.keys
            .iter()
            .filter(|(_, caches)| !caches.caches.is_empty())
            .map(|(key, _)| key.clone())
            .collect()
    }

    /// Takes in what the profile keeps of `key`: its caches' records and
    /// its entries', each by its number. A record that does not read is
    /// left out, and deleted.
    pub(crate) fn load(
        &mut self,
        key: &str,
        caches: Vec<(u64, Vec<u8>)>,
        entries: Vec<(u64, Vec<u8>)>,
    ) {
        let mut found = KeyCaches::default();
        for (id, bytes) in caches {
            match record::decode_cache(&bytes) {
                Some(name) => found.caches.push(Cache {
                    id,
                    name,
                    entries: Vec::new(),
                }),
                None => self.write(Written::Cache {
                    key: key.to_owned(),
                    id,
                    record: None,
                }),
            }
            found.next_cache = found.next_cache.max(id + 1);
        }
        for (id, bytes) in entries {
            found.next_entry = found.next_entry.max(id + 1);
            let read = record::decode_entry(&bytes);
            let cache = read
                .as_ref()
                .and_then(|r| found.caches.iter_mut().find(|c| c.id == r.cache));
            match (read, cache) {
                (Some(read), Some(cache)) => cache.entries.push(Entry {
                    id,
                    request: read.request,
                    response: read.response,
                    body: Body::Kept(read.body_len),
                }),
                _ => self.write(Written::Entry {
                    key: key.to_owned(),
                    id,
                    record: None,
                    body: None,
                }),
            }
        }
        found.caches.sort_by_key(|cache| cache.id);
        for cache in &mut found.caches {
            cache.entries.sort_by_key(|entry| entry.id);
        }
        self.keys.insert(key.to_owned(), found);
    }

    fn write(&mut self, written: Written) {
        if self.writing {
            self.written.push(written);
        }
    }

    /// Whether there are changes the profile has not been given.
    pub(crate) fn has_written(&self) -> bool {
        !self.written.is_empty()
    }

    /// The changes the profile has not been given, taken; the bodies
    /// written leave memory, to be read from the profile.
    pub(crate) fn take_written(&mut self) -> Vec<Written> {
        let written = std::mem::take(&mut self.written);
        for change in &written {
            if let Written::Entry {
                key,
                id,
                record: Some(_),
                body: Some(_),
            } = change
                && let Some(caches) = self.keys.get_mut(key)
            {
                for entry in caches
                    .caches
                    .iter_mut()
                    .flat_map(|cache| &mut cache.entries)
                {
                    if entry.id == *id {
                        entry.body = Body::Kept(entry.body_len());
                    }
                }
            }
        }
        written
    }

    fn key_mut(&mut self, key: &str) -> &mut KeyCaches {
        self.keys.entry(key.to_owned()).or_default()
    }

    /// The bytes `key`'s caches use.
    pub(crate) fn usage(&self, key: &str) -> u64 {
        self.keys.get(key).map_or(0, KeyCaches::used)
    }

    /// The names of `key`'s caches, in the order they were made.
    pub(crate) fn names(&self, key: &str) -> Vec<String> {
        self.keys.get(key).map_or_else(Vec::new, |caches| {
            caches.caches.iter().map(|c| c.name.clone()).collect()
        })
    }

    /// The number of `key`'s cache `name`, if it has one.
    pub(crate) fn number(&self, key: &str, name: &str) -> Option<u64> {
        let caches = self.keys.get(key)?;
        caches
            .caches
            .iter()
            .find(|cache| cache.name == name)
            .map(|cache| cache.id)
    }

    /// The number of `key`'s cache `name`, made if it has none.
    pub(crate) fn open(&mut self, key: &str, name: &str) -> Result<u64, CacheError> {
        if let Some(id) = self.number(key, name) {
            return Ok(id);
        }
        if name.len() > MAX_CACHE_NAME {
            return Err(CacheError::new(
                "TypeError",
                format!("The cache name is longer than {MAX_CACHE_NAME} bytes."),
            ));
        }
        let caches = self.key_mut(key);
        if caches.caches.len() >= MAX_CACHES {
            return Err(CacheError::new(
                "QuotaExceededError",
                format!("A storage key keeps at most {MAX_CACHES} caches."),
            ));
        }
        let id = caches.next_cache;
        caches.next_cache += 1;
        caches.caches.push(Cache {
            id,
            name: name.to_owned(),
            entries: Vec::new(),
        });
        self.write(Written::Cache {
            key: key.to_owned(),
            id,
            record: Some(record::encode_cache(name)),
        });
        Ok(id)
    }

    /// Deletes `key`'s cache `name`: says whether there was one. A
    /// `Cache` object of it still reads what it held, from memory.
    pub(crate) fn delete(&mut self, key: &str, name: &str) -> bool {
        let Some(caches) = self.keys.get_mut(key) else {
            return false;
        };
        let Some(at) = caches.caches.iter().position(|c| c.name == name) else {
            return false;
        };
        let cache = caches.caches.remove(at);
        let ids: Vec<u64> = cache.entries.iter().map(|e| e.id).collect();
        let id = cache.id;
        caches.orphans.push(cache);
        if caches.orphans.len() > MAX_ORPHANS {
            caches.orphans.remove(0);
        }
        for entry in ids {
            self.write(Written::Entry {
                key: key.to_owned(),
                id: entry,
                record: None,
                body: None,
            });
        }
        self.write(Written::Cache {
            key: key.to_owned(),
            id,
            record: None,
        });
        true
    }

    /// The entries of `key`'s cache `cache` that answer `query` (all of
    /// them for none), in their order, without their bodies.
    fn query(
        &self,
        key: &str,
        cache: u64,
        query: Option<&StoredRequest>,
        options: QueryOptions,
    ) -> Result<Vec<Entry>, CacheError> {
        let cache = self
            .keys
            .get(key)
            .and_then(|caches| caches.find(cache))
            .ok_or_else(|| CacheError::new("InvalidStateError", "The cache is gone."))?;
        Ok(cache
            .entries
            .iter()
            .filter(|entry| {
                query.is_none_or(|query| matches(query, &entry.request, &entry.response, options))
            })
            .cloned()
            .collect())
    }

    /// A matched entry's body, read from the profile if it is kept there.
    fn body(entry: &Entry, body_of: BodyOf<'_>) -> Arc<[u8]> {
        match &entry.body {
            Body::Here(bytes) => bytes.clone(),
            Body::Kept(_) => body_of(entry.id)
                .map(Arc::from)
                .unwrap_or_else(|| Arc::from(Vec::new())),
        }
    }

    /// `matchAll()` of `key`'s cache `cache`: the responses answering
    /// `query`, or all, with their bodies.
    pub(crate) fn match_all(
        &self,
        key: &str,
        cache: u64,
        query: Option<&StoredRequest>,
        options: QueryOptions,
        body_of: BodyOf<'_>,
    ) -> Result<Vec<Matched>, CacheError> {
        Ok(self
            .query(key, cache, query, options)?
            .iter()
            .map(|entry| Matched {
                response: entry.response.clone(),
                body: Self::body(entry, body_of),
            })
            .collect())
    }

    /// `keys()` of `key`'s cache `cache`: the requests answering `query`,
    /// or all.
    pub(crate) fn requests(
        &self,
        key: &str,
        cache: u64,
        query: Option<&StoredRequest>,
        options: QueryOptions,
    ) -> Result<Vec<StoredRequest>, CacheError> {
        Ok(self
            .query(key, cache, query, options)?
            .into_iter()
            .map(|entry| entry.request)
            .collect())
    }

    /// `caches.match()`: the first response answering `query` in `key`'s
    /// caches in their order, or in the one named `name` alone.
    pub(crate) fn match_any(
        &self,
        key: &str,
        name: Option<&str>,
        query: &StoredRequest,
        options: QueryOptions,
        body_of: BodyOf<'_>,
    ) -> Option<Matched> {
        let caches = self.keys.get(key)?;
        caches
            .caches
            .iter()
            .filter(|cache| name.is_none_or(|name| cache.name == name))
            .find_map(|cache| {
                self.match_all(key, cache.id, Some(query), options, body_of)
                    .ok()?
                    .into_iter()
                    .next()
            })
    }

    /// Batch Cache Operations on `key`'s cache `cache`: each delete's
    /// result in order, or an error and nothing changed.
    pub(crate) fn batch(
        &mut self,
        key: &str,
        cache: u64,
        operations: Vec<Operation>,
    ) -> Result<Vec<bool>, CacheError> {
        let caches = self
            .keys
            .get(key)
            .ok_or_else(|| CacheError::new("InvalidStateError", "The cache is gone."))?;
        let orphan = !caches.caches.iter().any(|c| c.id == cache);
        let mut entries = caches
            .find(cache)
            .ok_or_else(|| CacheError::new("InvalidStateError", "The cache is gone."))?
            .entries
            .clone();
        let before = caches.used();
        let old_cost: u64 = entries.iter().map(Entry::cost).sum();
        let mut next = caches.next_entry;
        let mut results = Vec::new();
        let mut removed: Vec<u64> = Vec::new();
        let mut added: Vec<u64> = Vec::new();
        for operation in operations {
            match operation {
                Operation::Delete { request, options } => {
                    let before = entries.len();
                    entries.retain(|entry| {
                        let gone = matches(&request, &entry.request, &entry.response, options);
                        if gone {
                            removed.push(entry.id);
                        }
                        !gone
                    });
                    results.push(entries.len() != before);
                }
                Operation::Put {
                    request,
                    response,
                    body,
                } => {
                    entries.retain(|entry| {
                        let gone = matches(
                            &request,
                            &entry.request,
                            &entry.response,
                            QueryOptions::default(),
                        );
                        if gone {
                            removed.push(entry.id);
                        }
                        !gone
                    });
                    let id = next;
                    next += 1;
                    added.push(id);
                    entries.push(Entry {
                        id,
                        request,
                        response,
                        body: Body::Here(Arc::from(body)),
                    });
                }
            }
        }
        let new_cost: u64 = entries.iter().map(Entry::cost).sum();
        if !orphan && before - old_cost + new_cost > CACHE_QUOTA {
            return Err(CacheError::new(
                "QuotaExceededError",
                "The storage key's caches would pass their quota.",
            ));
        }
        let caches = self.key_mut(key);
        caches.next_entry = next;
        let target = caches
            .caches
            .iter_mut()
            .chain(caches.orphans.iter_mut())
            .find(|c| c.id == cache)
            .expect("found above");
        target.entries = entries;
        if orphan {
            return Ok(results);
        }
        let changes: Vec<Written> = removed
            .iter()
            .filter(|id| !added.contains(id))
            .map(|&id| Written::Entry {
                key: key.to_owned(),
                id,
                record: None,
                body: None,
            })
            .chain(
                target
                    .entries
                    .iter()
                    .filter(|entry| added.contains(&entry.id))
                    .map(|entry| Written::Entry {
                        key: key.to_owned(),
                        id: entry.id,
                        record: Some(record::encode_entry(
                            cache,
                            &entry.request,
                            &entry.response,
                            entry.body_len(),
                        )),
                        body: match &entry.body {
                            Body::Here(bytes) => Some(bytes.to_vec()),
                            Body::Kept(_) => None,
                        },
                    }),
            )
            .collect();
        for change in changes {
            self.write(change);
        }
        Ok(results)
    }

    /// Deletes every cache of `key` (a site cleared).
    pub(crate) fn clear(&mut self, key: &str) {
        for name in self.names(key) {
            self.delete(key, &name);
        }
        if let Some(caches) = self.keys.get_mut(key) {
            caches.orphans.clear();
        }
    }

    /// What the DevTools storage panel shows of `key`'s caches: each
    /// cache's name and its entries' URLs, types, statuses and bodies'
    /// sizes.
    pub(crate) fn summary(&self, key: &str) -> Vec<(String, Vec<Shown>)> {
        self.keys.get(key).map_or_else(Vec::new, |caches| {
            caches
                .caches
                .iter()
                .map(|cache| {
                    (
                        cache.name.clone(),
                        cache
                            .entries
                            .iter()
                            .map(|e| {
                                (
                                    e.request.url.clone(),
                                    e.response.kind.clone(),
                                    e.response.status,
                                    e.body_len(),
                                )
                            })
                            .collect(),
                    )
                })
                .collect()
        })
    }
}
