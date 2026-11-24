//! The operations against published vectors, and the errors against the
//! reference browser's (Chrome 157, probed 2026-11-22/23).

use super::*;

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn s(text: &str) -> Value {
    Value::Str(text.into())
}

fn dict(members: &[(&str, Value)]) -> Value {
    Value::Dict(
        members
            .iter()
            .map(|(n, v)| ((*n).to_owned(), v.clone()))
            .collect(),
    )
}

fn bytes(out: Result<Outcome, Error>) -> Vec<u8> {
    match out {
        Ok(Outcome::Bytes(b)) => b,
        other => panic!("{other:?}"),
    }
}

fn key(out: Result<Outcome, Error>) -> KeyInfo {
    match out {
        Ok(Outcome::Key(k)) => k,
        other => panic!("{other:?}"),
    }
}

fn error(out: Result<Outcome, Error>) -> String {
    match out {
        Err(e) => format!("{}: {}", e.name, e.message),
        other => panic!("{other:?}"),
    }
}

fn import_raw(keys: &mut Keys, name: Value, raw: &[u8], usages: &[Usage]) -> KeyInfo {
    key(run(
        keys,
        Call::ImportKey {
            format: "raw".into(),
            key_data: Value::Bytes(raw.to_vec()),
            algorithm: name,
            extractable: false,
            usages: usages.to_vec(),
        },
    ))
}

#[test]
fn digests_are_the_standards() {
    let mut keys = Keys::default();
    let sha256 = bytes(run(
        &mut keys,
        Call::Digest {
            algorithm: s("sha-256"),
            data: b"abc".to_vec(),
        },
    ));
    assert_eq!(
        sha256,
        hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    );
    let sha1 = bytes(run(
        &mut keys,
        Call::Digest {
            algorithm: dict(&[("name", s("SHA-1"))]),
            data: b"abc".to_vec(),
        },
    ));
    assert_eq!(sha1, hex("a9993e364706816aba3e25717850c26c9cd0d89d"));
}

#[test]
fn hkdf_pbkdf2_and_hmac_match_their_rfcs() {
    let mut keys = Keys::default();
    // RFC 5869 appendix A.1.
    let ikm = import_raw(&mut keys, s("HKDF"), &[0x0b; 22], &[Usage::DeriveBits]);
    let okm = bytes(run(
        &mut keys,
        Call::DeriveBits {
            algorithm: dict(&[
                ("name", s("HKDF")),
                ("hash", s("SHA-256")),
                ("salt", Value::Bytes(hex("000102030405060708090a0b0c"))),
                ("info", Value::Bytes(hex("f0f1f2f3f4f5f6f7f8f9"))),
            ]),
            key: ikm.id,
            length: Some(42 * 8),
        },
    ));
    assert_eq!(
        okm,
        hex("3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865")
    );
    // Past 255 hash lengths: refused, as a length of -8 the bindings wrap
    // (as the reference's do) to 2^32 - 8 bits; at the most, derived.
    let hkdf = |keys: &mut Keys, length| {
        run(
            keys,
            Call::DeriveBits {
                algorithm: dict(&[
                    ("name", s("HKDF")),
                    ("hash", s("SHA-256")),
                    ("salt", Value::Bytes(vec![])),
                    ("info", Value::Bytes(vec![])),
                ]),
                key: ikm.id,
                length: Some(length),
            },
        )
    };
    for length in [255 * 256 + 8, u32::MAX - 7] {
        assert_eq!(
            error(hkdf(&mut keys, length)),
            "OperationError: The length provided for HKDF is too large.",
            "{length}"
        );
    }
    assert_eq!(bytes(hkdf(&mut keys, 255 * 256)).len(), 255 * 32);
    // RFC 6070's second case.
    let password = import_raw(&mut keys, s("PBKDF2"), b"password", &[Usage::DeriveBits]);
    let dk = bytes(run(
        &mut keys,
        Call::DeriveBits {
            algorithm: dict(&[
                ("name", s("PBKDF2")),
                ("salt", Value::Bytes(b"salt".to_vec())),
                ("iterations", Value::Num(2.0)),
                ("hash", s("SHA-1")),
            ]),
            key: password.id,
            length: Some(160),
        },
    ));
    assert_eq!(dk, hex("ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957"));
    // RFC 4231's first case.
    let mac_key = key(run(
        &mut keys,
        Call::ImportKey {
            format: "raw".into(),
            key_data: Value::Bytes(vec![0x0b; 20]),
            algorithm: dict(&[("name", s("HMAC")), ("hash", s("SHA-256"))]),
            extractable: false,
            usages: vec![Usage::Sign, Usage::Verify],
        },
    ));
    let mac = bytes(run(
        &mut keys,
        Call::Sign {
            algorithm: s("HMAC"),
            key: mac_key.id,
            data: b"Hi There".to_vec(),
        },
    ));
    assert_eq!(
        mac,
        hex("b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7")
    );
}

#[test]
fn aes_gcm_round_trips_and_a_wrong_tag_fails() {
    let mut keys = Keys::default();
    let k = import_raw(
        &mut keys,
        s("AES-GCM"),
        &[7; 16],
        &[Usage::Encrypt, Usage::Decrypt],
    );
    let params = dict(&[("name", s("AES-GCM")), ("iv", Value::Bytes(vec![1; 12]))]);
    let sealed = bytes(run(
        &mut keys,
        Call::Encrypt {
            algorithm: params.clone(),
            key: k.id,
            data: b"hello".to_vec(),
        },
    ));
    assert_eq!(sealed.len(), 5 + 16);
    let opened = bytes(run(
        &mut keys,
        Call::Decrypt {
            algorithm: params.clone(),
            key: k.id,
            data: sealed.clone(),
        },
    ));
    assert_eq!(opened, b"hello");
    let mut tampered = sealed;
    tampered[0] ^= 1;
    assert_eq!(
        error(run(
            &mut keys,
            Call::Decrypt {
                algorithm: params,
                key: k.id,
                data: tampered,
            },
        )),
        "OperationError: "
    );
}

#[test]
fn errors_are_the_references() {
    let mut keys = Keys::default();
    let aes = key(run(
        &mut keys,
        Call::GenerateKey {
            algorithm: dict(&[("name", s("AES-GCM")), ("length", Value::Num(128.0))]),
            extractable: false,
            usages: vec![Usage::Decrypt, Usage::Encrypt, Usage::Decrypt],
        },
    ));
    assert_eq!(aes.usages, vec![Usage::Encrypt, Usage::Decrypt]);
    let encrypt = |keys: &mut Keys, algorithm: Value| {
        error(run(
            keys,
            Call::Encrypt {
                algorithm,
                key: aes.id,
                data: vec![1],
            },
        ))
    };
    assert_eq!(
        encrypt(&mut keys, dict(&[("name", s("AES-GCM"))])),
        "TypeError: Failed to execute 'encrypt' on 'SubtleCrypto': AeadParams: iv: Missing required property"
    );
    assert_eq!(
        encrypt(&mut keys, dict(&[("name", s("AES-GCM")), ("iv", s("abc"))])),
        "TypeError: Failed to execute 'encrypt' on 'SubtleCrypto': AeadParams: iv: Not a BufferSource"
    );
    assert_eq!(
        encrypt(
            &mut keys,
            dict(&[
                ("name", s("AES-GCM")),
                ("iv", Value::Bytes(vec![0; 12])),
                ("tagLength", Value::Num(-1.0)),
            ])
        ),
        "TypeError: Failed to execute 'encrypt' on 'SubtleCrypto': AeadParams: tagLength: Outside of numeric range"
    );
    // NaN (undefined there, as the reference converts it) is not a number;
    // the infinities are out of range (probed: dict-int, undef-member).
    for (n, said) in [
        (f64::NAN, "Is not a number"),
        (f64::INFINITY, "Outside of numeric range"),
        (f64::NEG_INFINITY, "Outside of numeric range"),
    ] {
        assert_eq!(
            encrypt(
                &mut keys,
                dict(&[
                    ("name", s("AES-GCM")),
                    ("iv", Value::Bytes(vec![0; 12])),
                    ("tagLength", Value::Num(n)),
                ])
            ),
            format!(
                "TypeError: Failed to execute 'encrypt' on 'SubtleCrypto': AeadParams: tagLength: {said}"
            )
        );
    }
    assert_eq!(
        encrypt(
            &mut keys,
            dict(&[
                ("name", s("AES-GCM")),
                ("iv", Value::Bytes(vec![0; 12])),
                ("tagLength", Value::Num(0.0)),
            ])
        ),
        "OperationError: The tag length is invalid: Must be 32, 64, 96, 104, 112, 120, or 128 bits"
    );
    assert_eq!(
        error(run(
            &mut keys,
            Call::Digest {
                algorithm: dict(&[]),
                data: vec![],
            }
        )),
        "TypeError: Failed to execute 'digest' on 'SubtleCrypto': Algorithm: name: Missing or not a string"
    );
    assert_eq!(
        error(run(
            &mut keys,
            Call::Digest {
                algorithm: Value::Num(5.0),
                data: vec![],
            }
        )),
        "NotSupportedError: Failed to execute 'digest' on 'SubtleCrypto': Algorithm: Unrecognized name"
    );
    let hmac = |keys: &mut Keys, hash: Value| {
        error(run(
            keys,
            Call::GenerateKey {
                algorithm: dict(&[("name", s("HMAC")), ("hash", hash)]),
                extractable: true,
                usages: vec![Usage::Sign],
            },
        ))
    };
    assert_eq!(
        hmac(&mut keys, Value::Absent),
        "TypeError: Failed to execute 'generateKey' on 'SubtleCrypto': HmacKeyGenParams: hash: Missing or not an AlgorithmIdentifier"
    );
    assert_eq!(
        hmac(&mut keys, s("SHA-3")),
        "NotSupportedError: Failed to execute 'generateKey' on 'SubtleCrypto': HmacKeyGenParams: hash: Algorithm: Unrecognized name"
    );
    assert_eq!(
        hmac(&mut keys, s("AES-GCM")),
        "NotSupportedError: Failed to execute 'generateKey' on 'SubtleCrypto': HmacKeyGenParams: hash: AES-GCM: Unsupported operation: digest"
    );
    assert_eq!(
        error(run(
            &mut keys,
            Call::GenerateKey {
                algorithm: dict(&[("name", s("AES-GCM"))]),
                extractable: true,
                usages: vec![Usage::Encrypt],
            }
        )),
        "TypeError: Failed to execute 'generateKey' on 'SubtleCrypto': AesKeyGenParams: length: Missing required property"
    );
    assert_eq!(
        error(run(
            &mut keys,
            Call::ExportKey {
                format: "raw".into(),
                key: aes.id,
            }
        )),
        "InvalidAccessError: Failed to execute 'exportKey' on 'SubtleCrypto': key is not extractable"
    );
    assert_eq!(
        error(run(
            &mut keys,
            Call::ImportKey {
                format: "raw".into(),
                key_data: Value::Bytes(b"pw".to_vec()),
                algorithm: s("PBKDF2"),
                extractable: true,
                usages: vec![Usage::DeriveBits],
            }
        )),
        "SyntaxError: KDF keys must set extractable=false"
    );
}

