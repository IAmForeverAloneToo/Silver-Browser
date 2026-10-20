//! The profile: what the user's browsing keeps on disk (roadmap M28.1,
//! decision D10; see `docs/design/storage.md`).
//!
//! A leaf crate, the only one that knows `redb`. A profile is one
//! database file, `profile.redb`, in a directory the shell chooses, or
//! a database in memory for tests and for a browser that keeps nothing.
//! It holds each origin's `localStorage` items and the cookies that
//! outlive the session, and nothing reads across origins: an origin's
//! items are the range of the table its serialization begins.
//!
//! A profile is written by [`Changes`], each committed in one
//! transaction that is durable when [`Profile::commit`] returns. What
//! the engine keeps it keeps in memory first and commits in batches.
//!
//! The cookies are records of `net`'s encoding, which this crate keeps
//! without reading: their type is `net`'s.
//!
//! The values are encrypted at rest (M33, D27): each sealed with
//! AES-256-GCM under the profile's data key, a random nonce of its own,
//! and its table and key as associated data, so that a value cannot be
//! moved to another key. The data key is made with the profile and kept
//! in it wrapped by a [`KeyWrap`], the operating system's key for the
//! user where the shell has one. The keys of the tables, origins and
//! item keys, stay readable. A value that does not open, of a profile
//! copied to another user or machine, is left out.

#![forbid(unsafe_code)]

use std::fmt;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use redb::{
    Database, ReadableDatabase, ReadableTable, StorageBackend, TableDefinition, TableHandle,
    WriteTransaction,
};

#[cfg(test)]
mod tests;

/// The file a profile directory holds.
pub const FILE: &str = "profile.redb";

/// The version of the tables' layout this code writes. A profile of a
/// newer one is refused, one of an older one upgraded: 2 seals the
/// values (M33); 3 adds IndexedDB's tables (M36), which an older profile
/// is given empty.
pub const SCHEMA: u64 = 3;

/// The length of the data key: AES-256's.
const KEY_BYTES: usize = 32;

/// The memory `redb` may keep of the file's pages.
const CACHE_BYTES: usize = 16 << 20;

/// What the profile is: its schema's version under `"schema"`.
const META: TableDefinition<&str, u64> = TableDefinition::new("meta");

/// Each origin's `localStorage` items: keyed by the origin's
/// serialization and the item's key, each the order of its first
/// setting and its value, sealed.
const LOCAL_STORAGE: TableDefinition<(&str, &str), (u64, &[u8])> =
    TableDefinition::new("local_items");

/// Schema 1's, the values in the clear: read once, to be sealed.
const LOCAL_STORAGE_1: TableDefinition<(&str, &str), (u64, &str)> =
    TableDefinition::new("local_storage");

/// The data key, wrapped, under `"data"`, and the name of what wrapped
/// it under `"wrap"`.
const KEY: TableDefinition<&str, &[u8]> = TableDefinition::new("key");

/// The cookies kept, in the order given, as `net` encodes them.
const COOKIES: TableDefinition<u64, &[u8]> = TableDefinition::new("cookies");

/// The HSTS hosts kept (M33.3), as `net` encodes them.
const HSTS: TableDefinition<u64, &[u8]> = TableDefinition::new("hsts");

/// Each storage key's IndexedDB databases (M36.4): keyed by the storage
/// key and the database's name, each its description as the engine
/// encodes it, sealed.
const IDB_DATABASES: TableDefinition<(&str, &str), &[u8]> = TableDefinition::new("idb_databases");

/// Their records: keyed by the storage key, the database, the object
/// store and the record's key as the engine encodes keys, so that their
/// bytes sort as the keys do; each the record as the engine encodes it,
/// sealed.
const IDB_RECORDS: TableDefinition<(&str, &str, &str, &[u8]), &[u8]> =
    TableDefinition::new("idb_records");

/// The permissions sites were given (M33.4), as the engine encodes them.
const PERMISSIONS: TableDefinition<u64, &[u8]> = TableDefinition::new("permissions");

/// A profile open: one database, one writer.
pub struct Profile {
    /// `None` once the store was found damaged: never used again, nor
    /// dropped, what it holds being past trusting.
    db: Mutex<Option<Database>>,
    key: DataKey,
}

