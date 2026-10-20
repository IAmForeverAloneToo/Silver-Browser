//! IndexedDB's databases (M36.4, docs/design/devtools.md, "IndexedDB"):
//! each storage key's, with their object stores, records and indices,
//! and the transactions that change them. A script's side evaluates the
//! key paths and serializes the values, which are its objects; this side
//! keeps the records by their keys' encodings, so that a range of keys is
//! a range of the map, answers the requests, and makes a transaction's
//! writes whole or not at all. What a transaction committed is written to
//! the profile as the rest of the storage is.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Bound;

use super::key::{Key, MAX_IDB_KEY_BYTES};
use super::range::KeyRange;

/// The largest value the store keeps, its structured serialization.
pub const MAX_IDB_VALUE_BYTES: usize = 64 << 20;

/// The most a storage key's databases hold together, their keys and
/// values: past it a write fails with a `QuotaExceededError`.
pub const IDB_ORIGIN_QUOTA: u64 = 512 << 20;

/// The most records a request gives back at once.
pub const MAX_IDB_RESULTS: usize = 100_000;

/// The key generator's largest key (section 2.11): 2^53.
const MAX_GENERATED: f64 = 9_007_199_254_740_992.0;

/// Why a request failed: the name of the `DOMException` a script sees,
/// and its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdbError {
    pub name: &'static str,
    pub message: String,
}

impl IdbError {
    fn new(name: &'static str, message: impl Into<String>) -> IdbError {
        IdbError {
            name,
            message: message.into(),
        }
    }
}

type Result<T> = std::result::Result<T, IdbError>;

/// A key path (section 2.5): none, a string, or a list of strings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum KeyPath {
    #[default]
    None,
    String(String),
    Array(Vec<String>),
}

/// A transaction's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    ReadOnly,
    ReadWrite,
    VersionChange,
}

/// A cursor's direction (section 2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Next,
    NextUnique,
    Prev,
    PrevUnique,
}

/// A record as a request gives it: its key, its primary key (the same
/// for an object store's), and its value unless only keys were asked.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub key: Key,
    pub primary: Key,
    pub value: Option<Vec<u8>>,
}