#[test]
fn a_jwks_members_are_checked_as_the_reference_checks_them() {
    let mut keys = Keys::default();
    let import = |keys: &mut Keys, members: &[(&str, Value)], extractable: bool| {
        run(
            keys,
            Call::ImportKey {
                format: "jwk".into(),
                key_data: dict(members),
                algorithm: s("AES-GCM"),
                extractable,
                usages: vec![Usage::Encrypt],
            },
        )
    };
    let k = s("AAAAAAAAAAAAAAAAAAAAAA");
    assert_eq!(
        error(import(&mut keys, &[("kty", s("oct"))], true)),
        "DataError: The required JWK member \"k\" was missing"
    );
    assert_eq!(
        error(import(
            &mut keys,
            &[("k", k.clone()), ("kty", s("RSA"))],
            true
        )),
        "DataError: The JWK \"kty\" member was not \"oct\""
    );
    assert_eq!(
        error(import(
            &mut keys,
            &[("k", s("!!!")), ("kty", s("oct"))],
            true
        )),
        "DataError: The JWK member \"k\" could not be base64url decoded or contained padding"
    );
    assert_eq!(
        error(import(
            &mut keys,
            &[("alg", s("A256GCM")), ("k", k.clone()), ("kty", s("oct"))],
            true
        )),
        "DataError: The JWK \"k\" member did not include the right length of key data for the given algorithm."
    );
    assert_eq!(
        error(import(
            &mut keys,
            &[
                ("ext", Value::Bool(false)),
                ("k", k.clone()),
                ("kty", s("oct"))
            ],
            true
        )),
        "DataError: The \"ext\" member of the JWK dictionary is inconsistent what that specified by the Web Crypto call"
    );
    let ok = key(import(&mut keys, &[("k", k), ("kty", s("oct"))], true));
    match run(
        &mut keys,
        Call::ExportKey {
            format: "jwk".into(),
            key: ok.id,
        },
    ) {
        Ok(Outcome::Jwk(members)) => {
            let names: Vec<&str> = members.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(names, ["alg", "ext", "k", "key_ops", "kty"]);
            assert_eq!(members[0].1, JwkValue::Str("A128GCM".into()));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn protons_rs1_support_check_imports_and_exports_back() {
    // The key Proton's page imports as it loads, to see RSASSA with SHA-1.
    let mut keys = Keys::default();
    let n =
        "rVMOV0fT7doXOgDU-drrk9yXIgEsB9aK5x9fSOpRuWa8aS9hmX9obPoOub1Boz0Rk_C4maZMKo8m7mHx3n6JXQ";
    let rs1 = key(run(
        &mut keys,
        Call::ImportKey {
            format: "jwk".into(),
            key_data: dict(&[
                ("alg", s("RS1")),
                ("e", s("AQAB")),
                ("kty", s("RSA")),
                ("n", s(n)),
            ]),
            algorithm: dict(&[
                ("name", s("RSASSA-PKCS1-v1_5")),
                ("hash", dict(&[("name", s("SHA-1"))])),
            ]),
            extractable: true,
            usages: vec![Usage::Verify],
        },
    ));
    assert_eq!(rs1.kind, KeyType::Public);
    assert_eq!(
        rs1.algorithm,
        vec![
            ("name", AlgorithmValue::Str("RSASSA-PKCS1-v1_5".into())),
            ("hash", AlgorithmValue::Named("SHA-1".into())),
            ("modulusLength", AlgorithmValue::Num(512)),
            ("publicExponent", AlgorithmValue::Bytes(vec![1, 0, 1])),
        ]
    );
    let spki = bytes(run(
        &mut keys,
        Call::ExportKey {
            format: "spki".into(),
            key: rs1.id,
        },
    ));
    let again = key(run(
        &mut keys,
        Call::ImportKey {
            format: "spki".into(),
            key_data: Value::Bytes(spki),
            algorithm: dict(&[("name", s("RSASSA-PKCS1-v1_5")), ("hash", s("SHA-1"))]),
            extractable: true,
            usages: vec![Usage::Verify],
        },
    ));
    assert_eq!(again.algorithm, rs1.algorithm);
}

#[test]
fn aes_gcm_takes_any_iv_and_tag_the_reference_allows() {
    let mut keys = Keys::default();
    let k = import_raw(
        &mut keys,
        s("AES-GCM"),
        &[7; 32],
        &[Usage::Encrypt, Usage::Decrypt],
    );
    // As the reference: a one-byte IV and a 96-bit tag.
    let params = dict(&[
        ("name", s("AES-GCM")),
        ("iv", Value::Bytes(vec![1])),
        ("tagLength", Value::Num(96.0)),
    ]);
    let sealed = bytes(run(
        &mut keys,
        Call::Encrypt {
            algorithm: params.clone(),
            key: k.id,
            data: b"x".to_vec(),
        },
    ));
    assert_eq!(sealed.len(), 1 + 12);
    let opened = bytes(run(
        &mut keys,
        Call::Decrypt {
            algorithm: params,
            key: k.id,
            data: sealed,
        },
    ));
    assert_eq!(opened, b"x");
}

#[test]
fn base64url_round_trips_and_refuses_padding() {
    for len in 0..20 {
        let data: Vec<u8> = (0..len as u8).map(|b| b.wrapping_mul(37)).collect();
        assert_eq!(jwk::decode(&jwk::encode(&data)), Some(data));
    }
    assert_eq!(jwk::decode("AQAB"), Some(vec![1, 0, 1]));
    assert_eq!(jwk::decode("AQ=="), None);
    assert_eq!(jwk::decode("A"), None);
}

/// The lead's ruling (a): RSASSA-PKCS1-v1_5 verification runs on
/// aws-lc-rs's safe API inside its ranges and on the library's own calls
/// outside them, each side of each boundary pinned.
#[test]
fn pkcs1_verification_is_routed_by_the_safe_apis_ranges() {
    use crate::algorithms::{Route, pkcs1_verify_route};
    let safe = |hash, bits| matches!(pkcs1_verify_route(hash, bits), Route::Safe(_));
    for hash in ["SHA-1", "SHA-256", "SHA-512"] {
        assert!(!safe(hash, 1023) && safe(hash, 1024), "{hash} at 1024");
        assert!(safe(hash, 8192) && !safe(hash, 8193), "{hash} at 8192");
    }
    assert!(
        !safe("SHA-384", 2047) && safe("SHA-384", 2048),
        "SHA-384 at 2048"
    );
    assert!(
        safe("SHA-384", 8192) && !safe("SHA-384", 8193),
        "SHA-384 at 8192"
    );
}

/// Keys of 512 to 16384 bits, each side of each range (openssl's,
/// `tests/rsa_pkcs1.txt`): imported as SPKI, a good signature verifies
/// and a flipped one does not, on the key's route; and the library's
/// own verification agrees wherever the safe API runs too. Never a
/// false verify for want of a key (the reference verifies them all).
#[test]
fn pkcs1_verifies_at_every_size_the_reference_does() {
    let vectors = include_str!("../tests/rsa_pkcs1.txt");
    let mut seen = (0, 0);
    for line in vectors.lines().filter(|l| !l.starts_with('#')) {
        let parts: Vec<&str> = line.split(' ').collect();
        let (bits, hash, spki, signature) = (parts[0], parts[1], hex(parts[2]), hex(parts[3]));
        let mut keys = Keys::default();
        let k = key(run(
            &mut keys,
            Call::ImportKey {
                format: "spki".into(),
                key_data: Value::Bytes(spki),
                algorithm: dict(&[("name", s("RSASSA-PKCS1-v1_5")), ("hash", s(hash))]),
                extractable: true,
                usages: vec![Usage::Verify],
            },
        ));
        let verify = |keys: &mut Keys, signature: Vec<u8>| {
            run(
                keys,
                Call::Verify {
                    algorithm: s("RSASSA-PKCS1-v1_5"),
                    key: k.id,
                    signature,
                    data: b"routed by size".to_vec(),
                },
            )
        };
        assert_eq!(
            verify(&mut keys, signature.clone()),
            Ok(Outcome::Bool(true)),
            "{bits} {hash}"
        );
        let mut flipped = signature.clone();
        flipped[signature.len() / 2] ^= 1;
        assert_eq!(
            verify(&mut keys, flipped.clone()),
            Ok(Outcome::Bool(false)),
            "{bits} {hash} flipped"
        );
        assert_eq!(
            verify(&mut keys, signature[1..].to_vec()),
            Ok(Outcome::Bool(false)),
            "{bits} {hash} short"
        );
        let bits: u32 = bits.parse().unwrap();
        match crate::algorithms::pkcs1_verify_route(hash, bits) {
            crate::algorithms::Route::Safe(_) => seen.0 += 1,
            crate::algorithms::Route::Native(_) => seen.1 += 1,
        }
        // The library's own verification of the same key and signatures.
        let Material::RsaPublic { n, e } = &keys.get(k.id).unwrap().material else {
            panic!("a public key");
        };
        let der = crate::algorithms::rsa_public_key_der(n, e);
        let native_hash = match hash {
            "SHA-1" => crate::native::Hash::Sha1,
            "SHA-256" => crate::native::Hash::Sha256,
            "SHA-384" => crate::native::Hash::Sha384,
            _ => crate::native::Hash::Sha512,
        };
        let algorithm = crate::algorithms::digest_algorithm(hash);
        let digest = aws_lc_rs::digest::digest(algorithm, b"routed by size");
        let native =
            |sig: &[u8]| crate::native::rsa_verify_pkcs1(&der, native_hash, digest.as_ref(), sig);
        assert_eq!(native(&signature), Ok(true), "{bits} {hash} native");
        assert_eq!(native(&flipped), Ok(false), "{bits} {hash} native flipped");
        // A key the library refuses as the native route verifies is an
        // OperationError, never `false`.
        if matches!(
            crate::algorithms::pkcs1_verify_route(hash, bits),
            crate::algorithms::Route::Native(_)
        ) {
            let refused =
                crate::native::tests::failing("RsaCheck", || verify(&mut keys, signature.clone()));
            assert_eq!(error(refused), "OperationError: ", "{bits} {hash} refused");
        }
    }
    assert!(seen.0 >= 10 && seen.1 >= 10, "routes seen {seen:?}");
}

/// A key the library refuses is refused at its import (DataError), so
/// that no verification of it is answered false: a modulus past 16384
/// bits.
#[test]
fn an_rsa_key_past_the_librarys_bound_is_refused_at_import() {
    let mut n = vec![0u8; 2049];
    n[0] = 0x80;
    n[2048] = 1;
    let mut keys = Keys::default();
    let b64 = |b: &[u8]| jwk::encode(b);
    let out = run(
        &mut keys,
        Call::ImportKey {
            format: "jwk".into(),
            key_data: dict(&[("kty", s("RSA")), ("e", s("AQAB")), ("n", s(&b64(&n)))]),
            algorithm: dict(&[("name", s("RSASSA-PKCS1-v1_5")), ("hash", s("SHA-256"))]),
            extractable: true,
            usages: vec![Usage::Verify],
        },
    );
    assert_eq!(error(out), "DataError: The RSA key is not valid");
}

/// The lead's addition to ruling (a), over every key and hash of the
/// pool and a few messages: a signature the library made verifies true
/// (and another message false) whichever route its key takes.
#[test]
fn a_signature_the_library_made_always_verifies() {
    for choice in 0..36u8 {
        for message in [&b""[..], b"routed by size", &[0xa5; 300]] {
            rsa_signed_natively_verifies(choice, message);
        }
    }
}

fn rsa_gen(bits: u32, e: &[u8]) -> Value {
    dict(&[
        ("name", s("RSASSA-PKCS1-v1_5")),
        ("modulusLength", Value::Num(f64::from(bits))),
        ("publicExponent", Value::Bytes(e.to_vec())),
        ("hash", s("SHA-256")),
    ])
}

fn generate_pair(keys: &mut Keys, bits: u32, e: &[u8], hash: &str) -> (KeyInfo, KeyInfo) {
    let mut algorithm = rsa_gen(bits, e);
    if let Value::Dict(members) = &mut algorithm {
        members[3].1 = s(hash);
    }
    match run(
        keys,
        Call::GenerateKey {
            algorithm,
            extractable: true,
            usages: vec![Usage::Sign, Usage::Verify],
        },
    ) {
        Ok(Outcome::Pair(public, private)) => (public, private),
        other => panic!("{bits} {hash}: {other:?}"),
    }
}

fn sign_with(keys: &mut Keys, key: u32, hash: &str, data: &[u8]) -> Result<Outcome, Error> {
    let _ = hash;
    run(
        keys,
        Call::Sign {
            algorithm: s("RSASSA-PKCS1-v1_5"),
            key,
            data: data.to_vec(),
        },
    )
}

fn verify_with(
    keys: &mut Keys,
    key: u32,
    signature: Vec<u8>,
    data: &[u8],
) -> Result<Outcome, Error> {
    run(
        keys,
        Call::Verify {
            algorithm: s("RSASSA-PKCS1-v1_5"),
            key,
            signature,
            data: data.to_vec(),
        },
    )
}

/// generateKey's errors in the reference's order (probed, Chrome 157):
/// the usages' kinds, the modulus, the exponent, the generation (none
/// under 512 bits, an OperationError), then a private key with no usage.
#[test]
fn rsa_generation_refuses_as_the_reference_does() {
    let mut keys = Keys::default();
    let generate = |keys: &mut Keys, bits, e: &[u8], usages: Vec<Usage>| {
        run(
            keys,
            Call::GenerateKey {
                algorithm: rsa_gen(bits, e),
                extractable: true,
                usages,
            },
        )
    };
    let e = [1, 0, 1];
    let modulus =
        "OperationError: The modulus length must be a multiple of 8 bits and >= 256 and <= 8192";
    assert_eq!(
        error(generate(
            &mut keys,
            1024,
            &e,
            vec![Usage::Sign, Usage::Encrypt]
        )),
        "SyntaxError: Cannot create a key using the specified key usages."
    );
    assert_eq!(error(generate(&mut keys, 100, &e, vec![])), modulus);
    for bits in [248, 1023, 8200] {
        assert_eq!(
            error(generate(&mut keys, bits, &e, vec![Usage::Sign])),
            modulus,
            "{bits}"
        );
    }
    let exponent = "OperationError: The \"publicExponent\" must be either 3 or 65537";
    assert_eq!(
        error(generate(&mut keys, 1024, &[17], vec![Usage::Verify])),
        exponent
    );
    assert_eq!(
        error(generate(&mut keys, 1024, &[1, 0, 0, 1], vec![Usage::Sign])),
        exponent
    );
    assert_eq!(
        error(generate(&mut keys, 100, &[17], vec![Usage::Sign])),
        modulus
    );
    for bits in [256, 264, 504] {
        assert_eq!(
            error(generate(&mut keys, bits, &e, vec![Usage::Verify])),
            "OperationError: ",
            "{bits}"
        );
    }
    assert_eq!(
        error(generate(&mut keys, 1024, &e, vec![Usage::Verify])),
        "SyntaxError: Usages cannot be empty when creating a key."
    );
    // A pair: the public key extractable whatever was asked, each with
    // its own usages; an exponent's leading zeros gone.
    let Ok(Outcome::Pair(public, private)) = run(
        &mut keys,
        Call::GenerateKey {
            algorithm: rsa_gen(512, &[0, 0, 3]),
            extractable: false,
            usages: vec![Usage::Sign],
        },
    ) else {
        panic!("a pair");
    };
    assert_eq!(
        (public.kind, public.extractable, public.usages.clone()),
        (KeyType::Public, true, vec![])
    );
    assert_eq!(
        (private.kind, private.extractable, private.usages.clone()),
        (KeyType::Private, false, vec![Usage::Sign])
    );
    assert_eq!(
        private.algorithm,
        vec![
            ("name", AlgorithmValue::Str("RSASSA-PKCS1-v1_5".into())),
            ("hash", AlgorithmValue::Named("SHA-256".into())),
            ("modulusLength", AlgorithmValue::Num(512)),
            ("publicExponent", AlgorithmValue::Bytes(vec![3])),
        ]
    );
    assert_eq!(public.algorithm, private.algorithm);
}

/// Signing's route: aws-lc-rs for SHA-256, SHA-384 and SHA-512 at 2048 to
/// 8192 bits; SHA-1 and every other size, the library.
#[test]
fn pkcs1_signing_is_routed_by_the_safe_apis_ranges() {
    use crate::algorithms::pkcs1_sign_route as route;
    for hash in ["SHA-256", "SHA-384", "SHA-512"] {
        assert!(
            route(hash, 2047).is_none() && route(hash, 2048).is_some(),
            "{hash} at 2048"
        );
        assert!(
            route(hash, 8192).is_some() && route(hash, 8193).is_none(),
            "{hash} at 8192"
        );
    }
    assert!(route("SHA-1", 2048).is_none() && route("SHA-1", 4096).is_none());
}

/// Keys of each side of each range, generated on their route, sign on
/// theirs and verify on the public key's: true, and a changed message
/// false; a digest too long for the key an OperationError (the
/// reference's, at 512 bits with SHA-384).
#[test]
fn generated_rsa_keys_sign_and_verify_at_every_size() {
    let mut keys = Keys::default();
    for (bits, e, hashes) in [
        (512, &[3u8][..], &["SHA-1", "SHA-256"][..]),
        (1016, &[1, 0, 1], &["SHA-1", "SHA-384", "SHA-512"]),
        (
            2048,
            &[1, 0, 1],
            &["SHA-1", "SHA-256", "SHA-384", "SHA-512"],
        ),
        (2048, &[3], &["SHA-256"]),
        (2056, &[1, 0, 1], &["SHA-256", "SHA-512"]),
    ] {
        for hash in hashes {
            let (public, private) = generate_pair(&mut keys, bits, e, hash);
            let signature = bytes(sign_with(&mut keys, private.id, hash, b"sign me"));
            // The library makes a key of the size asked rounded down to a
            // multiple of 128 bits, and says so, as the reference does
            // (probed: 1016 makes 896, 2040 makes 1920, 3000 makes 2944).
            let made = bits / 128 * 128;
            assert_eq!(signature.len() as u32 * 8, made, "{bits} {hash}");
            assert!(
                private
                    .algorithm
                    .contains(&("modulusLength", AlgorithmValue::Num(made))),
                "{bits} {hash}"
            );
            assert_eq!(
                verify_with(&mut keys, public.id, signature.clone(), b"sign me"),
                Ok(Outcome::Bool(true)),
                "{bits} {hash}"
            );
            assert_eq!(
                verify_with(&mut keys, public.id, signature, b"sign me too"),
                Ok(Outcome::Bool(false)),
                "{bits} {hash} changed"
            );
        }
    }
    let (_, private) = generate_pair(&mut keys, 512, &[1, 0, 1], "SHA-384");
    assert_eq!(
        error(sign_with(&mut keys, private.id, "SHA-384", b"x")),
        "OperationError: "
    );
}

/// PKCS #1 v1.5's signatures are deterministic: inside aws-lc-rs's range
/// its signature and the library's are the same bytes.
#[test]
fn the_safe_and_native_signatures_are_the_same_bytes() {
    let key = crate::native::rsa_generate(2048, 65537).unwrap();
    let parts = key.parts();
    let pair = crate::algorithms::safe_key_pair_for_tests(&parts);
    for (hash, encoding) in [
        ("SHA-256", &aws_lc_rs::signature::RSA_PKCS1_SHA256),
        ("SHA-512", &aws_lc_rs::signature::RSA_PKCS1_SHA512),
    ] {
        let mut safe = vec![0u8; pair.public_modulus_len()];
        pair.sign(
            encoding,
            &aws_lc_rs::rand::SystemRandom::new(),
            b"same",
            &mut safe,
        )
        .unwrap();
        let digest = aws_lc_rs::digest::digest(crate::algorithms::digest_algorithm(hash), b"same");
        let native_hash = if hash == "SHA-256" {
            crate::native::Hash::Sha256
        } else {
            crate::native::Hash::Sha512
        };
        let native = crate::native::rsa_sign_pkcs1(&key, native_hash, digest.as_ref()).unwrap();
        assert_eq!(safe, native, "{hash}");
    }
}

/// Two keys too small to make through Web Crypto (made once by a script,
/// for the wrapper's length forms): n, e, d, p, q, dp, dq, qi.
const TINY: [[&str; 8]; 2] = [
    // 128 bits: every length of the PKCS #8 in DER's short form.
    [
        "946e350f1194868dd058b00ee0ddb405",
        "010001",
        "4adc0e5c8c34496ba5fe54835458f25d",
        "c506cd2515531ebb",
        "c0dbb8ffccd32c3f",
        "3ceae06cd1ce1457",
        "5b69ca8eb52f8351",
        "44b564f6f0e054d1",
    ],
    // 256 bits: its PKCS #1 in the one-byte long form.
    [
        "8aa962335b2e9ae73af9eff6928abcca955cc68af8d5bbd89f3f8997dec045dd",
        "010001",
        "0f40ac9dab9507d311a8b69dafe6f4f7465687d2e73593db70ba38622101ca5d",
        "c36c94e454ad7fafef6bac95876604f3",
        "b5a48ca691ac9e4691041563734d7def",
        "5d8ab9f2597dfae2b20dae3c9ce4fe75",
        "1fc55b509b344d2edc98914126decc03",
        "63f6fb4de0825d333a600921e407fd37",
    ],
];

fn tiny(at: usize) -> crate::native::RsaPrivateParts {
    let [n, e, d, p, q, dp, dq, qi] = TINY[at].map(hex);
    crate::native::RsaPrivateParts {
        n,
        e,
        d,
        p,
        q,
        dp,
        dq,
        qi,
    }
}

/// The PKCS #8 wrapper written here (the lead's ruling of 2026-11-23):
/// byte for byte aws-lc-rs's inside its range; outside it, read back by
/// the library to the same parts, in each of DER's length forms.
#[test]
fn the_pkcs8_wrapper_is_the_safe_apis_and_reads_back() {
    use crate::algorithms::pkcs8_wrap;
    use crate::native::{
        rsa_generate, rsa_private_from_parts, rsa_private_from_pkcs8, rsa_private_pkcs1,
    };
    for bits in [2048, 3072] {
        let parts = rsa_generate(bits, 65537).unwrap().parts();
        let pair = crate::algorithms::safe_key_pair_for_tests(&parts);
        let theirs =
            aws_lc_rs::encoding::AsDer::<aws_lc_rs::encoding::Pkcs8V1Der>::as_der(&pair).unwrap();
        let key = rsa_private_from_parts(&parts).unwrap();
        let ours = pkcs8_wrap(&rsa_private_pkcs1(&key).unwrap());
        assert_eq!(ours, theirs.as_ref(), "{bits} bits");
    }
    let mut forms = Vec::new();
    let generated = [512, 1024, 1032, 2040].map(|bits| rsa_generate(bits, 65537).unwrap().parts());
    for parts in [tiny(0), tiny(1)].into_iter().chain(generated) {
        let key = rsa_private_from_parts(&parts).unwrap();
        let wrapped = pkcs8_wrap(&rsa_private_pkcs1(&key).unwrap());
        forms.push(wrapped[1]);
        let back = rsa_private_from_pkcs8(&wrapped).unwrap().parts();
        assert_eq!(
            back,
            parts,
            "{} bits",
            crate::algorithms::bit_length_for_tests(&parts.n)
        );
    }
    // The outer length's forms seen: short, one byte long, two bytes long.
    assert!(
        forms[0] < 0x80 && forms[1] == 0x81 && forms[2] == 0x82,
        "{forms:x?}"
    );
}

/// A private key's PKCS #8 and JWK, as the reference has them: exported,
/// imported again, still signing what the public key verifies; its
/// errors the reference's.
#[test]
fn rsa_private_keys_export_and_import_as_the_reference_does() {
    let mut keys = Keys::default();
    for bits in [1024, 2048] {
        let (public, private) = generate_pair(&mut keys, bits, &[1, 0, 1], "SHA-256");
        let pkcs8 = bytes(run(
            &mut keys,
            Call::ExportKey {
                format: "pkcs8".into(),
                key: private.id,
            },
        ));
        let alg = dict(&[("name", s("RSASSA-PKCS1-v1_5")), ("hash", s("SHA-256"))]);
        let import = |keys: &mut Keys, format: &str, data: Value, usages: Vec<Usage>| {
            run(
                keys,
                Call::ImportKey {
                    format: format.into(),
                    key_data: data,
                    algorithm: alg.clone(),
                    extractable: true,
                    usages,
                },
            )
        };
        let again = key(import(
            &mut keys,
            "pkcs8",
            Value::Bytes(pkcs8.clone()),
            vec![Usage::Sign],
        ));
        assert_eq!(
            (again.kind, again.algorithm.clone()),
            (KeyType::Private, private.algorithm.clone())
        );
        let signature = bytes(sign_with(&mut keys, again.id, "SHA-256", b"again"));
        assert_eq!(
            verify_with(&mut keys, public.id, signature, b"again"),
            Ok(Outcome::Bool(true))
        );
        assert_eq!(
            error(import(
                &mut keys,
                "pkcs8",
                Value::Bytes(pkcs8.clone()),
                vec![Usage::Verify]
            )),
            "SyntaxError: Cannot create a key using the specified key usages."
        );
        let mut trailing = pkcs8.clone();
        trailing.push(0);
        assert_eq!(
            error(import(
                &mut keys,
                "pkcs8",
                Value::Bytes(trailing),
                vec![Usage::Sign]
            )),
            "DataError: "
        );
        assert_eq!(
            error(import(
                &mut keys,
                "pkcs8",
                Value::Bytes(vec![48, 3, 2, 1, 0]),
                vec![Usage::Sign]
            )),
            "DataError: "
        );
        // The JWK: its members in the reference's order, and back.
        let Ok(Outcome::Jwk(members)) = run(
            &mut keys,
            Call::ExportKey {
                format: "jwk".into(),
                key: private.id,
            },
        ) else {
            panic!("a JWK");
        };
        let names: Vec<&str> = members.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "alg", "d", "dp", "dq", "e", "ext", "key_ops", "kty", "n", "p", "q", "qi"
            ]
        );
        let as_value = |members: &[(String, JwkValue)], without: &[&str]| {
            Value::Dict(
                members
                    .iter()
                    .filter(|(n, _)| !without.contains(&n.as_str()))
                    .map(|(n, v)| {
                        let v = match v {
                            JwkValue::Str(t) => Value::Str(t.clone()),
                            JwkValue::Bool(b) => Value::Bool(*b),
                            JwkValue::List(l) => {
                                Value::List(l.iter().map(|t| Value::Str(t.clone())).collect())
                            }
                        };
                        (n.clone(), v)
                    })
                    .collect(),
            )
        };
        let jwk_again = key(import(
            &mut keys,
            "jwk",
            as_value(&members, &[]),
            vec![Usage::Sign],
        ));
        let signature = bytes(sign_with(&mut keys, jwk_again.id, "SHA-256", b"jwk"));
        assert_eq!(
            verify_with(&mut keys, public.id, signature, b"jwk"),
            Ok(Outcome::Bool(true))
        );
        for missing in ["p", "q", "dp", "dq", "qi", "n", "e"] {
            assert_eq!(
                error(import(
                    &mut keys,
                    "jwk",
                    as_value(&members, &[missing]),
                    vec![Usage::Sign]
                )),
                format!("DataError: The required JWK member \"{missing}\" was missing")
            );
        }
        assert_eq!(
            error(import(
                &mut keys,
                "jwk",
                as_value(&members, &["p", "q", "dp", "dq", "qi"]),
                vec![Usage::Sign]
            )),
            "DataError: The required JWK member \"p\" was missing"
        );
        assert_eq!(
            error(import(
                &mut keys,
                "jwk",
                as_value(&members, &["d"]),
                vec![Usage::Sign]
            )),
            "SyntaxError: Cannot create a key using the specified key usages."
        );
        let mut swapped = members.clone();
        let q = swapped.iter().find(|(n, _)| n == "q").unwrap().1.clone();
        swapped.iter_mut().find(|(n, _)| n == "p").unwrap().1 = q;
        assert_eq!(
            error(import(
                &mut keys,
                "jwk",
                as_value(&swapped, &[]),
                vec![Usage::Sign]
            )),
            "DataError: "
        );
        // Each key exports in its own formats alone.
        let export = |keys: &mut Keys, format: &str, id| {
            run(
                keys,
                Call::ExportKey {
                    format: format.into(),
                    key: id,
                },
            )
        };
        let wrong = "InvalidAccessError: The key is not of the expected type";
        assert_eq!(error(export(&mut keys, "spki", private.id)), wrong);
        assert_eq!(error(export(&mut keys, "pkcs8", public.id)), wrong);
        assert_eq!(
            error(export(&mut keys, "raw", private.id)),
            "NotSupportedError: Unsupported export key format for algorithm"
        );
    }
}

/// What the library refuses as an RSA private key, or fails to write, is
/// the reference's error: an EC key's PKCS #8 (a throwaway P-256 key made
/// for this test) a DataError; a PKCS #1 the library fails to write an
/// OperationError. A private key's parts never print. And a generated
/// key's exponent is the one asked for, on either route.
#[test]
fn rsa_private_keys_refuse_and_keep_as_they_should() {
    let mut keys = Keys::default();
    let ec = hex(
        "308187020100301306072a8648ce3d020106082a8648ce3d030107046d306b0201010420cc0f467d358f58ca1313c9df3efa8c715bc6c5bce6f0c682308c1936aa71c83ba14403420004f215d866e57ee46881315dea9a3445a4d607105e54b579e7f0f21b7e58d508edf43efe7ccf33ed3fea09fe0f81c3defb7930e2a6281ec5f04961a6885560c8fb",
    );
    let out = run(
        &mut keys,
        Call::ImportKey {
            format: "pkcs8".into(),
            key_data: Value::Bytes(ec),
            algorithm: dict(&[("name", s("RSASSA-PKCS1-v1_5")), ("hash", s("SHA-256"))]),
            extractable: true,
            usages: vec![Usage::Sign],
        },
    );
    assert_eq!(error(out), "DataError: ");
    let (_, private) = generate_pair(&mut keys, 1024, &[1, 0, 1], "SHA-256");
    let exported = crate::native::tests::failing("RsaToBytes", || {
        run(
            &mut keys,
            Call::ExportKey {
                format: "pkcs8".into(),
                key: private.id,
            },
        )
    });
    assert_eq!(error(exported), "OperationError: ");
    let parts = crate::native::rsa_generate(512, 3).unwrap().parts();
    assert_eq!(format!("{parts:?}"), "RsaPrivateParts(..)");
    assert_eq!(
        format!("{:?}", Material::RsaPrivate(Box::new(parts))),
        "Material(..)"
    );
    for (bits, e) in [(2048, vec![3u8]), (2048, vec![1, 0, 1]), (1024, vec![3])] {
        let (public, private) = generate_pair(&mut keys, bits, &e, "SHA-256");
        let want = ("publicExponent", AlgorithmValue::Bytes(e.clone()));
        assert!(
            public.algorithm.contains(&want) && private.algorithm.contains(&want),
            "{bits} {e:?}"
        );
    }
}

/// A PKCS #1 `RSAPrivateKey` of any parts, consistent or not, written
/// here for the tests alone (the library writes none it would refuse).
fn pkcs1_of(parts: &crate::native::RsaPrivateParts) -> Vec<u8> {
    fn length(out: &mut Vec<u8>, len: usize) {
        if len < 0x80 {
            out.push(len as u8);
        } else {
            let bytes: Vec<u8> = len
                .to_be_bytes()
                .into_iter()
                .skip_while(|&b| b == 0)
                .collect();
            out.push(0x80 | bytes.len() as u8);
            out.extend_from_slice(&bytes);
        }
    }
    let mut body = vec![0x02, 0x01, 0x00];
    for part in [
        &parts.n, &parts.e, &parts.d, &parts.p, &parts.q, &parts.dp, &parts.dq, &parts.qi,
    ] {
        body.push(0x02);
        let pad = part.first().is_some_and(|&b| b & 0x80 != 0);
        length(&mut body, part.len() + usize::from(pad));
        if pad {
            body.push(0);
        }
        body.extend_from_slice(part);
    }
    let mut out = vec![0x30];
    length(&mut out, body.len());
    out.extend_from_slice(&body);
    out
}

/// An inconsistent private key (a wrong `d`, a CRT part off by one) is
/// refused with a DataError through both import paths, inside aws-lc-rs's
/// range and outside it: whichever check refuses it, aws-lc's own or the
/// one `native` repeats (the lead's ruling, 2026-11-23).
#[test]
fn an_inconsistent_rsa_private_key_is_refused_both_ways() {
    let mut keys = Keys::default();
    for bits in [1024, 2048] {
        let good = crate::native::rsa_generate(bits, 65537).unwrap().parts();
        let mut wrong_d = good.clone();
        *wrong_d.d.last_mut().unwrap() ^= 2;
        let mut wrong_dp = good.clone();
        *wrong_dp.dp.last_mut().unwrap() ^= 1;
        let mut wrong_qi = good.clone();
        *wrong_qi.qi.last_mut().unwrap() ^= 1;
        let alg = dict(&[("name", s("RSASSA-PKCS1-v1_5")), ("hash", s("SHA-256"))]);
        for (label, parts) in [
            ("d", &wrong_d),
            ("dp", &wrong_dp),
            ("qi", &wrong_qi),
            ("good", &good),
        ] {
            let pkcs8 = crate::algorithms::pkcs8_wrap(&pkcs1_of(parts));
            let b64 = |b: &[u8]| s(&jwk::encode(b));
            let jwk = dict(&[
                ("kty", s("RSA")),
                ("n", b64(&parts.n)),
                ("e", b64(&parts.e)),
                ("d", b64(&parts.d)),
                ("p", b64(&parts.p)),
                ("q", b64(&parts.q)),
                ("dp", b64(&parts.dp)),
                ("dq", b64(&parts.dq)),
                ("qi", b64(&parts.qi)),
            ]);
            for (format, data) in [("pkcs8", Value::Bytes(pkcs8)), ("jwk", jwk)] {
                let out = run(
                    &mut keys,
                    Call::ImportKey {
                        format: format.into(),
                        key_data: data,
                        algorithm: alg.clone(),
                        extractable: true,
                        usages: vec![Usage::Sign],
                    },
                );
                if label == "good" {
                    assert!(
                        matches!(out, Ok(Outcome::Key(_))),
                        "{bits} {format}: the good key"
                    );
                } else {
                    assert_eq!(
                        error(out),
                        "DataError: ",
                        "{bits} {format}: a wrong {label}"
                    );
                }
            }
        }
    }
}

fn pss(salt: f64) -> Value {
    dict(&[("name", s("RSA-PSS")), ("saltLength", Value::Num(salt))])
}

fn pss_pair(keys: &mut Keys, bits: u32, hash: &str) -> (KeyInfo, KeyInfo) {
    let algorithm = dict(&[
        ("name", s("RSA-PSS")),
        ("modulusLength", Value::Num(f64::from(bits))),
        ("publicExponent", Value::Bytes(vec![1, 0, 1])),
        ("hash", s(hash)),
    ]);
    match run(
        keys,
        Call::GenerateKey {
            algorithm,
            extractable: true,
            usages: vec![Usage::Sign, Usage::Verify],
        },
    ) {
        Ok(Outcome::Pair(public, private)) => (public, private),
        other => panic!("{bits} {hash}: {other:?}"),
    }
}

/// RSA-PSS's route: aws-lc-rs for SHA-256/384/512 at 2048 to 8192 bits
/// with a salt as long as the digest; every other hash, size or salt,
/// the library.
#[test]
fn pss_is_routed_by_hash_size_and_salt() {
    use crate::algorithms::pss_route as route;
    for (hash, len) in [("SHA-256", 32), ("SHA-384", 48), ("SHA-512", 64)] {
        assert!(route(hash, 2048, len).is_some(), "{hash}");
        assert!(
            route(hash, 2047, len).is_none() && route(hash, 8193, len).is_none(),
            "{hash} size"
        );
        assert!(route(hash, 8192, len).is_some(), "{hash} at 8192");
        assert!(
            route(hash, 2048, len - 1).is_none() && route(hash, 2048, len + 1).is_none(),
            "{hash} salt"
        );
        assert!(route(hash, 2048, 0).is_none(), "{hash} salt 0");
    }
    assert!(route("SHA-1", 2048, 20).is_none());
}

/// RSA-PSS at each size, hash and salt the reference probed (Chrome 157):
/// up to the most a key holds (emLen - hLen - 2) signs and verifies, and
/// randomly but at salt 0; one more is an OperationError to sign and
/// false to verify; a salt past 2^31 - 1 an OperationError; a signature
/// with another salt, or too short, false.
#[test]
fn pss_signs_and_verifies_as_the_reference_does() {
    let mut keys = Keys::default();
    let data = b"pss".to_vec();
    for (bits, hash, hlen) in [
        (512, "SHA-1", 20),
        (512, "SHA-256", 32),
        (1024, "SHA-512", 64),
        (2048, "SHA-1", 20),
        (2048, "SHA-256", 32),
        (3072, "SHA-384", 48),
    ] {
        let (public, private) = pss_pair(&mut keys, bits, hash);
        let max = (bits - 1).div_ceil(8) - hlen - 2;
        let sign = |keys: &mut Keys, salt: u32| {
            run(
                keys,
                Call::Sign {
                    algorithm: pss(f64::from(salt)),
                    key: private.id,
                    data: data.clone(),
                },
            )
        };
        let verify = |keys: &mut Keys, salt: f64, signature: Vec<u8>| {
            run(
                keys,
                Call::Verify {
                    algorithm: pss(salt),
                    key: public.id,
                    signature,
                    data: data.clone(),
                },
            )
        };
        for salt in [0, hlen, max] {
            if salt > max {
                continue;
            }
            let signature = bytes(sign(&mut keys, salt));
            assert_eq!(signature.len() as u32 * 8, bits, "{bits} {hash} {salt}");
            let label = format!("{bits} {hash} salt {salt}");
            assert_eq!(
                verify(&mut keys, f64::from(salt), signature.clone()),
                Ok(Outcome::Bool(true)),
                "{label}"
            );
            let other = if salt == 0 { 1 } else { salt - 1 };
            assert_eq!(
                verify(&mut keys, f64::from(other), signature.clone()),
                Ok(Outcome::Bool(false)),
                "{label} other"
            );
            assert_eq!(
                verify(&mut keys, f64::from(max + 1), signature.clone()),
                Ok(Outcome::Bool(false)),
                "{label} past max"
            );
            assert_eq!(
                verify(&mut keys, f64::from(salt), signature[1..].to_vec()),
                Ok(Outcome::Bool(false)),
                "{label} short"
            );
            let again = bytes(sign(&mut keys, salt));
            assert_eq!(
                again == signature,
                salt == 0,
                "{label}: random but at salt 0"
            );
        }
        assert_eq!(
            error(sign(&mut keys, max + 1)),
            "OperationError: ",
            "{bits} {hash} past max"
        );
        let signature = bytes(sign(&mut keys, 0));
        assert_eq!(
            error(verify(&mut keys, 4_294_967_295.0, signature)),
            "OperationError: ",
            "{bits} {hash} huge"
        );
    }
    // The params' own errors, the reference's words.
    let (_, private) = pss_pair(&mut keys, 1024, "SHA-256");
    let sign = |keys: &mut Keys, algorithm| {
        run(
            keys,
            Call::Sign {
                algorithm,
                key: private.id,
                data: data.clone(),
            },
        )
    };
    let prefix =
        "TypeError: Failed to execute 'sign' on 'SubtleCrypto': RsaPssParams: saltLength: ";
    assert_eq!(
        error(sign(&mut keys, dict(&[("name", s("RSA-PSS"))]))),
        format!("{prefix}Missing required property")
    );
    assert_eq!(
        error(sign(&mut keys, pss(-1.0))),
        format!("{prefix}Outside of numeric range")
    );
    assert_eq!(
        error(sign(&mut keys, pss(4_294_967_296.0))),
        format!("{prefix}Outside of numeric range")
    );
    assert_eq!(
        error(sign(&mut keys, s("RSASSA-PKCS1-v1_5"))),
        "InvalidAccessError: key.algorithm does not match that of operation"
    );
}

/// Where both routes run, each verifies the other's signature: the safe
/// API's at a salt as long as the digest, the library's of the same key.
#[test]
fn pss_safe_and_native_signatures_verify_each_other() {
    let key = crate::native::rsa_generate(2048, 65537).unwrap();
    let parts = key.parts();
    let pair = crate::algorithms::safe_key_pair_for_tests(&parts);
    let der = crate::algorithms::rsa_public_key_der(&parts.n, &parts.e);
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, b"both");
    let mut safe = vec![0u8; pair.public_modulus_len()];
    pair.sign(
        &aws_lc_rs::signature::RSA_PSS_SHA256,
        &aws_lc_rs::rand::SystemRandom::new(),
        b"both",
        &mut safe,
    )
    .unwrap();
    let hash = crate::native::Hash::Sha256;
    assert_eq!(
        crate::native::rsa_verify_pss(&der, hash, digest.as_ref(), 32, &safe),
        Ok(true)
    );
    let native = crate::native::rsa_sign_pss(&key, hash, digest.as_ref(), 32).unwrap();
    let components = aws_lc_rs::signature::RsaPublicKeyComponents {
        n: parts.n.as_slice(),
        e: parts.e.as_slice(),
    };
    assert!(
        components
            .verify(
                &aws_lc_rs::signature::RSA_PSS_2048_8192_SHA256,
                b"both",
                &native
            )
            .is_ok()
    );
}

