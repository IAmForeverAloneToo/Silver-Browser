//! For fuzzing (M39): the store driven by any sequence of operations,
//! compared after each with a model written apart from it, the
//! standard's algorithms done the plain way; written to a profile of
//! records and read back by a new store, which must answer the same.

use std::collections::HashMap;

use super::{Operation, QueryOptions, Store, StoredRequest, StoredResponse, Written, record};

const NAMES: [&str; 3] = ["a", "b", "c"];
const URLS: [&str; 5] = [
    "https://a.test/x",
    "https://a.test/x?q=1",
    "https://a.test/x#f",
    "https://a.test/y",
    "https://b.test/x",
];
const VALUES: [Option<&str>; 3] = [None, Some("1"), Some("2")];
const KEY: &str = "https://a.test";

struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn byte(&mut self) -> u8 {
        let (&first, rest) = self.0.split_first().unwrap_or((&0, &[]));
        self.0 = rest;
        first
    }

    fn pick<T: Copy>(&mut self, table: &[T]) -> T {
        table[usize::from(self.byte()) % table.len()]
    }
}

fn request(input: &mut Input<'_>) -> StoredRequest {
    let flags = input.byte();
    let method = if flags & 1 == 0 { "GET" } else { "POST" };
    let mut headers = Vec::new();
    if let Some(value) = VALUES[usize::from(flags >> 1) % 3] {
        headers.push(("X-V".to_owned(), value.to_owned()));
    }
    StoredRequest {
        method: method.to_owned(),
        url: input.pick(&URLS).to_owned(),
        headers,
    }
}

fn options(input: &mut Input<'_>) -> QueryOptions {
    let flags = input.byte();
    QueryOptions {
        ignore_search: flags & 1 != 0,
        ignore_method: flags & 2 != 0,
        ignore_vary: flags & 4 != 0,
    }
}

/// The model's entry: a GET request, its response's `x-n` and `Vary`.
#[derive(Clone)]
struct Model {
    request: StoredRequest,
    n: u8,
    vary: bool,
}

fn plain(url: &str, search: bool) -> &str {
    let url = url.split('#').next().unwrap_or(url);
    if search {
        url.split('?').next().unwrap_or(url)
    } else {
        url
    }
}

fn value<'a>(request: &'a StoredRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Request Matches Cached Item, as the standard writes it.
fn model_matches(query: &StoredRequest, item: &Model, options: QueryOptions) -> bool {
    if !options.ignore_method && query.method != "GET" {
        return false;
    }
    if plain(&query.url, options.ignore_search) != plain(&item.request.url, options.ignore_search) {
        return false;
    }
    options.ignore_vary || !item.vary || value(query, "x-v") == value(&item.request, "x-v")
}

fn response(n: u8, vary: bool) -> StoredResponse {
    let mut headers = vec![("x-n".to_owned(), n.to_string())];
    if vary {
        headers.push(("Vary".to_owned(), "x-v".to_owned()));
    }
    StoredResponse {
        kind: "default".to_owned(),
        urls: Vec::new(),
        status: 200,
        status_text: String::new(),
        headers,
    }
}

fn ns(found: &[super::Matched]) -> Vec<u8> {
    found
        .iter()
        .map(|m| {
            super::header(&m.response.headers, "x-n")
                .and_then(|n| n.parse().ok())
                .unwrap_or(255)
        })
        .collect()
}

/// The profile: what the store wrote, applied.
#[derive(Default)]
struct Kept {
    caches: HashMap<u64, Vec<u8>>,
    entries: HashMap<u64, Vec<u8>>,
    bodies: HashMap<u64, Vec<u8>>,
}

impl Kept {
    fn apply(&mut self, written: Vec<Written>) {
        for change in written {
            match change {
                Written::Cache { id, record, .. } => match record {
                    Some(record) => {
                        self.caches.insert(id, record);
                    }
                    None => {
                        self.caches.remove(&id);
                    }
                },
                Written::Entry {
                    id, record, body, ..
                } => match record {
                    Some(record) => {
                        self.entries.insert(id, record);
                        if let Some(body) = body {
                            self.bodies.insert(id, body);
                        }
                    }
                    None => {
                        self.entries.remove(&id);
                        self.bodies.remove(&id);
                    }
                },
            }
        }
    }

    fn store(&self) -> Store {
        let mut store = Store::default();
        store.set_writing(true);
        let sorted = |map: &HashMap<u64, Vec<u8>>| {
            let mut list: Vec<(u64, Vec<u8>)> = map.iter().map(|(k, v)| (*k, v.clone())).collect();
            list.sort_by_key(|(id, _)| *id);
            list
        };
        store.load(KEY, sorted(&self.caches), sorted(&self.entries));
        store
    }
}