/// An object store's description, as a script's `IDBObjectStore` reads
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreInfo {
    pub name: String,
    pub key_path: KeyPath,
    pub auto_increment: bool,
    pub indices: Vec<IndexInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexInfo {
    pub name: String,
    pub key_path: KeyPath,
    pub unique: bool,
    pub multi_entry: bool,
}

/// What a write left for the profile, once its transaction committed.
#[derive(Debug, Clone, PartialEq)]
pub enum Written {
    /// A database's description, its stores and indices, as encoded.
    Database {
        origin: String,
        name: String,
        meta: Vec<u8>,
    },
    /// A database deleted, and every record of it.
    DatabaseDeleted { origin: String, name: String },
    /// A record, as encoded, or none where it was deleted.
    Record {
        origin: String,
        database: String,
        store: String,
        key: Vec<u8>,
        record: Option<Vec<u8>>,
    },
    /// Every record of a store deleted.
    StoreDeleted {
        origin: String,
        database: String,
        store: String,
    },
}

#[derive(Debug, Clone, Default)]
struct Origin {
    databases: BTreeMap<String, Database>,
    /// The bytes of its records' keys and values.
    used: u64,
}

#[derive(Debug, Clone, Default)]
struct Database {
    version: u64,
    stores: BTreeMap<String, ObjectStore>,
}

#[derive(Debug, Clone, Default)]
struct ObjectStore {
    key_path: KeyPath,
    auto_increment: bool,
    /// The key generator's current number.
    current: f64,
    records: BTreeMap<Vec<u8>, Record>,
    indices: BTreeMap<String, Index>,
}

#[derive(Debug, Clone)]
struct Record {
    key: Key,
    value: Vec<u8>,
    /// Its keys in each index, as the script's side evaluated them.
    index_keys: Vec<(String, Vec<Key>)>,
}

impl Record {
    fn size(&self, key: &[u8]) -> u64 {
        (key.len() + self.value.len()) as u64
    }
}

#[derive(Debug, Clone, Default)]
struct Index {
    key_path: KeyPath,
    unique: bool,
    multi_entry: bool,
    /// Its entries, by index key and then primary key, encoded.
    entries: BTreeSet<(Vec<u8>, Vec<u8>)>,
}

/// What undoes a write, for an abort.
#[derive(Debug)]
enum Undo {
    Record {
        store: String,
        key: Vec<u8>,
        before: Option<Record>,
    },
    Generator {
        store: String,
        before: f64,
    },
    /// A version change's: the database as it was, whole.
    Database(Box<Database>),
}

#[derive(Debug)]
struct Transaction {
    origin: String,
    database: String,
    mode: Mode,
    scope: Vec<String>,
    undo: Vec<Undo>,
    /// The records it wrote, by store and key, and whether it changed the
    /// database's description.
    touched: BTreeSet<(String, Vec<u8>)>,
    changed_schema: bool,
    /// The stores it deleted, whose records the profile drops.
    dropped: BTreeSet<String>,
}

/// Every storage key's databases, and the transactions under way.
#[derive(Debug, Default)]
pub struct IdbStore {
    origins: HashMap<String, Origin>,
    transactions: HashMap<u64, Transaction>,
    next: u64,
    /// What the transactions committed, for the profile, kept while
    /// there is a profile to write it to.
    written: Vec<Written>,
    writing: bool,
}

impl IdbStore {
    /// `origin`'s databases' names and versions (`indexedDB.databases()`).
    pub fn databases(&self, origin: &str) -> Vec<(String, u64)> {
        self.origins.get(origin).map_or_else(Vec::new, |o| {
            o.databases
                .iter()
                .map(|(name, db)| (name.clone(), db.version))
                .collect()
        })
    }

    /// Whether `origin`'s databases are known to the store: read from
    /// the profile, or made.
    pub fn knows(&self, origin: &str) -> bool {
        self.origins.contains_key(origin)
    }

    /// The bytes `origin`'s databases hold.
    pub fn used(&self, origin: &str) -> u64 {
        self.origins.get(origin).map_or(0, |o| o.used)
    }

    /// Opens `origin`'s database `name` (section 4.1's open steps, the
    /// connections aside): its version before, and the transaction that
    /// upgrades it to `version` when that is higher. A database not there
    /// is made at version 0, and opened at 1 when no version is asked.
    pub fn open(
        &mut self,
        origin: &str,
        name: &str,
        version: Option<u64>,
    ) -> Result<(u64, Option<u64>)> {
        if version == Some(0) {
            return Err(IdbError::new("TypeError", "The version is 0."));
        }
        let entry = self.origins.entry(origin.to_owned()).or_default();
        let current = entry.databases.get(name).map_or(0, |db| db.version);
        let wanted = version.unwrap_or(current.max(1));
        if wanted < current {
            return Err(IdbError::new(
                "VersionError",
                format!(
                    "The requested version ({wanted}) is less than the existing version ({current})."
                ),
            ));
        }
        if wanted == current {
            return Ok((current, None));
        }
        let db = entry.databases.entry(name.to_owned()).or_default();
        let before = db.clone();
        db.version = wanted;
        let stores: Vec<String> = db.stores.keys().cloned().collect();
        let id = self.begin_unchecked(origin, name, Mode::VersionChange, stores);
        let txn = self.transactions.get_mut(&id).expect("just begun");
        txn.undo.push(Undo::Database(Box::new(before)));
        txn.changed_schema = true;
        Ok((current, Some(id)))
    }

    /// Deletes `origin`'s database `name`: its version, 0 when there was
    /// none.
    pub fn delete_database(&mut self, origin: &str, name: &str) -> u64 {
        let Some(entry) = self.origins.get_mut(origin) else {
            return 0;
        };
        let Some(db) = entry.databases.remove(name) else {
            return 0;
        };
        let freed: u64 = db
            .stores
            .values()
            .flat_map(|store| store.records.iter().map(|(k, r)| r.size(k)))
            .sum();
        entry.used = entry.used.saturating_sub(freed);
        self.transactions
            .retain(|_, txn| !(txn.origin == origin && txn.database == name));
        if !self.writing {
            return db.version;
        }
        self.written.push(Written::DatabaseDeleted {
            origin: origin.to_owned(),
            name: name.to_owned(),
        });
        db.version
    }

    /// The object stores of a database and their indices.
    pub fn stores(&self, origin: &str, name: &str) -> Vec<StoreInfo> {
        let Some(db) = self.database(origin, name) else {
            return Vec::new();
        };
        db.stores
            .iter()
            .map(|(name, store)| StoreInfo {
                name: name.clone(),
                key_path: store.key_path.clone(),
                auto_increment: store.auto_increment,
                indices: store
                    .indices
                    .iter()
                    .map(|(name, index)| IndexInfo {
                        name: name.clone(),
                        key_path: index.key_path.clone(),
                        unique: index.unique,
                        multi_entry: index.multi_entry,
                    })
                    .collect(),
            })
            .collect()
    }

    fn database(&self, origin: &str, name: &str) -> Option<&Database> {
        self.origins.get(origin)?.databases.get(name)
    }

    /// Begins a transaction of `scope` in `mode` on a database: its
    /// number. The script's side runs the transactions of a database
    /// whose scopes overlap, a writer among them, one after another.
    pub fn begin(
        &mut self,
        origin: &str,
        name: &str,
        mode: Mode,
        scope: Vec<String>,
    ) -> Result<u64> {
        let db = self
            .database(origin, name)
            .ok_or_else(|| IdbError::new("InvalidStateError", "The database is gone."))?;
        if mode == Mode::VersionChange || scope.is_empty() {
            return Err(IdbError::new(
                "InvalidAccessError",
                "A transaction needs a scope and is no version change.",
            ));
        }
        if let Some(missing) = scope.iter().find(|s| !db.stores.contains_key(*s)) {
            return Err(IdbError::new(
                "NotFoundError",
                format!("No object store named '{missing}'."),
            ));
        }
        Ok(self.begin_unchecked(origin, name, mode, scope))
    }

    fn begin_unchecked(&mut self, origin: &str, name: &str, mode: Mode, scope: Vec<String>) -> u64 {
        self.next += 1;
        self.transactions.insert(
            self.next,
            Transaction {
                origin: origin.to_owned(),
                database: name.to_owned(),
                mode,
                scope,
                undo: Vec::new(),
                touched: BTreeSet::new(),
                changed_schema: false,
                dropped: BTreeSet::new(),
            },
        );
        self.next
    }

    /// Commits transaction `txn`: what it wrote stays, and goes to the
    /// profile.
    pub fn commit(&mut self, txn: u64) -> Result<()> {
        let txn = self.transactions.remove(&txn).ok_or_else(gone)?;
        let Some(db) = self.database(&txn.origin, &txn.database) else {
            return Ok(());
        };
        let mut written = Vec::new();
        for store in &txn.dropped {
            written.push(Written::StoreDeleted {
                origin: txn.origin.clone(),
                database: txn.database.clone(),
                store: store.clone(),
            });
        }
        if txn.changed_schema || !txn.touched.is_empty() {
            written.push(Written::Database {
                origin: txn.origin.clone(),
                name: txn.database.clone(),
                meta: encode_meta(db),
            });
        }
        for (store, key) in &txn.touched {
            let record = db
                .stores
                .get(store)
                .and_then(|s| s.records.get(key))
                .map(encode_record);
            written.push(Written::Record {
                origin: txn.origin.clone(),
                database: txn.database.clone(),
                store: store.clone(),
                key: key.clone(),
                record,
            });
        }
        if self.writing {
            self.written.extend(written);
        }
        Ok(())
    }

    /// Aborts transaction `txn`: what it wrote is undone.
    pub fn abort(&mut self, txn: u64) -> Result<()> {
        let txn = self.transactions.remove(&txn).ok_or_else(gone)?;
        let Some(origin) = self.origins.get_mut(&txn.origin) else {
            return Ok(());
        };
        for undo in txn.undo.into_iter().rev() {
            match undo {
                Undo::Database(before) => {
                    // A version change of a database that was not there
                    // leaves none.
                    if before.version == 0 && before.stores.is_empty() {
                        origin.databases.remove(&txn.database);
                    } else {
                        origin.databases.insert(txn.database.clone(), *before);
                    }
                }
                Undo::Generator { store, before } => {
                    if let Some(store) = origin
                        .databases
                        .get_mut(&txn.database)
                        .and_then(|db| db.stores.get_mut(&store))
                    {
                        store.current = before;
                    }
                }
                Undo::Record { store, key, before } => {
                    let Some(store) = origin
                        .databases
                        .get_mut(&txn.database)
                        .and_then(|db| db.stores.get_mut(&store))
                    else {
                        continue;
                    };
                    if let Some(now) = store.records.remove(&key) {
                        origin.used = origin.used.saturating_sub(now.size(&key));
                        unindex(store, &key, &now);
                    }
                    if let Some(before) = before {
                        origin.used += before.size(&key);
                        index(store, &key, &before);
                        store.records.insert(key, before);
                    }
                }
            }
        }
        origin.used = recount(origin);
        Ok(())
    }

    /// What committed transactions left for the profile since this was
    /// last asked.
    pub fn take_written(&mut self) -> Vec<Written> {
        std::mem::take(&mut self.written)
    }

    /// Whether committed transactions left something for the profile.
    pub fn has_written(&self) -> bool {
        !self.written.is_empty()
    }

    /// Keeps what transactions commit for a profile from now on, or,
    /// with none to write it to, nothing.
    pub fn set_writing(&mut self, on: bool) {
        self.writing = on;
        if !on {
            self.written.clear();
        }
    }

    fn txn(&self, txn: u64) -> Result<&Transaction> {
        self.transactions.get(&txn).ok_or_else(gone)
    }

    /// The store `name` of transaction `txn`'s scope, to read.
    fn store(&self, txn: u64, name: &str) -> Result<&ObjectStore> {
        let t = self.txn(txn)?;
        if t.mode != Mode::VersionChange && !t.scope.iter().any(|s| s == name) {
            return Err(not_found(name));
        }
        self.database(&t.origin, &t.database)
            .and_then(|db| db.stores.get(name))
            .ok_or_else(|| not_found(name))
    }

    /// The database and transaction of `txn`, to write.
    fn writing(&mut self, txn: u64) -> Result<(&mut Origin, &mut Transaction)> {
        let t = self.transactions.get_mut(&txn).ok_or_else(gone)?;
        if t.mode == Mode::ReadOnly {
            return Err(IdbError::new(
                "ReadOnlyError",
                "The transaction is read-only.",
            ));
        }
        let origin = self.origins.get_mut(&t.origin).ok_or_else(gone)?;
        Ok((origin, t))
    }

    /// Makes object store `name` in a version change.
    pub fn create_store(
        &mut self,
        txn: u64,
        name: &str,
        key_path: KeyPath,
        auto_increment: bool,
    ) -> Result<()> {
        let (origin, t) = self.writing(txn)?;
        if t.mode != Mode::VersionChange {
            return Err(not_version_change());
        }
        let db = origin.databases.get_mut(&t.database).ok_or_else(gone)?;
        if db.stores.contains_key(name) {
            return Err(IdbError::new(
                "ConstraintError",
                format!("An object store named '{name}' exists."),
            ));
        }
        db.stores.insert(
            name.to_owned(),
            ObjectStore {
                key_path,
                auto_increment,
                ..ObjectStore::default()
            },
        );
        t.dropped.remove(name);
        Ok(())
    }

    /// Deletes object store `name` in a version change.
    pub fn delete_store(&mut self, txn: u64, name: &str) -> Result<()> {
        let (origin, t) = self.writing(txn)?;
        if t.mode != Mode::VersionChange {
            return Err(not_version_change());
        }
        let db = origin.databases.get_mut(&t.database).ok_or_else(gone)?;
        let store = db.stores.remove(name).ok_or_else(|| not_found(name))?;
        let freed: u64 = store.records.iter().map(|(k, r)| r.size(k)).sum();
        origin.used = origin.used.saturating_sub(freed);
        t.touched.retain(|(s, _)| s != name);
        t.dropped.insert(name.to_owned());
        Ok(())
    }

    /// Makes index `name` of store `store` in a version change, of the
    /// records there now and their keys in it, `keys` (primary key,
    /// index keys), which the script's side evaluated.
    pub fn create_index(
        &mut self,
        txn: u64,
        store: &str,
        name: &str,
        info: (KeyPath, bool, bool),
        keys: Vec<(Key, Vec<Key>)>,
    ) -> Result<()> {
        let (origin, t) = self.writing(txn)?;
        if t.mode != Mode::VersionChange {
            return Err(not_version_change());
        }
        let db = origin.databases.get_mut(&t.database).ok_or_else(gone)?;
        let os = db.stores.get_mut(store).ok_or_else(|| not_found(store))?;
        if os.indices.contains_key(name) {
            return Err(IdbError::new(
                "ConstraintError",
                format!("An index named '{name}' exists."),
            ));
        }
        let (key_path, unique, multi_entry) = info;
        let mut index = Index {
            key_path,
            unique,
            multi_entry,
            entries: BTreeSet::new(),
        };
        let mut seen = BTreeSet::new();
        for (primary, index_keys) in &keys {
            let pk = primary.encode();
            if !os.records.contains_key(&pk) {
                continue;
            }
            for key in index_keys {
                let ik = key.encode();
                if unique && !seen.insert(ik.clone()) {
                    // Section 4.7: the version change aborts.
                    return Err(IdbError::new(
                        "ConstraintError",
                        format!("The index '{name}' is unique, and two records share a key."),
                    ));
                }
                index.entries.insert((ik, pk.clone()));
            }
        }
        // The records keep their keys in it, for their deletion.
        for (primary, index_keys) in keys {
            if let Some(record) = os.records.get_mut(&primary.encode()) {
                record.index_keys.retain(|(n, _)| n != name);
                record.index_keys.push((name.to_owned(), index_keys));
            }
        }
        os.indices.insert(name.to_owned(), index);
        let touched: Vec<Vec<u8>> = os.records.keys().cloned().collect();
        for key in touched {
            t.touched.insert((store.to_owned(), key));
        }
        Ok(())
    }

    /// Deletes index `name` of store `store` in a version change.
    pub fn delete_index(&mut self, txn: u64, store: &str, name: &str) -> Result<()> {
        let (origin, t) = self.writing(txn)?;
        if t.mode != Mode::VersionChange {
            return Err(not_version_change());
        }
        let db = origin.databases.get_mut(&t.database).ok_or_else(gone)?;
        let os = db.stores.get_mut(store).ok_or_else(|| not_found(store))?;
        os.indices.remove(name).ok_or_else(|| not_found(name))?;
        for (key, record) in os.records.iter_mut() {
            record.index_keys.retain(|(n, _)| n != name);
            t.touched.insert((store.to_owned(), key.clone()));
        }
        Ok(())
    }

    /// The key generator's next key for store `store` (section 2.11's
    /// "generate a key"), taken: for a value the script's side puts the
    /// key in before it serializes it.
    pub fn generate_key(&mut self, txn: u64, store: &str) -> Result<Key> {
        let (origin, t) = self.writing(txn)?;
        let os = origin
            .databases
            .get_mut(&t.database)
            .and_then(|db| db.stores.get_mut(store))
            .ok_or_else(|| not_found(store))?;
        if !os.auto_increment {
            return Err(IdbError::new(
                "DataError",
                "The object store has no key generator.",
            ));
        }
        if os.current >= MAX_GENERATED {
            return Err(IdbError::new(
                "ConstraintError",
                "The key generator has reached its limit.",
            ));
        }
        t.undo.push(Undo::Generator {
            store: store.to_owned(),
            before: os.current,
        });
        os.current += 1.0;
        Ok(Key::Number(os.current))
    }

    /// Stores `value` under `key` in store `store` (section 5.4's
    /// "store a record"), or under a generated key when none is given,
    /// with its keys in each index, `index_keys`: the key it is stored
    /// under. With `no_overwrite` (`add()`), a record there already is a
    /// `ConstraintError`; so is a unique index's key another has.
    pub fn put(
        &mut self,
        txn: u64,
        store: &str,
        key: Option<Key>,
        value: Vec<u8>,
        index_keys: Vec<(String, Vec<Key>)>,
        no_overwrite: bool,
    ) -> Result<Key> {
        if value.len() > MAX_IDB_VALUE_BYTES {
            return Err(IdbError::new(
                "DataError",
                format!("The value is larger than {MAX_IDB_VALUE_BYTES} bytes."),
            ));
        }
        let key = match key {
            Some(key) => key,
            None => self.generate_key(txn, store)?,
        };
        let pk = key.encode();
        if pk.len() > MAX_IDB_KEY_BYTES {
            return Err(IdbError::new("DataError", "The key is too large."));
        }
        let (origin, t) = self.writing(txn)?;
        if t.mode != Mode::VersionChange && !t.scope.iter().any(|s| s == store) {
            return Err(not_found(store));
        }
        let os = origin
            .databases
            .get_mut(&t.database)
            .and_then(|db| db.stores.get_mut(store))
            .ok_or_else(|| not_found(store))?;
        // A number given as a key moves the generator past it.
        if os.auto_increment
            && let Key::Number(n) = key
            && n > os.current
        {
            t.undo.push(Undo::Generator {
                store: store.to_owned(),
                before: os.current,
            });
            os.current = n.floor().min(MAX_GENERATED);
        }
        let before = os.records.get(&pk).cloned();
        if no_overwrite && before.is_some() {
            return Err(IdbError::new(
                "ConstraintError",
                "A record with the key exists.",
            ));
        }
        let index_keys: Vec<(String, Vec<Key>)> = index_keys
            .into_iter()
            .filter(|(name, _)| os.indices.contains_key(name))
            .collect();
        for (name, keys) in &index_keys {
            let index = &os.indices[name];
            if !index.unique {
                continue;
            }
            for k in keys {
                let ik = k.encode();
                let taken = index
                    .entries
                    .range((ik.clone(), Vec::new())..)
                    .take_while(|(i, _)| *i == ik)
                    .any(|(_, p)| *p != pk);
                if taken {
                    return Err(IdbError::new(
                        "ConstraintError",
                        format!("The index '{name}' is unique, and a record has the key."),
                    ));
                }
            }
        }
        let record = Record {
            key: key.clone(),
            value,
            index_keys,
        };
        let size = record.size(&pk);
        let freed = before.as_ref().map_or(0, |b| b.size(&pk));
        if origin.used - freed + size > IDB_ORIGIN_QUOTA {
            return Err(IdbError::new(
                "QuotaExceededError",
                "The origin's quota is used up.",
            ));
        }
        if let Some(before) = &before {
            unindex(os, &pk, before);
        }
        index(os, &pk, &record);
        os.records.insert(pk.clone(), record);
        origin.used = origin.used - freed + size;
        t.undo.push(Undo::Record {
            store: store.to_owned(),
            key: pk.clone(),
            before,
        });
        t.touched.insert((store.to_owned(), pk));
        Ok(key)
    }

    /// Deletes the records of store `store` in `range`.
    pub fn delete(&mut self, txn: u64, store: &str, range: &KeyRange) -> Result<()> {
        let (origin, t) = self.writing(txn)?;
        if t.mode != Mode::VersionChange && !t.scope.iter().any(|s| s == store) {
            return Err(not_found(store));
        }
        let os = origin
            .databases
            .get_mut(&t.database)
            .and_then(|db| db.stores.get_mut(store))
            .ok_or_else(|| not_found(store))?;
        let keys: Vec<Vec<u8>> = os
            .records
            .range(range.bounds())
            .map(|(k, _)| k.clone())
            .collect();
        for pk in keys {
            let Some(record) = os.records.remove(&pk) else {
                continue;
            };
            unindex(os, &pk, &record);
            origin.used = origin.used.saturating_sub(record.size(&pk));
            t.undo.push(Undo::Record {
                store: store.to_owned(),
                key: pk.clone(),
                before: Some(record),
            });
            t.touched.insert((store.to_owned(), pk));
        }
        Ok(())
    }

    /// The records of store `store`, or of its index `index`, in
    /// `range`, in `direction`, at most `count` (0 for all, to the bound
    /// [`MAX_IDB_RESULTS`]), their values unless `keys_only`.
    pub fn get_all(
        &self,
        txn: u64,
        (store, index): (&str, Option<&str>),
        range: &KeyRange,
        count: usize,
        keys_only: bool,
    ) -> Result<Vec<Found>> {
        let os = self.store(txn, store)?;
        let limit = if count == 0 {
            MAX_IDB_RESULTS
        } else {
            count.min(MAX_IDB_RESULTS)
        };
        let value = |pk: &Vec<u8>| -> Option<Vec<u8>> {
            (!keys_only).then(|| {
                os.records
                    .get(pk)
                    .map(|r| r.value.clone())
                    .unwrap_or_default()
            })
        };
        match index {
            None => Ok(os
                .records
                .range(range.bounds())
                .take(limit)
                .map(|(_, record)| Found {
                    key: record.key.clone(),
                    primary: record.key.clone(),
                    value: (!keys_only).then(|| record.value.clone()),
                })
                .collect()),
            Some(name) => {
                let index = os.indices.get(name).ok_or_else(|| not_found(name))?;
                Ok(index_range(index, range)
                    .take(limit)
                    .filter_map(|(ik, pk)| {
                        Some(Found {
                            key: Key::decode(ik)?,
                            primary: os.records.get(pk)?.key.clone(),
                            value: value(pk),
                        })
                    })
                    .collect())
            }
        }
    }

    /// How many records of store `store`, or of its index, are in
    /// `range`.
    pub fn count(
        &self,
        txn: u64,
        (store, index): (&str, Option<&str>),
        range: &KeyRange,
    ) -> Result<u64> {
        let os = self.store(txn, store)?;
        Ok(match index {
            None => os.records.range(range.bounds()).count() as u64,
            Some(name) => {
                let index = os.indices.get(name).ok_or_else(|| not_found(name))?;
                index_range(index, range).count() as u64
            }
        })
    }

    /// A cursor's next record (section 4.15's "iterate a cursor"): of
    /// store `store` or its index, in `range` and `direction`, past the
    /// cursor's `position` (its key and, for an index, its primary key),
    /// at or past `key` and `primary` when `continue()` names them, the
    /// `count`th that follows (`advance()`).
    #[allow(clippy::too_many_arguments)]
    pub fn iterate(
        &self,
        txn: u64,
        (store, index): (&str, Option<&str>),
        range: &KeyRange,
        direction: Direction,
        position: Option<(&Key, &Key)>,
        target: Option<(&Key, Option<&Key>)>,
        count: u32,
        keys_only: bool,
    ) -> Result<Option<Found>> {
        let os = self.store(txn, store)?;
        // Every record in range, as (key, primary key), in order.
        let all: Vec<(Vec<u8>, Vec<u8>)> = match index {
            None => os
                .records
                .range(range.bounds())
                .map(|(pk, _)| (pk.clone(), pk.clone()))
                .collect(),
            Some(name) => {
                let index = os.indices.get(name).ok_or_else(|| not_found(name))?;
                index_range(index, range)
                    .map(|(ik, pk)| (ik.clone(), pk.clone()))
                    .collect()
            }
        };
        let forward = matches!(direction, Direction::Next | Direction::NextUnique);
        let unique = matches!(direction, Direction::NextUnique | Direction::PrevUnique);
        let position = position.map(|(k, p)| (k.encode(), p.encode()));
        let target = target.map(|(k, p)| (k.encode(), p.map(Key::encode)));
        let ordered: Box<dyn Iterator<Item = &(Vec<u8>, Vec<u8>)>> = if forward {
            Box::new(all.iter())
        } else {
            Box::new(all.iter().rev())
        };
        let mut left = count.max(1);
        let mut last_key: Option<&Vec<u8>> = position.as_ref().map(|(k, _)| k);
        for entry in ordered {
            let (k, p) = entry;
            // Past the position.
            if let Some((pk_key, pk_primary)) = &position {
                let past = if forward {
                    (k, p) > (pk_key, pk_primary)
                } else {
                    (k, p) < (pk_key, pk_primary)
                };
                if !past {
                    continue;
                }
                if unique && k == pk_key {
                    continue;
                }
            }
            if let Some((tk, tp)) = &target {
                let reached = match tp {
                    None => {
                        if forward {
                            k >= tk
                        } else {
                            k <= tk
                        }
                    }
                    Some(tp) => {
                        if forward {
                            (k, p) >= (tk, tp)
                        } else {
                            (k, p) <= (tk, tp)
                        }
                    }
                };
                if !reached {
                    continue;
                }
            }
            // A unique iteration takes the first of each key: backwards,
            // the one with the least primary key.
            if unique && last_key == Some(k) {
                continue;
            }
            let entry_key = if unique && !forward {
                all.iter()
                    .find(|(ik, _)| ik == k)
                    .map_or(entry, |first| first)
            } else {
                entry
            };
            last_key = Some(k);
            left -= 1;
            if left > 0 {
                continue;
            }
            let (k, p) = entry_key;
            let record = os.records.get(p).ok_or_else(gone)?;
            return Ok(Some(Found {
                key: Key::decode(k).ok_or_else(gone)?,
                primary: record.key.clone(),
                value: (!keys_only).then(|| record.value.clone()),
            }));
        }
        Ok(None)
    }

    /// Reads `origin`'s databases from what the profile kept of them:
    /// their descriptions and their records. What does not decode is left
    /// out.
    pub fn load(
        &mut self,
        origin: &str,
        databases: Vec<(String, Vec<u8>)>,
        records: Vec<(String, String, Vec<u8>, Vec<u8>)>,
    ) {
        let mut loaded = Origin::default();
        for (name, meta) in databases {
            if let Some(db) = decode_meta(&meta) {
                loaded.databases.insert(name, db);
            }
        }
        for (database, store, key, bytes) in records {
            let Some(os) = loaded
                .databases
                .get_mut(&database)
                .and_then(|db| db.stores.get_mut(&store))
            else {
                continue;
            };
            let (Some(k), Some(record)) = (Key::decode(&key), decode_record(&bytes)) else {
                continue;
            };
            let record = Record { key: k, ..record };
            loaded.used += record.size(&key);
            index(os, &key, &record);
            os.records.insert(key, record);
        }
        self.origins.insert(origin.to_owned(), loaded);
    }
}

/// An index's entries whose keys are in `range`, in order.
fn index_range<'a>(
    index: &'a Index,
    range: &KeyRange,
) -> impl Iterator<Item = &'a (Vec<u8>, Vec<u8>)> + 'a {
    let (lower, upper) = range.bounds();
    // The entries of a key start at it with the least primary key; an
    // excluded lower key's are passed over.
    let (start, skip) = match lower {
        Bound::Unbounded => (Bound::Unbounded, None),
        Bound::Included(k) => (Bound::Included((k, Vec::new())), None),
        Bound::Excluded(k) => (Bound::Included((k.clone(), Vec::new())), Some(k)),
    };
    index
        .entries
        .range((start, Bound::Unbounded))
        .skip_while(move |(ik, _)| skip.as_ref().is_some_and(|s| s == ik))
        .take_while(move |(ik, _)| match &upper {
            Bound::Unbounded => true,
            Bound::Included(k) => ik <= k,
            Bound::Excluded(k) => ik < k,
        })
}