/// An RSA-PSS key's JWK names PS1 to PS512, and refuses to be imported
/// as RSASSA-PKCS1-v1_5's.
#[test]
fn a_pss_keys_jwk_is_its_own() {
    let mut keys = Keys::default();
    for (hash, want) in [
        ("SHA-1", "PS1"),
        ("SHA-256", "PS256"),
        ("SHA-384", "PS384"),
        ("SHA-512", "PS512"),
    ] {
        let (public, _) = pss_pair(&mut keys, 1024, hash);
        let Ok(Outcome::Jwk(members)) = run(
            &mut keys,
            Call::ExportKey {
                format: "jwk".into(),
                key: public.id,
            },
        ) else {
            panic!("a JWK");
        };
        assert_eq!(members[0], ("alg".to_owned(), JwkValue::Str(want.into())));
        let as_value = Value::Dict(
            members
                .iter()
                .map(|(n, v)| {
                    let v = match v {
                        JwkValue::Str(t) => Value::Str(t.clone()),
                        JwkValue::Bool(b) => Value::Bool(*b),
                        JwkValue::List(l) => {
                            Value::List(l.iter().map(|t| Value::Str(t.clone())).collect())
                        }
                    };
                    (n.clone(), v)
                })
                .collect(),
        );
        let out = run(
            &mut keys,
            Call::ImportKey {
                format: "jwk".into(),
                key_data: as_value,
                algorithm: dict(&[("name", s("RSASSA-PKCS1-v1_5")), ("hash", s(hash))]),
                extractable: true,
                usages: vec![Usage::Verify],
            },
        );
        assert_eq!(
            error(out),
            "DataError: The JWK \"alg\" member was inconsistent with that specified by the Web Crypto call"
        );
    }
}

