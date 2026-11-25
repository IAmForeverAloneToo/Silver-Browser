//! ML-KEM's and ML-DSA's keys and operations (G255 steps 5b and 5c) on
//! any bytes. A ciphertext mutated by the input decapsulates to a shared
//! key, FIPS 203's implicit rejection, never an error or a panic, and the
//! untouched one to the encapsulated key; each kind of key's export,
//! mutated, imports or is refused (a DataError), never a panic, and one
//! taken works (an ML-KEM public key encapsulates, a seed's key
//! decapsulates what its public key encapsulates, an ML-DSA public key
//! verifies to true or false); an ML-DSA signature mutated verifies
//! false, the untouched one true.

#![no_main]
#![forbid(unsafe_code)]

use std::cell::RefCell;

use crypto::{Call, Keys, Outcome, Usage, Value};
use libfuzzer_sys::fuzz_target;

const ALGORITHMS: [&str; 5] = [
    "ML-KEM-768",
    "ML-KEM-1024",
    "ML-DSA-44",
    "ML-DSA-65",
    "ML-DSA-87",
];

/// A pair of each algorithm, made once and kept as its public key's raw
/// bytes and its seed: each input makes its keys again in a store of its
/// own, so that nothing it imports outlives it.
struct Made {
    pairs: Vec<(Vec<u8>, Vec<u8>)>,
}

thread_local! {
    static MADE: RefCell<Option<Made>> = const { RefCell::new(None) };
}

fn usages(name: &str) -> Vec<Usage> {
    if name.starts_with("ML-KEM") {
        vec![
            Usage::EncapsulateKey,
            Usage::EncapsulateBits,
            Usage::DecapsulateKey,
            Usage::DecapsulateBits,
        ]
    } else {
        vec![Usage::Sign, Usage::Verify]
    }
}

fn made() -> Made {
    let mut keys = Keys::default();
    let pairs = ALGORITHMS
        .iter()
        .map(|name| {
            match crypto::run(
                &mut keys,
                Call::GenerateKey {
                    algorithm: Value::Str((*name).into()),
                    extractable: true,
                    usages: usages(name),
                },
            ) {
                Ok(Outcome::Pair(public, private)) => (
                    export(&mut keys, public.id, "raw-public"),
                    export(&mut keys, private.id, "raw-seed"),
                ),
                other => panic!("{name}: {other:?}"),
            }
        })
        .collect();
    Made { pairs }
}

/// `bytes` mutated as the input says: pairs of (position, xor), then a
/// length to cut to or grow by.
fn mutated(mut bytes: Vec<u8>, input: &[u8]) -> Vec<u8> {
    let mut pairs = input.chunks_exact(3);
    for pair in &mut pairs {
        if bytes.is_empty() {
            break;
        }
        let at = usize::from(u16::from_le_bytes([pair[0], pair[1]])) % bytes.len();
        bytes[at] ^= pair[2];
    }
    match pairs.remainder() {
        [cut] if cut & 1 == 0 => bytes.truncate(bytes.len().saturating_sub(usize::from(*cut >> 1))),
        [grow] => bytes.extend(std::iter::repeat_n(0x5a, usize::from(*grow >> 1))),
        _ => {}
    }
    bytes
}

fn export(keys: &mut Keys, key: u32, format: &str) -> Vec<u8> {
    match crypto::run(
        keys,
        Call::ExportKey {
            format: format.into(),
            key,
        },
    ) {
        Ok(Outcome::Bytes(bytes)) => bytes,
        other => panic!("{format}: {other:?}"),
    }
}

fn import(keys: &mut Keys, format: &str, bytes: Vec<u8>, name: &str, private: bool) -> Option<u32> {
    let usages: Vec<Usage> = usages(name)
        .into_iter()
        .filter(|u| {
            private
                == matches!(
                    u,
                    Usage::Sign | Usage::DecapsulateKey | Usage::DecapsulateBits
                )
        })
        .collect();
    match crypto::run(
        keys,
        Call::ImportKey {
            format: format.into(),
            key_data: Value::Bytes(bytes),
            algorithm: Value::Str(name.into()),
            extractable: true,
            usages,
        },
    ) {
        Ok(Outcome::Key(key)) => Some(key.id),
        Err(error) => {
            assert!(
                matches!(error.name, "DataError"),
                "{name} {format}: refused as {}: {}",
                error.name,
                error.message
            );
            None
        }
        other => panic!("{name} {format}: {other:?}"),
    }
}

fn encapsulate(keys: &mut Keys, name: &str, key: u32) -> (Vec<u8>, Vec<u8>) {
    match crypto::run(
        keys,
        Call::EncapsulateBits {
            algorithm: Value::Str(name.into()),
            key,
        },
    ) {
        Ok(Outcome::Encapsulated { ciphertext, shared }) => match *shared {
            Outcome::Bytes(shared) => (ciphertext, shared),
            other => panic!("{other:?}"),
        },
        other => panic!("{name}: {other:?}"),
    }
}

