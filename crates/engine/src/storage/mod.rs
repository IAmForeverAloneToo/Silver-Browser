//! What the browsing keeps (M28, docs/design/storage.md): the profile
//! the engine writes to, and when; and the Web Storage areas (HTML
//! section 12.2), each origin's `localStorage` kept in the profile and
//! the tab's `sessionStorage` in memory. The cookies with an expiry
//! join the jar when the engine is given a profile, an origin's items
//! are read from it at their first use, and what changes is written in
//! one transaction no more than once in [`FLUSH_INTERVAL`]. The HTTP
//! cache's entries are written with them (M38), and read back by
//! [`load_cache`].

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use crate::permissions::Permissions;
use std::time::{Duration, SystemTime};

use net::cookies::Jar;
use profile::{Changes, Profile};

use crate::loader::PageId;

pub(crate) mod devtools;
#[cfg(test)]
mod devtools_tests;
#[cfg(test)]
mod isolation_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod web_tests;

/// The most changes an engine's documents may have waiting to hear of
/// in `storage` events (M37): past it the oldest are dropped.
pub(crate) const MAX_WAITING_CHANGES: usize = 1024;

/// The least time between two writes of the profile: a change waits at
/// most this long to be written, and a crash loses at most this much.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// How much an area holds at most (HTML 12.2.1's "storage quota"), in
/// UTF-16 code units of its keys and values together: the reference
/// browser's, for each origin's `localStorage` and, apart, for its
/// `sessionStorage`.
pub const STORAGE_QUOTA: usize = 5 << 20;

/// The engine's profile, its Web Storage areas, and what of them waits
/// to be written or to be told to other documents.
pub(crate) struct Storage {
    /// `None` keeps nothing: an engine of tests and tools.
    profile: Option<Profile>,
    /// The jar's count of changes when its cookies were last written;
    /// `None` when they must be.
    cookies_written: Option<u64>,
    /// When the profile was last written, or tried, by the engine's
    /// clock.
    flushed: Option<Duration>,
    /// Each origin's `localStorage` area, by its serialization, read
    /// from the profile at its first use.
    local: HashMap<String, Area>,
    /// Each tab's `sessionStorage` areas, by the tab's number and the
    /// origin (M37: the tabs of a context share the storage).
    session: HashMap<(u64, String), Area>,
    /// What changed in the `localStorage` areas since the profile was
    /// written, by origin.
    unwritten: HashMap<String, Unwritten>,
    /// Those that hear of changes in `storage` events (M37): each
    /// engine, by its number, with the changes it has not taken; and the
    /// next number.
    listeners: HashMap<u64, Listener>,
    next_listener: u64,
    /// The engine's HSTS hosts (M33.3), and their count of changes when
    /// they were last written.
    hsts: Option<Arc<Mutex<net::Hsts>>>,
    hsts_written: Option<u64>,
    /// The permissions sites were given (M33.4), and their count of
    /// changes when they were last written.
    permissions: Option<Arc<Permissions>>,
    permissions_written: Option<u64>,
    /// IndexedDB's databases (M36.4), each storage key's read from the
    /// profile at their first use, and what they committed that the
    /// profile has not had yet.
    idb: crate::idb::IdbStore,
    idb_unwritten: Vec<crate::idb::Written>,
    /// The Cache API's store (M39), each storage key's read from the
    /// profile at its first use, and what it changed that the profile
    /// has not had yet.
    caches: crate::caches::Store,
    caches_unwritten: Vec<crate::caches::Written>,
    /// The service workers' registry (M39), each storage key's
    /// registrations read from the profile at their first use, and what
    /// changed that the profile has not had yet.
    registry: crate::service_worker::Registry,
    registry_unwritten: Vec<crate::service_worker::Written>,
    /// What has the registry's effects done when it changed outside its
    /// driver: the driver's wake.
    registry_wake: Option<Arc<dyn Fn() + Send + Sync>>,
    /// The registry's driver, which its clients' jobs go to.
    service_workers: Option<crate::service_worker::WeakServiceWorkers>,
    /// The settings as read from the profile, until taken, and as they
    /// changed since it was last written (M37).
    settings_read: Option<Vec<u8>>,
    settings_unwritten: Option<Vec<u8>>,
    /// The HTTP cache (M38); the records of its entries the profile held
    /// when it was given, until taken; its changes taken, not yet
    /// written.
    cache: Option<Arc<net::Cache>>,
    cache_read: Option<Vec<(u64, Vec<u8>)>>,
    cache_unwritten: Vec<net::cache::Change>,
}

