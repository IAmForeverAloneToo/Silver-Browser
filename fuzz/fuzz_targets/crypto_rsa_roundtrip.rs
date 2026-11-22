//! RSA's sign-then-verify on keys each side of aws-lc-rs's ranges (G255,
//! the lead's addition to ruling (a)): a signature the library made, of
//! any message, under the hash the input picks, verifies true through
//! Web Crypto's own calls on whichever route the key takes, and another
//! message false. Never a silent false. Kept apart from crypto_import so
//! that the import parsers keep their throughput.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&choice, message)) = data.split_first() else {
        return;
    };
    crypto::fuzzing::rsa_signed_natively_verifies(choice, message);
});