/// Puts `record`'s keys in its store's indices.
fn index(store: &mut ObjectStore, pk: &[u8], record: &Record) {
    for (name, keys) in &record.index_keys {
        if let Some(index) = store.indices.get_mut(name) {
            for key in keys {
                index.entries.insert((key.encode(), pk.to_vec()));
            }
        }
    }
}

/// Takes `record`'s keys out of its store's indices.
fn unindex(store: &mut ObjectStore, pk: &[u8], record: &Record) {
    for (name, keys) in &record.index_keys {
        if let Some(index) = store.indices.get_mut(name) {
            for key in keys {
                index.entries.remove(&(key.encode(), pk.to_vec()));
            }
        }
    }
}

/// The bytes an origin's records hold, counted again.
fn recount(origin: &Origin) -> u64 {
    origin
        .databases
        .values()
        .flat_map(|db| db.stores.values())
        .flat_map(|store| store.records.iter().map(|(k, r)| r.size(k)))
        .sum()
}

fn gone() -> IdbError {
    IdbError::new("InvalidStateError", "The transaction has finished.")
}

fn not_found(name: &str) -> IdbError {
    IdbError::new(
        "NotFoundError",
        format!("No object store or index named '{name}'."),
    )
}

fn not_version_change() -> IdbError {
    IdbError::new(
        "InvalidStateError",
        "The database is not running a version change transaction.",
    )
}