/// Keeps `cache` in the profile of `storage` (M38), when it has one:
/// takes in the entries the profile held, their bodies read through
/// `storage` at their first use, not holding the cache's lock.
pub(crate) fn load_cache(storage: &Arc<Mutex<Storage>>, cache: &Arc<net::Cache>) {
    let records = {
        let mut storage = storage.lock().unwrap_or_else(PoisonError::into_inner);
        storage.set_cache(cache.clone());
        storage.cache_read.take()
    };
    let Some(records) = records else {
        return;
    };
    let storage = Arc::downgrade(storage);
    cache.load(
        records,
        Arc::new(move |id| {
            let storage = storage.upgrade()?;
            let storage = storage.lock().unwrap_or_else(PoisonError::into_inner);
            storage.profile.as_ref()?.cache_body(id).ok().flatten()
        }),
    );
}

/// An area (HTML 12.2.1's storage map): its items, and their order, the
/// order of their first setting.
#[derive(Debug, Default)]
pub(crate) struct Area {
    items: HashMap<String, Item>,
    /// The keys in order, made again when one comes or goes.
    keys: Option<Vec<String>>,
    /// The order of the next item set.
    next: u64,
    /// The UTF-16 code units its keys and values hold.
    used: usize,
}

#[derive(Debug)]
struct Item {
    value: String,
    order: u64,
}

/// What changed in an origin's `localStorage` since it was written.
#[derive(Debug, Default)]
struct Unwritten {
    cleared: bool,
    keys: HashSet<String>,
}

/// One that hears of changes: an engine, or a tab that tells its
/// renderers.
#[derive(Default)]
struct Listener {
    changes: Vec<Change>,
    /// The `BroadcastChannel` messages it has not taken (gap batch A).
    broadcasts: Vec<Broadcast>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// A `BroadcastChannel` message (gap batch A, D46): for the channels of
/// its name and storage key, as the sender serialized it, which channel
/// sent it (that one hears nothing of it), and whose it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Broadcast {
    pub(crate) key: AreaKey,
    pub(crate) name: String,
    pub(crate) data: String,
    pub(crate) sender: u64,
    pub(crate) source: Source,
}

/// Which area of an origin: its `localStorage`, or a tab's
/// `sessionStorage`, by the tab's number (M37).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Local,
    Session(u64),
}

impl Scope {
    /// The `localStorage` area if `local`, else tab `tab`'s
    /// `sessionStorage` one.
    pub(crate) fn of(local: bool, tab: u64) -> Scope {
        match local {
            true => Scope::Local,
            false => Scope::Session(tab),
        }
    }

    pub(crate) fn is_local(self) -> bool {
        self == Scope::Local
    }
}

/// Where a change was made: which document is not to hear of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// A document of the engine listening as this number, its page the
    /// change's.
    Engine(u64),
    /// In the browser (M37): a document of the renderer of this number,
    /// whose own documents heard of it there.
    Renderer(u32),
    /// Another renderer's, the browser says: every document hears of it.
    Elsewhere,
}

/// A change of an area that other documents of its origin hear of
/// (HTML 12.2.1's "broadcast"): `None`s for `null`s, all three for a
/// `clear()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) scope: Scope,
    pub(crate) origin: String,
    pub(crate) key: Option<String>,
    pub(crate) old: Option<String>,
    pub(crate) new: Option<String>,
    /// The URL of the document that changed it.
    pub(crate) url: String,
    /// That document's page, which hears nothing of it.
    pub(crate) page: PageId,
    pub(crate) source: Source,
    /// The area's storage key, its origin and partition: by its origin
    /// the browser picks the renderers that may hear of it (M37).
    pub(crate) owner: net::Origin,
    pub(crate) partition: Option<net::Partition>,
}

/// Whose area a document's scripts reach (the Storage Standard's storage
/// key, M33.2): its origin and, in a context of another site than the
/// top, the top-level site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AreaKey {
    pub(crate) origin: net::Origin,
    pub(crate) partition: Option<net::Partition>,
}

impl AreaKey {
    /// The key the engine keeps the area by: the origin's serialization,
    /// and for a cross-site context the top-level site's after a `^`.
    pub(crate) fn serialization(&self) -> String {
        let origin = self.origin.serialization();
        match &self.partition {
            Some(partition) if partition.cross_site => {
                format!("{origin}^{}", partition.cookie_key())
            }
            _ => origin,
        }
    }
}

