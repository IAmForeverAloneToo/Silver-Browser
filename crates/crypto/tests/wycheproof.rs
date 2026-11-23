//! Wycheproof's vectors (C2SP/wycheproof, `tests/wycheproof/`) through
//! Web Crypto's own calls (G255): each key imported, each operation made
//! as a page would make it, each valid vector's output the vector's, each
//! invalid one refused.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::Path;

use aws_lc_rs::digest;
use crypto::{Call, Keys, Outcome, Usage, Value, run};
use devtools::Json;

/// The vendored files are upstream's (the lead's ruling, 2026-11-23):
/// every file of `tests/wycheproof/` but its manifest and README is named
/// in `MANIFEST` and hashes to its line, so that gitleaks' allowlist of
/// the vector files (`/.gitleaks.toml`) hides nothing upstream lacked.
#[test]
fn every_file_here_is_upstreams() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wycheproof");
    let manifest = std::fs::read_to_string(dir.join("MANIFEST")).unwrap();
    let mut listed: HashMap<&str, &str> = manifest
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split_once('\t').expect("name, a tab, its SHA-256"))
        .collect();
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        if name == "MANIFEST" || name == "README.md" {
            continue;
        }
        assert!(entry.file_type().unwrap().is_file(), "{name}: not a file");
        let want = listed
            .remove(name.as_str())
            .unwrap_or_else(|| panic!("{name} is not in the manifest"));
        let bytes = std::fs::read(entry.path()).unwrap();
        let got: String = digest::digest(&digest::SHA256, &bytes)
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(got, want, "{name} is not upstream's bytes");
        checked += 1;
    }
    assert!(listed.is_empty(), "named but missing: {listed:?}");
    assert!(checked >= 2, "checked {checked}");
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn vectors(name: &str) -> Json {
    let path = format!("{}/tests/wycheproof/{name}", env!("CARGO_MANIFEST_DIR"));
    Json::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn field<'a>(json: &'a Json, name: &str) -> &'a str {
    json.get(name).and_then(Json::as_str).unwrap_or("")
}

#[test]
fn aes_gcm_every_vector() {
    let file = vectors("aes_gcm_test.json");
    let mut ran = 0;
    for group in file.get("testGroups").and_then(Json::as_array).unwrap() {
        let key_size = group.get("keySize").and_then(Json::as_u64).unwrap();
        let tag_size = group.get("tagSize").and_then(Json::as_u64).unwrap();
        for test in group.get("tests").and_then(Json::as_array).unwrap() {
            let id = test.get("tcId").and_then(Json::as_u64).unwrap();
            let key = hex(field(test, "key"));
            let iv = hex(field(test, "iv"));
            let aad = hex(field(test, "aad"));
            let msg = hex(field(test, "msg"));
            let sealed = [hex(field(test, "ct")), hex(field(test, "tag"))].concat();
            let valid = field(test, "result") == "valid";
            let mut keys = Keys::default();
            let imported = run(
                &mut keys,
                Call::ImportKey {
                    format: "raw".into(),
                    key_data: Value::Bytes(key),
                    algorithm: Value::Str("AES-GCM".into()),
                    extractable: false,
                    usages: vec![Usage::Encrypt, Usage::Decrypt],
                },
            );
            // 192-bit keys the reference refuses, as the store does.
            if key_size == 192 {
                assert!(imported.is_err(), "tc {id}: a 192-bit key refused");
                continue;
            }
            let Ok(Outcome::Key(k)) = imported else {
                panic!("tc {id}: the key imports");
            };
            let params = Value::Dict(vec![
                ("name".into(), Value::Str("AES-GCM".into())),
                ("iv".into(), Value::Bytes(iv.clone())),
                ("additionalData".into(), Value::Bytes(aad.clone())),
                ("tagLength".into(), Value::Num(tag_size as f64)),
            ]);
            let opened = run(
                &mut keys,
                Call::Decrypt {
                    algorithm: params.clone(),
                    key: k.id,
                    data: sealed.clone(),
                },
            );
            // An empty IV the reference refuses before any decryption.
            if iv.is_empty() {
                assert!(opened.is_err(), "tc {id}: an empty IV refused");
                continue;
            }
            if valid {
                assert_eq!(opened, Ok(Outcome::Bytes(msg.clone())), "tc {id}: decrypts");
                let made = run(
                    &mut keys,
                    Call::Encrypt {
                        algorithm: params,
                        key: k.id,
                        data: msg,
                    },
                );
                assert_eq!(made, Ok(Outcome::Bytes(sealed)), "tc {id}: encrypts");
            } else {
                assert!(opened.is_err(), "tc {id}: an invalid vector refused");
            }
            ran += 1;
        }
    }
    assert!(ran > 150, "ran {ran}");
}

