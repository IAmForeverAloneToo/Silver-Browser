//! What a site keeps, as the DevTools' storage panel shows it (M36.8,
//! docs/design/devtools.md, "The remaining storage"): a page's
//! `localStorage` and `sessionStorage`, the cookies that go to it, its
//! IndexedDB databases and their records; their deletion and the
//! site's clearing; and every storage key the browser keeps anything
//! for, with what it uses. The browser's own records: the router asks
//! them of the tab, never of the page's renderer.

use std::sync::{Mutex, PoisonError};
use std::time::UNIX_EPOCH;

use devtools::{Json, Request};
use net::Url;
use net::cookies::Jar;

use super::{STORAGE_QUOTA, Scope, Storage};
use crate::idb::{IDB_ORIGIN_QUOTA, KeyRange, Mode, key_json};

/// The most records of each object store the panel is given.
pub const MAX_PANEL_RECORDS: usize = 100;

/// The bytes a storage key may use: its `localStorage`, two bytes a
/// code unit, and its IndexedDB databases (`navigator.storage`'s quota).
pub const SITE_QUOTA: u64 = IDB_ORIGIN_QUOTA + 2 * STORAGE_QUOTA as u64;

/// The bytes `origin` uses: its `localStorage`'s code units, two bytes
/// each, and its IndexedDB databases' keys and values.
pub(crate) fn usage(storage: &mut Storage, origin: &str) -> u64 {
    let local = storage.area(Scope::Local, origin).used() as u64 * 2;
    local + storage.idb(origin).used(origin)
}

/// The answer to the panel's `message` of the page at `url`, as the
/// protocol writes answers: nothing for what is not a request.
pub(crate) fn inspect(
    storage: &Mutex<Storage>,
    jar: &Mutex<Jar>,
    tab: u64,
    url: Option<&Url>,
    message: &str,
) -> String {
    let Ok(request) = Request::parse(message) else {
        return String::new();
    };
    let answered = match url {
        Some(url) if !url.origin().is_opaque() => answer(storage, jar, tab, url, &request),
        _ => Err("the page keeps nothing".to_owned()),
    };
    match answered {
        Ok(result) => devtools::result(request.id, result),
        Err(why) => devtools::error(request.id, &why),
    }
}

/// The answer to `request` of the page at `url`.
pub(crate) fn answer(
    storage: &Mutex<Storage>,
    jar: &Mutex<Jar>,
    tab: u64,
    url: &Url,
    request: &Request,
) -> Result<Json, String> {
    let origin = url.origin().serialization();
    let mut storage = storage.lock().unwrap_or_else(PoisonError::into_inner);
    let host = url
        .host()
        .map(ToString::to_string)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let param = |name: &str| {
        request
            .param(name)
            .and_then(Json::as_str)
            .unwrap_or_default()
    };
    Ok(match request.method.as_str() {
        "storage.local" | "storage.session" => {
            let area = storage.area(Scope::of(request.method == "storage.local", tab), &origin);
            let keys = area.keys().to_vec();
            Json::Array(
                keys.iter()
                    .map(|key| {
                        Json::Array(vec![
                            Json::from(key.as_str()),
                            Json::from(area.get(key).unwrap_or_default()),
                        ])
                    })
                    .collect(),
            )
        }
        "storage.removeItem" => {
            let local = param("area") != "session";
            storage.remove_item(Scope::of(local, tab), &origin, param("key"));
            Json::Null
        }
        "storage.cookies" => {
            let jar = jar.lock().unwrap_or_else(PoisonError::into_inner);
            Json::Array(
                jar.cookies()
                    .iter()
                    .filter(|cookie| goes_to(cookie, &host))
                    .map(cookie_json)
                    .collect(),
            )
        }
        "storage.removeCookie" => {
            let (name, domain, path) = (param("name"), param("domain"), param("path"));
            let mut jar = jar.lock().unwrap_or_else(PoisonError::into_inner);
            jar.remove_where(|cookie| {
                cookie.name == name.as_bytes() && cookie.domain == domain && cookie.path == path
            });
            Json::Null
        }
        "storage.idb" => idb_json(&mut storage, &origin),
        "storage.deleteDatabase" => {
            storage.idb(&origin).delete_database(&origin, param("name"));
            Json::Null
        }
        // The page's site, or another the listing names.
        "storage.clearSite" => {
            let (origin, host) = match request.param("origin").and_then(Json::as_str) {
                Some(other) => {
                    let url = Url::parse(other).map_err(|_| format!("no site {other}"))?;
                    (
                        other.to_owned(),
                        url.host()
                            .map(ToString::to_string)
                            .unwrap_or_default()
                            .to_ascii_lowercase(),
                    )
                }
                None => (origin.clone(), host.clone()),
            };
            clear_site(&mut storage, jar, &origin, &host);
            Json::Null
        }
        "storage.sites" => {
            let sites = storage.sites();
            Json::Array(
                sites
                    .into_iter()
                    .map(|site| {
                        let used = usage(&mut storage, &site);
                        Json::object([("origin", Json::from(site)), ("usage", Json::from(used))])
                    })
                    .collect(),
            )
        }
        "storage.usage" => Json::object([
            ("usage", Json::from(usage(&mut storage, &origin))),
            ("quota", Json::from(SITE_QUOTA)),
        ]),
        method => return Err(format!("no method {method}")),
    })
}