/// The Web Storage areas as a document's scripts reach them (M34.1): the
/// engine's in one process, the browser's in a renderer. `local` names
/// `localStorage`, otherwise `sessionStorage`.
pub(crate) trait Areas: Send + Sync {
    fn length(&self, local: bool, key: &AreaKey) -> usize;
    fn key(&self, local: bool, key: &AreaKey, index: usize) -> Option<String>;
    fn keys(&self, local: bool, key: &AreaKey) -> Vec<String>;
    fn get(&self, local: bool, key: &AreaKey, item: &str) -> Option<String>;
    /// `setItem`, as [`Area::set`] answers.
    fn set(
        &self,
        local: bool,
        key: &AreaKey,
        item: &str,
        value: &str,
    ) -> Result<Option<Option<String>>, OverQuota>;
    fn remove(&self, local: bool, key: &AreaKey, item: &str) -> Option<String>;
    fn clear(&self, local: bool, key: &AreaKey) -> bool;
    /// The bytes the storage key uses (`navigator.storage.estimate()`,
    /// M36.8).
    fn usage(&self, key: &AreaKey) -> u64;
    /// IndexedDB's `request` of the storage key's databases (M36.7), as
    /// [`crate::idb::serve`] answers it.
    fn idb(&self, key: &AreaKey, request: &str) -> String;
    /// The Cache API's `request` of the storage key's caches (M39), the
    /// bodies it carries in `bytes`, as [`crate::caches::serve`] answers
    /// it.
    fn caches(&self, key: &AreaKey, request: &str, bytes: &[u8]) -> (String, Vec<u8>);
    /// A request of the client's `navigator.serviceWorker` or a
    /// registration's (M39), as
    /// [`crate::service_worker::page::serve`] answers it.
    fn service_worker(&self, client: &crate::service_worker::page::Client, request: &str)
    -> String;
}

/// The answer to a service worker's request where there is no driver.
pub(crate) const NO_SERVICE_WORKERS: &str =
    r#"{"error":{"name":"NotSupportedError","message":"Service workers are not available."}}"#;

/// A tab's Web Storage in an engine of its own (M37): the context's
/// storage, `sessionStorage` the tab's own.
pub(crate) struct TabAreas {
    pub(crate) storage: Arc<Mutex<Storage>>,
    pub(crate) tab: u64,
}

impl TabAreas {
    fn lock(&self) -> std::sync::MutexGuard<'_, Storage> {
        self.storage.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Areas for TabAreas {
    fn usage(&self, key: &AreaKey) -> u64 {
        devtools::usage(&mut self.lock(), &key.serialization())
    }

    fn idb(&self, key: &AreaKey, request: &str) -> String {
        let mut storage = self.lock();
        let origin = key.serialization();
        crate::idb::serve(storage.idb(&origin), &origin, request)
    }

    fn caches(&self, key: &AreaKey, request: &str, bytes: &[u8]) -> (String, Vec<u8>) {
        self.lock()
            .serve_caches(&key.serialization(), request, bytes)
    }

    fn service_worker(
        &self,
        client: &crate::service_worker::page::Client,
        request: &str,
    ) -> String {
        let driver = self.lock().service_worker_driver();
        match driver {
            Some(driver) => crate::service_worker::page::serve(&driver, client, request),
            None => NO_SERVICE_WORKERS.to_owned(),
        }
    }

    fn length(&self, local: bool, key: &AreaKey) -> usize {
        let scope = Scope::of(local, self.tab);
        self.lock().area(scope, &key.serialization()).len()
    }

    fn key(&self, local: bool, key: &AreaKey, index: usize) -> Option<String> {
        let scope = Scope::of(local, self.tab);
        self.lock().area(scope, &key.serialization()).key(index)
    }

    fn keys(&self, local: bool, key: &AreaKey) -> Vec<String> {
        let scope = Scope::of(local, self.tab);
        self.lock()
            .area(scope, &key.serialization())
            .keys()
            .to_vec()
    }

    fn get(&self, local: bool, key: &AreaKey, item: &str) -> Option<String> {
        let scope = Scope::of(local, self.tab);
        self.lock()
            .area(scope, &key.serialization())
            .get(item)
            .map(str::to_owned)
    }

    fn set(
        &self,
        local: bool,
        key: &AreaKey,
        item: &str,
        value: &str,
    ) -> Result<Option<Option<String>>, OverQuota> {
        let scope = Scope::of(local, self.tab);
        self.lock()
            .set_item(scope, &key.serialization(), item, value)
    }

    fn remove(&self, local: bool, key: &AreaKey, item: &str) -> Option<String> {
        let scope = Scope::of(local, self.tab);
        self.lock().remove_item(scope, &key.serialization(), item)
    }

    fn clear(&self, local: bool, key: &AreaKey) -> bool {
        let scope = Scope::of(local, self.tab);
        self.lock().clear_area(scope, &key.serialization())
    }
}

/// The engine's Web Storage as a page's scripts reach it, but without
/// a page (M28): for the `storage` fuzz target and the benchmarks. An
/// origin is named by its serialization; `local` names the
/// `localStorage` areas, and not the `sessionStorage` ones.
pub struct StorageDriver {
    storage: Storage,
    jar: Mutex<Jar>,
}

impl StorageDriver {
    /// Storage keeping what it keeps in `profile`, or nothing; fails as
    /// [`crate::Engine::set_profile`] does.
    pub fn new(profile: Option<Profile>) -> Result<StorageDriver, profile::Error> {
        let mut driver = StorageDriver {
            storage: Storage::new(),
            jar: Mutex::default(),
        };
        if let Some(profile) = profile {
            driver.storage.set_profile(profile, &driver.jar)?;
        }
        Ok(driver)
    }