// ---- What the profile keeps --------------------------------------------

/// A writer of the profile's encoding: lengths as 4 bytes, little-endian.
struct Out(Vec<u8>);

impl Out {
    fn bytes(&mut self, bytes: &[u8]) {
        self.0
            .extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        self.0.extend_from_slice(bytes);
    }
    fn u64(&mut self, n: u64) {
        self.0.extend_from_slice(&n.to_le_bytes());
    }
    fn key_path(&mut self, path: &KeyPath) {
        match path {
            KeyPath::None => self.0.push(0),
            KeyPath::String(s) => {
                self.0.push(1);
                self.bytes(s.as_bytes());
            }
            KeyPath::Array(items) => {
                self.0.push(2);
                self.u64(items.len() as u64);
                for item in items {
                    self.bytes(item.as_bytes());
                }
            }
        }
    }
}

/// A reader of it.
struct In<'a>(&'a [u8]);

impl In<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn bytes(&mut self) -> Option<Vec<u8>> {
        let len = u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize;
        self.take(len).map(<[u8]>::to_vec)
    }
    fn string(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?).ok()
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn byte(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn key_path(&mut self) -> Option<KeyPath> {
        Some(match self.byte()? {
            0 => KeyPath::None,
            1 => KeyPath::String(self.string()?),
            2 => {
                let n = self.u64()?;
                let mut items = Vec::new();
                for _ in 0..n {
                    items.push(self.string()?);
                }
                KeyPath::Array(items)
            }
            _ => return None,
        })
    }
}