fn decapsulate(keys: &mut Keys, name: &str, key: u32, ciphertext: Vec<u8>) -> Vec<u8> {
    match crypto::run(
        keys,
        Call::DecapsulateBits {
            algorithm: Value::Str(name.into()),
            key,
            ciphertext,
        },
    ) {
        Ok(Outcome::Bytes(shared)) => shared,
        other => panic!("{name}: a ciphertext of its length decapsulates: {other:?}"),
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&choice, rest)) = data.split_first() else {
        return;
    };
    MADE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let made = cell.get_or_insert_with(made);
        let at = usize::from(choice) % ALGORITHMS.len();
        let name = ALGORITHMS[at];
        let (public_bytes, seed) = made.pairs[at].clone();
        let keys = &mut Keys::default();
        let public = import(keys, "raw-public", public_bytes, name, false).expect("its public key");
        let private = import(keys, "raw-seed", seed, name, true).expect("its seed");
        let kem = name.starts_with("ML-KEM");
        match (choice / 8) % 4 {
            // A ciphertext mutated in place: implicit rejection's key.
            0 if kem => {
                let (ciphertext, shared) = encapsulate(keys, name, public);
                let changed = mutated(ciphertext.clone(), rest);
                let same_length = changed.len() == ciphertext.len();
                if !same_length {
                    let out = crypto::run(
                        keys,
                        Call::DecapsulateBits {
                            algorithm: Value::Str(name.into()),
                            key: private,
                            ciphertext: changed,
                        },
                    );
                    assert!(matches!(&out, Err(e) if e.name == "DataError"), "{out:?}");
                    return;
                }
                let got = decapsulate(keys, name, private, changed.clone());
                assert_eq!(got.len(), 32);
                assert_eq!(got == shared, changed == ciphertext, "{name}");
            }
            // A signature mutated: false, the untouched one true.
            0 => {
                let signature = match crypto::run(
                    keys,
                    Call::Sign {
                        algorithm: Value::Str(name.into()),
                        key: private,
                        data: rest.to_vec(),
                    },
                ) {
                    Ok(Outcome::Bytes(signature)) => signature,
                    other => panic!("{name}: {other:?}"),
                };
                let changed = mutated(signature.clone(), rest);
                let out = crypto::run(
                    keys,
                    Call::Verify {
                        algorithm: Value::Str(name.into()),
                        key: public,
                        signature: changed.clone(),
                        data: rest.to_vec(),
                    },
                );
                assert_eq!(out, Ok(Outcome::Bool(changed == signature)), "{name}");
            }
            // A public key mutated, raw or in its SPKI.
            case @ (1 | 2) => {
                let format = if case == 1 { "raw-public" } else { "spki" };
                let bytes = mutated(export(keys, public, format), rest);
                let Some(key) = import(keys, format, bytes, name, false) else {
                    return;
                };
                if kem {
                    let (ciphertext, _) = encapsulate(keys, name, key);
                    assert!(!ciphertext.is_empty());
                } else {
                    let out = crypto::run(
                        keys,
                        Call::Verify {
                            algorithm: Value::Str(name.into()),
                            key,
                            signature: rest.to_vec(),
                            data: rest.to_vec(),
                        },
                    );
                    assert!(matches!(out, Ok(Outcome::Bool(_))), "{name}: {out:?}");
                }
            }
            // A private key mutated, its seed or its PKCS #8: one taken
            // agrees with its own public key.
            _ => {
                let format = if choice & 1 == 0 { "raw-seed" } else { "pkcs8" };
                let bytes = mutated(export(keys, private, format), rest);
                let Some(key) = import(keys, format, bytes, name, true) else {
                    return;
                };
                let usage = if kem {
                    Usage::EncapsulateBits
                } else {
                    Usage::Verify
                };
                let derived = match crypto::run(
                    keys,
                    Call::GetPublicKey {
                        key,
                        usages: vec![usage],
                    },
                ) {
                    Ok(Outcome::Key(info)) => info.id,
                    other => panic!("{name}: {other:?}"),
                };
                if kem {
                    let (ciphertext, shared) = encapsulate(keys, name, derived);
                    assert_eq!(decapsulate(keys, name, key, ciphertext), shared, "{name}");
                } else {
                    let signature = match crypto::run(
                        keys,
                        Call::Sign {
                            algorithm: Value::Str(name.into()),
                            key,
                            data: rest.to_vec(),
                        },
                    ) {
                        Ok(Outcome::Bytes(signature)) => signature,
                        other => panic!("{name}: {other:?}"),
                    };
                    let out = crypto::run(
                        keys,
                        Call::Verify {
                            algorithm: Value::Str(name.into()),
                            key: derived,
                            signature,
                            data: rest.to_vec(),
                        },
                    );
                    assert_eq!(out, Ok(Outcome::Bool(true)), "{name}");
                }
            }
        }
    });
});