    /// `setItem`: false when the area cannot hold it.
    pub fn set(&mut self, local: bool, origin: &str, key: &str, value: &str) -> bool {
        self.storage
            .set_item(Scope::of(local, 0), origin, key, value)
            .is_ok()
    }

    /// `removeItem`: the value the item had.
    pub fn remove(&mut self, local: bool, origin: &str, key: &str) -> Option<String> {
        self.storage.remove_item(Scope::of(local, 0), origin, key)
    }

    /// `clear()`: whether the area had an item.
    pub fn clear(&mut self, local: bool, origin: &str) -> bool {
        self.storage.clear_area(Scope::of(local, 0), origin)
    }

    /// `getItem`.
    pub fn get(&mut self, local: bool, origin: &str, key: &str) -> Option<String> {
        self.storage
            .area(Scope::of(local, 0), origin)
            .get(key)
            .map(str::to_owned)
    }

    /// The area's keys and values, in their order.
    pub fn items(&mut self, local: bool, origin: &str) -> Vec<(String, String)> {
        let area = self.storage.area(Scope::of(local, 0), origin);
        let keys = area.keys().to_vec();
        keys.into_iter()
            .map(|key| {
                let value = area.get(&key).unwrap_or_default().to_owned();
                (key, value)
            })
            .collect()
    }

    /// Writes to the profile what changed since it was last written.
    pub fn flush(&mut self) -> Result<(), profile::Error> {
        self.storage.flush(&self.jar, Duration::ZERO)
    }
}

/// A `setItem` that would take an area past [`STORAGE_QUOTA`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OverQuota;

/// The UTF-16 code units of `text`.
fn units(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

impl Area {
    /// The UTF-16 code units its keys and values hold.
    pub(crate) fn used(&self) -> usize {
        self.used
    }

    /// How many items it has.
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    /// The key of the `index`th item, in their order.
    pub(crate) fn key(&mut self, index: usize) -> Option<String> {
        self.keys().get(index).cloned()
    }

    /// The keys, in their order.
    pub(crate) fn keys(&mut self) -> &[String] {
        let items = &self.items;
        self.keys.get_or_insert_with(|| {
            let mut keys: Vec<(&String, u64)> =
                items.iter().map(|(key, item)| (key, item.order)).collect();
            keys.sort_by_key(|&(_, order)| order);
            keys.into_iter().map(|(key, _)| key.clone()).collect()
        })
    }

    /// The value of the item `key`.
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.items.get(key).map(|item| item.value.as_str())
    }

    /// Sets the item `key` to `value` (`setItem`): the value it had,
    /// `Some(None)` for a new one, or `None` when it had this one and
    /// nothing changed; [`OverQuota`] when the area cannot hold it.
    pub(crate) fn set(
        &mut self,
        key: &str,
        value: &str,
    ) -> Result<Option<Option<String>>, OverQuota> {
        let size = units(value);
        if let Some(item) = self.items.get_mut(key) {
            if item.value == value {
                return Ok(None);
            }
            let used = self.used - units(&item.value) + size;
            if used > STORAGE_QUOTA {
                return Err(OverQuota);
            }
            self.used = used;
            return Ok(Some(Some(std::mem::replace(
                &mut item.value,
                value.to_owned(),
            ))));
        }
        let used = self.used + units(key) + size;
        if used > STORAGE_QUOTA {
            return Err(OverQuota);
        }
        self.used = used;
        self.items.insert(
            key.to_owned(),
            Item {
                value: value.to_owned(),
                order: self.next,
            },
        );
        self.next += 1;
        self.keys = None;
        Ok(Some(None))
    }

    /// Removes the item `key` (`removeItem`): its value, `None` when
    /// there was none.
    pub(crate) fn remove(&mut self, key: &str) -> Option<String> {
        let item = self.items.remove(key)?;
        self.used -= units(key) + units(&item.value);
        self.keys = None;
        Some(item.value)
    }

    /// Removes every item (`clear()`): whether there was one.
    pub(crate) fn clear(&mut self) -> bool {
        if self.items.is_empty() {
            return false;
        }
        self.items.clear();
        self.used = 0;
        self.keys = None;
        true
    }

    /// The item `key` with its order, for the profile.
    fn kept(&self, key: &str) -> Option<(&str, u64)> {
        self.items
            .get(key)
            .map(|item| (item.value.as_str(), item.order))
    }

    /// The area the profile's `items` make, in their order.
    fn from_profile(items: Vec<profile::Item>) -> Area {
        let mut area = Area::default();
        for item in items {
            area.used += units(&item.key) + units(&item.value);
            area.next = area.next.max(item.order.saturating_add(1));
            area.items.insert(
                item.key,
                Item {
                    value: item.value,
                    order: item.order,
                },
            );
        }
        area
    }
}

impl Storage {
    pub(crate) fn new() -> Storage {
        Storage {
            profile: None,
            cookies_written: None,
            flushed: None,
            local: HashMap::new(),
            session: HashMap::new(),
            unwritten: HashMap::new(),
            listeners: HashMap::new(),
            next_listener: 1,
            hsts: None,
            hsts_written: None,
            permissions: None,
            permissions_written: None,
            idb: crate::idb::IdbStore::default(),
            idb_unwritten: Vec::new(),
            caches: crate::caches::Store::default(),
            caches_unwritten: Vec::new(),
            registry: crate::service_worker::Registry::default(),
            registry_unwritten: Vec::new(),
            registry_wake: None,
            service_workers: None,
            settings_read: None,
            settings_unwritten: None,
            cache: None,
            cache_read: None,
            cache_unwritten: Vec::new(),
        }
    }

