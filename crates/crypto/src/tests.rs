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
    // The DER aws-lc-rs wrote: an SPKI of rsaEncryption around the key.
    assert_eq!(&spki[..2], &[0x30, 0x5c]);
    assert!(spki.ends_with(&[0x02, 0x03, 0x01, 0x00, 0x01]));
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
