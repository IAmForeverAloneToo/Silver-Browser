//! Web Crypto's key import (G255) on any bytes: as raw key data, as an
//! SPKI, as a PKCS #8, and as a JWK whose members are cut from the bytes,
//! for each algorithm the store imports, RSA's private keys among them. It never panics; and a key imported
//! extractable exports, in the format it came in, to what imports again
//! as the same key; an RSA key verifies any bytes to true or false.

#![no_main]
#![forbid(unsafe_code)]

use crypto::{Call, Keys, Outcome, Usage, Value};
use libfuzzer_sys::fuzz_target;

const ALGORITHMS: &[(&str, Option<&str>, &[Usage])] = &[
    ("AES-GCM", None, &[Usage::Encrypt]),
    ("HMAC", Some("SHA-256"), &[Usage::Sign]),
    ("HKDF", None, &[Usage::DeriveBits]),
    ("PBKDF2", None, &[Usage::DeriveBits]),
    ("RSASSA-PKCS1-v1_5", Some("SHA-1"), &[Usage::Verify]),
    // A private key: as PKCS #8, parsed by the library (G255 part 2b).
    ("RSASSA-PKCS1-v1_5", Some("SHA-256"), &[Usage::Sign]),
];

fn algorithm(name: &str, hash: Option<&str>) -> Value {
    let mut members = vec![("name".to_owned(), Value::Str(name.into()))];
    if let Some(hash) = hash {
        members.push(("hash".to_owned(), Value::Str(hash.into())));
    }
    Value::Dict(members)
}

/// A JWK whose members are the bytes' pieces, cut at each 0xff, as text.
fn jwk(bytes: &[u8]) -> Value {
    const NAMES: &[&str] = &[
        "kty", "k", "n", "e", "alg", "use", "d", "p", "q", "dp", "dq", "qi",
    ];
    let pieces: Vec<String> = bytes
        .split(|&b| b == 0xff)
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    Value::Dict(
        NAMES
            .iter()
            .zip(pieces)
            .map(|(name, piece)| ((*name).to_owned(), Value::Str(piece)))
            .collect(),
    )
}

fuzz_target!(|data: &[u8]| {
    let Some((&choice, rest)) = data.split_first() else {
        return;
    };
    let (name, hash, usages) = ALGORITHMS[usize::from(choice) % ALGORITHMS.len()];
    let format = ["raw", "spki", "jwk", "pkcs8"][usize::from(choice / 8) % 4];
    let key_data = if format == "jwk" {
        jwk(rest)
    } else {
        Value::Bytes(rest.to_vec())
    };
    let extractable = !matches!(name, "HKDF" | "PBKDF2");
    let mut keys = Keys::default();
    let import = |keys: &mut Keys, key_data: Value| {
        crypto::run(
            keys,
            Call::ImportKey {
                format: format.into(),
                key_data,
                algorithm: algorithm(name, hash),
                extractable,
                usages: usages.to_vec(),
            },
        )
    };
    let Ok(Outcome::Key(first)) = import(&mut keys, key_data) else {
        return;
    };
    // An RSA key that imports always verifies to an answer, on either of
    // its routes (G255, the lead's ruling (a)): never an error, so never
    // a refusal mistaken for a false signature or the other way round.
    if name.starts_with("RSA") && usages.contains(&Usage::Verify) {
        let signature = data.iter().rev().take(512).copied().collect();
        let verified = crypto::run(
            &mut keys,
            Call::Verify {
                algorithm: algorithm(name, hash),
                key: first.id,
                signature,
                data: data.to_vec(),
            },
        );
        assert!(
            matches!(verified, Ok(Outcome::Bool(_))),
            "an imported RSA key verifies to an answer: {verified:?}"
        );
    }
    if !first.extractable {
        return;
    }
    let exported = crypto::run(
        &mut keys,
        Call::ExportKey {
            format: format.into(),
            key: first.id,
        },
    );
    let again = match exported {
        Ok(Outcome::Bytes(bytes)) => Value::Bytes(bytes),
        Ok(Outcome::Jwk(members)) => Value::Dict(
            members
                .into_iter()
                .map(|(n, v)| {
                    let v = match v {
                        crypto::JwkValue::Str(s) => Value::Str(s),
                        crypto::JwkValue::Bool(b) => Value::Bool(b),
                        crypto::JwkValue::List(l) => {
                            Value::List(l.into_iter().map(Value::Str).collect())
                        }
                    };
                    (n, v)
                })
                .collect(),
        ),
        other => panic!("an extractable key exports: {other:?}"),
    };
    match import(&mut keys, again) {
        Ok(Outcome::Key(second)) => {
            assert_eq!(second.algorithm, first.algorithm, "the same key again");
            assert_eq!(second.kind, first.kind);
        }
        other => panic!("an export imports again: {other:?}"),
    }
});