/// openssl's RSA-PSS signatures (`tests/rsa_pss.txt`), each hash, salt
/// and size on each route, verify true through the page's calls, and a
/// changed message false: the library's digest and MGF1 are the hash's.
#[test]
fn pss_verifies_openssls_signatures() {
    let vectors = include_str!("../tests/rsa_pss.txt");
    for line in vectors.lines().filter(|l| !l.starts_with('#')) {
        let parts: Vec<&str> = line.split(' ').collect();
        let (bits, hash, salt) = (parts[0], parts[1], parts[2].parse::<f64>().unwrap());
        let (spki, signature) = (hex(parts[3]), hex(parts[4]));
        let mut keys = Keys::default();
        let k = key(run(
            &mut keys,
            Call::ImportKey {
                format: "spki".into(),
                key_data: Value::Bytes(spki),
                algorithm: dict(&[("name", s("RSA-PSS")), ("hash", s(hash))]),
                extractable: true,
                usages: vec![Usage::Verify],
            },
        ));
        for (data, want) in [(&b"pss by openssl"[..], true), (b"pss by openssl!", false)] {
            let out = run(
                &mut keys,
                Call::Verify {
                    algorithm: pss(salt),
                    key: k.id,
                    signature: signature.clone(),
                    data: data.to_vec(),
                },
            );
            assert_eq!(out, Ok(Outcome::Bool(want)), "{bits} {hash} salt {salt}");
        }
    }
}

fn oaep_pair(keys: &mut Keys, bits: u32, hash: &str, usages: Vec<Usage>) -> Result<Outcome, Error> {
    run(
        keys,
        Call::GenerateKey {
            algorithm: dict(&[
                ("name", s("RSA-OAEP")),
                ("modulusLength", Value::Num(f64::from(bits))),
                ("publicExponent", Value::Bytes(vec![1, 0, 1])),
                ("hash", s(hash)),
            ]),
            extractable: true,
            usages,
        },
    )
}

fn oaep(label: Option<&[u8]>) -> Value {
    let mut members = vec![("name", s("RSA-OAEP"))];
    if let Some(label) = label {
        members.push(("label", Value::Bytes(label.to_vec())));
    }
    dict(&members)
}

/// RSA-OAEP as the reference answers (probed, Chrome 157): up to the most
/// a key encrypts (k - 2 hLen - 2), randomly, and back; one more, a wrong
/// or missing label, or a changed, short or made-up ciphertext an
/// OperationError with no message; an empty label is none; its params'
/// and usages' errors; JWK alg RSA-OAEP to RSA-OAEP-512.
#[test]
fn oaep_encrypts_and_decrypts_as_the_reference_does() {
    let mut keys = Keys::default();
    let crypt = |keys: &mut Keys, encrypt: bool, key: u32, algorithm: Value, data: Vec<u8>| {
        let call = if encrypt {
            Call::Encrypt {
                algorithm,
                key,
                data,
            }
        } else {
            Call::Decrypt {
                algorithm,
                key,
                data,
            }
        };
        run(keys, call)
    };
    for (bits, hash, hlen, alg) in [
        (2048, "SHA-1", 20, "RSA-OAEP"),
        (2048, "SHA-256", 32, "RSA-OAEP-256"),
        (2048, "SHA-512", 64, "RSA-OAEP-512"),
        (3072, "SHA-384", 48, "RSA-OAEP-384"),
    ] {
        let Ok(Outcome::Pair(public, private)) =
            oaep_pair(&mut keys, bits, hash, vec![Usage::Encrypt, Usage::Decrypt])
        else {
            panic!("{bits} {hash}: a pair");
        };
        let max = bits / 8 - 2 * hlen - 2;
        for len in [0, max] {
            let plain = vec![7u8; len as usize];
            let sealed = bytes(crypt(&mut keys, true, public.id, oaep(None), plain.clone()));
            let again = bytes(crypt(&mut keys, true, public.id, oaep(None), plain.clone()));
            assert_eq!(sealed.len() as u32, bits / 8, "{bits} {hash} {len}");
            assert_ne!(sealed, again, "{bits} {hash} {len}: random");
            assert_eq!(
                bytes(crypt(&mut keys, false, private.id, oaep(None), sealed)),
                plain,
                "{bits} {hash} {len}"
            );
        }
        let past = vec![7u8; max as usize + 1];
        assert_eq!(
            error(crypt(&mut keys, true, public.id, oaep(None), past)),
            "OperationError: ",
            "{bits} {hash}"
        );
        let Ok(Outcome::Jwk(members)) = run(
            &mut keys,
            Call::ExportKey {
                format: "jwk".into(),
                key: public.id,
            },
        ) else {
            panic!("a JWK");
        };
        assert_eq!(
            members[0],
            ("alg".to_owned(), JwkValue::Str(alg.into())),
            "{bits} {hash}"
        );
        if hash != "SHA-256" {
            continue;
        }
        let label = [1u8, 2, 3];
        let sealed = bytes(crypt(
            &mut keys,
            true,
            public.id,
            oaep(Some(&label)),
            vec![9],
        ));
        assert_eq!(
            bytes(crypt(
                &mut keys,
                false,
                private.id,
                oaep(Some(&label)),
                sealed.clone()
            )),
            [9]
        );
        for (what, algorithm) in [
            ("other label", oaep(Some(&[1, 2]))),
            ("no label", oaep(None)),
        ] {
            assert_eq!(
                error(crypt(
                    &mut keys,
                    false,
                    private.id,
                    algorithm,
                    sealed.clone()
                )),
                "OperationError: ",
                "{what}"
            );
        }
        let mut flipped = sealed.clone();
        flipped[5] ^= 1;
        for (what, data) in [
            ("flipped", flipped),
            ("short", vec![0; 10]),
            ("made up", vec![0; 256]),
        ] {
            assert_eq!(
                error(crypt(
                    &mut keys,
                    false,
                    private.id,
                    oaep(Some(&label)),
                    data
                )),
                "OperationError: ",
                "{what}"
            );
        }
        let sealed = bytes(crypt(&mut keys, true, public.id, oaep(Some(&[])), vec![5]));
        assert_eq!(
            bytes(crypt(&mut keys, false, private.id, oaep(None), sealed)),
            [5],
            "an empty label is none"
        );
        assert_eq!(
            error(crypt(
                &mut keys,
                true,
                public.id,
                dict(&[("name", s("RSA-OAEP")), ("label", Value::Num(5.0))]),
                vec![1]
            )),
            "TypeError: Failed to execute 'encrypt' on 'SubtleCrypto': RsaOaepParams: label: Not a BufferSource"
        );
        assert_eq!(
            error(crypt(&mut keys, true, private.id, oaep(None), vec![1])),
            "InvalidAccessError: key.usages does not permit this operation"
        );
    }
    assert_eq!(
        error(oaep_pair(&mut keys, 1024, "SHA-256", vec![Usage::Sign])),
        "SyntaxError: Cannot create a key using the specified key usages."
    );
    assert_eq!(
        error(oaep_pair(&mut keys, 1024, "SHA-256", vec![Usage::WrapKey])),
        "SyntaxError: Usages cannot be empty when creating a key."
    );
}

/// The known gap (the lead's ruling, 2026-11-23): RSA-OAEP with a key
/// outside aws-lc-rs's range, where the reference works, is an explicit
/// NotSupportedError both ways, never a wrong answer; the key itself is
/// made, imported and exported as the reference makes it.
#[test]
fn oaep_outside_the_safe_range_is_an_explicit_operation_error() {
    let mut keys = Keys::default();
    let Ok(Outcome::Pair(public, private)) = oaep_pair(
        &mut keys,
        1024,
        "SHA-256",
        vec![Usage::Encrypt, Usage::Decrypt],
    ) else {
        panic!("a 1024-bit pair");
    };
    let gap = format!("NotSupportedError: {}", crate::algorithms::OAEP_GAP);
    let encrypted = run(
        &mut keys,
        Call::Encrypt {
            algorithm: oaep(None),
            key: public.id,
            data: vec![1],
        },
    );
    assert_eq!(error(encrypted), gap);
    let decrypted = run(
        &mut keys,
        Call::Decrypt {
            algorithm: oaep(None),
            key: private.id,
            data: vec![0; 128],
        },
    );
    assert_eq!(error(decrypted), gap);
    assert!(matches!(
        run(
            &mut keys,
            Call::ExportKey {
                format: "pkcs8".into(),
                key: private.id
            }
        ),
        Ok(Outcome::Bytes(_))
    ));
}

fn aes_key(keys: &mut Keys, name: &str, raw: &[u8], usages: Vec<Usage>) -> Result<Outcome, Error> {
    run(
        keys,
        Call::ImportKey {
            format: "raw".into(),
            key_data: Value::Bytes(raw.to_vec()),
            algorithm: s(name),
            extractable: true,
            usages,
        },
    )
}

fn hex_of(out: Result<Outcome, Error>) -> String {
    bytes(out).iter().map(|b| format!("{b:02x}")).collect()
}

fn crypt_call(
    keys: &mut Keys,
    encrypt: bool,
    key: u32,
    algorithm: Value,
    data: Vec<u8>,
) -> Result<Outcome, Error> {
    if encrypt {
        run(
            keys,
            Call::Encrypt {
                algorithm,
                key,
                data,
            },
        )
    } else {
        run(
            keys,
            Call::Decrypt {
                algorithm,
                key,
                data,
            },
        )
    }
}

/// AES-CBC and AES-CTR as Chrome 157 encrypts (probed, aes-modes.json):
/// its ciphertexts on fixed keys, CTR's counter wrapping in its low
/// `length` bits, and the params' and operations' errors.
#[test]
fn aes_cbc_and_ctr_are_the_references() {
    let mut keys = Keys::default();
    let k16: Vec<u8> = (0..16).collect();
    let k32: Vec<u8> = (0..32).collect();
    let cbc = key(aes_key(
        &mut keys,
        "AES-CBC",
        &k16,
        vec![Usage::Encrypt, Usage::Decrypt],
    ));
    let cbc_iv = |iv: Vec<u8>| dict(&[("name", s("AES-CBC")), ("iv", Value::Bytes(iv))]);
    for (len, want) in [
        (0, "6feb2efdf86680b714d03eda40e51c01"),
        (15, "074d3268c34064431e664c2566420f59"),
        (
            16,
            "074d3268c34064431e664c2566420f59646b67faeb1a1b5e075ef731f17f97c0",
        ),
        (
            17,
            "074d3268c34064431e664c2566420f598d44336571f6be92d583ef84333310f6",
        ),
    ] {
        let sealed = bytes(crypt_call(
            &mut keys,
            true,
            cbc.id,
            cbc_iv(vec![9; 16]),
            vec![1; len],
        ));
        assert_eq!(
            sealed
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            want,
            "cbc {len}"
        );
        let back = bytes(crypt_call(
            &mut keys,
            false,
            cbc.id,
            cbc_iv(vec![9; 16]),
            sealed,
        ));
        assert_eq!(back, vec![1; len], "cbc {len} back");
    }
    let cbc256 = key(aes_key(&mut keys, "AES-CBC", &k32, vec![Usage::Encrypt]));
    assert_eq!(
        hex_of(crypt_call(
            &mut keys,
            true,
            cbc256.id,
            cbc_iv(vec![9; 16]),
            vec![0; 3]
        )),
        "546b0c69339a77cb854ad5aa72bc9534"
    );
    let iv_error = "OperationError: The \"iv\" has an unexpected length -- must be 16 bytes";
    for len in [15, 17] {
        assert_eq!(
            error(crypt_call(
                &mut keys,
                true,
                cbc.id,
                cbc_iv(vec![0; len]),
                vec![1]
            )),
            iv_error
        );
    }
    assert_eq!(
        error(crypt_call(&mut keys, true, cbc.id, s("AES-CBC"), vec![1])),
        "TypeError: Failed to execute 'encrypt' on 'SubtleCrypto': AesCbcParams: iv: Missing required property"
    );
    for data in [vec![0; 16], vec![0; 15], vec![]] {
        assert_eq!(
            error(crypt_call(
                &mut keys,
                false,
                cbc.id,
                cbc_iv(vec![9; 16]),
                data
            )),
            "OperationError: "
        );
    }
    assert_eq!(
        error(aes_key(
            &mut keys,
            "AES-CBC",
            &[0; 24],
            vec![Usage::Encrypt]
        )),
        "OperationError: 192-bit AES keys are not supported"
    );
    // CTR.
    let ctr = key(aes_key(
        &mut keys,
        "AES-CTR",
        &k16,
        vec![Usage::Encrypt, Usage::Decrypt],
    ));
    let ctr_of = |counter: Vec<u8>, length: f64| {
        dict(&[
            ("name", s("AES-CTR")),
            ("counter", Value::Bytes(counter)),
            ("length", Value::Num(length)),
        ])
    };
    for (length, want) in [
        (1.0, "ee68613a0882e37671c9580dd2eb985661aea1fa"),
        (64.0, "ee68613a0882e37671c9580dd2eb9856d3eef88c"),
        (128.0, "ee68613a0882e37671c9580dd2eb9856d3eef88c"),
    ] {
        let out = crypt_call(
            &mut keys,
            true,
            ctr.id,
            ctr_of(vec![9; 16], length),
            vec![2; 20],
        );
        assert_eq!(hex_of(out), want, "ctr {length}");
    }
    assert_eq!(
        hex_of(crypt_call(
            &mut keys,
            true,
            ctr.id,
            ctr_of(vec![0xff; 16], 8.0),
            vec![0; 48]
        )),
        "3c441f32ce07822364d7a2990e50bb1390dcf07a729615b8537aaafb90b8ec834be90bda27f92f1df1adadf588516235"
    );
    assert_eq!(
        hex_of(crypt_call(
            &mut keys,
            true,
            ctr.id,
            ctr_of(vec![0xff; 16], 128.0),
            vec![0; 32]
        )),
        "3c441f32ce07822364d7a2990e50bb13c6a13b37878f5b826f4f8162a1c8d879"
    );
    let sealed = bytes(crypt_call(
        &mut keys,
        true,
        ctr.id,
        ctr_of(vec![0; 16], 64.0),
        vec![1, 2, 3],
    ));
    assert_eq!(
        bytes(crypt_call(
            &mut keys,
            false,
            ctr.id,
            ctr_of(vec![0; 16], 64.0),
            sealed
        )),
        [1, 2, 3]
    );
    assert_eq!(
        bytes(crypt_call(
            &mut keys,
            true,
            ctr.id,
            ctr_of(vec![9; 16], 1.0),
            vec![0; 32]
        ))
        .len(),
        32
    );
    assert_eq!(
        error(crypt_call(
            &mut keys,
            true,
            ctr.id,
            ctr_of(vec![9; 16], 1.0),
            vec![0; 33]
        )),
        "DataError: The input is too large for the counter length."
    );
    for length in [0.0, 129.0] {
        assert_eq!(
            error(crypt_call(
                &mut keys,
                true,
                ctr.id,
                ctr_of(vec![9; 16], length),
                vec![1]
            )),
            "OperationError: The \"length\" member must be >= 1 and <= 128"
        );
    }
    assert_eq!(
        error(crypt_call(
            &mut keys,
            true,
            ctr.id,
            ctr_of(vec![0; 15], 64.0),
            vec![1]
        )),
        "OperationError: The \"counter\" has an unexpected length -- must be 16 bytes"
    );
}

