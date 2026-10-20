//! What the browsing keeps (M28, docs/design/storage.md): the profile
//! the engine writes to, and when; and the Web Storage areas (HTML
//! section 12.2), each origin's `localStorage` kept in the profile and
//! the tab's `sessionStorage` in memory. The cookies with an expiry
//! join the jar when the engine is given a profile, an origin's items
//! are read from it at their first use, and what changes is written in
//! one transaction no more than once in [`FLUSH_INTERVAL`].

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use crate::permissions::Permissions;
use std::time::{Duration, SystemTime};

use net::cookies::Jar;
use profile::{Changes, Profile};

use crate::loader::PageId;

#[cfg(test)]
mod isolation_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod web_tests;

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
    /// The tab's `sessionStorage` areas, by origin: the engine's alone.
    session: HashMap<String, Area>,
    /// What changed in the `localStorage` areas since the profile was
    /// written, by origin.
    unwritten: HashMap<String, Unwritten>,
    /// The changes other documents are to hear of in a `storage` event,
    /// for the engine to fire.
    changes: Vec<Change>,
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

/// A change of an area that other documents of its origin hear of
/// (HTML 12.2.1's "broadcast"): `None`s for `null`s, all three for a
/// `clear()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) local: bool,
    pub(crate) origin: String,
    pub(crate) key: Option<String>,
    pub(crate) old: Option<String>,
    pub(crate) new: Option<String>,
    /// The URL of the document that changed it.
    pub(crate) url: String,
    /// That document's page, which hears nothing of it.
    pub(crate) page: PageId,
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
}

impl Areas for Mutex<Storage> {
    fn length(&self, local: bool, key: &AreaKey) -> usize {
        let mut storage = self.lock().unwrap_or_else(PoisonError::into_inner);
        storage.area(local, &key.serialization()).len()
    }

    fn key(&self, local: bool, key: &AreaKey, index: usize) -> Option<String> {
        let mut storage = self.lock().unwrap_or_else(PoisonError::into_inner);
        storage.area(local, &key.serialization()).key(index)
    }

    fn keys(&self, local: bool, key: &AreaKey) -> Vec<String> {
        let mut storage = self.lock().unwrap_or_else(PoisonError::into_inner);
        storage.area(local, &key.serialization()).keys().to_vec()
    }

    fn get(&self, local: bool, key: &AreaKey, item: &str) -> Option<String> {
        let mut storage = self.lock().unwrap_or_else(PoisonError::into_inner);
        storage
            .area(local, &key.serialization())
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
        let mut storage = self.lock().unwrap_or_else(PoisonError::into_inner);
        storage.set_item(local, &key.serialization(), item, value)
    }

    fn remove(&self, local: bool, key: &AreaKey, item: &str) -> Option<String> {
        let mut storage = self.lock().unwrap_or_else(PoisonError::into_inner);
        storage.remove_item(local, &key.serialization(), item)
    }

    fn clear(&self, local: bool, key: &AreaKey) -> bool {
        let mut storage = self.lock().unwrap_or_else(PoisonError::into_inner);
        storage.clear_area(local, &key.serialization())
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
        self.storage.set_item(local, origin, key, value).is_ok()
    }

    /// `removeItem`: the value the item had.
    pub fn remove(&mut self, local: bool, origin: &str, key: &str) -> Option<String> {
        self.storage.remove_item(local, origin, key)
    }

    /// `clear()`: whether the area had an item.
    pub fn clear(&mut self, local: bool, origin: &str) -> bool {
        self.storage.clear_area(local, origin)
    }

    /// `getItem`.
    pub fn get(&mut self, local: bool, origin: &str, key: &str) -> Option<String> {
        self.storage.area(local, origin).get(key).map(str::to_owned)
    }

    /// The area's keys and values, in their order.
    pub fn items(&mut self, local: bool, origin: &str) -> Vec<(String, String)> {
        let area = self.storage.area(local, origin);
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
            changes: Vec::new(),
            hsts: None,
            hsts_written: None,
            permissions: None,
            permissions_written: None,
            idb: crate::idb::IdbStore::default(),
            idb_unwritten: Vec::new(),
        }
    }

    /// IndexedDB's store, `origin`'s databases read from the profile if
    /// they were not (M36.4). The bindings reach it from M36.7.
    #[cfg_attr(not(test), allow(dead_code))]
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
        self.profile = Some(profile);
        // IndexedDB's databases are the profile's from now on.
        self.idb = crate::idb::IdbStore::default();
        self.idb.set_writing(true);
        self.idb_unwritten.clear();
        self.local.clear();
        self.unwritten.clear();
        Ok(())
    }

    /// The `localStorage` area of `origin` if `local`, else its
    /// `sessionStorage` area; a `localStorage` area is read from the
    /// profile at its first use, empty if it cannot be.
    pub(crate) fn area(&mut self, local: bool, origin: &str) -> &mut Area {
        if !local {
            return self.session.entry(origin.to_owned()).or_default();
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
        local: bool,
        origin: &str,
        key: &str,
        value: &str,
    ) -> Result<Option<Option<String>>, OverQuota> {
        let changed = self.area(local, origin).set(key, value)?;
        if changed.is_some() {
            self.changed(local, origin, Some(key));
        }
        Ok(changed)
    }

    /// `removeItem` of `origin`'s area: the value it had, the change
    /// noted to be written.
    pub(crate) fn remove_item(&mut self, local: bool, origin: &str, key: &str) -> Option<String> {
        let old = self.area(local, origin).remove(key);
        if old.is_some() {
            self.changed(local, origin, Some(key));
        }
        old
    }

    /// `clear()` of `origin`'s area: whether it had an item, the change
    /// noted to be written.
    pub(crate) fn clear_area(&mut self, local: bool, origin: &str) -> bool {
        let had = self.area(local, origin).clear();
        if had {
            self.changed(local, origin, None);
        }
        had
    }

    /// Notes that `key` of `origin`'s `localStorage` changed, or, for
    /// `None`, that it was cleared, to be written.
    fn changed(&mut self, local: bool, origin: &str, key: Option<&str>) {
        if !local || self.profile.is_none() {
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

    /// Queues `change` for the other documents of its origin.
    pub(crate) fn broadcast(&mut self, change: Change) {
        self.changes.push(change);
    }

    /// Whether changes wait to be told to other documents.
    pub(crate) fn has_changes(&self) -> bool {
        !self.changes.is_empty()
    }

    /// The changes queued for other documents, taken.
    pub(crate) fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.changes)
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
            || !self.unwritten.is_empty()
            || self.idb.has_written()
            || !self.idb_unwritten.is_empty();
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
        if changes.is_empty() {
            return Ok(());
        }
        self.flushed = Some(now);
        profile.commit(&changes)?;
        self.cookies_written = Some(count);
        self.hsts_written = hsts_count;
        self.permissions_written = permissions_count;
        self.unwritten.clear();
        self.idb_unwritten.clear();
        Ok(())
    }
}