/// How the profile's data key is kept (D27): wrapped by the operating
/// system's key for the user, which the shell's platform module reaches,
/// or as it is where there is none (G98).
#[derive(Debug, Clone, Copy)]
pub struct KeyWrap {
    /// Its name, kept with the wrapped key: what can unwrap it.
    pub name: &'static str,
    pub wrap: fn(&[u8]) -> Option<Vec<u8>>,
    pub unwrap: fn(&[u8]) -> Option<Vec<u8>>,
}

fn as_it_is(key: &[u8]) -> Option<Vec<u8>> {
    Some(key.to_vec())
}

impl KeyWrap {
    /// The key kept as it is: what hides the values from a search of the
    /// disk, and nothing more.
    pub const NONE: KeyWrap = KeyWrap {
        name: "none",
        wrap: as_it_is,
        unwrap: as_it_is,
    };
}

/// The data key, which seals and opens the values.
struct DataKey(LessSafeKey);

impl DataKey {
    fn new(bytes: &[u8]) -> Option<DataKey> {
        let key = UnboundKey::new(&AES_256_GCM, bytes).ok()?;
        Some(DataKey(LessSafeKey::new(key)))
    }

    /// `value` sealed for `table` and the key `at`: the nonce, then the
    /// ciphertext and its tag.
    fn seal(&self, table: &str, at: &[u8], value: &[u8]) -> Result<Vec<u8>, Error> {
        let sealing = || Error("the value could not be sealed".to_owned());
        let mut nonce = [0u8; NONCE_LEN];
        aws_lc_rs::rand::fill(&mut nonce).map_err(|_| sealing())?;
        let mut sealed = value.to_vec();
        self.0
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(associated(table, at)),
                &mut sealed,
            )
            .map_err(|_| sealing())?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    /// The value `sealed` holds, if it opens for `table` and `at`.
    fn open(&self, table: &str, at: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
        let nonce = Nonce::try_assume_unique_for_key(sealed.get(..NONCE_LEN)?).ok()?;
        let mut value = sealed[NONCE_LEN..].to_vec();
        let opened = self
            .0
            .open_in_place(nonce, Aad::from(associated(table, at)), &mut value)
            .ok()?;
        Some(opened.to_vec())
    }
}

/// What a value is sealed with beside the key: its table's name and its
/// key in that table.
fn associated(table: &str, at: &[u8]) -> Vec<u8> {
    let mut data = table.as_bytes().to_vec();
    data.push(0);
    data.extend_from_slice(at);
    data
}

/// A local item's key as associated data: its origin, a NUL, its key.
/// Where an IndexedDB record is, as its sealing's associated data: each
/// part after its length.
fn record_at(parts: [&[u8]; 4]) -> Vec<u8> {
    let mut at = Vec::new();
    for part in parts {
        at.extend_from_slice(&(part.len() as u64).to_le_bytes());
        at.extend_from_slice(part);
    }
    at
}

fn item_at(origin: &str, key: &str) -> Vec<u8> {
    let mut at = origin.as_bytes().to_vec();
    at.push(0);
    at.extend_from_slice(key.as_bytes());
    at
}

/// The error of a profile whose store panicked.
fn damaged() -> Error {
    Error("the profile is damaged".to_owned())
}

impl fmt::Debug for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Profile").finish_non_exhaustive()
    }
}

/// Why a profile could not be opened.
#[derive(Debug)]
pub enum OpenError {
    /// Another browser has it open.
    InUse,
    /// It was written by a newer browser, whose tables this one does
    /// not know: the schema's version.
    Newer(u64),
    /// It cannot be read: not a profile, or damaged past repair; what
    /// went wrong.
    Unreadable(String),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::InUse => write!(f, "another browser has it open"),
            OpenError::Newer(schema) => {
                write!(f, "a newer browser wrote it (schema {schema})")
            }
            OpenError::Unreadable(why) => write!(f, "it cannot be read: {why}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// A read or a write of an open profile that failed: the store's
/// error, as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn failed(error: impl Into<redb::Error>) -> Error {
    Error(error.into().to_string())
}

/// An item of an origin's `localStorage` as kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub key: String,
    pub value: String,
    /// The order of its first setting: an area's items come back in
    /// it.
    pub order: u64,
}