    /// Writes the changes of `cache`, the HTTP cache, to the profile too
    /// (M38), and clears a site's entries with its data.
    pub(crate) fn set_cache(&mut self, cache: Arc<net::Cache>) {
        self.cache = Some(cache);
    }

    /// The HTTP cache, when it has one.
    pub(crate) fn cache(&self) -> Option<&Arc<net::Cache>> {
        self.cache.as_ref()
    }

    /// The settings the profile held when it was given, taken (M37).
    pub(crate) fn take_read_settings(&mut self) -> Option<Vec<u8>> {
        self.settings_read.take()
    }

    /// Keeps the settings `bytes` in the profile at the next write.
    pub(crate) fn keep_settings(&mut self, bytes: Vec<u8>) {
        if self.profile.is_some() {
            self.settings_unwritten = Some(bytes);
        }
    }

    /// Every storage key the storage keeps anything for (M36.8): in the
    /// profile, or in memory not yet written.
    pub(crate) fn sites(&self) -> Vec<String> {
        let mut sites: std::collections::BTreeSet<String> = self
            .profile
            .as_ref()
            .and_then(|profile| profile.storage_keys().ok())
            .unwrap_or_default()
            .into_iter()
            .collect();
        sites.extend(
            self.local
                .iter()
                .filter(|(_, area)| area.len() > 0)
                .map(|(key, _)| key.clone()),
        );
        sites.extend(self.idb.origins());
        sites.extend(self.caches.keys_with_caches());
        sites.extend(self.registry.keys());
        sites.into_iter().collect()
    }

    /// The Cache API's store, `key`'s caches read from the profile if
    /// they were not (M39).
    pub(crate) fn caches(&mut self, key: &str) -> &mut crate::caches::Store {
        if !self.caches.knows(key) {
            let (caches, entries) = self
                .profile
                .as_ref()
                .and_then(|profile| profile.cache_api(key).ok())
                .unwrap_or_default();
            self.caches.load(key, caches, entries);
        }
        &mut self.caches
    }

    /// The service workers' registry, `key`'s registrations read from the
    /// profile if they were not (M39).
    pub(crate) fn service_workers(&mut self, key: &str) -> &mut crate::service_worker::Registry {
        if !self.registry.knows(key) {
            let records = self
                .profile
                .as_ref()
                .and_then(|profile| profile.service_workers(key).ok())
                .unwrap_or_default();
            self.registry.load(key, records);
        }
        &mut self.registry
    }

    /// The registry as it is, no key read.
    pub(crate) fn registry(&mut self) -> &mut crate::service_worker::Registry {
        &mut self.registry
    }

    /// Has the registry's effects done by `wake` when it changes outside
    /// its driver.
    pub(crate) fn set_registry_wake(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.registry_wake = Some(wake);
    }

    /// Has the registry's clients reach `driver` from now on.
    pub(crate) fn set_service_workers(
        &mut self,
        driver: crate::service_worker::WeakServiceWorkers,
    ) {
        self.service_workers = Some(driver);
    }

    /// The registry's driver, while it lives.
    pub(crate) fn service_worker_driver(&self) -> Option<crate::service_worker::ServiceWorkers> {
        self.service_workers.as_ref()?.upgrade()
    }

    /// The registry changed outside its driver: its effects are done.
    pub(crate) fn wake_registry(&self) {
        if let Some(wake) = &self.registry_wake {
            wake();
        }
    }

