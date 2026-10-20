//! IndexedDB (M36.4, Indexed Database API 3.0; docs/design/devtools.md,
//! "IndexedDB"): its keys and their order, its key ranges, and the
//! store that keeps each storage key's databases and runs their
//! transactions.

#[cfg(test)]
mod api_tests;
mod key;
mod range;
mod serve;
mod store;
#[cfg(test)]
mod store_tests;

pub use key::{Key, MAX_IDB_KEY_BYTES, MAX_KEY_DEPTH};
pub use range::KeyRange;
pub use serve::{key_json, key_of, serve};
pub use store::{
    Direction, Found, IDB_ORIGIN_QUOTA, IdbError, IdbStore, IndexInfo, KeyPath, MAX_IDB_RESULTS,
    MAX_IDB_VALUE_BYTES, Mode, StoreInfo, Written,
};
