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