/// What a commit changes, applied in the order given.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Changes {
    local: Vec<Local>,
    idb: Vec<Idb>,
    cookies: Option<Vec<Vec<u8>>>,
    hsts: Option<Vec<Vec<u8>>>,
    permissions: Option<Vec<Vec<u8>>>,
}

/// A change of IndexedDB's (M36.4).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Idb {
    /// A database's description, or none for a database deleted with
    /// its records.
    Database {
        origin: String,
        name: String,
        meta: Option<Vec<u8>>,
    },
    /// A record, or none for one deleted.
    Record {
        origin: String,
        database: String,
        store: String,
        key: Vec<u8>,
        record: Option<Vec<u8>>,
    },
    /// Every record of an object store deleted.
    StoreDeleted {
        origin: String,
        database: String,
        store: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Local {
    Set {
        origin: String,
        key: String,
        value: String,
        order: u64,
    },
    Remove {
        origin: String,
        key: String,
    },
    Clear {
        origin: String,
    },
}

impl Changes {
    /// No changes.
    pub fn new() -> Changes {
        Changes::default()
    }

    /// Whether there is nothing to commit.
    pub fn is_empty(&self) -> bool {
        self.local.is_empty()
            && self.idb.is_empty()
            && self.cookies.is_none()
            && self.hsts.is_none()
            && self.permissions.is_none()
    }

    /// Sets `origin`'s item `key` to `value`, `order` its place.
    pub fn set_item(&mut self, origin: &str, key: &str, value: &str, order: u64) {
        self.local.push(Local::Set {
            origin: origin.to_owned(),
            key: key.to_owned(),
            value: value.to_owned(),
            order,
        });
    }

    /// Removes `origin`'s item `key`, if it has one.
    pub fn remove_item(&mut self, origin: &str, key: &str) {
        self.local.push(Local::Remove {
            origin: origin.to_owned(),
            key: key.to_owned(),
        });
    }

    /// Removes every item of `origin`.
    pub fn clear(&mut self, origin: &str) {
        self.local.push(Local::Clear {
            origin: origin.to_owned(),
        });
    }

    /// Keeps `origin`'s IndexedDB database `name`'s description, `meta`
    /// (M36.4), or deletes the database and its records for none.
    pub fn idb_database(&mut self, origin: &str, name: &str, meta: Option<Vec<u8>>) {
        self.idb.push(Idb::Database {
            origin: origin.to_owned(),
            name: name.to_owned(),
            meta,
        });
    }

    /// Keeps a record of `origin`'s database `database`'s object store
    /// `store` under the key `key` encodes, or deletes it for none.
    pub fn idb_record(
        &mut self,
        (origin, database, store): (&str, &str, &str),
        key: &[u8],
        record: Option<Vec<u8>>,
    ) {
        self.idb.push(Idb::Record {
            origin: origin.to_owned(),
            database: database.to_owned(),
            store: store.to_owned(),
            key: key.to_vec(),
            record,
        });
    }

    /// Deletes every record of an object store.
    pub fn idb_store_deleted(&mut self, origin: &str, database: &str, store: &str) {
        self.idb.push(Idb::StoreDeleted {
            origin: origin.to_owned(),
            database: database.to_owned(),
            store: store.to_owned(),
        });
    }

    /// Replaces the cookies kept with `cookies`, records of `net`'s
    /// encoding, in their order.
    pub fn cookies(&mut self, cookies: Vec<Vec<u8>>) {
        self.cookies = Some(cookies);
    }

    /// Replaces the HSTS hosts kept with `hosts`, records of `net`'s
    /// encoding (M33.3).
    pub fn hsts(&mut self, hosts: Vec<Vec<u8>>) {
        self.hsts = Some(hosts);
    }

    /// Replaces the permissions kept with `permissions`, records of the
    /// engine's encoding (M33.4).
    pub fn permissions(&mut self, permissions: Vec<Vec<u8>>) {
        self.permissions = Some(permissions);
    }
}

impl Profile {
    /// Opens the profile in the directory `dir`, making the directory
    /// and the file if they are not there, its data key kept by `wrap`.
    pub fn open_with(dir: &Path, wrap: KeyWrap) -> Result<Profile, OpenError> {
        std::fs::create_dir_all(dir).map_err(|e| OpenError::Unreadable(e.to_string()))?;
        let path = dir.join(FILE);
        Profile::opened(wrap, || {
            Database::builder()
                .set_cache_size(CACHE_BYTES)
                .create(&path)
        })
    }

    /// [`Profile::open_with`], the data key kept as it is.
    pub fn open(dir: &Path) -> Result<Profile, OpenError> {
        Profile::open_with(dir, KeyWrap::NONE)
    }

    /// Opens the profile in `memory`, empty or as an earlier profile
    /// left it, its data key kept by `wrap`. The profile before must have
    /// been dropped: memory has no lock to say it is open.
    pub fn in_memory_with(memory: &Memory, wrap: KeyWrap) -> Result<Profile, OpenError> {
        let memory = memory.clone();
        Profile::opened(wrap, move || {
            Database::builder()
                .set_cache_size(CACHE_BYTES)
                .create_with_backend(memory)
        })
    }

    /// [`Profile::in_memory_with`], the data key kept as it is.
    pub fn in_memory(memory: &Memory) -> Result<Profile, OpenError> {
        Profile::in_memory_with(memory, KeyWrap::NONE)
    }

    /// The profile `open` opens, its schema checked: refused as damaged
    /// when `redb` panics, as it does on damage its checksums do not
    /// reach (it unwraps some of what it reads).
    fn opened(
        wrap: KeyWrap,
        open: impl FnOnce() -> Result<Database, redb::DatabaseError>,
    ) -> Result<Profile, OpenError> {
        let damaged = || OpenError::Unreadable("it is damaged".to_owned());
        let db = catch_unwind(AssertUnwindSafe(open))
            .map_err(|_| damaged())?
            .map_err(refused)?;
        match catch_unwind(AssertUnwindSafe(|| Profile::prepare(&db, wrap))) {
            Ok(Ok(key)) => Ok(Profile {
                db: Mutex::new(Some(db)),
                key,
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => {
                std::mem::forget(db);
                Err(damaged())
            }
        }
    }

    /// `f` of the store, its panic the profile's error: the store is
    /// then not used again, nor dropped.
    fn guarded<T>(&self, f: impl FnOnce(&Database) -> Result<T, Error>) -> Result<T, Error> {
        let mut db = self.db.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(store) = db.as_ref() else {
            return Err(damaged());
        };
        match catch_unwind(AssertUnwindSafe(|| f(store))) {
            Ok(result) => result,
            Err(_) => {
                if let Some(store) = db.take() {
                    std::mem::forget(store);
                }
                Err(damaged())
            }
        }
    }

    /// Checks the schema of a profile just opened, writes it into a
    /// new one, and makes the tables, so that a reader finds them; reads
    /// the data key, or makes it; and seals the values of a profile of
    /// schema 1 in place.
    fn prepare(db: &Database, wrap: KeyWrap) -> Result<DataKey, OpenError> {
        let unreadable = |error: Error| OpenError::Unreadable(error.0);
        let txn = db.begin_write().map_err(|e| unreadable(failed(e)))?;
        let key = {
            let mut meta = txn.open_table(META).map_err(|e| unreadable(failed(e)))?;
            let schema = meta
                .get("schema")
                .map_err(|e| unreadable(failed(e)))?
                .map(|guard| guard.value());
            if let Some(schema) = schema.filter(|&schema| schema > SCHEMA) {
                return Err(OpenError::Newer(schema));
            }
            meta.insert("schema", SCHEMA)
                .map_err(|e| unreadable(failed(e)))?;
            let key = Profile::data_key(&txn, wrap).map_err(unreadable)?;
            if schema == Some(1) {
                Profile::seal_schema_1(&txn, &key).map_err(unreadable)?;
            }
            txn.open_table(LOCAL_STORAGE)
                .map_err(|e| unreadable(failed(e)))?;
            txn.open_table(COOKIES).map_err(|e| unreadable(failed(e)))?;
            txn.open_table(HSTS).map_err(|e| unreadable(failed(e)))?;
            txn.open_table(IDB_DATABASES)
                .map_err(|e| unreadable(failed(e)))?;
            txn.open_table(IDB_RECORDS)
                .map_err(|e| unreadable(failed(e)))?;
            txn.open_table(PERMISSIONS)
                .map_err(|e| unreadable(failed(e)))?;
            key
        };
        txn.commit().map_err(|e| unreadable(failed(e)))?;
        Ok(key)
    }

    /// The profile's data key: unwrapped by `wrap`, or by nothing when it
    /// was kept as it is; else a new one, kept wrapped. A key wrapped by
    /// something else, or that does not unwrap, is replaced: what it
    /// sealed will not open, and is left out.
    fn data_key(txn: &WriteTransaction, wrap: KeyWrap) -> Result<DataKey, Error> {
        let mut table = txn.open_table(KEY).map_err(failed)?;
        let stored = |name: &str| -> Result<Option<Vec<u8>>, Error> {
            Ok(table.get(name).map_err(failed)?.map(|v| v.value().to_vec()))
        };
        let (data, by) = (stored("data")?, stored("wrap")?);
        let unwrapped = match (&data, by.as_deref()) {
            (Some(data), Some(by)) if by == wrap.name.as_bytes() => (wrap.unwrap)(data),
            (Some(data), Some(b"none")) => Some(data.clone()),
            _ => None,
        };
        let bytes = match unwrapped.filter(|bytes| bytes.len() == KEY_BYTES) {
            Some(bytes) if by.as_deref() == Some(wrap.name.as_bytes()) => bytes,
            unwrapped => {
                let bytes = match unwrapped {
                    // Kept as it was: wrapped now.
                    Some(bytes) => bytes,
                    None => {
                        let mut bytes = vec![0u8; KEY_BYTES];
                        aws_lc_rs::rand::fill(&mut bytes)
                            .map_err(|_| Error("no random key could be made".to_owned()))?;
                        bytes
                    }
                };
                let (wrapped, by) = match (wrap.wrap)(&bytes) {
                    Some(wrapped) => (wrapped, wrap.name),
                    None => (bytes.clone(), KeyWrap::NONE.name),
                };
                table.insert("data", wrapped.as_slice()).map_err(failed)?;
                table.insert("wrap", by.as_bytes()).map_err(failed)?;
                bytes
            }
        };
        DataKey::new(&bytes).ok_or_else(|| Error("the key is not one".to_owned()))
    }

    /// Seals the values of a profile of schema 1 (M28's), in the clear
    /// until now: its `localStorage` items into the table that holds
    /// them sealed, and its records in place.
    fn seal_schema_1(txn: &WriteTransaction, key: &DataKey) -> Result<(), Error> {
        let mut items = Vec::new();
        {
            let old = txn.open_table(LOCAL_STORAGE_1).map_err(failed)?;
            for entry in old.iter().map_err(failed)? {
                let (at, value) = entry.map_err(failed)?;
                let (origin, item) = at.value();
                let (order, value) = value.value();
                let sealed = key.seal(
                    LOCAL_STORAGE.name(),
                    &item_at(origin, item),
                    value.as_bytes(),
                )?;
                items.push((origin.to_owned(), item.to_owned(), order, sealed));
            }
        }
        txn.delete_table(LOCAL_STORAGE_1).map_err(failed)?;
        let mut table = txn.open_table(LOCAL_STORAGE).map_err(failed)?;
        for (origin, item, order, sealed) in &items {
            table
                .insert(
                    (origin.as_str(), item.as_str()),
                    (*order, sealed.as_slice()),
                )
                .map_err(failed)?;
        }
        for definition in [COOKIES, HSTS, PERMISSIONS] {
            let mut table = txn.open_table(definition).map_err(failed)?;
            let mut records = Vec::new();
            for entry in table.iter().map_err(failed)? {
                let (index, record) = entry.map_err(failed)?;
                records.push((index.value(), record.value().to_vec()));
            }
            for (index, record) in records {
                let sealed = key.seal(definition.name(), &index.to_le_bytes(), &record)?;
                table.insert(index, sealed.as_slice()).map_err(failed)?;
            }
        }
        Ok(())
    }

    /// The `localStorage` items of `origin`, in their order; an item
    /// whose value does not open is left out.
    pub fn local_storage(&self, origin: &str) -> Result<Vec<Item>, Error> {
        self.guarded(|db| self.read_local(db, origin))
    }

    fn read_local(&self, db: &Database, origin: &str) -> Result<Vec<Item>, Error> {
        let txn = db.begin_read().map_err(failed)?;
        let table = txn.open_table(LOCAL_STORAGE).map_err(failed)?;
        let mut items = Vec::new();
        for entry in table.range((origin, "")..).map_err(failed)? {
            let (key, value) = entry.map_err(failed)?;
            let (owner, key) = key.value();
            if owner != origin {
                break;
            }
            let (order, sealed) = value.value();
            let opened = self
                .key
                .open(LOCAL_STORAGE.name(), &item_at(owner, key), sealed)
                .and_then(|value| String::from_utf8(value).ok());
            if let Some(value) = opened {
                items.push(Item {
                    key: key.to_owned(),
                    value,
                    order,
                });
            }
        }
        items.sort_by_key(|item| item.order);
        Ok(items)
    }

    /// `origin`'s IndexedDB databases (M36.4): each one's name and
    /// description, and each record's database, object store, encoded
    /// key and encoding. What does not open is left out.
    #[allow(clippy::type_complexity)]
    pub fn idb(
        &self,
        origin: &str,
    ) -> Result<
        (
            Vec<(String, Vec<u8>)>,
            Vec<(String, String, Vec<u8>, Vec<u8>)>,
        ),
        Error,
    > {
        self.guarded(|db| {
            let txn = db.begin_read().map_err(failed)?;
            let databases_table = txn.open_table(IDB_DATABASES).map_err(failed)?;
            let mut databases = Vec::new();
            for entry in databases_table.range((origin, "")..).map_err(failed)? {
                let (key, value) = entry.map_err(failed)?;
                let (owner, name) = key.value();
                if owner != origin {
                    break;
                }
                let at = record_at([owner.as_bytes(), name.as_bytes(), b"", b""]);
                if let Some(meta) = self.key.open(IDB_DATABASES.name(), &at, value.value()) {
                    databases.push((name.to_owned(), meta));
                }
            }
            let records_table = txn.open_table(IDB_RECORDS).map_err(failed)?;
            let mut records = Vec::new();
            for entry in records_table
                .range((origin, "", "", &[][..])..)
                .map_err(failed)?
            {
                let (key, value) = entry.map_err(failed)?;
                let (owner, database, store, record_key) = key.value();
                if owner != origin {
                    break;
                }
                let at = record_at([
                    owner.as_bytes(),
                    database.as_bytes(),
                    store.as_bytes(),
                    record_key,
                ]);
                if let Some(record) = self.key.open(IDB_RECORDS.name(), &at, value.value()) {
                    records.push((
                        database.to_owned(),
                        store.to_owned(),
                        record_key.to_vec(),
                        record,
                    ));
                }
            }
            Ok((databases, records))
        })
    }

    /// The storage keys the profile keeps `localStorage` items or
    /// IndexedDB databases for (M36.8), in order, each once.
    pub fn storage_keys(&self) -> Result<Vec<String>, Error> {
        self.guarded(|db| {
            let txn = db.begin_read().map_err(failed)?;
            let mut keys = std::collections::BTreeSet::new();
            let local = txn.open_table(LOCAL_STORAGE).map_err(failed)?;
            for entry in local.iter().map_err(failed)? {
                let (key, _) = entry.map_err(failed)?;
                keys.insert(key.value().0.to_owned());
            }
            let databases = txn.open_table(IDB_DATABASES).map_err(failed)?;
            for entry in databases.iter().map_err(failed)? {
                let (key, _) = entry.map_err(failed)?;
                keys.insert(key.value().0.to_owned());
            }
            Ok(keys.into_iter().collect())
        })
    }

    /// The cookies kept, records of `net`'s encoding, in their order.
    pub fn cookies(&self) -> Result<Vec<Vec<u8>>, Error> {
        self.guarded(|db| self.read_records(db, COOKIES))
    }

    /// The HSTS hosts kept (M33.3), records of `net`'s encoding.
    pub fn hsts(&self) -> Result<Vec<Vec<u8>>, Error> {
        self.guarded(|db| self.read_records(db, HSTS))
    }

    /// The permissions sites were given (M33.4), records of the engine's
    /// encoding.
    pub fn permissions(&self) -> Result<Vec<Vec<u8>>, Error> {
        self.guarded(|db| self.read_records(db, PERMISSIONS))
    }

    /// The records of `definition`, in their order; one that does not
    /// open is left out.
    fn read_records(
        &self,
        db: &Database,
        definition: TableDefinition<u64, &[u8]>,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let txn = db.begin_read().map_err(failed)?;
        let table = txn.open_table(definition).map_err(failed)?;
        let mut records = Vec::new();
        for entry in table.iter().map_err(failed)? {
            let (index, sealed) = entry.map_err(failed)?;
            let at = index.value().to_le_bytes();
            if let Some(record) = self.key.open(definition.name(), &at, sealed.value()) {
                records.push(record);
            }
        }
        Ok(records)
    }

    /// Writes `changes` in one transaction, durable when this returns.
    pub fn commit(&self, changes: &Changes) -> Result<(), Error> {
        if changes.is_empty() {
            return Ok(());
        }
        self.guarded(|db| self.write(db, changes))
    }

    fn write(&self, db: &Database, changes: &Changes) -> Result<(), Error> {
        let txn = db.begin_write().map_err(failed)?;
        {
            let mut table = txn.open_table(LOCAL_STORAGE).map_err(failed)?;
            for change in &changes.local {
                match change {
                    Local::Set {
                        origin,
                        key,
                        value,
                        order,
                    } => {
                        let at = item_at(origin, key);
                        let sealed = self.key.seal(LOCAL_STORAGE.name(), &at, value.as_bytes())?;
                        table
                            .insert((origin.as_str(), key.as_str()), (*order, sealed.as_slice()))
                            .map_err(failed)?;
                    }
                    Local::Remove { origin, key } => {
                        table
                            .remove((origin.as_str(), key.as_str()))
                            .map_err(failed)?;
                    }
                    Local::Clear { origin } => {
                        let mut keys = Vec::new();
                        for entry in table.range((origin.as_str(), "")..).map_err(failed)? {
                            let (key, _) = entry.map_err(failed)?;
                            let (owner, key) = key.value();
                            if owner != origin {
                                break;
                            }
                            keys.push(key.to_owned());
                        }
                        for key in keys {
                            table
                                .remove((origin.as_str(), key.as_str()))
                                .map_err(failed)?;
                        }
                    }
                }
            }
        }
        self.write_idb(&txn, &changes.idb)?;
        let replaced = [
            (COOKIES, &changes.cookies),
            (HSTS, &changes.hsts),
            (PERMISSIONS, &changes.permissions),
        ];
        for (definition, records) in replaced {
            let Some(records) = records else {
                continue;
            };
            let mut table = txn.open_table(definition).map_err(failed)?;
            table.retain(|_, _| false).map_err(failed)?;
            for (index, record) in (0u64..).zip(records) {
                let sealed = self
                    .key
                    .seal(definition.name(), &index.to_le_bytes(), record)?;
                table.insert(index, sealed.as_slice()).map_err(failed)?;
            }
        }
        txn.commit().map_err(failed)
    }
}

impl Profile {
    /// Writes IndexedDB's `changes` in `txn`, in order.
    fn write_idb(&self, txn: &WriteTransaction, changes: &[Idb]) -> Result<(), Error> {
        if changes.is_empty() {
            return Ok(());
        }
        let mut databases = txn.open_table(IDB_DATABASES).map_err(failed)?;
        let mut records = txn.open_table(IDB_RECORDS).map_err(failed)?;
        // The records of a database, or of one of its stores, removed.
        let drop_records = |records: &mut redb::Table<(&str, &str, &str, &[u8]), &[u8]>,
                            origin: &str,
                            database: &str,
                            store: Option<&str>|
         -> Result<(), Error> {
            let mut keys: Vec<(String, Vec<u8>)> = Vec::new();
            let start = (origin, database, store.unwrap_or_default(), &[][..]);
            for entry in records.range(start..).map_err(failed)? {
                let (key, _) = entry.map_err(failed)?;
                let (o, d, s, k) = key.value();
                if o != origin || d != database || store.is_some_and(|store| s != store) {
                    break;
                }
                keys.push((s.to_owned(), k.to_vec()));
            }
            for (s, k) in keys {
                records
                    .remove((origin, database, s.as_str(), k.as_slice()))
                    .map_err(failed)?;
            }
            Ok(())
        };
        for change in changes {
            match change {
                Idb::Database { origin, name, meta } => {
                    let at = record_at([origin.as_bytes(), name.as_bytes(), b"", b""]);
                    match meta {
                        Some(meta) => {
                            let sealed = self.key.seal(IDB_DATABASES.name(), &at, meta)?;
                            databases
                                .insert((origin.as_str(), name.as_str()), sealed.as_slice())
                                .map_err(failed)?;
                        }
                        None => {
                            databases
                                .remove((origin.as_str(), name.as_str()))
                                .map_err(failed)?;
                            drop_records(&mut records, origin, name, None)?;
                        }
                    }
                }
                Idb::Record {
                    origin,
                    database,
                    store,
                    key,
                    record,
                } => {
                    let place = (
                        origin.as_str(),
                        database.as_str(),
                        store.as_str(),
                        key.as_slice(),
                    );
                    match record {
                        Some(record) => {
                            let at = record_at([
                                origin.as_bytes(),
                                database.as_bytes(),
                                store.as_bytes(),
                                key,
                            ]);
                            let sealed = self.key.seal(IDB_RECORDS.name(), &at, record)?;
                            records.insert(place, sealed.as_slice()).map_err(failed)?;
                        }
                        None => {
                            records.remove(place).map_err(failed)?;
                        }
                    }
                }
                Idb::StoreDeleted {
                    origin,
                    database,
                    store,
                } => drop_records(&mut records, origin, database, Some(store))?,
            }
        }
        Ok(())
    }
}

/// Why opening the store failed, as the profile's reason.
fn refused(error: redb::DatabaseError) -> OpenError {
    match error {
        redb::DatabaseError::DatabaseAlreadyOpen => OpenError::InUse,
        error => OpenError::Unreadable(error.to_string()),
    }
}

/// A profile's bytes in memory, which outlive the profile on them: a
/// clone is the same memory, so a profile dropped can be opened again,
/// as from a file. For tests, fuzzing, and a browser that keeps
/// nothing.
#[derive(Debug, Default, Clone)]
pub struct Memory(Arc<RwLock<Vec<u8>>>);

impl Memory {
    /// Empty memory, for a new profile.
    pub fn new() -> Memory {
        Memory::default()
    }

    /// A copy of the bytes as they are now, apart from these: what a
    /// file would hold if the browser stopped here.
    pub fn snapshot(&self) -> Memory {
        let bytes = self.0.read().unwrap_or_else(PoisonError::into_inner);
        Memory(Arc::new(RwLock::new(bytes.clone())))
    }

    /// Changes the byte at `at`, if it holds one, by `mask`, as a
    /// damaged disk might: for tests and fuzzing.
    pub fn flip(&self, at: usize, mask: u8) {
        let mut bytes = self.0.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(byte) = bytes.get_mut(at) {
            *byte ^= mask;
        }
    }

    /// How many bytes it holds.
    pub fn len(&self) -> usize {
        self.0.read().unwrap_or_else(PoisonError::into_inner).len()
    }

    /// Whether it holds nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn out_of_range() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "out of range")
}

impl StorageBackend for Memory {
    fn len(&self) -> Result<u64, io::Error> {
        Ok(Memory::len(self) as u64)
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), io::Error> {
        let bytes = self.0.read().unwrap_or_else(PoisonError::into_inner);
        let start = usize::try_from(offset).map_err(|_| out_of_range())?;
        let end = start.checked_add(out.len()).ok_or_else(out_of_range)?;
        out.copy_from_slice(bytes.get(start..end).ok_or_else(out_of_range)?);
        Ok(())
    }

    fn set_len(&self, len: u64) -> Result<(), io::Error> {
        let len = usize::try_from(len).map_err(|_| out_of_range())?;
        let mut bytes = self.0.write().unwrap_or_else(PoisonError::into_inner);
        bytes.resize(len, 0);
        Ok(())
    }

    fn sync_data(&self) -> Result<(), io::Error> {
        Ok(())
    }

    fn write(&self, offset: u64, data: &[u8]) -> Result<(), io::Error> {
        let mut bytes = self.0.write().unwrap_or_else(PoisonError::into_inner);
        let start = usize::try_from(offset).map_err(|_| out_of_range())?;
        let end = start.checked_add(data.len()).ok_or_else(out_of_range)?;
        bytes
            .get_mut(start..end)
            .ok_or_else(out_of_range)?
            .copy_from_slice(data);
        Ok(())
    }
}