fn hmac_key(keys: &mut Keys, raw: &[u8], length: Option<f64>, extractable: bool) -> KeyInfo {
    let mut alg = vec![("name", s("HMAC")), ("hash", s("SHA-256"))];
    if let Some(l) = length {
        alg.push(("length", Value::Num(l)));
    }
    key(run(
        keys,
        Call::ImportKey {
            format: "raw".into(),
            key_data: Value::Bytes(raw.to_vec()),
            algorithm: dict(&alg),
            extractable,
            usages: vec![Usage::Sign, Usage::Verify],
        },
    ))
}

fn wrap_call(
    keys: &mut Keys,
    format: &str,
    key: u32,
    wrapping: u32,
    algorithm: Value,
) -> Result<Outcome, Error> {
    run(
        keys,
        Call::WrapKey {
            format: format.into(),
            key,
            wrapping_key: wrapping,
            algorithm,
        },
    )
}

fn unwrap_call(
    keys: &mut Keys,
    format: &str,
    data: Vec<u8>,
    unwrapping: u32,
    algorithm: Value,
    extractable: bool,
    usages: Vec<Usage>,
) -> Result<Outcome, Error> {
    run(
        keys,
        Call::UnwrapKey {
            format: format.into(),
            wrapped: data,
            unwrapping_key: unwrapping,
            algorithm,
            unwrapped_algorithm: dict(&[("name", s("HMAC")), ("hash", s("SHA-256"))]),
            extractable,
            usages,
        },
    )
}

/// AES-KW and wrapKey/unwrapKey as Chrome 157 answers (probed,
/// aes-modes.json and aes-modes2.json): KW's own bytes, a wrapped JWK's
/// exact text, and the errors in the reference's order.
#[test]
fn aes_kw_and_wrapping_are_the_references() {
    let mut keys = Keys::default();
    let k16: Vec<u8> = (0..16).collect();
    let kw = key(aes_key(
        &mut keys,
        "AES-KW",
        &k16,
        vec![Usage::WrapKey, Usage::UnwrapKey],
    ));
    let secret = hmac_key(&mut keys, &[5; 32], None, true);
    let wrapped = bytes(wrap_call(&mut keys, "raw", secret.id, kw.id, s("AES-KW")));
    assert_eq!(
        wrapped
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "f5e7c56fcfb7fe11e79873433832d0bc0993fafc624753d4a4123705041be60f661214323ec097ee"
    );
    let back = key(unwrap_call(
        &mut keys,
        "raw",
        wrapped.clone(),
        kw.id,
        s("AES-KW"),
        true,
        vec![Usage::Sign],
    ));
    assert_eq!(
        bytes(run(
            &mut keys,
            Call::ExportKey {
                format: "raw".into(),
                key: back.id
            }
        )),
        vec![5; 32]
    );
    let mut tampered = wrapped.clone();
    tampered[3] ^= 1;
    let out = unwrap_call(
        &mut keys,
        "raw",
        tampered,
        kw.id,
        s("AES-KW"),
        true,
        vec![Usage::Sign],
    );
    assert_eq!(error(out), "OperationError: ");
    let small = "OperationError: The provided data is too small";
    for len in [16, 23] {
        let out = unwrap_call(
            &mut keys,
            "raw",
            vec![0; len],
            kw.id,
            s("AES-KW"),
            true,
            vec![Usage::Sign],
        );
        assert_eq!(error(out), small, "{len}");
    }
    let nine = hmac_key(&mut keys, &[5; 9], Some(72.0), true);
    let eight = hmac_key(&mut keys, &[5; 8], Some(64.0), true);
    assert_eq!(
        error(wrap_call(&mut keys, "raw", nine.id, kw.id, s("AES-KW"))),
        small
    );
    assert_eq!(
        error(wrap_call(&mut keys, "raw", eight.id, kw.id, s("AES-KW"))),
        small
    );
    assert_eq!(
        error(wrap_call(&mut keys, "jwk", secret.id, kw.id, s("AES-KW"))),
        "DataError: The AES-KW input data length is invalid: not a multiple of 8 bytes"
    );
    assert_eq!(
        error(aes_key(&mut keys, "AES-KW", &k16, vec![Usage::Encrypt])),
        "SyntaxError: Cannot create a key using the specified key usages."
    );
    assert_eq!(
        error(run(
            &mut keys,
            Call::Encrypt {
                algorithm: s("AES-KW"),
                key: kw.id,
                data: vec![0; 16]
            }
        )),
        "NotSupportedError: Failed to execute 'encrypt' on 'SubtleCrypto': AES-KW: Unsupported operation: encrypt"
    );
    // A JWK through AES-GCM: its exact text, and back.
    let usages = vec![
        Usage::WrapKey,
        Usage::UnwrapKey,
        Usage::Encrypt,
        Usage::Decrypt,
    ];
    let gcm = key(aes_key(&mut keys, "AES-GCM", &k16, usages));
    let gcm_params = || dict(&[("name", s("AES-GCM")), ("iv", Value::Bytes(vec![0; 12]))]);
    let sealed = bytes(wrap_call(&mut keys, "jwk", secret.id, gcm.id, gcm_params()));
    let text = bytes(crypt_call(
        &mut keys,
        false,
        gcm.id,
        gcm_params(),
        sealed.clone(),
    ));
    assert_eq!(
        String::from_utf8(text).unwrap(),
        r#"{"alg":"HS256","ext":true,"k":"BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU","key_ops":["sign","verify"],"kty":"oct"}"#
    );
    let unwrapped = key(unwrap_call(
        &mut keys,
        "jwk",
        sealed,
        gcm.id,
        gcm_params(),
        false,
        vec![Usage::Sign],
    ));
    assert_eq!(
        (
            unwrapped.kind,
            unwrapped.extractable,
            unwrapped.usages.clone()
        ),
        (KeyType::Secret, false, vec![Usage::Sign])
    );
    // The errors, the reference's.
    let not_extractable = hmac_key(&mut keys, &[0; 32], None, false);
    assert_eq!(
        error(wrap_call(
            &mut keys,
            "raw",
            not_extractable.id,
            kw.id,
            s("AES-KW")
        )),
        "InvalidAccessError: key is not extractable"
    );
    let unwrap_only = key(aes_key(&mut keys, "AES-KW", &k16, vec![Usage::UnwrapKey]));
    let usage = "InvalidAccessError: key.usages does not permit this operation";
    assert_eq!(
        error(wrap_call(
            &mut keys,
            "raw",
            secret.id,
            unwrap_only.id,
            s("AES-KW")
        )),
        usage
    );
    assert_eq!(
        error(wrap_call(&mut keys, "raw", secret.id, kw.id, gcm_params())),
        "InvalidAccessError: key.algorithm does not match that of operation"
    );
    assert_eq!(
        error(wrap_call(&mut keys, "spki", secret.id, kw.id, s("AES-KW"))),
        "NotSupportedError: Unsupported export key format for algorithm"
    );
    assert_eq!(
        error(wrap_call(&mut keys, "raw", secret.id, secret.id, s("HMAC"))),
        "NotSupportedError: Failed to execute 'wrapKey' on 'SubtleCrypto': HMAC: Unsupported operation: wrapKey"
    );
    let not_json = "DataError: JWK input could not be parsed to a JSON dictionary";
    for text in [&b"{not json"[..], b"[1,2]"] {
        let junk = bytes(crypt_call(
            &mut keys,
            true,
            gcm.id,
            gcm_params(),
            text.to_vec(),
        ));
        let out = unwrap_call(
            &mut keys,
            "jwk",
            junk,
            gcm.id,
            gcm_params(),
            true,
            vec![Usage::Sign],
        );
        assert_eq!(error(out), not_json);
    }
    let raw_wrapped = bytes(wrap_call(&mut keys, "raw", secret.id, gcm.id, gcm_params()));
    assert_eq!(
        error(unwrap_call(
            &mut keys,
            "raw",
            raw_wrapped.clone(),
            gcm.id,
            gcm_params(),
            true,
            vec![]
        )),
        "SyntaxError: Usages cannot be empty when creating a key."
    );
    assert_eq!(
        error(unwrap_call(
            &mut keys,
            "raw",
            raw_wrapped,
            gcm.id,
            gcm_params(),
            true,
            vec![Usage::Encrypt]
        )),
        "SyntaxError: Cannot create a key using the specified key usages."
    );
    let wrap_only = key(aes_key(&mut keys, "AES-GCM", &k16, vec![Usage::WrapKey]));
    let out = unwrap_call(
        &mut keys,
        "raw",
        vec![0; 3],
        wrap_only.id,
        gcm_params(),
        true,
        vec![Usage::Sign],
    );
    assert_eq!(error(out), usage);
    let out = unwrap_call(
        &mut keys,
        "raw",
        vec![0; 48],
        gcm.id,
        gcm_params(),
        true,
        vec![Usage::Sign],
    );
    assert_eq!(error(out), "OperationError: ");
    // CBC wraps a raw key padded: 32 bytes to 48.
    let cbc = key(aes_key(&mut keys, "AES-CBC", &k16, vec![Usage::WrapKey]));
    let cbc_params = dict(&[("name", s("AES-CBC")), ("iv", Value::Bytes(vec![9; 16]))]);
    assert_eq!(
        bytes(wrap_call(&mut keys, "raw", secret.id, cbc.id, cbc_params)).len(),
        48
    );
    // Each AES key's JWK names its mode.
    for (name, alg, usage) in [
        ("AES-CBC", "A256CBC", Usage::Encrypt),
        ("AES-CTR", "A256CTR", Usage::Encrypt),
        ("AES-KW", "A256KW", Usage::WrapKey),
    ] {
        let k = key(aes_key(&mut keys, name, &[1; 32], vec![usage]));
        let Ok(Outcome::Jwk(members)) = run(
            &mut keys,
            Call::ExportKey {
                format: "jwk".into(),
                key: k.id,
            },
        ) else {
            panic!("a JWK");
        };
        assert_eq!(
            members[0],
            ("alg".to_owned(), JwkValue::Str(alg.into())),
            "{name}"
        );
    }
}

/// An unwrapped JWK's text read as JSON.parse reads it, a member named
/// twice its last; and unwrapKey's algorithms normalized before anything
/// is decrypted (the specification's order).
#[test]
fn unwrapping_reads_a_jwk_as_json_does_and_normalizes_first() {
    let mut keys = Keys::default();
    let usages = vec![Usage::UnwrapKey, Usage::Encrypt];
    let gcm = key(aes_key(&mut keys, "AES-GCM", &[3; 16], usages));
    let gcm_params = || dict(&[("name", s("AES-GCM")), ("iv", Value::Bytes(vec![0; 12]))]);
    let text = br#"{"kty":"oct","k":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","k":"BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU"}"#;
    let sealed = bytes(crypt_call(
        &mut keys,
        true,
        gcm.id,
        gcm_params(),
        text.to_vec(),
    ));
    let k = key(unwrap_call(
        &mut keys,
        "jwk",
        sealed,
        gcm.id,
        gcm_params(),
        true,
        vec![Usage::Sign],
    ));
    assert_eq!(
        bytes(run(
            &mut keys,
            Call::ExportKey {
                format: "raw".into(),
                key: k.id
            }
        )),
        vec![5; 32]
    );
    let out = run(
        &mut keys,
        Call::UnwrapKey {
            format: "raw".into(),
            wrapped: vec![0; 3],
            unwrapping_key: gcm.id,
            algorithm: gcm_params(),
            unwrapped_algorithm: s("AES-FOO"),
            extractable: true,
            usages: vec![Usage::Sign],
        },
    );
    assert_eq!(
        error(out),
        "NotSupportedError: Failed to execute 'unwrapKey' on 'SubtleCrypto': Algorithm: Unrecognized name"
    );
}

fn ec_gen(keys: &mut Keys, name: &str, curve: &str, usages: Vec<Usage>) -> Result<Outcome, Error> {
    run(
        keys,
        Call::GenerateKey {
            algorithm: dict(&[("name", s(name)), ("namedCurve", s(curve))]),
            extractable: true,
            usages,
        },
    )
}

fn ec_pair(keys: &mut Keys, name: &str, curve: &str, usages: Vec<Usage>) -> (KeyInfo, KeyInfo) {
    match ec_gen(keys, name, curve, usages) {
        Ok(Outcome::Pair(public, private)) => (public, private),
        other => panic!("{name} {curve}: {other:?}"),
    }
}

fn ec_import(
    keys: &mut Keys,
    format: &str,
    data: Value,
    name: &str,
    curve: &str,
    usages: Vec<Usage>,
) -> Result<Outcome, Error> {
    run(
        keys,
        Call::ImportKey {
            format: format.into(),
            key_data: data,
            algorithm: dict(&[("name", s(name)), ("namedCurve", s(curve))]),
            extractable: true,
            usages,
        },
    )
}

fn jwk_dict(members: &[(String, JwkValue)], without: &[&str], with: &[(&str, &str)]) -> Value {
    let mut out: Vec<(String, Value)> = members
        .iter()
        .filter(|(n, _)| !without.contains(&n.as_str()))
        .map(|(n, v)| {
            let v = match v {
                JwkValue::Str(t) => Value::Str(t.clone()),
                JwkValue::Bool(b) => Value::Bool(*b),
                JwkValue::List(l) => Value::List(l.iter().map(|t| Value::Str(t.clone())).collect()),
            };
            (n.clone(), v)
        })
        .collect();
    for (n, v) in with {
        out.retain(|(m, _)| m != n);
        out.push(((*n).to_owned(), Value::Str((*v).to_owned())));
    }
    Value::Dict(out)
}