/// A database's description: its version, and each store's name, key
/// path, key generator and indices.
fn encode_meta(db: &Database) -> Vec<u8> {
    let mut out = Out(Vec::new());
    out.u64(db.version);
    out.u64(db.stores.len() as u64);
    for (name, store) in &db.stores {
        out.bytes(name.as_bytes());
        out.key_path(&store.key_path);
        out.0.push(u8::from(store.auto_increment));
        out.u64(store.current.to_bits());
        out.u64(store.indices.len() as u64);
        for (name, index) in &store.indices {
            out.bytes(name.as_bytes());
            out.key_path(&index.key_path);
            out.0
                .push(u8::from(index.unique) | u8::from(index.multi_entry) << 1);
        }
    }
    out.0
}

fn decode_meta(bytes: &[u8]) -> Option<Database> {
    let mut input = In(bytes);
    let version = input.u64()?;
    let mut stores = BTreeMap::new();
    for _ in 0..input.u64()? {
        let name = input.string()?;
        let key_path = input.key_path()?;
        let auto_increment = input.byte()? == 1;
        let current = f64::from_bits(input.u64()?);
        let mut indices = BTreeMap::new();
        for _ in 0..input.u64()? {
            let name = input.string()?;
            let key_path = input.key_path()?;
            let flags = input.byte()?;
            indices.insert(
                name,
                Index {
                    key_path,
                    unique: flags & 1 != 0,
                    multi_entry: flags & 2 != 0,
                    entries: BTreeSet::new(),
                },
            );
        }
        stores.insert(
            name,
            ObjectStore {
                key_path,
                auto_increment,
                current,
                records: BTreeMap::new(),
                indices,
            },
        );
    }
    input.0.is_empty().then_some(Database { version, stores })
}