/// Clears what `origin` keeps, and the cookies that go to `host`.
fn clear_site(storage: &mut Storage, jar: &Mutex<Jar>, origin: &str, host: &str) {
    storage.clear_area(Scope::Local, origin);
    storage.clear_sessions(origin);
    let store = storage.idb(origin);
    for (name, _) in store.databases(origin) {
        store.delete_database(origin, &name);
    }
    jar.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove_where(|cookie| goes_to(cookie, host));
    // M38: what the HTTP cache keeps for the site, at the top or framed.
    if let (Some(cache), Ok(url)) = (storage.cache(), Url::parse(origin)) {
        cache.clear_site(&url.origin().site());
    }
}

/// Whether `cookie` goes to `host`: its own, or a domain's it is under.
fn goes_to(cookie: &net::cookies::Cookie, host: &str) -> bool {
    if cookie.host_only {
        cookie.domain == host
    } else {
        host == cookie.domain || host.ends_with(&format!(".{}", cookie.domain))
    }
}

fn cookie_json(cookie: &net::cookies::Cookie) -> Json {
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    let expires = cookie
        .expires
        .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_millis() as f64);
    Json::object([
        ("name", Json::from(text(&cookie.name))),
        ("value", Json::from(text(&cookie.value))),
        ("domain", Json::from(cookie.domain.as_str())),
        ("hostOnly", Json::from(cookie.host_only)),
        ("path", Json::from(cookie.path.as_str())),
        ("expires", Json::from(expires)),
        ("secure", Json::from(cookie.secure)),
        ("httpOnly", Json::from(cookie.http_only)),
        ("sameSite", Json::from(format!("{:?}", cookie.same_site))),
    ])
}

/// `origin`'s IndexedDB databases: each one's stores, with their count
/// of records and the first [`MAX_PANEL_RECORDS`] of them.
fn idb_json(storage: &mut Storage, origin: &str) -> Json {
    let store = storage.idb(origin);
    let mut databases = Vec::new();
    for (name, version) in store.databases(origin) {
        let infos = store.stores(origin, &name);
        let scope: Vec<String> = infos.iter().map(|info| info.name.clone()).collect();
        let txn = if scope.is_empty() {
            None
        } else {
            store.begin(origin, &name, Mode::ReadOnly, scope).ok()
        };
        let mut stores = Vec::new();
        for info in infos {
            let (count, records) = match txn {
                Some(txn) => (
                    store
                        .count(txn, (&info.name, None), &KeyRange::all())
                        .unwrap_or(0),
                    store
                        .get_all(
                            txn,
                            (&info.name, None),
                            &KeyRange::all(),
                            MAX_PANEL_RECORDS,
                            false,
                        )
                        .unwrap_or_default(),
                ),
                None => (0, Vec::new()),
            };
            stores.push(Json::object([
                ("name", Json::from(info.name.as_str())),
                ("autoIncrement", Json::from(info.auto_increment)),
                (
                    "indices",
                    Json::Array(
                        info.indices
                            .iter()
                            .map(|i| Json::from(i.name.as_str()))
                            .collect(),
                    ),
                ),
                ("count", Json::from(count)),
                (
                    "records",
                    Json::Array(
                        records
                            .iter()
                            .map(|found| {
                                Json::Array(vec![
                                    key_json(&found.key),
                                    Json::from(
                                        String::from_utf8_lossy(
                                            found.value.as_deref().unwrap_or_default(),
                                        )
                                        .into_owned(),
                                    ),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]));
        }
        if let Some(txn) = txn {
            let _ = store.commit(txn);
        }
        databases.push(Json::object([
            ("name", Json::from(name)),
            ("version", Json::from(version)),
            ("stores", Json::Array(stores)),
        ]));
    }
    Json::Array(databases)
}