fn str_value(text: &str) -> Value {
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

/// An RSA key of `format` (`spki` or `pkcs8`) for `name` with `hash`.
fn rsa_key(
    keys: &mut Keys,
    format: &str,
    der: Vec<u8>,
    name: &str,
    hash: &str,
    usage: Usage,
) -> Option<u32> {
    let out = run(
        keys,
        Call::ImportKey {
            format: format.into(),
            key_data: Value::Bytes(der),
            algorithm: dict(&[("name", str_value(name)), ("hash", str_value(hash))]),
            extractable: false,
            usages: vec![usage],
        },
    );
    match out {
        Ok(Outcome::Key(k)) => Some(k.id),
        _ => None,
    }
}

/// Each test of each group of the vector files whose names start with
/// `prefix`: the file, the group and the test.
fn each(prefix: &str, mut f: impl FnMut(&str, &Json, &Json)) -> usize {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wycheproof");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.starts_with(prefix) && n.ends_with("_test.json"))
        .collect();
    names.sort();
    let mut ran = 0;
    for name in &names {
        let file = vectors(name);
        for group in file.get("testGroups").and_then(Json::as_array).unwrap() {
            for test in group.get("tests").and_then(Json::as_array).unwrap() {
                f(name, group, test);
                ran += 1;
            }
        }
    }
    assert!(!names.is_empty(), "no {prefix} files");
    ran
}

/// RSASSA-PKCS1-v1_5's verification (G255 part 2b): every valid vector
/// true, every invalid one false (never an error: a bad signature is no
/// reason to refuse), an acceptable one either.
#[test]
fn rsa_pkcs1_verify_every_vector() {
    let mut keys = Keys::default();
    let ran = each("rsa_signature_", |name, group, test| {
        let id = test.get("tcId").and_then(Json::as_u64).unwrap();
        let hash = field(group, "sha");
        let key = rsa_key(
            &mut keys,
            "spki",
            hex(field(group, "publicKeyDer")),
            "RSASSA-PKCS1-v1_5",
            hash,
            Usage::Verify,
        )
        .unwrap_or_else(|| panic!("{name} tc {id}: the key imports"));
        let out = run(
            &mut keys,
            Call::Verify {
                algorithm: str_value("RSASSA-PKCS1-v1_5"),
                key,
                signature: hex(field(test, "sig")),
                data: hex(field(test, "msg")),
            },
        );
        match field(test, "result") {
            "valid" => assert_eq!(out, Ok(Outcome::Bool(true)), "{name} tc {id}"),
            "invalid" => assert_eq!(out, Ok(Outcome::Bool(false)), "{name} tc {id}"),
            _ => assert!(
                matches!(out, Ok(Outcome::Bool(_))),
                "{name} tc {id}: {out:?}"
            ),
        }
    });
    assert!(ran > 1000, "ran {ran}");
}

/// RSASSA-PKCS1-v1_5's signing: the private key's PKCS #8 imported, the
/// signature the vector's, byte for byte (it is deterministic), at 1024
/// and 1536 bits (the library's route) and 2048 (aws-lc-rs's).
#[test]
fn rsa_pkcs1_sign_every_vector() {
    let mut keys = Keys::default();
    let mut skipped = 0;
    let ran = each("rsa_pkcs1_", |name, group, test| {
        let id = test.get("tcId").and_then(Json::as_u64).unwrap();
        let hash = field(group, "sha");
        // SHA-224 and the like, which Web Crypto has none of.
        if !matches!(hash, "SHA-1" | "SHA-256" | "SHA-384" | "SHA-512") {
            skipped += 1;
            return;
        }
        let key = rsa_key(
            &mut keys,
            "pkcs8",
            hex(field(group, "privateKeyPkcs8")),
            "RSASSA-PKCS1-v1_5",
            hash,
            Usage::Sign,
        )
        .unwrap_or_else(|| panic!("{name} tc {id}: the key imports"));
        let out = run(
            &mut keys,
            Call::Sign {
                algorithm: str_value("RSASSA-PKCS1-v1_5"),
                key,
                data: hex(field(test, "msg")),
            },
        );
        assert_eq!(
            out,
            Ok(Outcome::Bytes(hex(field(test, "sig")))),
            "{name} tc {id}"
        );
    });
    assert!(ran - skipped > 50, "ran {ran}, skipped {skipped}");
}

/// RSA-PSS's verification, at the group's salt, MGF1 with the hash: every
/// valid vector true, every invalid one false.
#[test]
fn rsa_pss_verify_every_vector() {
    let mut keys = Keys::default();
    let ran = each("rsa_pss_", |name, group, test| {
        let id = test.get("tcId").and_then(Json::as_u64).unwrap();
        let hash = field(group, "sha");
        assert_eq!(
            field(group, "mgfSha"),
            hash,
            "{name}: MGF1 with the hash alone"
        );
        let salt = group.get("sLen").and_then(Json::as_u64).unwrap();
        let key = rsa_key(
            &mut keys,
            "spki",
            hex(field(group, "publicKeyDer")),
            "RSA-PSS",
            hash,
            Usage::Verify,
        )
        .unwrap_or_else(|| panic!("{name} tc {id}: the key imports"));
        let out = run(
            &mut keys,
            Call::Verify {
                algorithm: dict(&[
                    ("name", str_value("RSA-PSS")),
                    ("saltLength", Value::Num(salt as f64)),
                ]),
                key,
                signature: hex(field(test, "sig")),
                data: hex(field(test, "msg")),
            },
        );
        let want = field(test, "result") == "valid";
        assert_eq!(out, Ok(Outcome::Bool(want)), "{name} tc {id}");
    });
    assert!(ran > 300, "ran {ran}");
}

/// RSA-OAEP's decryption (aws-lc-rs's, 2048 and 3072 bits), with each
/// vector's label: a valid one its message, an invalid one an
/// OperationError.
#[test]
fn rsa_oaep_decrypt_every_vector() {
    let mut keys = Keys::default();
    let ran = each("rsa_oaep_", |name, group, test| {
        let id = test.get("tcId").and_then(Json::as_u64).unwrap();
        let hash = field(group, "sha");
        assert_eq!(
            field(group, "mgfSha"),
            hash,
            "{name}: MGF1 with the hash alone"
        );
        let key = rsa_key(
            &mut keys,
            "pkcs8",
            hex(field(group, "privateKeyPkcs8")),
            "RSA-OAEP",
            hash,
            Usage::Decrypt,
        )
        .unwrap_or_else(|| panic!("{name} tc {id}: the key imports"));
        let out = run(
            &mut keys,
            Call::Decrypt {
                algorithm: dict(&[
                    ("name", str_value("RSA-OAEP")),
                    ("label", Value::Bytes(hex(field(test, "label")))),
                ]),
                key,
                data: hex(field(test, "ct")),
            },
        );
        if field(test, "result") == "valid" {
            assert_eq!(
                out,
                Ok(Outcome::Bytes(hex(field(test, "msg")))),
                "{name} tc {id}"
            );
        } else {
            let refused = matches!(&out, Err(e) if e.name == "OperationError");
            assert!(refused, "{name} tc {id}: {out:?}");
        }
    });
    assert!(ran > 100, "ran {ran}");
}

/// A JSON object as the importKey dictionary it is.
fn json_value(json: &Json) -> Value {
    match json {
        Json::Null => Value::Absent,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => Value::Num(*n),
        Json::String(s) => Value::Str(s.clone()),
        Json::Array(items) => Value::List(items.iter().map(json_value).collect()),
        Json::Object(members) => Value::Dict(
            members
                .iter()
                .map(|(n, v)| (n.clone(), json_value(v)))
                .collect(),
        ),
    }
}

/// ECDSA's verification of r and s (P1363, Web Crypto's form), on both
/// of aws-lc-rs's ways (the fixed form, and the ASN.1 form this crate
/// writes for P-256 and P-384 with SHA-512): every valid vector true,
/// every invalid one false, never an error.
#[test]
fn ecdsa_p1363_verify_every_vector() {
    let mut keys = Keys::default();
    let ran = each("ecdsa_", |name, group, test| {
        let id = test.get("tcId").and_then(Json::as_u64).unwrap();
        let hash = field(group, "sha");
        let curve = match field(group.get("publicKey").unwrap(), "curve") {
            "secp384r1" => "P-384",
            "secp521r1" => "P-521",
            _ => "P-256",
        };
        let out = run(
            &mut keys,
            Call::ImportKey {
                format: "spki".into(),
                key_data: Value::Bytes(hex(field(group, "publicKeyDer"))),
                algorithm: dict(&[
                    ("name", str_value("ECDSA")),
                    ("namedCurve", str_value(curve)),
                ]),
                extractable: true,
                usages: vec![Usage::Verify],
            },
        );
        let Ok(Outcome::Key(key)) = out else {
            panic!("{name} tc {id}: the key imports: {out:?}");
        };
        let out = run(
            &mut keys,
            Call::Verify {
                algorithm: dict(&[("name", str_value("ECDSA")), ("hash", str_value(hash))]),
                key: key.id,
                signature: hex(field(test, "sig")),
                data: hex(field(test, "msg")),
            },
        );
        let want = field(test, "result") == "valid";
        assert_eq!(out, Ok(Outcome::Bool(want)), "{name} tc {id}");
    });
    assert!(ran > 1000, "ran {ran}");
}

/// ECDH with Web Crypto's own JWKs: every valid vector's shared secret,
/// every invalid one refused (as its key's import or its derivation),
/// never a wrong secret.
#[test]
fn ecdh_webcrypto_every_vector() {
    let mut keys = Keys::default();
    let ran = each("ecdh_", |name, group, test| {
        let id = test.get("tcId").and_then(Json::as_u64).unwrap();
        let curve = field(group, "curve");
        let crv = match curve {
            "secp384r1" => "P-384",
            "secp521r1" => "P-521",
            _ => "P-256",
        };
        let import = |keys: &mut Keys, json: &Json, usages: Vec<Usage>| {
            run(
                keys,
                Call::ImportKey {
                    format: "jwk".into(),
                    key_data: json_value(json),
                    algorithm: dict(&[("name", str_value("ECDH")), ("namedCurve", str_value(crv))]),
                    extractable: true,
                    usages,
                },
            )
        };
        let public = import(&mut keys, test.get("public").unwrap(), vec![]);
        let private = import(
            &mut keys,
            test.get("private").unwrap(),
            vec![Usage::DeriveBits],
        );
        let valid = field(test, "result") == "valid";
        let (Ok(Outcome::Key(public)), Ok(Outcome::Key(private))) = (&public, &private) else {
            assert!(
                !valid,
                "{name} tc {id}: the keys import: {public:?} {private:?}"
            );
            return;
        };
        let out = run(
            &mut keys,
            Call::DeriveBits {
                algorithm: dict(&[
                    ("name", str_value("ECDH")),
                    ("public", Value::Key(public.id)),
                ]),
                key: private.id,
                length: None,
            },
        );
        if valid {
            assert_eq!(
                out,
                Ok(Outcome::Bytes(hex(field(test, "shared")))),
                "{name} tc {id}"
            );
        } else {
            assert!(out.is_err(), "{name} tc {id}: {out:?}");
        }
    });
    assert!(ran > 300, "ran {ran}");
}
