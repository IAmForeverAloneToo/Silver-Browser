//! The service workers' registry (M39): any sequence of register,
//! update and unregister jobs over scopes and scripts, each script's
//! fetch, evaluation and install answered as the input says, against a
//! model of the jobs written apart from the registry, with the profile
//! written and read back; and the input read as a registration's
//! record, which, if it reads, writes back to one that reads the same.
//! Nothing panics, every promise settles once as the model says, the
//! slots hold versions of their states, and a URL matches the longest
//! scope that is a prefix of it.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    engine::service_worker::fuzz_registry(data);
});