    /// Answers the Cache API's `request` of `key`'s caches (M39), a body
    /// the profile keeps read from it when matched.
    pub(crate) fn serve_caches(
        &mut self,
        key: &str,
        request: &str,
        bytes: &[u8],
    ) -> (String, Vec<u8>) {
        self.caches(key);
        let profile = self.profile.as_ref();
        let body_of =
            |id: u64| profile.and_then(|profile| profile.cache_api_body(key, id).ok().flatten());
        crate::caches::serve(&mut self.caches, key, request, bytes, &body_of)
    }

    /// IndexedDB's store, `origin`'s databases read from the profile if
    /// they were not (M36.4).
    pub(crate) fn idb(&mut self, origin: &str) -> &mut crate::idb::IdbStore {
        if !self.idb.knows(origin) {
            let (databases, records) = self
                .profile
                .as_ref()
                .and_then(|profile| profile.idb(origin).ok())
                .unwrap_or_default();
            self.idb.load(origin, databases, records);
        }
        &mut self.idb
    }

    /// Keeps `permissions`, the answers the user gave sites, in the
    /// profile too (M33.4): read in when one is given, written as they
    /// change.
    pub(crate) fn set_permissions(&mut self, permissions: Arc<Permissions>) {
        self.permissions = Some(permissions);
    }

    /// Keeps `hsts`, the engine's HSTS hosts, in the profile too
    /// (M33.3): read in when one is given, written as they change.
    pub(crate) fn set_hsts(&mut self, hsts: Arc<Mutex<net::Hsts>>) {
        self.hsts = Some(hsts);
    }

    /// The HSTS hosts' count of changes, when there are hosts to keep.
    fn hsts_changes(&self) -> Option<u64> {
        let hsts = self.hsts.as_ref()?;
        Some(
            hsts.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .changes(),
        )
    }

    /// Keeps what changes in `profile` from now on, and takes its
    /// cookies into `jar`. The `localStorage` areas read so far are
    /// dropped, to be read from it.
    pub(crate) fn set_profile(
        &mut self,
        profile: Profile,
        jar: &Mutex<Jar>,
    ) -> Result<(), profile::Error> {
        let records = profile.cookies()?;
        let mut jar = jar.lock().unwrap_or_else(PoisonError::into_inner);
        let other = jar.restore(&records, SystemTime::now());
        self.cookies_written = (!other).then(|| jar.kept_changes());
        if let Some(hsts) = &self.hsts {
            let records = profile.hsts()?;
            let mut hsts = hsts.lock().unwrap_or_else(PoisonError::into_inner);
            hsts.restore(&records, SystemTime::now());
            self.hsts_written = Some(hsts.changes());
        }
        if let Some(permissions) = &self.permissions {
            permissions.restore(&profile.permissions()?);
            self.permissions_written = Some(permissions.changes());
        }
        self.settings_read = profile.settings()?.into_iter().next();
        self.settings_unwritten = None;
        self.cache_read = match self.cache {
            Some(_) => Some(profile.cache_entries()?),
            None => None,
        };
        self.cache_unwritten.clear();
        self.profile = Some(profile);
        // IndexedDB's databases are the profile's from now on, and the
        // Cache API's caches (M39).
        self.idb = crate::idb::IdbStore::default();
        self.idb.set_writing(true);
        self.idb_unwritten.clear();
        self.caches = crate::caches::Store::default();
        self.caches.set_writing(true);
        self.caches_unwritten.clear();
        self.registry = crate::service_worker::Registry::default();
        self.registry.set_writing(true);
        self.registry_unwritten.clear();
        self.local.clear();
        self.unwritten.clear();
        Ok(())
    }

    /// The area of `origin` in `scope`; a `localStorage` area is read
    /// from the profile at its first use, empty if it cannot be.
    pub(crate) fn area(&mut self, scope: Scope, origin: &str) -> &mut Area {
        if let Scope::Session(tab) = scope {
            return self.session.entry((tab, origin.to_owned())).or_default();
        }
        let profile = &self.profile;
        self.local.entry(origin.to_owned()).or_insert_with(|| {
            profile
                .as_ref()
                .and_then(|profile| profile.local_storage(origin).ok())
                .map(Area::from_profile)
                .unwrap_or_default()
        })
    }

    /// `setItem` of `origin`'s area (HTML 12.2.1), as [`Area::set`]
    /// answers, a change noted to be written.
    pub(crate) fn set_item(
        &mut self,
        scope: Scope,
        origin: &str,
        key: &str,
        value: &str,
    ) -> Result<Option<Option<String>>, OverQuota> {
        let changed = self.area(scope, origin).set(key, value)?;
        if changed.is_some() {
            self.changed(scope, origin, Some(key));
        }
        Ok(changed)
    }

