//! The Cache API's store (M39): any sequence of opens, deletes, puts,
//! deletes of entries, matches and the profile written and read back,
//! against a model of the standard's query written apart from the
//! store; and the input read as a record, which, if it reads, writes
//! back as it was. Nothing panics, the store answers as the model, its
//! caches and entries keep their order, and its usage its quota.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    engine::caches::fuzz_store(data);
});