/// ECDSA as Chrome 157 answers (probed, ec.json): each curve's keys, their
/// formats' lengths and JWK members (no `alg`), signatures random and the
/// curve's length, a short one false; the curve and hash pairs aws-lc-rs
/// signs on the safe API, the others the gap's OperationError.
#[test]
fn ecdsa_is_the_references() {
    let mut keys = Keys::default();
    let err =
        |keys: &mut Keys, name: &str, curve: &str, usages| error(ec_gen(keys, name, curve, usages));
    assert_eq!(
        err(&mut keys, "ECDSA", "P-224", vec![Usage::Sign]),
        "NotSupportedError: Failed to execute 'generateKey' on 'SubtleCrypto': EcKeyGenParams: Unrecognized namedCurve"
    );
    assert_eq!(
        err(&mut keys, "ECDSA", "p-256", vec![Usage::Sign]),
        err(&mut keys, "ECDSA", "P-224", vec![Usage::Sign])
    );
    assert_eq!(
        error(run(
            &mut keys,
            Call::GenerateKey {
                algorithm: s("ECDSA"),
                extractable: true,
                usages: vec![Usage::Sign]
            }
        )),
        "TypeError: Failed to execute 'generateKey' on 'SubtleCrypto': EcKeyGenParams: namedCurve: Missing or not a string"
    );
    assert_eq!(
        err(&mut keys, "ECDSA", "P-256", vec![Usage::Encrypt]),
        "SyntaxError: Cannot create a key using the specified key usages."
    );
    for (curve, hash, raw, spki, pkcs8) in [
        ("P-256", "SHA-256", 65, 91, 138),
        ("P-384", "SHA-384", 97, 120, 185),
        ("P-521", "SHA-512", 133, 158, 241),
    ] {
        let (public, private) =
            ec_pair(&mut keys, "ECDSA", curve, vec![Usage::Sign, Usage::Verify]);
        assert_eq!(
            (public.usages.clone(), private.usages.clone()),
            (vec![Usage::Verify], vec![Usage::Sign])
        );
        let export = |keys: &mut Keys, format: &str, id| {
            run(
                keys,
                Call::ExportKey {
                    format: format.into(),
                    key: id,
                },
            )
        };
        let point = bytes(export(&mut keys, "raw", public.id));
        assert_eq!((point.len(), point[0]), (raw, 4), "{curve}");
        assert_eq!(
            bytes(export(&mut keys, "spki", public.id)).len(),
            spki,
            "{curve}"
        );
        let der = bytes(export(&mut keys, "pkcs8", private.id));
        assert_eq!(der.len(), pkcs8, "{curve}");
        let Ok(Outcome::Jwk(members)) = export(&mut keys, "jwk", private.id) else {
            panic!("a JWK");
        };
        let names: Vec<&str> = members.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["crv", "d", "ext", "key_ops", "kty", "x", "y"],
            "{curve}"
        );
        let params = || dict(&[("name", s("ECDSA")), ("hash", s(hash))]);
        let sign = |keys: &mut Keys| {
            bytes(run(
                keys,
                Call::Sign {
                    algorithm: params(),
                    key: private.id,
                    data: vec![1],
                },
            ))
        };
        let (a, b) = (sign(&mut keys), sign(&mut keys));
        assert_eq!(a.len(), raw - 1, "{curve}");
        assert_ne!(a, b, "{curve}: random");
        let verify = |keys: &mut Keys, signature: Vec<u8>, data: Vec<u8>| {
            run(
                keys,
                Call::Verify {
                    algorithm: params(),
                    key: public.id,
                    signature,
                    data,
                },
            )
        };
        assert_eq!(
            verify(&mut keys, a.clone(), vec![1]),
            Ok(Outcome::Bool(true)),
            "{curve}"
        );
        assert_eq!(
            verify(&mut keys, a.clone(), vec![2]),
            Ok(Outcome::Bool(false)),
            "{curve}"
        );
        assert_eq!(
            verify(&mut keys, vec![0; 10], vec![1]),
            Ok(Outcome::Bool(false)),
            "{curve} short"
        );
        // The key again, from each format, still verifying.
        let again = key(ec_import(
            &mut keys,
            "raw",
            Value::Bytes(point.clone()),
            "ECDSA",
            curve,
            vec![Usage::Verify],
        ));
        let out = run(
            &mut keys,
            Call::Verify {
                algorithm: params(),
                key: again.id,
                signature: a.clone(),
                data: vec![1],
            },
        );
        assert_eq!(out, Ok(Outcome::Bool(true)), "{curve} raw again");
        let from_pkcs8 = key(ec_import(
            &mut keys,
            "pkcs8",
            Value::Bytes(der),
            "ECDSA",
            curve,
            vec![Usage::Sign],
        ));
        let from_jwk = key(ec_import(
            &mut keys,
            "jwk",
            jwk_dict(&members, &[], &[]),
            "ECDSA",
            curve,
            vec![Usage::Sign],
        ));
        for k in [from_pkcs8.id, from_jwk.id] {
            let signature = bytes(run(
                &mut keys,
                Call::Sign {
                    algorithm: params(),
                    key: k,
                    data: vec![3],
                },
            ));
            assert_eq!(
                verify(&mut keys, signature, vec![3]),
                Ok(Outcome::Bool(true)),
                "{curve} again"
            );
        }
    }
    // Cross pairs: those aws-lc-rs verifies verify; those it cannot, the gap.
    let (public, private) = ec_pair(
        &mut keys,
        "ECDSA",
        "P-521",
        vec![Usage::Sign, Usage::Verify],
    );
    let sha1 = || dict(&[("name", s("ECDSA")), ("hash", s("SHA-1"))]);
    assert_eq!(
        error(run(
            &mut keys,
            Call::Sign {
                algorithm: sha1(),
                key: private.id,
                data: vec![1]
            }
        )),
        format!("NotSupportedError: {}", crate::algorithms::ECDSA_GAP)
    );
    let out = run(
        &mut keys,
        Call::Verify {
            algorithm: sha1(),
            key: public.id,
            signature: vec![0; 132],
            data: vec![1],
        },
    );
    assert_eq!(out, Ok(Outcome::Bool(false)));
}

/// ECDSA's import as the reference refuses it (probed): a compressed raw
/// point taken, a changed point and another curve's refused, and the
/// JWK's and PKCS #8's own errors.
#[test]
fn ec_import_refuses_as_the_reference_does() {
    let mut keys = Keys::default();
    let (public, private) = ec_pair(
        &mut keys,
        "ECDSA",
        "P-256",
        vec![Usage::Sign, Usage::Verify],
    );
    let point = bytes(run(
        &mut keys,
        Call::ExportKey {
            format: "raw".into(),
            key: public.id,
        },
    ));
    let mut compressed = vec![2 + (point[64] & 1)];
    compressed.extend_from_slice(&point[1..33]);
    let k = key(ec_import(
        &mut keys,
        "raw",
        Value::Bytes(compressed),
        "ECDSA",
        "P-256",
        vec![Usage::Verify],
    ));
    assert_eq!(
        bytes(run(
            &mut keys,
            Call::ExportKey {
                format: "raw".into(),
                key: k.id
            }
        )),
        point
    );
    let mut changed = point.clone();
    changed[5] ^= 1;
    assert_eq!(
        error(ec_import(
            &mut keys,
            "raw",
            Value::Bytes(changed),
            "ECDSA",
            "P-256",
            vec![Usage::Verify]
        )),
        "DataError: "
    );
    assert_eq!(
        error(ec_import(
            &mut keys,
            "raw",
            Value::Bytes(point.clone()),
            "ECDSA",
            "P-384",
            vec![Usage::Verify]
        )),
        "DataError: "
    );
    assert_eq!(
        error(ec_import(
            &mut keys,
            "raw",
            Value::Bytes(point.clone()),
            "ECDSA",
            "P-256",
            vec![Usage::Sign]
        )),
        "SyntaxError: Cannot create a key using the specified key usages."
    );
    let Ok(Outcome::Jwk(members)) = run(
        &mut keys,
        Call::ExportKey {
            format: "jwk".into(),
            key: private.id,
        },
    ) else {
        panic!("a JWK");
    };
    let jwk_err = |keys: &mut Keys, value: Value, curve: &str| {
        error(ec_import(
            keys,
            "jwk",
            value,
            "ECDSA",
            curve,
            vec![Usage::Sign],
        ))
    };
    assert_eq!(
        jwk_err(&mut keys, jwk_dict(&members, &[], &[]), "P-384"),
        "DataError: The JWK's \"crv\" member specifies a different curve than requested"
    );
    assert_eq!(
        jwk_err(&mut keys, jwk_dict(&members, &["y"], &[]), "P-256"),
        "DataError: The required JWK member \"y\" was missing"
    );
    let x = members
        .iter()
        .find(|(n, _)| n == "x")
        .map(|(_, v)| v.clone())
        .unwrap();
    let JwkValue::Str(x) = x else { panic!("x") };
    assert_eq!(
        jwk_err(&mut keys, jwk_dict(&members, &[], &[("d", &x)]), "P-256"),
        "DataError: The imported EC key is invalid"
    );
    assert_eq!(
        jwk_err(
            &mut keys,
            jwk_dict(&members, &[], &[("alg", "ES384")]),
            "P-256"
        ),
        "DataError: The JWK \"alg\" member was inconsistent with that specified by the Web Crypto call"
    );
    let der = bytes(run(
        &mut keys,
        Call::ExportKey {
            format: "pkcs8".into(),
            key: private.id,
        },
    ));
    assert_eq!(
        error(ec_import(
            &mut keys,
            "pkcs8",
            Value::Bytes(der.clone()),
            "ECDSA",
            "P-384",
            vec![Usage::Sign]
        )),
        "DataError: The imported EC key specifies a different curve than requested"
    );
    let as_ecdh = key(ec_import(
        &mut keys,
        "pkcs8",
        Value::Bytes(der),
        "ECDH",
        "P-256",
        vec![Usage::DeriveBits],
    ));
    assert_eq!(as_ecdh.kind, KeyType::Private);
}

/// ECDH as Chrome 157 answers (probed): both sides' secret the same, its
/// length the curve's by default, cut to the bits asked (the last byte's
/// spare bits zero), too many an OperationError, the public parameter's
/// errors; a key derived from it; its usages.
#[test]
fn ecdh_is_the_references() {
    let mut keys = Keys::default();
    let both = vec![Usage::DeriveBits, Usage::DeriveKey];
    let (a_public, a) = ec_pair(&mut keys, "ECDH", "P-256", both.clone());
    assert!(a_public.usages.is_empty());
    assert_eq!(a.usages, [Usage::DeriveKey, Usage::DeriveBits]);
    let (b_public, b) = ec_pair(&mut keys, "ECDH", "P-256", both.clone());
    let (c_public, _) = ec_pair(&mut keys, "ECDH", "P-384", both.clone());
    let (ecdsa_public, _) = ec_pair(
        &mut keys,
        "ECDSA",
        "P-256",
        vec![Usage::Sign, Usage::Verify],
    );
    let derive = |keys: &mut Keys, public: u32, private: u32, length: Option<u32>| {
        run(
            keys,
            Call::DeriveBits {
                algorithm: dict(&[("name", s("ECDH")), ("public", Value::Key(public))]),
                key: private,
                length,
            },
        )
    };
    let ab = bytes(derive(&mut keys, b_public.id, a.id, None));
    assert_eq!(ab.len(), 32);
    assert_eq!(bytes(derive(&mut keys, a_public.id, b.id, None)), ab);
    assert_eq!(bytes(derive(&mut keys, b_public.id, a.id, Some(256))), ab);
    let twelve = bytes(derive(&mut keys, b_public.id, a.id, Some(12)));
    assert_eq!(twelve, [ab[0], ab[1] & 0xf0]);
    assert!(bytes(derive(&mut keys, b_public.id, a.id, Some(0))).is_empty());
    assert_eq!(
        error(derive(&mut keys, b_public.id, a.id, Some(257))),
        "OperationError: Length specified for ECDH key derivation is too large. Maximum allowed is 256 bits"
    );
    let prefix = "InvalidAccessError: The public parameter for ECDH key derivation";
    assert_eq!(
        error(derive(&mut keys, c_public.id, a.id, None)),
        format!("{prefix} is for a different named curve")
    );
    assert_eq!(
        error(derive(&mut keys, b.id, a.id, None)),
        format!("{prefix} is not a public EC key")
    );
    assert_eq!(
        error(derive(&mut keys, ecdsa_public.id, a.id, None)),
        format!("{prefix} must be for ECDH")
    );
    assert_eq!(
        error(run(
            &mut keys,
            Call::DeriveBits {
                algorithm: s("ECDH"),
                key: a.id,
                length: None
            }
        )),
        "TypeError: Failed to execute 'deriveBits' on 'SubtleCrypto': EcdhKeyDeriveParams: public: Missing required property"
    );
    let derived = key(run(
        &mut keys,
        Call::DeriveKey {
            algorithm: dict(&[("name", s("ECDH")), ("public", Value::Key(b_public.id))]),
            key: a.id,
            derived: dict(&[("name", s("AES-GCM")), ("length", Value::Num(128.0))]),
            extractable: true,
            usages: vec![Usage::Encrypt],
        },
    ));
    assert_eq!(
        bytes(run(
            &mut keys,
            Call::ExportKey {
                format: "raw".into(),
                key: derived.id
            }
        )),
        ab[..16]
    );
    assert_eq!(
        error(ec_gen(&mut keys, "ECDH", "P-256", vec![Usage::Sign])),
        "SyntaxError: Cannot create a key using the specified key usages."
    );
    let point = bytes(run(
        &mut keys,
        Call::ExportKey {
            format: "raw".into(),
            key: a_public.id,
        },
    ));
    assert_eq!(
        error(ec_import(
            &mut keys,
            "raw",
            Value::Bytes(point.clone()),
            "ECDH",
            "P-256",
            vec![Usage::DeriveBits]
        )),
        "SyntaxError: Cannot create a key using the specified key usages."
    );
    let imported = key(ec_import(
        &mut keys,
        "raw",
        Value::Bytes(point),
        "ECDH",
        "P-256",
        vec![],
    ));
    assert!(imported.usages.is_empty());
    for curve in ["P-384", "P-521"] {
        let (public, private) = ec_pair(&mut keys, "ECDH", curve, both.clone());
        let out = bytes(derive(&mut keys, public.id, private.id, None));
        assert_eq!(out.len(), if curve == "P-384" { 48 } else { 66 }, "{curve}");
    }
}

/// The known gaps' name is NotSupportedError, and must stay so: other
/// code's fallbacks key on it. Proton's OpenPGP.js, on ECDSA with P-256
/// and P-384, rethrows a DataError or an OperationError and fails the
/// message, but falls back to its own implementation for any other name
/// (the lead's ruling, 2026-11-23; its crypto-worker bundle read).
#[test]
fn the_known_gaps_are_not_supported_errors_so_fallbacks_run() {
    let mut keys = Keys::default();
    let name = |out: Result<Outcome, Error>| match out {
        Err(e) => e.name,
        other => panic!("{other:?}"),
    };
    let (_, p256) = ec_pair(
        &mut keys,
        "ECDSA",
        "P-256",
        vec![Usage::Sign, Usage::Verify],
    );
    let sha512 = dict(&[("name", s("ECDSA")), ("hash", s("SHA-512"))]);
    let signed = run(
        &mut keys,
        Call::Sign {
            algorithm: sha512,
            key: p256.id,
            data: vec![1],
        },
    );
    assert_eq!(
        name(signed),
        "NotSupportedError",
        "ECDSA P-256 with SHA-512, to sign"
    );
    let (p384, _) = ec_pair(
        &mut keys,
        "ECDSA",
        "P-384",
        vec![Usage::Sign, Usage::Verify],
    );
    let sha1 = dict(&[("name", s("ECDSA")), ("hash", s("SHA-1"))]);
    let verified = run(
        &mut keys,
        Call::Verify {
            algorithm: sha1,
            key: p384.id,
            signature: vec![0; 96],
            data: vec![1],
        },
    );
    assert_eq!(
        name(verified),
        "NotSupportedError",
        "ECDSA P-384 with SHA-1, to verify"
    );
    let Ok(Outcome::Pair(public, _)) = oaep_pair(
        &mut keys,
        1024,
        "SHA-256",
        vec![Usage::Encrypt, Usage::Decrypt],
    ) else {
        panic!("a pair");
    };
    let sealed = run(
        &mut keys,
        Call::Encrypt {
            algorithm: oaep(None),
            key: public.id,
            data: vec![1],
        },
    );
    assert_eq!(name(sealed), "NotSupportedError", "RSA-OAEP at 1024 bits");
}

fn okp_pair(keys: &mut Keys, name: &str, usages: Vec<Usage>) -> (KeyInfo, KeyInfo) {
    match run(
        keys,
        Call::GenerateKey {
            algorithm: s(name),
            extractable: true,
            usages,
        },
    ) {
        Ok(Outcome::Pair(public, private)) => (public, private),
        other => panic!("{name}: {other:?}"),
    }
}

