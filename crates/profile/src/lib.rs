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

#![forbid(unsafe_code)]

use std::fmt;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use redb::{Database, ReadableDatabase, ReadableTable, StorageBackend, TableDefinition};

#[cfg(test)]
mod tests;

/// The file a profile directory holds.
pub const FILE: &str = "profile.redb";

/// The version of the tables' layout this code writes. A profile of a
/// newer one is refused, one of an older one upgraded.
pub const SCHEMA: u64 = 1;

/// The memory `redb` may keep of the file's pages.
const CACHE_BYTES: usize = 16 << 20;

/// What the profile is: its schema's version under `"schema"`.
const META: TableDefinition<&str, u64> = TableDefinition::new("meta");

/// Each origin's `localStorage` items: keyed by the origin's
/// serialization and the item's key, each the order of its first
/// setting and its value.
const LOCAL_STORAGE: TableDefinition<(&str, &str), (u64, &str)> =
    TableDefinition::new("local_storage");

/// The cookies kept, in the order given, as `net` encodes them.
const COOKIES: TableDefinition<u64, &[u8]> = TableDefinition::new("cookies");

/// The HSTS hosts kept (M33.3), as `net` encodes them.
const HSTS: TableDefinition<u64, &[u8]> = TableDefinition::new("hsts");

/// The permissions sites were given (M33.4), as the engine encodes them.
const PERMISSIONS: TableDefinition<u64, &[u8]> = TableDefinition::new("permissions");

/// A profile open: one database, one writer.
pub struct Profile {
    /// `None` once the store was found damaged: never used again, nor
    /// dropped, what it holds being past trusting.
    db: Mutex<Option<Database>>,
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
    cookies: Option<Vec<Vec<u8>>>,
    hsts: Option<Vec<Vec<u8>>>,
    permissions: Option<Vec<Vec<u8>>>,
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
    /// and the file if they are not there.
    pub fn open(dir: &Path) -> Result<Profile, OpenError> {
        std::fs::create_dir_all(dir).map_err(|e| OpenError::Unreadable(e.to_string()))?;
        let path = dir.join(FILE);
        Profile::opened(|| {
            Database::builder()
                .set_cache_size(CACHE_BYTES)
                .create(&path)
        })
    }

    /// Opens the profile in `memory`, empty or as an earlier profile
    /// left it. The profile before must have been dropped: memory has
    /// no lock to say it is open.
    pub fn in_memory(memory: &Memory) -> Result<Profile, OpenError> {
        let memory = memory.clone();
        Profile::opened(move || {
            Database::builder()
                .set_cache_size(CACHE_BYTES)
                .create_with_backend(memory)
        })
    }

