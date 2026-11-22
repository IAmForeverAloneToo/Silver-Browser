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