fn okp_import(
    keys: &mut Keys,
    format: &str,
    data: Value,
    name: &str,
    usages: Vec<Usage>,
) -> Result<Outcome, Error> {
    run(
        keys,
        Call::ImportKey {
            format: format.into(),
            key_data: data,
            algorithm: s(name),
            extractable: true,
            usages,
        },
    )
}

/// Ed25519 as Chrome 157 answers (probed, okp.json): RFC 8032's first
/// vector signed from its PKCS #8; each format's bytes and JWK's members
/// (`alg` Ed25519, EdDSA taken too); signatures deterministic, a short one
/// false; the errors.
#[test]
fn ed25519_is_the_references() {
    let mut keys = Keys::default();
    let seed = hex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
    let pkcs8 = [&hex("302e020100300506032b657004220420")[..], &seed].concat();
    let k = key(okp_import(
        &mut keys,
        "pkcs8",
        Value::Bytes(pkcs8.clone()),
        "Ed25519",
        vec![Usage::Sign],
    ));
    assert_eq!(
        hex_of(run(
            &mut keys,
            Call::Sign {
                algorithm: s("Ed25519"),
                key: k.id,
                data: vec![]
            }
        )),
        "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
    );
    assert_eq!(
        bytes(run(
            &mut keys,
            Call::ExportKey {
                format: "pkcs8".into(),
                key: k.id
            }
        )),
        pkcs8
    );
    let (public, private) = okp_pair(&mut keys, "Ed25519", vec![Usage::Sign, Usage::Verify]);
    assert_eq!(
        (public.usages.clone(), private.usages.clone()),
        (vec![Usage::Verify], vec![Usage::Sign])
    );
    let export = |keys: &mut Keys, format: &str, id| {
        run(
            keys,
            Call::ExportKey {
                format: format.into(),
                key: id,
            },
        )
    };
    let raw = bytes(export(&mut keys, "raw", public.id));
    assert_eq!(raw.len(), 32);
    let spki = bytes(export(&mut keys, "spki", public.id));
    assert_eq!(
        (spki.len(), &spki[..12]),
        (44, &hex("302a300506032b6570032100")[..])
    );
    assert_eq!(&spki[12..], raw.as_slice());
    assert_eq!(bytes(export(&mut keys, "pkcs8", private.id)).len(), 48);
    let Ok(Outcome::Jwk(members)) = export(&mut keys, "jwk", private.id) else {
        panic!("a JWK");
    };
    let names: Vec<&str> = members.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["alg", "crv", "d", "ext", "key_ops", "kty", "x"]);
    assert_eq!(members[0].1, JwkValue::Str("Ed25519".into()));
    let wrong = "InvalidAccessError: The key is not of the expected type";
    assert_eq!(error(export(&mut keys, "raw", private.id)), wrong);
    let sign = |keys: &mut Keys| {
        bytes(run(
            keys,
            Call::Sign {
                algorithm: s("Ed25519"),
                key: private.id,
                data: vec![1],
            },
        ))
    };
    let (a, b) = (sign(&mut keys), sign(&mut keys));
    assert_eq!((a.len(), a == b), (64, true));
    let verify = |keys: &mut Keys, signature: Vec<u8>| {
        run(
            keys,
            Call::Verify {
                algorithm: s("Ed25519"),
                key: public.id,
                signature,
                data: vec![1],
            },
        )
    };
    assert_eq!(verify(&mut keys, a.clone()), Ok(Outcome::Bool(true)));
    assert_eq!(
        verify(&mut keys, a[..63].to_vec()),
        Ok(Outcome::Bool(false))
    );
    // Back from each format.
    for (format, data) in [
        ("raw", Value::Bytes(raw.clone())),
        ("spki", Value::Bytes(spki)),
    ] {
        let again = key(okp_import(
            &mut keys,
            format,
            data,
            "Ed25519",
            vec![Usage::Verify],
        ));
        let out = run(
            &mut keys,
            Call::Verify {
                algorithm: s("Ed25519"),
                key: again.id,
                signature: a.clone(),
                data: vec![1],
            },
        );
        assert_eq!(out, Ok(Outcome::Bool(true)), "{format}");
    }
    let from_jwk = key(okp_import(
        &mut keys,
        "jwk",
        jwk_dict(&members, &[], &[("alg", "EdDSA")]),
        "Ed25519",
        vec![Usage::Sign],
    ));
    assert_eq!(
        bytes(run(
            &mut keys,
            Call::Sign {
                algorithm: s("Ed25519"),
                key: from_jwk.id,
                data: vec![1]
            }
        )),
        a
    );
    // The errors.
    let syntax = "SyntaxError: Cannot create a key using the specified key usages.";
    assert_eq!(
        error(run(
            &mut keys,
            Call::GenerateKey {
                algorithm: s("Ed25519"),
                extractable: true,
                usages: vec![Usage::Encrypt]
            }
        )),
        syntax
    );
    assert_eq!(
        error(run(
            &mut keys,
            Call::GenerateKey {
                algorithm: s("Ed25519"),
                extractable: true,
                usages: vec![Usage::Verify]
            }
        )),
        "SyntaxError: Usages cannot be empty when creating a key."
    );
    assert_eq!(
        error(okp_import(
            &mut keys,
            "raw",
            Value::Bytes(vec![0; 31]),
            "Ed25519",
            vec![Usage::Verify]
        )),
        "DataError: Ed25519 key data must be 256 bits"
    );
    assert_eq!(
        error(okp_import(
            &mut keys,
            "raw",
            Value::Bytes(raw),
            "Ed25519",
            vec![Usage::Sign]
        )),
        syntax
    );
    assert_eq!(
        error(okp_import(
            &mut keys,
            "jwk",
            jwk_dict(&members, &[], &[("crv", "X25519")]),
            "Ed25519",
            vec![Usage::Sign]
        )),
        "DataError: The JWK's \"crv\" member specifies a different curve than requested"
    );
    assert_eq!(
        error(okp_import(
            &mut keys,
            "jwk",
            jwk_dict(&members, &[], &[("alg", "ES256")]),
            "Ed25519",
            vec![Usage::Sign]
        )),
        "DataError: The JWK \"alg\" member was inconsistent with that specified by the Web Crypto call"
    );
    let (_, other) = okp_pair(&mut keys, "Ed25519", vec![Usage::Sign]);
    let Ok(Outcome::Jwk(other)) = export(&mut keys, "jwk", other.id) else {
        panic!("a JWK");
    };
    let JwkValue::Str(other_d) = other.iter().find(|(n, _)| n == "d").unwrap().1.clone() else {
        panic!("d");
    };
    assert_eq!(
        error(okp_import(
            &mut keys,
            "jwk",
            jwk_dict(&members, &[], &[("d", &other_d)]),
            "Ed25519",
            vec![Usage::Sign]
        )),
        "DataError: "
    );
}

/// X25519 as Chrome 157 answers (probed): RFC 7748's vector through JWK
/// imports; both sides' secret the same, cut to the bits asked, past 256
/// an error, a small-order point's all-zero secret an OperationError; the
/// public parameter's errors; SPKI and PKCS #8 written as RFC 8410 has
/// them, the PKCS #8 read back by the library.
#[test]
fn x25519_is_the_references() {
    let mut keys = Keys::default();
    let b64 = |h: &str| jwk::encode(&hex(h));
    let alice = dict(&[
        ("kty", s("OKP")),
        ("crv", s("X25519")),
        (
            "x",
            s(&b64(
                "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a",
            )),
        ),
        (
            "d",
            s(&b64(
                "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
            )),
        ),
    ]);
    let bob = dict(&[
        ("kty", s("OKP")),
        ("crv", s("X25519")),
        (
            "x",
            s(&b64(
                "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f",
            )),
        ),
    ]);
    let alice = key(okp_import(
        &mut keys,
        "jwk",
        alice,
        "X25519",
        vec![Usage::DeriveBits],
    ));
    let bob = key(okp_import(&mut keys, "jwk", bob, "X25519", vec![]));
    let derive = |keys: &mut Keys, public: u32, private: u32, length: Option<u32>| {
        run(
            keys,
            Call::DeriveBits {
                algorithm: dict(&[("name", s("X25519")), ("public", Value::Key(public))]),
                key: private,
                length,
            },
        )
    };
    assert_eq!(
        hex_of(derive(&mut keys, bob.id, alice.id, None)),
        "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742"
    );
    let both = vec![Usage::DeriveBits, Usage::DeriveKey];
    let (a_public, a) = okp_pair(&mut keys, "X25519", both.clone());
    let (b_public, b) = okp_pair(&mut keys, "X25519", both);
    assert!(a_public.usages.is_empty());
    let ab = bytes(derive(&mut keys, b_public.id, a.id, None));
    assert_eq!(ab.len(), 32);
    assert_eq!(bytes(derive(&mut keys, a_public.id, b.id, None)), ab);
    assert_eq!(
        bytes(derive(&mut keys, b_public.id, a.id, Some(12))),
        [ab[0], ab[1] & 0xf0]
    );
    assert_eq!(
        error(derive(&mut keys, b_public.id, a.id, Some(257))),
        "OperationError: The length provided for X25519 is too large."
    );
    let zero = key(okp_import(
        &mut keys,
        "raw",
        Value::Bytes(vec![0; 32]),
        "X25519",
        vec![],
    ));
    assert_eq!(
        error(derive(&mut keys, zero.id, a.id, None)),
        "OperationError: "
    );
    let prefix = "InvalidAccessError: The public parameter for X25519 key derivation";
    let (ed_public, _) = okp_pair(&mut keys, "Ed25519", vec![Usage::Sign, Usage::Verify]);
    assert_eq!(
        error(derive(&mut keys, ed_public.id, a.id, None)),
        format!("{prefix} must be for X25519")
    );
    assert_eq!(
        error(derive(&mut keys, b.id, a.id, None)),
        format!("{prefix} is not a public key")
    );
    let export = |keys: &mut Keys, format: &str, id| {
        run(
            keys,
            Call::ExportKey {
                format: format.into(),
                key: id,
            },
        )
    };
    let spki = bytes(export(&mut keys, "spki", a_public.id));
    assert_eq!(
        (spki.len(), &spki[..12]),
        (44, &hex("302a300506032b656e032100")[..])
    );
    let again = key(okp_import(
        &mut keys,
        "spki",
        Value::Bytes(spki),
        "X25519",
        vec![],
    ));
    assert_eq!(bytes(derive(&mut keys, again.id, b.id, None)), ab);
    let pkcs8 = bytes(export(&mut keys, "pkcs8", a.id));
    assert_eq!(
        (pkcs8.len(), &pkcs8[..16]),
        (48, &hex("302e020100300506032b656e04220420")[..])
    );
    // Round trip (the lead's ruling): the PKCS #8 imported again, by the
    // library, derives the same secret.
    let back = key(okp_import(
        &mut keys,
        "pkcs8",
        Value::Bytes(pkcs8),
        "X25519",
        vec![Usage::DeriveBits],
    ));
    assert_eq!(bytes(derive(&mut keys, b_public.id, back.id, None)), ab);
    // Another kind of key's PKCS #8, an Ed25519's, refused as X25519's.
    let (_, ed_private) = okp_pair(&mut keys, "Ed25519", vec![Usage::Sign]);
    let ed_pkcs8 = bytes(export(&mut keys, "pkcs8", ed_private.id));
    assert_eq!(
        error(okp_import(
            &mut keys,
            "pkcs8",
            Value::Bytes(ed_pkcs8),
            "X25519",
            vec![Usage::DeriveBits]
        )),
        "DataError: "
    );
    let mut trailing = bytes(export(&mut keys, "pkcs8", a.id));
    trailing.push(0);
    assert_eq!(
        error(okp_import(
            &mut keys,
            "pkcs8",
            Value::Bytes(trailing),
            "X25519",
            vec![Usage::DeriveBits]
        )),
        "DataError: "
    );
    let good = bytes(export(&mut keys, "pkcs8", a.id));
    let refused = crate::native::tests::failing("X25519Raw", || {
        okp_import(
            &mut keys,
            "pkcs8",
            Value::Bytes(good),
            "X25519",
            vec![Usage::DeriveBits],
        )
    });
    assert_eq!(error(refused), "DataError: ");
    let Ok(Outcome::Jwk(members)) = export(&mut keys, "jwk", a.id) else {
        panic!("a JWK");
    };
    let names: Vec<&str> = members.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["crv", "d", "ext", "key_ops", "kty", "x"]);
    let derived = key(run(
        &mut keys,
        Call::DeriveKey {
            algorithm: dict(&[("name", s("X25519")), ("public", Value::Key(b_public.id))]),
            key: a.id,
            derived: dict(&[("name", s("AES-GCM")), ("length", Value::Num(256.0))]),
            extractable: true,
            usages: vec![Usage::Encrypt],
        },
    ));
    assert_eq!(bytes(export(&mut keys, "raw", derived.id)), ab);
}

/// A key's structured clone (step 4): every kind of key the store keeps,
/// extractable or not, made again from its record with the same type,
/// algorithm, extractable and usages, and the same material (a secret's
/// HMAC the same, a pair's signature verifying); a record changed or of
/// another kind refused.
#[test]
fn every_kind_of_key_clones_through_its_record() {
    use crate::{deserialize_key, serialize_key};
    let mut keys = Keys::default();
    let again = |keys: &mut Keys, info: &KeyInfo| {
        let record = serialize_key(keys, info.id).expect("a record");
        let copy = deserialize_key(keys, &record).expect("the key again");
        assert_eq!(
            (
                copy.kind,
                copy.extractable,
                copy.algorithm.clone(),
                copy.usages.clone()
            ),
            (
                info.kind,
                info.extractable,
                info.algorithm.clone(),
                info.usages.clone()
            )
        );
        copy
    };
    // Secrets: AES, HMAC (a length past a byte's), HKDF.
    let aes = key(aes_key(
        &mut keys,
        "AES-GCM",
        &[7; 16],
        vec![Usage::Encrypt, Usage::Decrypt],
    ));
    again(&mut keys, &aes);
    let hmac = hmac_key(&mut keys, &[5; 9], Some(72.0), false);
    let copy = again(&mut keys, &hmac);
    let sign = |keys: &mut Keys, id| {
        bytes(run(
            keys,
            Call::Sign {
                algorithm: s("HMAC"),
                key: id,
                data: vec![1],
            },
        ))
    };
    assert_eq!(sign(&mut keys, copy.id), sign(&mut keys, hmac.id));
    let hkdf = key(run(
        &mut keys,
        Call::ImportKey {
            format: "raw".into(),
            key_data: Value::Bytes(vec![1; 32]),
            algorithm: s("HKDF"),
            extractable: false,
            usages: vec![Usage::DeriveBits],
        },
    ));
    again(&mut keys, &hkdf);
    // Pairs: each private key, not extractable, signs what its public
    // key's clone verifies.
    let pairs = [
        (
            "RSASSA-PKCS1-v1_5",
            dict(&[
                ("name", s("RSASSA-PKCS1-v1_5")),
                ("modulusLength", Value::Num(1024.0)),
                ("publicExponent", Value::Bytes(vec![1, 0, 1])),
                ("hash", s("SHA-256")),
            ]),
            s("RSASSA-PKCS1-v1_5"),
        ),
        (
            "ECDSA",
            dict(&[("name", s("ECDSA")), ("namedCurve", s("P-384"))]),
            dict(&[("name", s("ECDSA")), ("hash", s("SHA-384"))]),
        ),
        ("Ed25519", s("Ed25519"), s("Ed25519")),
    ];
    for (name, generate, params) in pairs {
        let Ok(Outcome::Pair(public, private)) = run(
            &mut keys,
            Call::GenerateKey {
                algorithm: generate,
                extractable: false,
                usages: vec![Usage::Sign, Usage::Verify],
            },
        ) else {
            panic!("{name}: a pair");
        };
        let (public2, private2) = (again(&mut keys, &public), again(&mut keys, &private));
        let signature = bytes(run(
            &mut keys,
            Call::Sign {
                algorithm: params.clone(),
                key: private2.id,
                data: vec![9],
            },
        ));
        let out = run(
            &mut keys,
            Call::Verify {
                algorithm: params,
                key: public2.id,
                signature,
                data: vec![9],
            },
        );
        assert_eq!(out, Ok(Outcome::Bool(true)), "{name}");
        assert_eq!(
            error(run(
                &mut keys,
                Call::ExportKey {
                    format: "pkcs8".into(),
                    key: private2.id
                }
            )),
            "InvalidAccessError: Failed to execute 'exportKey' on 'SubtleCrypto': key is not extractable",
            "{name}: still not extractable"
        );
    }
    let (x_public, x_private) = okp_pair(&mut keys, "X25519", vec![Usage::DeriveBits]);
    again(&mut keys, &x_public);
    again(&mut keys, &x_private);
    let (e_public, e_private) = ec_pair(&mut keys, "ECDH", "P-256", vec![Usage::DeriveBits]);
    again(&mut keys, &e_public);
    again(&mut keys, &e_private);
    // A record changed, or not a record, is refused.
    let mut record = serialize_key(&keys, aes.id).unwrap();
    let text = String::from_utf8(record.clone())
        .unwrap()
        .replace("\"AES-GCM\"", "\"AES-FOO\"");
    assert!(deserialize_key(&mut keys, text.as_bytes()).is_err());
    record.truncate(record.len() / 2);
    assert!(deserialize_key(&mut keys, &record).is_err());
    assert!(deserialize_key(&mut keys, b"\"not a record\"").is_err());
    assert!(serialize_key(&keys, 999_999).is_none());
}