/// Runs the operations `data` says, checking the store against the model
/// after each; panics where they differ.
#[doc(hidden)]
pub fn fuzz_store(data: &[u8]) {
    record::fuzz_entry_record(data);
    let mut input = Input(data);
    let mut kept = Kept::default();
    let mut store = kept.store();
    let mut model: Vec<(String, Vec<Model>)> = Vec::new();
    let mut steps = 0;
    while !input.0.is_empty() && steps < 64 {
        steps += 1;
        let name = input.pick(&NAMES);
        match input.byte() % 8 {
            0 => {
                store.open(KEY, name).expect("a name within bounds");
                if !model.iter().any(|(n, _)| n == name) {
                    model.push((name.to_owned(), Vec::new()));
                }
            }
            1 => {
                let deleted = store.delete(KEY, name);
                let had = model.iter().position(|(n, _)| n == name);
                assert_eq!(deleted, had.is_some());
                if let Some(at) = had {
                    model.remove(at);
                }
            }
            2 => {
                let Some(id) = store.number(KEY, name) else {
                    continue;
                };
                let mut query = request(&mut input);
                query.method = "GET".to_owned();
                let n = input.byte() % 100;
                let vary = input.byte() & 1 != 0;
                store
                    .batch(
                        KEY,
                        id,
                        vec![Operation::Put {
                            request: query.clone(),
                            response: response(n, vary),
                            body: vec![n],
                        }],
                    )
                    .expect("within the quota");
                let entries = &mut model
                    .iter_mut()
                    .find(|(m, _)| m == name)
                    .expect("a cache")
                    .1;
                entries.retain(|item| !model_matches(&query, item, QueryOptions::default()));
                entries.push(Model {
                    request: query,
                    n,
                    vary,
                });
            }
            3 => {
                let Some(id) = store.number(KEY, name) else {
                    continue;
                };
                let query = request(&mut input);
                let options = options(&mut input);
                let results = store
                    .batch(
                        KEY,
                        id,
                        vec![Operation::Delete {
                            request: query.clone(),
                            options,
                        }],
                    )
                    .expect("a delete");
                let entries = &mut model
                    .iter_mut()
                    .find(|(m, _)| m == name)
                    .expect("a cache")
                    .1;
                let before = entries.len();
                entries.retain(|item| !model_matches(&query, item, options));
                assert_eq!(results, [entries.len() != before]);
            }
            4 | 5 => {
                let Some(id) = store.number(KEY, name) else {
                    continue;
                };
                let query = request(&mut input);
                let options = options(&mut input);
                let found = store
                    .match_all(KEY, id, Some(&query), options, &|id| {
                        kept.bodies.get(&id).cloned()
                    })
                    .expect("a cache");
                let entries = &model.iter().find(|(m, _)| m == name).expect("a cache").1;
                let wanted: Vec<u8> = entries
                    .iter()
                    .filter(|item| model_matches(&query, item, options))
                    .map(|item| item.n)
                    .collect();
                assert_eq!(ns(&found), wanted, "matchAll");
                for matched in &found {
                    assert_eq!(matched.body.len(), 1, "the body whole");
                }
            }
            6 => {
                let query = request(&mut input);
                let options = options(&mut input);
                let found = store.match_any(KEY, None, &query, options, &|id| {
                    kept.bodies.get(&id).cloned()
                });
                let wanted = model
                    .iter()
                    .find_map(|(_, entries)| {
                        entries.iter().find(|i| model_matches(&query, i, options))
                    })
                    .map(|item| item.n);
                assert_eq!(found.map(|m| ns(&[m])[0]), wanted, "caches.match");
            }
            _ => {
                // The profile written and read again by a new store.
                kept.apply(store.take_written());
                store = kept.store();
            }
        }
        let names: Vec<&str> = model.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(store.names(KEY), names, "the caches in order");
        for (n, entries) in &model {
            let id = store.number(KEY, n).expect("the cache");
            let urls: Vec<String> = store
                .requests(KEY, id, None, QueryOptions::default())
                .expect("its entries")
                .into_iter()
                .map(|r| r.url)
                .collect();
            let wanted: Vec<String> = entries.iter().map(|i| i.request.url.clone()).collect();
            assert_eq!(urls, wanted, "the entries in order");
        }
        assert!(store.usage(KEY) <= super::CACHE_QUOTA);
    }
}
