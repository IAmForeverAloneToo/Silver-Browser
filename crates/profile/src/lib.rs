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
use std::path::Path;
use std::sync::{Arc, PoisonError, RwLock};

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

/// A profile open: one database, one writer.
pub struct Profile {
    db: Database,
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
        self.local.is_empty() && self.cookies.is_none()
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
}

impl Profile {
    /// Opens the profile in the directory `dir`, making the directory
    /// and the file if they are not there.
    pub fn open(dir: &Path) -> Result<Profile, OpenError> {
        std::fs::create_dir_all(dir).map_err(|e| OpenError::Unreadable(e.to_string()))?;
        let db = Database::builder()
            .set_cache_size(CACHE_BYTES)
            .create(dir.join(FILE))
            .map_err(refused)?;
        Profile::prepare(db)
    }

    /// Opens the profile in `memory`, empty or as an earlier profile
    /// left it. The profile before must have been dropped: memory has
    /// no lock to say it is open.
    pub fn in_memory(memory: &Memory) -> Result<Profile, OpenError> {
        let db = Database::builder()
            .set_cache_size(CACHE_BYTES)
            .create_with_backend(memory.clone())
            .map_err(refused)?;
        Profile::prepare(db)
    }

    /// Checks the schema of a profile just opened, writes it into a
    /// new one, and makes the tables, so that a reader finds them.
    fn prepare(db: Database) -> Result<Profile, OpenError> {
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
        }
        txn.commit().map_err(|e| unreadable(failed(e)))?;
        Ok(Profile { db })
    }

    /// The `localStorage` items of `origin`, in their order.
    pub fn local_storage(&self, origin: &str) -> Result<Vec<Item>, Error> {
        let txn = self.db.begin_read().map_err(failed)?;
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
        let txn = self.db.begin_read().map_err(failed)?;
        let table = txn.open_table(COOKIES).map_err(failed)?;
        let mut cookies = Vec::new();
        for entry in table.iter().map_err(failed)? {
            let (_, record) = entry.map_err(failed)?;
            cookies.push(record.value().to_vec());
        }
        Ok(cookies)
    }

    /// Writes `changes` in one transaction, durable when this returns.
    pub fn commit(&self, changes: &Changes) -> Result<(), Error> {
        if changes.is_empty() {
            return Ok(());
        }
        let txn = self.db.begin_write().map_err(failed)?;
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