/// Step 6's fixes from WebCryptoAPI's tests, each as Chrome 157 has it on
/// the same pages: deriveKey looks at the base key before the derived
/// key's length; an imported public key keeps the extractable asked; a
/// KDF key's export is refused before its extractable is looked at; and
/// raw-public and raw-secret are raw's aliases.
#[test]
fn step_six_orders_and_aliases_are_the_references() {
    let mut keys = Keys::default();
    // A key without deriveKey asked for a 192-bit AES key: the key's
    // refusal, not the length's.
    let hkdf = import_raw(&mut keys, s("HKDF"), &[1; 16], &[Usage::DeriveBits]);
    let derive = |keys: &mut Keys, length: f64| {
        run(
            keys,
            Call::DeriveKey {
                algorithm: dict(&[
                    ("name", s("HKDF")),
                    ("hash", s("SHA-256")),
                    ("salt", Value::Bytes(vec![])),
                    ("info", Value::Bytes(vec![])),
                ]),
                key: hkdf.id,
                derived: dict(&[("name", s("AES-CBC")), ("length", Value::Num(length))]),
                extractable: true,
                usages: vec![Usage::Encrypt],
            },
        )
    };
    for length in [192.0, 128.0] {
        assert!(
            error(derive(&mut keys, length)).starts_with("InvalidAccessError: "),
            "{length}"
        );
    }
    let hkdf2 = import_raw(&mut keys, s("HKDF"), &[1; 16], &[Usage::DeriveKey]);
    let derived = run(
        &mut keys,
        Call::DeriveKey {
            algorithm: dict(&[
                ("name", s("HKDF")),
                ("hash", s("SHA-256")),
                ("salt", Value::Bytes(vec![])),
                ("info", Value::Bytes(vec![])),
            ]),
            key: hkdf2.id,
            derived: dict(&[("name", s("AES-CBC")), ("length", Value::Num(192.0))]),
            extractable: true,
            usages: vec![Usage::Encrypt],
        },
    );
    assert_eq!(
        error(derived),
        "OperationError: 192-bit AES keys are not supported"
    );

    // Public keys imported not extractable stay so; raw-public is raw.
    let (ec_public, _) = ec_pair(
        &mut keys,
        "ECDSA",
        "P-256",
        vec![Usage::Sign, Usage::Verify],
    );
    let point = bytes(run(
        &mut keys,
        Call::ExportKey {
            format: "raw".into(),
            key: ec_public.id,
        },
    ));
    let (x_public, _) = okp_pair(&mut keys, "X25519", vec![Usage::DeriveBits]);
    let x = bytes(run(
        &mut keys,
        Call::ExportKey {
            format: "raw".into(),
            key: x_public.id,
        },
    ));
    let ec_alg = dict(&[("name", s("ECDSA")), ("namedCurve", s("P-256"))]);
    for (format, data, alg, usages) in [
        ("raw", point.clone(), ec_alg.clone(), vec![Usage::Verify]),
        (
            "raw-public",
            point.clone(),
            ec_alg.clone(),
            vec![Usage::Verify],
        ),
        ("raw", x.clone(), s("X25519"), vec![]),
        ("raw-public", x.clone(), s("X25519"), vec![]),
    ] {
        for extractable in [false, true] {
            let info = key(run(
                &mut keys,
                Call::ImportKey {
                    format: format.into(),
                    key_data: Value::Bytes(data.clone()),
                    algorithm: alg.clone(),
                    extractable,
                    usages: usages.clone(),
                },
            ));
            assert_eq!(info.extractable, extractable, "{format} {alg:?}");
            let out = run(
                &mut keys,
                Call::ExportKey {
                    format: "raw-public".into(),
                    key: info.id,
                },
            );
            if extractable {
                assert_eq!(bytes(out), data, "{format} {alg:?}: raw-public export");
            } else {
                assert_eq!(
                    error(out),
                    "InvalidAccessError: Failed to execute 'exportKey' on 'SubtleCrypto': key is not extractable"
                );
            }
        }
    }
    let (rsa_public, _) = generate_pair(&mut keys, 2048, &[1, 0, 1], "SHA-256");
    let spki = bytes(run(
        &mut keys,
        Call::ExportKey {
            format: "spki".into(),
            key: rsa_public.id,
        },
    ));
    let rsa = key(run(
        &mut keys,
        Call::ImportKey {
            format: "spki".into(),
            key_data: Value::Bytes(spki),
            algorithm: dict(&[("name", s("RSASSA-PKCS1-v1_5")), ("hash", s("SHA-256"))]),
            extractable: false,
            usages: vec![Usage::Verify],
        },
    ));
    assert!(!rsa.extractable);

    // PBKDF2's length not a multiple of 8, in the reference's words
    // (probed: enforce-range).
    let password = import_raw(&mut keys, s("PBKDF2"), &[4; 8], &[Usage::DeriveBits]);
    let pbkdf2 = dict(&[
        ("name", s("PBKDF2")),
        ("hash", s("SHA-256")),
        ("salt", Value::Bytes(vec![1])),
        ("iterations", Value::Num(1.0)),
    ]);
    assert_eq!(
        error(run(
            &mut keys,
            Call::DeriveBits {
                algorithm: pbkdf2,
                key: password.id,
                length: Some(12)
            }
        )),
        "OperationError: Length for PBKDF2 key derivation must be a multiple of 8 bits."
    );
    // A KDF key: raw-secret imports it; its export is refused as no
    // export, before its extractable (false) is looked at.
    let pbkdf2 = key(run(
        &mut keys,
        Call::ImportKey {
            format: "raw-secret".into(),
            key_data: Value::Bytes(vec![2; 16]),
            algorithm: s("PBKDF2"),
            extractable: false,
            usages: vec![Usage::DeriveBits],
        },
    ));
    for format in ["raw", "jwk", "raw-secret"] {
        assert_eq!(
            error(run(
                &mut keys,
                Call::ExportKey {
                    format: format.into(),
                    key: pbkdf2.id
                }
            )),
            "NotSupportedError: Failed to execute 'exportKey' on 'SubtleCrypto': PBKDF2: Unsupported operation: exportKey"
        );
    }
    let wrapping = key(aes_key(&mut keys, "AES-KW", &[3; 16], vec![Usage::WrapKey]));
    assert_eq!(
        error(run(
            &mut keys,
            Call::WrapKey {
                format: "raw".into(),
                key: hkdf.id,
                wrapping_key: wrapping.id,
                algorithm: s("AES-KW"),
            },
        )),
        "NotSupportedError: HKDF: Unsupported operation: exportKey"
    );
}

/// `supports` says yes exactly where the operation, on some throwaway key
/// that fits, takes the algorithm (the lead's ruling for
/// SubtleCrypto.supports): never yes to what the operation refuses, and
/// no only where it refuses. Every operation, every name the reference
/// recognizes and some it does not, and each algorithm's parameters,
/// valid and not, with lengths and derived or wrapped keys' algorithms
/// (`oracle::Oracle::variants`), each held to the operation itself.
#[test]
fn supports_answers_as_the_operation_does_everywhere() {
    let mut oracle = crate::oracle::Oracle::new();
    let variants = oracle.variants();
    let wrong: Vec<String> = variants
        .iter()
        .filter_map(|(op, algorithm, extra)| oracle.check(op, algorithm, extra).err())
        .collect();
    assert!(
        wrong.is_empty(),
        "{} of {} disagree:
{}",
        wrong.len(),
        variants.len(),
        wrong.join(
            "
"
        )
    );
}

/// An imported public key keeps the extractable it was asked, as the
/// reference's does (WPT import_export, which Chrome 157 passes): EC and
/// RSA by SPKI, EC by its raw point, Ed25519 and X25519 by their raw
/// bytes. A generated pair's public key alone is extractable whatever
/// was asked.
#[test]
fn an_imported_public_key_keeps_the_extractable_asked() {
    let mut keys = Keys::default();
    let export = |keys: &mut Keys, format: &str, id: u32| {
        bytes(run(
            keys,
            Call::ExportKey {
                format: format.into(),
                key: id,
            },
        ))
    };
    let (ec, _) = ec_pair(
        &mut keys,
        "ECDSA",
        "P-256",
        vec![Usage::Sign, Usage::Verify],
    );
    assert!(ec.extractable, "a generated pair's public key");
    let (ed, _) = okp_pair(&mut keys, "Ed25519", vec![Usage::Sign, Usage::Verify]);
    let (x, _) = okp_pair(&mut keys, "X25519", vec![Usage::DeriveBits]);
    let (rsa, _) = generate_pair(&mut keys, 2048, &[1, 0, 1], "SHA-256");
    let ec_alg = dict(&[("name", s("ECDSA")), ("namedCurve", s("P-256"))]);
    let cases = [
        (
            "spki",
            export(&mut keys, "spki", ec.id),
            ec_alg.clone(),
            vec![Usage::Verify],
        ),
        (
            "raw",
            export(&mut keys, "raw", ec.id),
            ec_alg,
            vec![Usage::Verify],
        ),
        (
            "raw",
            export(&mut keys, "raw", ed.id),
            s("Ed25519"),
            vec![Usage::Verify],
        ),
        ("raw", export(&mut keys, "raw", x.id), s("X25519"), vec![]),
        (
            "spki",
            export(&mut keys, "spki", rsa.id),
            dict(&[("name", s("RSASSA-PKCS1-v1_5")), ("hash", s("SHA-256"))]),
            vec![Usage::Verify],
        ),
    ];
    for (format, data, algorithm, usages) in cases {
        for extractable in [false, true] {
            let info = key(run(
                &mut keys,
                Call::ImportKey {
                    format: format.into(),
                    key_data: Value::Bytes(data.clone()),
                    algorithm: algorithm.clone(),
                    extractable,
                    usages: usages.clone(),
                },
            ));
            assert_eq!(info.extractable, extractable, "{format} {algorithm:?}");
            let out = run(
                &mut keys,
                Call::ExportKey {
                    format: format.into(),
                    key: info.id,
                },
            );
            match extractable {
                true => assert_eq!(bytes(out), data, "{format} {algorithm:?}"),
                false => assert_eq!(
                    error(out),
                    "InvalidAccessError: Failed to execute 'exportKey' on 'SubtleCrypto': key is not extractable",
                    "{format} {algorithm:?}"
                ),
            }
        }
    }
}

/// ChaCha20-Poly1305 (G255 step 5a): RFC 8439's AEAD vector (section
/// 2.8.2), and every refusal in the reference's words (Chrome 158,
/// probed: chacha), its formats raw-secret and jwk (C20P) alone.
#[test]
fn chacha20_poly1305_is_rfc_8439s_and_the_references() {
    use crate::{deserialize_key, serialize_key};
    let mut keys = Keys::default();
    let name = "ChaCha20-Poly1305";
    let key_bytes = hex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f");
    let import = |keys: &mut Keys, format: &str, data: Value| {
        run(
            keys,
            Call::ImportKey {
                format: format.into(),
                key_data: data,
                algorithm: s(name),
                extractable: true,
                usages: vec![Usage::Encrypt, Usage::Decrypt],
            },
        )
    };
    let key_info = key(import(
        &mut keys,
        "raw-secret",
        Value::Bytes(key_bytes.clone()),
    ));
    assert_eq!(
        key_info.algorithm,
        crate::algorithms::KeyAlgorithm::Chacha.members()
    );
    let params = |iv: Vec<u8>, extra: &[(&str, Value)]| {
        let mut members = vec![("name", s(name)), ("iv", Value::Bytes(iv))];
        members.extend(extra.iter().cloned());
        dict(&members)
    };
    let iv = hex("070000004041424344454647");
    let aad = hex("50515253c0c1c2c3c4c5c6c7");
    let plain = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.".to_vec();
    let sealed = bytes(run(
        &mut keys,
        Call::Encrypt {
            algorithm: params(iv.clone(), &[("additionalData", Value::Bytes(aad.clone()))]),
            key: key_info.id,
            data: plain.clone(),
        },
    ));
    let mut want = hex(
        "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b6116",
    );
    want.extend(hex("1ae10b594f09e26a7e902ecbd0600691"));
    assert_eq!(sealed, want);
    let decrypt = |keys: &mut Keys, algorithm: Value, data: Vec<u8>| {
        run(
            keys,
            Call::Decrypt {
                algorithm,
                key: key_info.id,
                data,
            },
        )
    };
    let opened = decrypt(
        &mut keys,
        params(iv.clone(), &[("additionalData", Value::Bytes(aad.clone()))]),
        sealed.clone(),
    );
    assert_eq!(bytes(opened), plain);
    // The refusals, in the reference's words.
    let mut tampered = sealed.clone();
    *tampered.last_mut().expect("a tag") ^= 1;
    assert_eq!(
        error(decrypt(
            &mut keys,
            params(iv.clone(), &[("additionalData", Value::Bytes(aad))]),
            tampered
        )),
        "OperationError: "
    );
    assert_eq!(
        error(decrypt(&mut keys, params(iv.clone(), &[]), vec![0; 15])),
        "OperationError: The provided data is too small"
    );
    let encrypt = |keys: &mut Keys, algorithm: Value| {
        run(
            keys,
            Call::Encrypt {
                algorithm,
                key: key_info.id,
                data: vec![1],
            },
        )
    };
    for iv in [vec![], vec![0; 16]] {
        assert_eq!(
            error(encrypt(&mut keys, params(iv, &[]))),
            "OperationError: The \"iv\" has an unexpected length -- must be 16 bytes"
        );
    }
    assert_eq!(
        error(encrypt(
            &mut keys,
            params(vec![0; 12], &[("tagLength", Value::Num(96.0))])
        )),
        "OperationError: The tag length is invalid: Must be 128 bits"
    );
    assert_eq!(
        bytes(encrypt(
            &mut keys,
            params(vec![0; 12], &[("tagLength", Value::Num(128.0))])
        ))
        .len(),
        17
    );
    // A JWK's alg is C20P's.
    let k = crate::jwk::encode(&key_bytes);
    let jwk = |alg: &str| dict(&[("kty", s("oct")), ("k", s(&k)), ("alg", s(alg))]);
    key(import(&mut keys, "jwk", jwk("C20P")));
    assert_eq!(
        error(import(&mut keys, "jwk", jwk("A256GCM"))),
        "DataError: The JWK \"alg\" member was inconsistent with that specified by the Web Crypto call"
    );
    // Its formats: raw-secret and jwk alone; 256 bits alone.
    for format in ["raw", "spki", "pkcs8"] {
        assert_eq!(
            error(import(&mut keys, format, Value::Bytes(key_bytes.clone()))),
            "NotSupportedError: Unsupported import key format for algorithm",
            "{format}"
        );
    }
    for length in [16, 33] {
        assert_eq!(
            error(import(
                &mut keys,
                "raw-secret",
                Value::Bytes(vec![0; length])
            )),
            "DataError: ChaCha20-Poly1305 key data must be 256 bits"
        );
    }
    match run(
        &mut keys,
        Call::ExportKey {
            format: "jwk".into(),
            key: key_info.id,
        },
    ) {
        Ok(Outcome::Jwk(members)) => {
            let names: Vec<&str> = members.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(names, ["alg", "ext", "k", "key_ops", "kty"]);
            assert_eq!(members[0].1, JwkValue::Str("C20P".into()));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        error(run(
            &mut keys,
            Call::ExportKey {
                format: "raw".into(),
                key: key_info.id
            }
        )),
        "NotSupportedError: Unsupported export key format for algorithm"
    );
    assert_eq!(
        bytes(run(
            &mut keys,
            Call::ExportKey {
                format: "raw-secret".into(),
                key: key_info.id
            }
        )),
        key_bytes
    );
    // A generated key: 256 bits, whatever length was asked; cloned as itself.
    let made = key(run(
        &mut keys,
        Call::GenerateKey {
            algorithm: dict(&[("name", s(name)), ("length", Value::Num(128.0))]),
            extractable: false,
            usages: vec![Usage::Encrypt],
        },
    ));
    let record = serialize_key(&keys, made.id).expect("a record");
    let copy = deserialize_key(&mut keys, &record).expect("the key again");
    assert_eq!(
        (copy.algorithm, copy.extractable),
        (crate::algorithms::KeyAlgorithm::Chacha.members(), false)
    );
}
