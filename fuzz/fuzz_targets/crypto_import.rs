//! Web Crypto's key import (G255) on any bytes: as raw key data, as an
//! SPKI, as a PKCS #8, and as a JWK whose members are cut from the bytes,
//! for each algorithm the store imports, RSA's private keys among them. It never panics; and a key imported
//! extractable exports, in the format it came in, to what imports again
//! as the same key; an RSA key verifies any bytes to true or false; and
//! ML-KEM's keys from any bytes, round-tripped (`ml_kem_round_trip`).

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

fn x25519_round_trip(bytes: &[u8]) {
    let mut keys = Keys::default();
    let import = |keys: &mut Keys, der: Vec<u8>| {
        crypto::run(
            keys,
            Call::ImportKey {
                format: "pkcs8".into(),
                key_data: Value::Bytes(der),
                algorithm: Value::Str("X25519".into()),
                extractable: true,
                usages: vec![Usage::DeriveBits],
            },
        )
    };
    let Ok(Outcome::Key(first)) = import(&mut keys, bytes.to_vec()) else {
        return;
    };
    let exported = crypto::run(
        &mut keys,
        Call::ExportKey {
            format: "pkcs8".into(),
            key: first.id,
        },
    );
    let Ok(Outcome::Bytes(der)) = exported else {
        panic!("an X25519 key exports as PKCS #8: {exported:?}");
    };
    let Ok(Outcome::Key(second)) = import(&mut keys, der) else {
        panic!("its PKCS #8 imports again");
    };
    let peer = crypto::run(
        &mut keys,
        Call::GenerateKey {
            algorithm: Value::Str("X25519".into()),
            extractable: true,
            usages: vec![Usage::DeriveBits],
        },
    );
    let Ok(Outcome::Pair(peer, _)) = peer else {
        panic!("a peer");
    };
    let derive = |keys: &mut Keys, key: u32| {
        crypto::run(
            keys,
            Call::DeriveBits {
                algorithm: Value::Dict(vec![
                    ("name".into(), Value::Str("X25519".into())),
                    ("public".into(), Value::Key(peer.id)),
                ]),
                key,
                length: None,
            },
        )
    };
    assert_eq!(
        derive(&mut keys, first.id),
        derive(&mut keys, second.id),
        "the same secret"
    );
}

/// ML-KEM's keys from any bytes (G255 step 5c, the library's seed key
/// generation, `native`): the bytes as a seed and as a public key, taken
/// or refused, never a panic; and the first 64 of them, padded, always a
/// seed, whose key decapsulates to the secret its public key
/// (getPublicKey's) encapsulates, and whose PKCS #8 imports again as the
/// same key.
fn ml_kem_round_trip(bytes: &[u8]) {
    let all = [
        Usage::EncapsulateKey,
        Usage::EncapsulateBits,
        Usage::DecapsulateKey,
        Usage::DecapsulateBits,
    ];
    for name in ["ML-KEM-768", "ML-KEM-1024"] {
        let mut keys = Keys::default();
        let import = |keys: &mut Keys, format: &str, data: Vec<u8>, usages: &[Usage]| {
            crypto::run(
                keys,
                Call::ImportKey {
                    format: format.into(),
                    key_data: Value::Bytes(data),
                    algorithm: Value::Str(name.into()),
                    extractable: true,
                    usages: usages.to_vec(),
                },
            )
        };
        let _ = import(&mut keys, "raw-seed", bytes.to_vec(), &all[2..]);
        let _ = import(&mut keys, "raw-public", bytes.to_vec(), &all[..2]);
        let mut seed: Vec<u8> = bytes.iter().copied().take(64).collect();
        seed.resize(64, 0);
        let Ok(Outcome::Key(private)) = import(&mut keys, "raw-seed", seed, &all[2..]) else {
            panic!("a 64-byte seed imports");
        };
        let Ok(Outcome::Key(public)) = crypto::run(
            &mut keys,
            Call::GetPublicKey {
                key: private.id,
                usages: vec![Usage::EncapsulateBits],
            },
        ) else {
            panic!("its public key");
        };
        let Ok(Outcome::Encapsulated { ciphertext, shared }) = crypto::run(
            &mut keys,
            Call::EncapsulateBits {
                algorithm: Value::Str(name.into()),
                key: public.id,
            },
        ) else {
            panic!("an encapsulation");
        };
        let again = crypto::run(
            &mut keys,
            Call::DecapsulateBits {
                algorithm: Value::Str(name.into()),
                key: private.id,
                ciphertext,
            },
        );
        assert_eq!(again.ok(), Some(*shared), "the same secret");
        let Ok(Outcome::Bytes(pkcs8)) = crypto::run(
            &mut keys,
            Call::ExportKey {
                format: "pkcs8".into(),
                key: private.id,
            },
        ) else {
            panic!("its PKCS #8");
        };
        let Ok(Outcome::Key(copy)) = import(&mut keys, "pkcs8", pkcs8, &all[2..]) else {
            panic!("its PKCS #8 imports again");
        };
        let public_of = |keys: &mut Keys, key: u32| {
            crypto::run(
                keys,
                Call::GetPublicKey {
                    key,
                    usages: Vec::new(),
                },
            )
            .and_then(|out| match out {
                Outcome::Key(info) => crypto::run(
                    keys,
                    Call::ExportKey {
                        format: "raw-public".into(),
                        key: info.id,
                    },
                ),
                other => Ok(other),
            })
        };
        assert_eq!(
            public_of(&mut keys, private.id),
            public_of(&mut keys, copy.id),
            "the same key"
        );
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&choice, rest)) = data.split_first() else {
        return;
    };
    // A CryptoKey's clone record from another renderer or the disk (G255
    // step 4): any bytes read as one, never a panic.
    let _ = crypto::deserialize_key(&mut Keys::default(), data);
    let (name, hash, usages) = ALGORITHMS[usize::from(choice) % ALGORITHMS.len()];
    let format = ["raw", "spki", "jwk", "pkcs8"][usize::from(choice / 8) % 4];
    // An X25519 PKCS #8 read by the library (G255 step 3, the lead's
    // ruling): any bytes; one that imports exports and imports again,
    // both deriving the same secret with a peer made here.
    if format == "pkcs8" {
        x25519_round_trip(rest);
    }
    // One input in 32: ML-KEM's keys cost more than an import's checks.
    if choice % 32 == 31 {
        ml_kem_round_trip(rest);
    }
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
