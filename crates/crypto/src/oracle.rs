//! `SubtleCrypto.supports`'s contract, checked (the lead's ruling,
//! G255): for an operation, an algorithm and its extra (a length, or a
//! derived or wrapped key's algorithm), `supports` says yes exactly when
//! the operation itself takes them, on some throwaway key that fits.
//! The oracle makes those keys once, and runs the operations through the
//! store's own entry point, as a page's calls run: the crate's tests
//! enumerate the combinations (`variants`), and the fuzz target
//! `crypto_supports` makes random ones.

use crate::{Call, JwkValue, Keys, Outcome, Usage, Value, run, supports};

/// The operations `supports` answers for: the store's, and those it has
/// not made (getPublicKey and the KEM methods, G255 step 5), which it
/// must refuse.
pub const OPERATIONS: &[&str] = &[
    "encrypt",
    "decrypt",
    "sign",
    "verify",
    "digest",
    "generateKey",
    "deriveKey",
    "deriveBits",
    "importKey",
    "exportKey",
    "wrapKey",
    "unwrapKey",
    "getPublicKey",
    "encapsulateKey",
    "encapsulateBits",
    "decapsulateKey",
    "decapsulateBits",
];

/// Every name the reference recognizes, and some it does not.
pub const NAMES: &[&str] = &[
    "AES-CTR",
    "AES-CBC",
    "AES-GCM",
    "AES-KW",
    "ChaCha20-Poly1305",
    "HMAC",
    "RSASSA-PKCS1-v1_5",
    "RSA-PSS",
    "RSA-OAEP",
    "ECDSA",
    "ECDH",
    "HKDF",
    "PBKDF2",
    "SHA-1",
    "SHA-256",
    "SHA-384",
    "SHA-512",
    "Ed25519",
    "X25519",
    "ML-KEM-768",
    "ML-KEM-1024",
    "ML-DSA-44",
    "ML-DSA-65",
    "ML-DSA-87",
    "aes-gcm",
    "MD5",
    "X448",
    "",
];

/// The usages a key is asked for, each tried in turn.
const USAGE_SETS: &[&[Usage]] = &[
    &[],
    &[
        Usage::Encrypt,
        Usage::Decrypt,
        Usage::WrapKey,
        Usage::UnwrapKey,
    ],
    &[Usage::Encrypt, Usage::Decrypt],
    &[Usage::WrapKey, Usage::UnwrapKey],
    &[Usage::Sign, Usage::Verify],
    &[Usage::Sign],
    &[Usage::Verify],
    &[Usage::DeriveKey, Usage::DeriveBits],
    &[Usage::Encrypt, Usage::WrapKey],
    &[Usage::Decrypt, Usage::UnwrapKey],
];

/// The formats a key is exported to and imported from.
const FORMATS: &[&str] = &[
    "raw",
    "raw-secret",
    "raw-public",
    "raw-seed",
    "spki",
    "pkcs8",
    "jwk",
];

/// A throwaway key: its number, its pair's other half, and the
/// algorithm it was made with (which imports it again).
struct Made {
    id: u32,
    partner: Option<u32>,
    algorithm: Value,
    /// One of the few a wrap is tried on.
    wrapped: bool,
    /// A pair's private half.
    private: bool,
}

/// The one bound the throwaway keys cannot reach: RSA-PSS's salt, which
/// fits keys up to the largest the store takes (16384 bits) while the
/// oracle's are 2048 bits; `supports` says yes to any salt a key of the
/// store's could take.
fn beyond_the_keys(op: &str, algorithm: &Value) -> bool {
    matches!(op, "sign" | "verify")
        && algorithm.get("name") == &s("RSA-PSS")
        && matches!(algorithm.get("saltLength"), Value::Num(salt) if *salt > 222.0 && *salt <= 2026.0)
}