/// A record: its index keys, then its value.
fn encode_record(record: &Record) -> Vec<u8> {
    let mut out = Out(Vec::new());
    out.u64(record.index_keys.len() as u64);
    for (name, keys) in &record.index_keys {
        out.bytes(name.as_bytes());
        out.u64(keys.len() as u64);
        for key in keys {
            out.bytes(&key.encode());
        }
    }
    out.bytes(&record.value);
    out.0
}

fn decode_record(bytes: &[u8]) -> Option<Record> {
    let mut input = In(bytes);
    let mut index_keys = Vec::new();
    for _ in 0..input.u64()? {
        let name = input.string()?;
        let mut keys = Vec::new();
        for _ in 0..input.u64()? {
            keys.push(Key::decode(&input.bytes()?)?);
        }
        index_keys.push((name, keys));
    }
    let value = input.bytes()?;
    input.0.is_empty().then_some(Record {
        key: Key::Number(0.0),
        value,
        index_keys,
    })
}

/// Encodes `record`'s parts as the profile keeps them, for the tests and
/// the fuzz target.
#[cfg(test)]
pub(crate) fn roundtrip_record(value: Vec<u8>, index_keys: Vec<(String, Vec<Key>)>) -> bool {
    let record = Record {
        key: Key::Number(0.0),
        value,
        index_keys,
    };
    decode_record(&encode_record(&record))
        .is_some_and(|back| back.value == record.value && back.index_keys == record.index_keys)
}
