//! The HTTP/2 client's session (M38): any bytes as a server's, over
//! three streams, never panic, never give a stream an event after its
//! end, and leave every stream ended when the session ends
//! (`net::h2::fuzz_session` holds the checks).

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    net::h2::fuzz_session(data);
});