    /// The profile `open` opens, its schema checked: refused as damaged
    /// when `redb` panics, as it does on damage its checksums do not
    /// reach (it unwraps some of what it reads).
    fn opened(
        open: impl FnOnce() -> Result<Database, redb::DatabaseError>,
    ) -> Result<Profile, OpenError> {
        let damaged = || OpenError::Unreadable("it is damaged".to_owned());
        let db = catch_unwind(AssertUnwindSafe(open))
            .map_err(|_| damaged())?
            .map_err(refused)?;
        match catch_unwind(AssertUnwindSafe(|| Profile::prepare(&db))) {
            Ok(Ok(())) => Ok(Profile {
                db: Mutex::new(Some(db)),
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
    /// new one, and makes the tables, so that a reader finds them.
    fn prepare(db: &Database) -> Result<(), OpenError> {
        let unreadable = |error: Error| OpenError::Unreadable(error.0);
        let txn = db.begin_write().map_err(|e| unreadable(failed(e)))?;
        {
            let mut meta = txn.open_table(META).map_err(|e| unreadable(failed(e)))?;
            let schema = meta
                .get("schema")
                .map_err(|e| unreadable(failed(e)))?
                .map(|guard| guard.value());
            match schema {
                Some(schema) if schema > SCHEMA => return Err(OpenError::Newer(schema)),
                Some(_) => {}
                None => {
                    meta.insert("schema", SCHEMA)
                        .map_err(|e| unreadable(failed(e)))?;
                }
            }
            txn.open_table(LOCAL_STORAGE)
                .map_err(|e| unreadable(failed(e)))?;
            txn.open_table(COOKIES).map_err(|e| unreadable(failed(e)))?;
            txn.open_table(HSTS).map_err(|e| unreadable(failed(e)))?;
            txn.open_table(PERMISSIONS)
                .map_err(|e| unreadable(failed(e)))?;
        }
        txn.commit().map_err(|e| unreadable(failed(e)))?;
        Ok(())
    }

    /// The `localStorage` items of `origin`, in their order.
    pub fn local_storage(&self, origin: &str) -> Result<Vec<Item>, Error> {
        self.guarded(|db| Profile::read_local(db, origin))
    }

    fn read_local(db: &Database, origin: &str) -> Result<Vec<Item>, Error> {
        let txn = db.begin_read().map_err(failed)?;
        let table = txn.open_table(LOCAL_STORAGE).map_err(failed)?;
        let mut items = Vec::new();
        for entry in table.range((origin, "")..).map_err(failed)? {
            let (key, value) = entry.map_err(failed)?;
            let (owner, key) = key.value();
            if owner != origin {
                break;
            }
            let (order, value) = value.value();
            items.push(Item {
                key: key.to_owned(),
                value: value.to_owned(),
                order,
            });
        }
        items.sort_by_key(|item| item.order);
        Ok(items)
    }

    /// The cookies kept, records of `net`'s encoding, in their order.
    pub fn cookies(&self) -> Result<Vec<Vec<u8>>, Error> {
        self.guarded(|db| Profile::read_records(db, COOKIES))
    }

    /// The HSTS hosts kept (M33.3), records of `net`'s encoding.
    pub fn hsts(&self) -> Result<Vec<Vec<u8>>, Error> {
        self.guarded(|db| Profile::read_records(db, HSTS))
    }

    /// The permissions sites were given (M33.4), records of the engine's
    /// encoding.
    pub fn permissions(&self) -> Result<Vec<Vec<u8>>, Error> {
        self.guarded(|db| Profile::read_records(db, PERMISSIONS))
    }

    /// The records of `table`, in their order.
    fn read_records(
        db: &Database,
        table: TableDefinition<u64, &[u8]>,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let txn = db.begin_read().map_err(failed)?;
        let table = txn.open_table(table).map_err(failed)?;
        let mut records = Vec::new();
        for entry in table.iter().map_err(failed)? {
            let (_, record) = entry.map_err(failed)?;
            records.push(record.value().to_vec());
        }
        Ok(records)
    }

    /// Writes `changes` in one transaction, durable when this returns.
    pub fn commit(&self, changes: &Changes) -> Result<(), Error> {
        if changes.is_empty() {
            return Ok(());
        }
        self.guarded(|db| Profile::write(db, changes))
    }

    fn write(db: &Database, changes: &Changes) -> Result<(), Error> {
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
                        table
                            .insert((origin.as_str(), key.as_str()), (*order, value.as_str()))
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
        if let Some(cookies) = &changes.cookies {
            let mut table = txn.open_table(COOKIES).map_err(failed)?;
            table.retain(|_, _| false).map_err(failed)?;
            for (index, record) in (0u64..).zip(cookies) {
                table.insert(index, record.as_slice()).map_err(failed)?;
            }
        }
        for (definition, records) in [(HSTS, &changes.hsts), (PERMISSIONS, &changes.permissions)] {
            let Some(records) = records else {
                continue;
            };
            let mut table = txn.open_table(definition).map_err(failed)?;
            table.retain(|_, _| false).map_err(failed)?;
            for (index, record) in (0u64..).zip(records) {
                table.insert(index, record.as_slice()).map_err(failed)?;
            }
        }
        txn.commit().map_err(failed)
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