/// The throwaway keys, the key data they export to, and the operations
/// run on them.
pub struct Oracle {
    keys: Keys,
    made: Vec<Made>,
    /// Key data to import from: a format and its data.
    materials: Vec<(&'static str, Value)>,
    /// Keys an agreement's `public` member names: a label and the key.
    publics: Vec<(&'static str, u32)>,
    /// HMAC keys whose raw bytes are a JWK's text, padded with spaces to
    /// a multiple of 8 bytes: an unwrap's data as any text may be.
    texts: Vec<u32>,
    /// The first number of a key made by an operation run here: those
    /// go once the run is done.
    scratch: u32,
}

fn s(text: &str) -> Value {
    Value::Str(text.to_owned())
}

fn n(number: f64) -> Value {
    Value::Num(number)
}

fn b(len: usize) -> Value {
    Value::Bytes(vec![0x5a; len])
}

fn d(members: &[(&str, Value)]) -> Value {
    Value::Dict(
        members
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

fn ok(out: &Result<Outcome, crate::Error>) -> bool {
    out.is_ok()
}

fn jwk_value(members: Vec<(String, JwkValue)>) -> Value {
    Value::Dict(
        members
            .into_iter()
            .map(|(name, value)| {
                let value = match value {
                    JwkValue::Str(text) => Value::Str(text),
                    JwkValue::Bool(flag) => Value::Bool(flag),
                    JwkValue::List(items) => {
                        Value::List(items.into_iter().map(Value::Str).collect())
                    }
                };
                (name, value)
            })
            .collect(),
    )
}

impl Default for Oracle {
    fn default() -> Self {
        Self::new()
    }
}

impl Oracle {
    /// The throwaway keys: one of each algorithm and its every curve,
    /// hash and AES size the store makes (RSA's at 2048 bits), and their
    /// exports as key data.
    pub fn new() -> Oracle {
        let mut keys = Keys::default();
        let mut made: Vec<Made> = Vec::new();
        let hashes = ["SHA-1", "SHA-256", "SHA-384", "SHA-512"];
        let mut specs: Vec<(Value, bool)> = Vec::new();
        for name in ["AES-CTR", "AES-CBC", "AES-GCM", "AES-KW"] {
            for length in [128.0, 256.0] {
                specs.push((d(&[("name", s(name)), ("length", n(length))]), false));
            }
        }
        specs.push((s("ChaCha20-Poly1305"), true));
        for hash in hashes {
            specs.push((
                d(&[("name", s("HMAC")), ("hash", s(hash))]),
                hash == "SHA-256",
            ));
        }
        for name in ["RSASSA-PKCS1-v1_5", "RSA-PSS", "RSA-OAEP"] {
            for hash in hashes {
                specs.push((
                    d(&[
                        ("name", s(name)),
                        ("modulusLength", n(2048.0)),
                        ("publicExponent", Value::Bytes(vec![1, 0, 1])),
                        ("hash", s(hash)),
                    ]),
                    name == "RSA-OAEP" && hash == "SHA-256",
                ));
            }
        }
        for name in ["ECDSA", "ECDH"] {
            for curve in ["P-256", "P-384", "P-521"] {
                specs.push((
                    d(&[("name", s(name)), ("namedCurve", s(curve))]),
                    curve == "P-256",
                ));
            }
        }
        // A key large enough to wrap a 2048-bit RSA key with RSA-OAEP.
        specs.push((
            d(&[
                ("name", s("RSA-OAEP")),
                ("modulusLength", n(4096.0)),
                ("publicExponent", Value::Bytes(vec![1, 0, 1])),
                ("hash", s("SHA-256")),
            ]),
            false,
        ));
        specs.push((s("Ed25519"), true));
        specs.push((s("X25519"), true));
        for name in ["ML-DSA-44", "ML-DSA-65", "ML-DSA-87"] {
            specs.push((s(name), name == "ML-DSA-44"));
        }
        specs.push((d(&[("name", s("AES-GCM")), ("length", n(256.0))]), true));
        for (algorithm, wrapped) in specs {
            let made_now = USAGE_SETS.iter().find_map(|usages| {
                match run(
                    &mut keys,
                    Call::GenerateKey {
                        algorithm: algorithm.clone(),
                        extractable: true,
                        usages: usages.to_vec(),
                    },
                ) {
                    Ok(Outcome::Key(key)) => Some((key.id, None)),
                    Ok(Outcome::Pair(public, private)) => Some((public.id, Some(private.id))),
                    _ => None,
                }
            });
            let Some((id, partner)) = made_now else {
                panic!("the oracle's key {algorithm:?} is not made");
            };
            made.push(Made {
                id,
                partner,
                algorithm: algorithm.clone(),
                wrapped,
                private: false,
            });
            if let Some(private) = partner {
                made.push(Made {
                    id: private,
                    partner: Some(id),
                    algorithm,
                    wrapped,
                    private: true,
                });
            }
        }
        // The KDFs' keys, which only import, and never export.
        for name in ["HKDF", "PBKDF2"] {
            let key = run(
                &mut keys,
                Call::ImportKey {
                    format: "raw".into(),
                    key_data: b(16),
                    algorithm: s(name),
                    extractable: false,
                    usages: vec![Usage::DeriveKey, Usage::DeriveBits],
                },
            );
            let Ok(Outcome::Key(key)) = key else {
                panic!("the oracle's {name} key is not made");
            };
            made.push(Made {
                id: key.id,
                partner: None,
                algorithm: s(name),
                wrapped: true,
                private: false,
            });
        }
        let mut materials: Vec<(&'static str, Value)> = Vec::new();
        let mut texts: Vec<u32> = Vec::new();
        for m in &made {
            for format in FORMATS {
                match run(
                    &mut keys,
                    Call::ExportKey {
                        format: (*format).into(),
                        key: m.id,
                    },
                ) {
                    Ok(Outcome::Bytes(bytes)) => materials.push((format, Value::Bytes(bytes))),
                    Ok(Outcome::Jwk(members)) => {
                        let mut text = crate::algorithms::jwk_text(&members).into_bytes();
                        while !text.len().is_multiple_of(8) {
                            text.push(b' ');
                        }
                        if let Ok(Outcome::Key(key)) = run(
                            &mut keys,
                            Call::ImportKey {
                                format: "raw".into(),
                                key_data: Value::Bytes(text),
                                algorithm: d(&[("name", s("HMAC")), ("hash", s("SHA-256"))]),
                                extractable: true,
                                usages: vec![Usage::Sign],
                            },
                        ) {
                            texts.push(key.id);
                        }
                        materials.push((format, jwk_value(members)));
                    }
                    _ => {}
                }
            }
        }
        // Secrets of other sizes: an HMAC key of any, a KDF's, AES's 192.
        for len in [0, 1, 16, 24, 32, 64] {
            materials.push(("raw", b(len)));
            materials.push(("raw-secret", b(len)));
        }
        let labelled = |name: &str, curve: Option<&str>, private: bool| {
            made.iter()
                .find(|m| {
                    let named = match &m.algorithm {
                        Value::Str(given) => given == name && curve.is_none(),
                        algorithm => {
                            algorithm.get("name") == &s(name)
                                && curve.is_none_or(|c| algorithm.get("namedCurve") == &s(c))
                        }
                    };
                    named && m.partner.is_some() && m.private == private
                })
                .map(|m| m.id)
                .expect("an oracle key")
        };
        let publics = vec![
            ("ECDH P-256 public", labelled("ECDH", Some("P-256"), false)),
            ("ECDH P-384 public", labelled("ECDH", Some("P-384"), false)),
            ("ECDH P-521 public", labelled("ECDH", Some("P-521"), false)),
            ("ECDH P-256 private", labelled("ECDH", Some("P-256"), true)),
            (
                "ECDSA P-256 public",
                labelled("ECDSA", Some("P-256"), false),
            ),
            ("X25519 public", labelled("X25519", None, false)),
            ("X25519 private", labelled("X25519", None, true)),
            ("Ed25519 public", labelled("Ed25519", None, false)),
        ];
        let scratch = keys.next_id();
        Oracle {
            keys,
            made,
            materials,
            publics,
            texts,
            scratch,
        }
    }

    /// The keys an agreement's `public` member may name, labelled.
    pub fn public_keys(&self) -> &[(&'static str, u32)] {
        &self.publics
    }

    /// Whether `supports` and the operation agree on `op`, `algorithm`
    /// and `extra`; what each said when they do not.
    pub fn check(&mut self, op: &str, algorithm: &Value, extra: &Value) -> Result<(), String> {
        let said = supports(&self.keys, op, algorithm, extra);
        let took = self.takes(op, algorithm, extra);
        if said == took || (said && beyond_the_keys(op, algorithm)) {
            return Ok(());
        }
        Err(format!(
            "supports({op}, {algorithm:?}, {extra:?}) says {said}; the operation {}",
            if took { "takes it" } else { "refuses it" }
        ))
    }

    /// Whether the operation takes `algorithm` and `extra` on some
    /// throwaway key that fits.
    pub fn takes(&mut self, op: &str, algorithm: &Value, extra: &Value) -> bool {
        let took = self.run_op(op, algorithm, extra);
        self.keys.forget_from(self.scratch);
        took
    }

    /// A key of `algorithm` imported from any key data that fits, of
    /// any usages, `extractable` as asked (a KDF's must not be).
    /// At most `most` of them. Only a refusal of the usages tries the
    /// next usages: any other refuses the data, or the algorithm.
    fn imported(&mut self, algorithm: &Value, extractable: bool, most: usize) -> Vec<u32> {
        let mut out = Vec::new();
        for (format, data) in self.materials.clone() {
            for usages in USAGE_SETS {
                match run(
                    &mut self.keys,
                    Call::ImportKey {
                        format: format.into(),
                        key_data: data.clone(),
                        algorithm: algorithm.clone(),
                        extractable,
                        usages: usages.to_vec(),
                    },
                ) {
                    Ok(Outcome::Key(key)) => {
                        out.push(key.id);
                        if out.len() == most {
                            return out;
                        }
                        break;
                    }
                    Err(error) if error.name == "SyntaxError" => {}
                    _ => break,
                }
            }
        }
        out
    }

    fn run_op(&mut self, op: &str, algorithm: &Value, extra: &Value) -> bool {
        let data = vec![7u8; 16];
        let alg = || algorithm.clone();
        let ids: Vec<(u32, Option<u32>)> = self.made.iter().map(|m| (m.id, m.partner)).collect();
        match op {
            "digest" => ok(&run(
                &mut self.keys,
                Call::Digest {
                    algorithm: alg(),
                    data,
                },
            )),
            "generateKey" => USAGE_SETS.iter().any(|usages| {
                ok(&run(
                    &mut Keys::default(),
                    Call::GenerateKey {
                        algorithm: alg(),
                        extractable: true,
                        usages: usages.to_vec(),
                    },
                ))
            }),
            "importKey" => {
                !self.imported(algorithm, false, 1).is_empty()
                    || !self.imported(algorithm, true, 1).is_empty()
            }
            "exportKey" => self.imported(algorithm, true, 1).into_iter().any(|key| {
                FORMATS.iter().any(|format| {
                    ok(&run(
                        &mut self.keys,
                        Call::ExportKey {
                            format: (*format).into(),
                            key,
                        },
                    ))
                })
            }),
            "encrypt" => ids.iter().any(|&(key, _)| {
                ok(&run(
                    &mut self.keys,
                    Call::Encrypt {
                        algorithm: alg(),
                        key,
                        data: data.clone(),
                    },
                ))
            }),
            "decrypt" => ids.iter().any(|&(key, partner)| {
                let sealed = match run(
                    &mut self.keys,
                    Call::Encrypt {
                        algorithm: alg(),
                        key: partner.unwrap_or(key),
                        data: data.clone(),
                    },
                ) {
                    Ok(Outcome::Bytes(sealed)) => sealed,
                    _ => vec![0; 256],
                };
                ok(&run(
                    &mut self.keys,
                    Call::Decrypt {
                        algorithm: alg(),
                        key,
                        data: sealed,
                    },
                ))
            }),
            "sign" => ids.iter().any(|&(key, _)| {
                ok(&run(
                    &mut self.keys,
                    Call::Sign {
                        algorithm: alg(),
                        key,
                        data: data.clone(),
                    },
                ))
            }),
            "verify" => ids.iter().any(|&(key, partner)| {
                let signature = match run(
                    &mut self.keys,
                    Call::Sign {
                        algorithm: alg(),
                        key: partner.unwrap_or(key),
                        data: data.clone(),
                    },
                ) {
                    Ok(Outcome::Bytes(signature)) => signature,
                    _ => vec![0; 64],
                };
                ok(&run(
                    &mut self.keys,
                    Call::Verify {
                        algorithm: alg(),
                        key,
                        signature,
                        data: data.clone(),
                    },
                ))
            }),
            "deriveBits" => {
                let length = match extra {
                    Value::Num(number) => Some(*number as u32),
                    _ => None,
                };
                ids.iter().any(|&(key, _)| {
                    ok(&run(
                        &mut self.keys,
                        Call::DeriveBits {
                            algorithm: alg(),
                            key,
                            length,
                        },
                    ))
                })
            }
            "deriveKey" => ids.iter().any(|&(key, _)| {
                USAGE_SETS.iter().any(|usages| {
                    [true, false].into_iter().any(|extractable| {
                        ok(&run(
                            &mut self.keys,
                            Call::DeriveKey {
                                algorithm: alg(),
                                key,
                                derived: extra.clone(),
                                extractable,
                                usages: usages.to_vec(),
                            },
                        ))
                    })
                })
            }),
            "wrapKey" | "unwrapKey" => self.wraps(op, algorithm, extra),
            _ => false,
        }
    }

    /// A wrap (and, for unwrapKey, its unwrap) on some wrapping key and
    /// some key of `extra`'s algorithm, or of any when none is given.
    fn wraps(&mut self, op: &str, algorithm: &Value, extra: &Value) -> bool {
        // Any key's bytes may unwrap as the unwrapped algorithm's (a
        // wrapped HMAC key's raw bytes as an HKDF key's); a wrap wants a
        // key of the wrapped algorithm, as many as differ in size.
        let targets: Vec<(u32, Value)> = match extra {
            Value::Absent => self
                .made
                .iter()
                .filter(|m| m.wrapped)
                .map(|m| (m.id, m.algorithm.clone()))
                .collect(),
            _ if op == "unwrapKey" => self
                .made
                .iter()
                .filter(|m| m.wrapped || m.partner.is_some())
                .map(|m| (m.id, extra.clone()))
                .collect(),
            _ => self
                .imported(extra, true, usize::MAX)
                .into_iter()
                .map(|id| (id, extra.clone()))
                .collect(),
        };
        let wrappers: Vec<(u32, Option<u32>)> =
            self.made.iter().map(|m| (m.id, m.partner)).collect();
        // A JWK's text, wrapped as raw bytes, unwrapped as a JWK.
        if op == "unwrapKey" && !matches!(extra, Value::Absent) {
            for &(wrapping, partner) in &wrappers {
                for &text in &self.texts.clone() {
                    let Ok(Outcome::Bytes(wrapped)) = run(
                        &mut self.keys,
                        Call::WrapKey {
                            format: "raw".into(),
                            key: text,
                            wrapping_key: wrapping,
                            algorithm: algorithm.clone(),
                        },
                    ) else {
                        continue;
                    };
                    for usages in USAGE_SETS {
                        if ok(&run(
                            &mut self.keys,
                            Call::UnwrapKey {
                                format: "jwk".into(),
                                wrapped: wrapped.clone(),
                                unwrapping_key: partner.unwrap_or(wrapping),
                                algorithm: algorithm.clone(),
                                unwrapped_algorithm: extra.clone(),
                                extractable: true,
                                usages: usages.to_vec(),
                            },
                        )) {
                            return true;
                        }
                    }
                }
            }
        }
        for &(wrapping, partner) in &wrappers {
            for (target, unwrapped) in &targets {
                for format in FORMATS {
                    let Ok(Outcome::Bytes(wrapped)) = run(
                        &mut self.keys,
                        Call::WrapKey {
                            format: (*format).into(),
                            key: *target,
                            wrapping_key: wrapping,
                            algorithm: algorithm.clone(),
                        },
                    ) else {
                        continue;
                    };
                    if op == "wrapKey" {
                        return true;
                    }
                    let unwrapping = partner.unwrap_or(wrapping);
                    for usages in USAGE_SETS {
                        for extractable in [true, false] {
                            if ok(&run(
                                &mut self.keys,
                                Call::UnwrapKey {
                                    format: (*format).into(),
                                    wrapped: wrapped.clone(),
                                    unwrapping_key: unwrapping,
                                    algorithm: algorithm.clone(),
                                    unwrapped_algorithm: unwrapped.clone(),
                                    extractable,
                                    usages: usages.to_vec(),
                                },
                            )) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// Every combination the tests hold `supports` to: each operation
    /// with each name alone, and each algorithm's parameters, valid and
    /// not, with lengths and derived or wrapped keys' algorithms.
    pub fn variants(&self) -> Vec<(&'static str, Value, Value)> {
        let mut out: Vec<(&'static str, Value, Value)> = Vec::new();
        let absent = Value::Absent;
        for &op in OPERATIONS {
            for name in NAMES {
                out.push((op, s(name), absent.clone()));
            }
        }
        let aes = ["AES-CTR", "AES-CBC", "AES-GCM", "AES-KW"];
        let hashes = ["SHA-1", "SHA-256", "SHA-384", "SHA-512", "MD5"];
        let rsa = ["RSASSA-PKCS1-v1_5", "RSA-PSS", "RSA-OAEP"];
        // generateKey, importKey and exportKey.
        for name in aes {
            for length in [128.0, 192.0, 256.0, 0.0, 100.0, 257.0] {
                out.push((
                    "generateKey",
                    d(&[("name", s(name)), ("length", n(length))]),
                    absent.clone(),
                ));
            }
        }
        out.push((
            "generateKey",
            d(&[("name", s("ChaCha20-Poly1305")), ("length", n(128.0))]),
            absent.clone(),
        ));
        for op in ["generateKey", "importKey", "exportKey"] {
            for hash in hashes {
                out.push((
                    op,
                    d(&[("name", s("HMAC")), ("hash", s(hash))]),
                    absent.clone(),
                ));
                for length in [0.0, 7.0, 256.0, 1024.0] {
                    out.push((
                        op,
                        d(&[
                            ("name", s("HMAC")),
                            ("hash", s(hash)),
                            ("length", n(length)),
                        ]),
                        absent.clone(),
                    ));
                }
            }
            for name in ["ECDSA", "ECDH"] {
                for curve in ["P-256", "P-384", "P-521", "P-51", "p-256"] {
                    out.push((
                        op,
                        d(&[("name", s(name)), ("namedCurve", s(curve))]),
                        absent.clone(),
                    ));
                }
                out.push((op, d(&[("name", s(name))]), absent.clone()));
            }
        }
        for op in ["importKey", "exportKey"] {
            for name in rsa {
                for hash in hashes {
                    out.push((
                        op,
                        d(&[("name", s(name)), ("hash", s(hash))]),
                        absent.clone(),
                    ));
                }
            }
        }
        for name in rsa {
            for (bits, exponent) in [
                (0.0, vec![1, 0, 1]),
                (3.0, vec![3]),
                (256.0, vec![1, 0, 1]),
                (504.0, vec![1, 0, 1]),
                (512.0, vec![3]),
                (1024.0, vec![1, 0, 1]),
                (1024.0, vec![1]),
                (1024.0, vec![0, 1, 0, 1]),
                (1032.0, vec![1, 0, 0]),
                (2049.0, vec![1, 0, 1]),
                (8200.0, vec![3]),
            ] {
                for hash in ["SHA-256", "MD5"] {
                    out.push((
                        "generateKey",
                        d(&[
                            ("name", s(name)),
                            ("modulusLength", n(bits)),
                            ("publicExponent", Value::Bytes(exponent.clone())),
                            ("hash", s(hash)),
                        ]),
                        absent.clone(),
                    ));
                }
            }
        }
        // The ciphers, for encrypt, decrypt, wrapKey and unwrapKey.
        let mut ciphers: Vec<Value> = Vec::new();
        for iv in [16, 0, 10, 32] {
            ciphers.push(d(&[("name", s("AES-CBC")), ("iv", b(iv))]));
        }
        for counter in [16, 8] {
            for length in [0.0, 1.0, 64.0, 128.0, 129.0] {
                ciphers.push(d(&[
                    ("name", s("AES-CTR")),
                    ("counter", b(counter)),
                    ("length", n(length)),
                ]));
            }
        }
        for iv in [12, 0, 1, 16] {
            for tag in [
                None,
                Some(32.0),
                Some(96.0),
                Some(128.0),
                Some(0.0),
                Some(129.0),
            ] {
                for aad in [false, true] {
                    let mut members = vec![("name", s("AES-GCM")), ("iv", b(iv))];
                    if let Some(tag) = tag {
                        members.push(("tagLength", n(tag)));
                    }
                    if aad {
                        members.push(("additionalData", b(3)));
                    }
                    ciphers.push(d(&members));
                }
            }
        }
        for iv in [12, 0, 16] {
            for tag in [None, Some(128.0), Some(96.0)] {
                let mut members = vec![("name", s("ChaCha20-Poly1305")), ("iv", b(iv))];
                if let Some(tag) = tag {
                    members.push(("tagLength", n(tag)));
                }
                ciphers.push(d(&members));
            }
        }
        ciphers.push(d(&[("name", s("RSA-OAEP"))]));
        ciphers.push(d(&[("name", s("RSA-OAEP")), ("label", b(0))]));
        ciphers.push(d(&[("name", s("RSA-OAEP")), ("label", b(5))]));
        ciphers.push(s("AES-KW"));
        for op in ["encrypt", "decrypt", "wrapKey", "unwrapKey"] {
            for cipher in &ciphers {
                out.push((op, cipher.clone(), absent.clone()));
            }
        }
        // The wrapped key's algorithm.
        let wrapped = [
            s("AES-GCM"),
            d(&[("name", s("AES-GCM")), ("length", n(256.0))]),
            s("HKDF"),
            s("PBKDF2"),
            d(&[("name", s("HMAC")), ("hash", s("SHA-256"))]),
            d(&[("name", s("ECDSA")), ("namedCurve", s("P-256"))]),
            d(&[("name", s("ECDSA")), ("namedCurve", s("P-51"))]),
            d(&[("name", s("RSA-OAEP")), ("hash", s("SHA-256"))]),
            s("Ed25519"),
            s("X25519"),
            s("nonsense"),
        ];
        for op in ["wrapKey", "unwrapKey"] {
            for cipher in [
                s("AES-KW"),
                d(&[("name", s("AES-GCM")), ("iv", b(12))]),
                d(&[("name", s("AES-GCM")), ("iv", b(0))]),
                d(&[("name", s("ChaCha20-Poly1305")), ("iv", b(12))]),
                d(&[("name", s("RSA-OAEP"))]),
            ] {
                for key in &wrapped {
                    out.push((op, cipher.clone(), key.clone()));
                }
            }
        }
        // sign and verify.
        for op in ["sign", "verify"] {
            for salt in [None, Some(0.0), Some(32.0), Some(222.0), Some(4294967295.0)] {
                let mut members = vec![("name", s("RSA-PSS"))];
                if let Some(salt) = salt {
                    members.push(("saltLength", n(salt)));
                }
                out.push((op, d(&members), absent.clone()));
            }
            for hash in hashes {
                out.push((
                    op,
                    d(&[("name", s("ECDSA")), ("hash", s(hash))]),
                    absent.clone(),
                ));
            }
            out.push((op, d(&[("name", s("ECDSA"))]), absent.clone()));
            for name in ["ML-DSA-44", "ML-DSA-65", "ML-DSA-87"] {
                for context in [0, 1, 255, 256] {
                    out.push((
                        op,
                        d(&[("name", s(name)), ("context", b(context))]),
                        absent.clone(),
                    ));
                }
            }
        }
        // deriveBits, its lengths.
        let lengths = |values: &[f64]| -> Vec<Value> {
            let mut out = vec![Value::Absent];
            out.extend(values.iter().map(|v| n(*v)));
            out
        };
        for hash in ["SHA-1", "SHA-256", "SHA-512", "MD5"] {
            let hkdf = d(&[
                ("name", s("HKDF")),
                ("hash", s(hash)),
                ("salt", b(0)),
                ("info", b(0)),
            ]);
            for length in lengths(&[
                0.0,
                7.0,
                8.0,
                256.0,
                40800.0,
                40808.0,
                65280.0,
                65288.0,
                130560.0,
                130568.0,
                4294967288.0,
            ]) {
                out.push(("deriveBits", hkdf.clone(), length));
            }
            for iterations in [0.0, 1.0] {
                let pbkdf2 = d(&[
                    ("name", s("PBKDF2")),
                    ("hash", s(hash)),
                    ("salt", b(4)),
                    ("iterations", n(iterations)),
                ]);
                for length in lengths(&[0.0, 7.0, 8.0, 256.0]) {
                    out.push(("deriveBits", pbkdf2.clone(), length));
                }
            }
        }
        for (_, public) in self.public_keys() {
            for name in ["ECDH", "X25519"] {
                let agreement = d(&[("name", s(name)), ("public", Value::Key(*public))]);
                for length in lengths(&[0.0, 8.0, 255.0, 256.0, 257.0, 384.0, 385.0, 528.0, 529.0])
                {
                    out.push(("deriveBits", agreement.clone(), length));
                }
            }
        }
        out.push(("deriveBits", d(&[("name", s("ECDH"))]), n(256.0)));
        out.push((
            "deriveBits",
            d(&[("name", s("ECDH")), ("public", Value::Key(u32::MAX))]),
            n(256.0),
        ));
        // deriveKey, the derived key's algorithm.
        let derived = [
            d(&[("name", s("AES-GCM")), ("length", n(128.0))]),
            d(&[("name", s("AES-GCM")), ("length", n(192.0))]),
            d(&[("name", s("AES-KW")), ("length", n(256.0))]),
            d(&[("name", s("HMAC")), ("hash", s("SHA-256"))]),
            d(&[
                ("name", s("HMAC")),
                ("hash", s("SHA-256")),
                ("length", n(256.0)),
            ]),
            d(&[
                ("name", s("HMAC")),
                ("hash", s("SHA-256")),
                ("length", n(0.0)),
            ]),
            d(&[("name", s("HMAC")), ("hash", s("SHA-512"))]),
            s("HKDF"),
            s("ChaCha20-Poly1305"),
            s("nonsense"),
        ];
        let mut bases = vec![
            d(&[
                ("name", s("HKDF")),
                ("hash", s("SHA-256")),
                ("salt", b(0)),
                ("info", b(0)),
            ]),
            d(&[
                ("name", s("PBKDF2")),
                ("hash", s("SHA-256")),
                ("salt", b(4)),
                ("iterations", n(1.0)),
            ]),
        ];
        for (label, public) in self.public_keys() {
            if label.ends_with("public") {
                for name in ["ECDH", "X25519"] {
                    bases.push(d(&[("name", s(name)), ("public", Value::Key(*public))]));
                }
            }
        }
        for base in &bases {
            for key in &derived {
                out.push(("deriveKey", base.clone(), key.clone()));
            }
        }
        out
    }
}