    /// `removeItem` of `origin`'s area: the value it had, the change
    /// noted to be written.
    pub(crate) fn remove_item(&mut self, scope: Scope, origin: &str, key: &str) -> Option<String> {
        let old = self.area(scope, origin).remove(key);
        if old.is_some() {
            self.changed(scope, origin, Some(key));
        }
        old
    }

    /// `clear()` of `origin`'s area: whether it had an item, the change
    /// noted to be written.
    pub(crate) fn clear_area(&mut self, scope: Scope, origin: &str) -> bool {
        let had = self.area(scope, origin).clear();
        if had {
            self.changed(scope, origin, None);
        }
        had
    }

    /// Clears `origin`'s `sessionStorage` areas in every tab (M37): a
    /// site cleared.
    pub(crate) fn clear_sessions(&mut self, origin: &str) {
        for ((_, of), area) in &mut self.session {
            if of == origin {
                area.clear();
            }
        }
    }

    /// Forgets tab `tab`'s `sessionStorage` areas: the tab closed.
    pub(crate) fn forget_tab(&mut self, tab: u64) {
        self.session.retain(|(of, _), _| *of != tab);
    }

    /// Notes that `key` of `origin`'s `localStorage` changed, or, for
    /// `None`, that it was cleared, to be written.
    fn changed(&mut self, scope: Scope, origin: &str, key: Option<&str>) {
        if !scope.is_local() || self.profile.is_none() {
            return;
        }
        let unwritten = self.unwritten.entry(origin.to_owned()).or_default();
        match key {
            Some(key) => {
                unwritten.keys.insert(key.to_owned());
            }
            None => {
                unwritten.cleared = true;
                unwritten.keys.clear();
            }
        }
    }

    /// A new listener's number: from now on it hears of every change
    /// (M37), until [`Storage::forget`].
    pub(crate) fn listen(&mut self) -> u64 {
        let id = self.next_listener;
        self.next_listener += 1;
        self.listeners.insert(id, Listener::default());
        id
    }

    /// Hears nothing more for listener `id`.
    pub(crate) fn forget(&mut self, id: u64) {
        self.listeners.remove(&id);
    }

    /// What listener `id` calls when a change comes for it.
    pub(crate) fn set_waker(&mut self, id: u64, wake: Arc<dyn Fn() + Send + Sync>) {
        if let Some(listener) = self.listeners.get_mut(&id) {
            listener.wake = Some(wake);
        }
    }

    /// Queues `change` for every listener, waking each, to be told to
    /// the other documents of its origin; a listener with
    /// [`MAX_WAITING_CHANGES`] waiting drops its oldest.
    pub(crate) fn broadcast(&mut self, change: Change) {
        for listener in self.listeners.values_mut() {
            if listener.changes.len() == MAX_WAITING_CHANGES {
                listener.changes.remove(0);
            }
            listener.changes.push(change.clone());
            if let Some(wake) = &listener.wake {
                wake();
            }
        }
    }

    /// Queues `message` for every listener, waking each, as
    /// [`Storage::broadcast`] queues a change; its own sender's too, whose
    /// other channels hear it.
    pub(crate) fn post(&mut self, message: Broadcast) {
        for listener in self.listeners.values_mut() {
            if listener.broadcasts.len() == MAX_WAITING_CHANGES {
                listener.broadcasts.remove(0);
            }
            listener.broadcasts.push(message.clone());
            if let Some(wake) = &listener.wake {
                wake();
            }
        }
    }

    /// The messages waiting for listener `id`, taken.
    pub(crate) fn take_broadcasts(&mut self, id: u64) -> Vec<Broadcast> {
        self.listeners
            .get_mut(&id)
            .map(|listener| std::mem::take(&mut listener.broadcasts))
            .unwrap_or_default()
    }

    /// Whether changes or messages wait for listener `id`.
    pub(crate) fn has_changes(&self, id: u64) -> bool {
        self.listeners
            .get(&id)
            .is_some_and(|listener| !listener.changes.is_empty() || !listener.broadcasts.is_empty())
    }

    /// The changes waiting for listener `id`, taken.
    pub(crate) fn take_changes(&mut self, id: u64) -> Vec<Change> {
        self.listeners
            .get_mut(&id)
            .map(|listener| std::mem::take(&mut listener.changes))
            .unwrap_or_default()
    }

    /// When the profile is next to be written, if something waits:
    /// [`FLUSH_INTERVAL`] after the last write.
    pub(crate) fn flush_at(&self, jar: &Mutex<Jar>) -> Option<Duration> {
        self.profile.as_ref()?;
        let count = jar
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .kept_changes();
        let hsts = self.hsts_changes() != self.hsts_written;
        let permissions =
            self.permissions.as_ref().map(|p| p.changes()) != self.permissions_written;
        let waiting = self.cookies_written != Some(count)
            || self.settings_unwritten.is_some()
            || !self.unwritten.is_empty()
            || self.idb.has_written()
            || !self.idb_unwritten.is_empty()
            || self.caches.has_written()
            || !self.caches_unwritten.is_empty()
            || self.registry.has_written()
            || !self.registry_unwritten.is_empty()
            || !self.cache_unwritten.is_empty()
            || self.cache.as_ref().is_some_and(|cache| cache.has_changes());
        (waiting || hsts || permissions).then(|| {
            self.flushed
                .map_or(Duration::ZERO, |at| at + FLUSH_INTERVAL)
        })
    }

    /// Writes what changed since the last write, at `now`. A write that
    /// fails leaves it waiting, to be tried again an interval on.
    pub(crate) fn flush(&mut self, jar: &Mutex<Jar>, now: Duration) -> Result<(), profile::Error> {
        let Some(profile) = &self.profile else {
            return Ok(());
        };
        let mut changes = Changes::new();
        let count = {
            let jar = jar.lock().unwrap_or_else(PoisonError::into_inner);
            let count = jar.kept_changes();
            if self.cookies_written != Some(count) {
                changes.cookies(jar.export());
            }
            count
        };
        let hsts_count = self.hsts_changes();
        if hsts_count != self.hsts_written
            && let Some(hsts) = &self.hsts
        {
            let hsts = hsts.lock().unwrap_or_else(PoisonError::into_inner);
            changes.hsts(hsts.export(SystemTime::now()));
        }
        let permissions_count = self.permissions.as_ref().map(|p| p.changes());
        if permissions_count != self.permissions_written
            && let Some(permissions) = &self.permissions
        {
            changes.permissions(permissions.export());
        }
        // IndexedDB's committed transactions, in the order they committed.
        self.idb_unwritten.extend(self.idb.take_written());
        for written in &self.idb_unwritten {
            match written {
                crate::idb::Written::Database { origin, name, meta } => {
                    changes.idb_database(origin, name, Some(meta.clone()));
                }
                crate::idb::Written::DatabaseDeleted { origin, name } => {
                    changes.idb_database(origin, name, None);
                }
                crate::idb::Written::Record {
                    origin,
                    database,
                    store,
                    key,
                    record,
                } => changes.idb_record((origin, database, store), key, record.clone()),
                crate::idb::Written::StoreDeleted {
                    origin,
                    database,
                    store,
                } => changes.idb_store_deleted(origin, database, store),
            }
        }
        for (origin, unwritten) in &self.unwritten {
            if unwritten.cleared {
                changes.clear(origin);
            }
            let area = self.local.get(origin);
            for key in &unwritten.keys {
                match area.and_then(|area| area.kept(key)) {
                    Some((value, order)) => changes.set_item(origin, key, value, order),
                    None => changes.remove_item(origin, key),
                }
            }
        }
        if let Some(settings) = &self.settings_unwritten {
            changes.settings(vec![settings.clone()]);
        }
        // The Cache API's (M39), in the order they were made.
        self.caches_unwritten.extend(self.caches.take_written());
        for written in &self.caches_unwritten {
            match written {
                crate::caches::Written::Cache { key, id, record } => {
                    changes.cache_api_cache(key, *id, record.clone());
                }
                crate::caches::Written::Entry {
                    key,
                    id,
                    record,
                    body,
                } => changes.cache_api_entry(key, *id, record.clone(), body.clone()),
            }
        }
        // The service workers' registrations (M39).
        self.registry_unwritten.extend(self.registry.take_written());
        for written in &self.registry_unwritten {
            changes.service_worker(&written.key, written.id, written.record.clone());
        }
        // The HTTP cache's, once it has read what the profile held.
        if let Some(cache) = self.cache.as_ref().filter(|_| self.cache_read.is_none()) {
            self.cache_unwritten.extend(cache.take_changes());
        }
        for change in &self.cache_unwritten {
            match change {
                net::cache::Change::Put { id, record, body } => {
                    changes.cache_put(*id, record.clone(), body.clone());
                }
                net::cache::Change::Delete { id } => changes.cache_delete(*id),
            }
        }
        if changes.is_empty() {
            return Ok(());
        }
        self.flushed = Some(now);
        profile.commit(&changes)?;
        self.settings_unwritten = None;
        self.cookies_written = Some(count);
        self.hsts_written = hsts_count;
        self.permissions_written = permissions_count;
        self.unwritten.clear();
        self.idb_unwritten.clear();
        self.cache_unwritten.clear();
        self.caches_unwritten.clear();
        self.registry_unwritten.clear();
        Ok(())
    }
}
