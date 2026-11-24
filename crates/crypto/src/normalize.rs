//! Web Crypto's "normalize an algorithm" (section 18.4), as the reference
//! browser does it: the names it recognizes (in any case), the members
//! each operation's dictionary has, read in the reference's order (not
//! Web IDL's alphabetical one: probed, `name, iv, additionalData,
//! tagLength`), and its messages (`AeadParams: iv: Missing required
//! property`).

use crate::algorithms::CHACHA;
use crate::{Error, Value};

/// The algorithms the reference recognizes, by their canonical names.
pub(crate) const NAMES: &[&str] = &[
    "AES-CBC",
    "AES-CTR",
    "AES-GCM",
    "AES-KW",
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
    "ChaCha20-Poly1305",
    "ML-KEM-768",
    "ML-KEM-1024",
    "ML-DSA-44",
    "ML-DSA-65",
    "ML-DSA-87",
];

/// What a member is converted as, by the bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    /// A `BufferSource`, copied.
    Buffer,
    /// A `BigInteger` (a `Uint8Array`), copied.
    BigInteger,
    /// A number, `ToNumber` of the value; the range is checked here.
    Number,
    /// A `DOMString`.
    Str,
    /// A nested `AlgorithmIdentifier`, read twice (the reference reads
    /// `hash` twice: probed).
    Algorithm,
    /// A `CryptoKey`.
    Key,
}

/// A member of an operation's dictionary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Member {
    pub name: &'static str,
    pub kind: MemberKind,
    /// The member's Web IDL range, for a number: its type's bound.
    pub(crate) max: f64,
    pub(crate) required: bool,
}

const fn req(name: &'static str, kind: MemberKind) -> Member {
    Member {
        name,
        kind,
        max: 0.0,
        required: true,
    }
}

const fn opt(name: &'static str, kind: MemberKind) -> Member {
    Member {
        name,
        kind,
        max: 0.0,
        required: false,
    }
}

const fn num(name: &'static str, max: f64, required: bool) -> Member {
    Member {
        name,
        kind: MemberKind::Number,
        max,
        required,
    }
}

const OCTET: f64 = 255.0;
const UNSIGNED_SHORT: f64 = 65535.0;
const UNSIGNED_LONG: f64 = 4_294_967_295.0;

use MemberKind::{Algorithm, BigInteger, Buffer, Key, Str};

/// The dictionary of operation `op` for algorithm `name` (canonical): its
/// name and members, in the reference's order; `None` when the algorithm
/// has no such operation.
fn dictionary(op: &str, name: &str) -> Option<(&'static str, &'static [Member])> {
    // wrapKey and unwrapKey take an algorithm's own, or else its encrypt's
    // and decrypt's (the specification's normalization, and Chrome's).
    let fallback = match op {
        "wrapKey" => "encrypt",
        "unwrapKey" => "decrypt",
        _ => return dictionary_of(op, name),
    };
    dictionary_of(op, name).or_else(|| dictionary_of(fallback, name))
}

fn dictionary_of(op: &str, name: &str) -> Option<(&'static str, &'static [Member])> {
    const NONE: &[Member] = &[];
    const AEAD: &[Member] = &[
        req("iv", Buffer),
        opt("additionalData", Buffer),
        num("tagLength", OCTET, false),
    ];
    const AES_GEN: &[Member] = &[num("length", UNSIGNED_SHORT, true)];
    const CONTEXT: &[Member] = &[opt("context", Buffer)];
    const HMAC_GEN: &[Member] = &[req("hash", Algorithm), num("length", UNSIGNED_LONG, false)];
    const HKDF: &[Member] = &[
        req("hash", Algorithm),
        req("salt", Buffer),
        req("info", Buffer),
    ];
    const PBKDF2: &[Member] = &[
        req("salt", Buffer),
        num("iterations", UNSIGNED_LONG, true),
        req("hash", Algorithm),
    ];
    const RSA_GEN: &[Member] = &[
        num("modulusLength", UNSIGNED_LONG, true),
        req("publicExponent", BigInteger),
        req("hash", Algorithm),
    ];
    const RSA_IMPORT: &[Member] = &[req("hash", Algorithm)];
    const RSA_PSS: &[Member] = &[num("saltLength", UNSIGNED_LONG, true)];
    const RSA_OAEP: &[Member] = &[opt("label", Buffer)];
    const AES_CBC: &[Member] = &[req("iv", Buffer)];
    const AES_CTR: &[Member] = &[req("counter", Buffer), num("length", OCTET, true)];
    const EC: &[Member] = &[req("namedCurve", Str)];
    const DERIVE: &[Member] = &[req("public", Key)];
    const ECDSA: &[Member] = &[req("hash", Algorithm)];
    let digest = matches!(name, "SHA-1" | "SHA-256" | "SHA-384" | "SHA-512");
    Some(match (op, name) {
        ("digest", _) if digest => ("Algorithm", NONE),
        ("generateKey" | "get key length", "AES-GCM" | "AES-CBC" | "AES-CTR" | "AES-KW") => {
            if op == "generateKey" {
                ("AesKeyGenParams", AES_GEN)
            } else {
                ("AesDerivedKeyParams", AES_GEN)
            }
        }
        ("importKey", "AES-GCM" | "AES-CBC" | "AES-CTR" | "AES-KW") => ("Algorithm", NONE),
        ("generateKey" | "importKey" | "get key length", CHACHA) => ("Algorithm", NONE),
        ("encrypt" | "decrypt", CHACHA) => ("AeadParams", AEAD),
        ("encrypt" | "decrypt", "AES-GCM") => ("AeadParams", AEAD),
        ("encrypt" | "decrypt", "RSA-OAEP") => ("RsaOaepParams", RSA_OAEP),
        ("encrypt" | "decrypt", "AES-CBC") => ("AesCbcParams", AES_CBC),
        ("encrypt" | "decrypt", "AES-CTR") => ("AesCtrParams", AES_CTR),
        ("wrapKey" | "unwrapKey", "AES-KW") => ("Algorithm", NONE),
        ("generateKey", "HMAC") => ("HmacKeyGenParams", HMAC_GEN),
        ("importKey" | "get key length", "HMAC") => ("HmacImportParams", HMAC_GEN),
        ("sign" | "verify", "HMAC" | "RSASSA-PKCS1-v1_5") => ("Algorithm", NONE),
        ("sign" | "verify", "RSA-PSS") => ("RsaPssParams", RSA_PSS),
        ("sign" | "verify", "ECDSA") => ("EcdsaParams", ECDSA),
        ("sign" | "verify" | "generateKey" | "importKey", "Ed25519") => ("Algorithm", NONE),
        ("generateKey" | "importKey", "ML-DSA-44" | "ML-DSA-65" | "ML-DSA-87") => {
            ("Algorithm", NONE)
        }
        ("sign" | "verify", "ML-DSA-44" | "ML-DSA-65" | "ML-DSA-87") => ("ContextParams", CONTEXT),
        ("generateKey" | "importKey", "X25519") => ("Algorithm", NONE),
        ("importKey", "HKDF" | "PBKDF2") => ("Algorithm", NONE),
        ("get key length", "HKDF" | "PBKDF2") => ("Algorithm", NONE),
        ("deriveBits", "HKDF") => ("HkdfParams", HKDF),
        ("deriveBits", "PBKDF2") => ("Pbkdf2Params", PBKDF2),
        ("generateKey", "RSASSA-PKCS1-v1_5" | "RSA-PSS" | "RSA-OAEP") => {
            ("RsaHashedKeyGenParams", RSA_GEN)
        }
        ("importKey", "RSASSA-PKCS1-v1_5" | "RSA-PSS" | "RSA-OAEP") => {
            ("RsaHashedImportParams", RSA_IMPORT)
        }
        ("generateKey" | "importKey", "ECDSA" | "ECDH") => {
            if op == "generateKey" {
                ("EcKeyGenParams", EC)
            } else {
                ("EcKeyImportParams", EC)
            }
        }
        ("deriveBits", "ECDH" | "X25519") => ("EcdhKeyDeriveParams", DERIVE),
        _ => return None,
    })
}

/// The canonical name of a recognized algorithm, matched in any case.
pub(crate) fn canonical(name: &str) -> Option<&'static str> {
    NAMES.iter().copied().find(|n| n.eq_ignore_ascii_case(name))
}

/// What the bindings read of an algorithm of name `name` for operation
/// `op` (a method's name, or `get key length` for a derived key's
/// type): its members, in order. `None` for a name not recognized, or
/// an operation it has none of: nothing more is read.
pub fn members(op: &str, name: &str) -> Option<&'static [Member]> {
    let name = canonical(name)?;
    dictionary(op, name).map(|(_, members)| members)
}

/// A normalized algorithm: its canonical name and members.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Normalized {
    pub(crate) name: &'static str,
    members: Vec<(&'static str, Param)>,
}

/// A normalized member.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Param {
    Bytes(Vec<u8>),
    Num(u32),
    Str(String),
    /// A digest's canonical name (`hash`).
    Hash(&'static str),
    Key(u32),
}

impl Normalized {
    fn get(&self, name: &str) -> Option<&Param> {
        self.members
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, p)| p)
    }

    pub(crate) fn key(&self, name: &str) -> Option<u32> {
        match self.get(name) {
            Some(Param::Key(k)) => Some(*k),
            _ => None,
        }
    }

    pub(crate) fn str(&self, name: &str) -> Option<&str> {
        match self.get(name) {
            Some(Param::Str(s)) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn bytes(&self, name: &str) -> Option<&[u8]> {
        match self.get(name) {
            Some(Param::Bytes(b)) => Some(b),
            _ => None,
        }
    }

    pub(crate) fn num(&self, name: &str) -> Option<u32> {
        match self.get(name) {
            Some(Param::Num(n)) => Some(*n),
            _ => None,
        }
    }

    pub(crate) fn hash(&self) -> Option<&'static str> {
        match self.get("hash") {
            Some(Param::Hash(h)) => Some(h),
            _ => None,
        }
    }
}

fn unrecognized(context: &str) -> Error {
    Error::not_supported(format!("{context}Algorithm: Unrecognized name")).of_method()
}

/// The name an algorithm identifier gives: a string's, or a dictionary's
/// `name` member (a TypeError when it has none).
fn name_of(algorithm: &Value, context: &str) -> Result<String, Error> {
    match algorithm {
        Value::Dict(_) => match algorithm.get("name") {
            Value::Str(name) => Ok(name.clone()),
            _ => Err(Error::type_error(format!(
                "{context}Algorithm: name: Missing or not a string"
            ))),
        },
        Value::Str(name) => Ok(name.clone()),
        // A number or null, converted to a string: no name the reference
        // knows.
        _ => Ok(String::new()),
    }
}

/// Normalizes `algorithm` for operation `op`.
pub(crate) fn normalize(op: &str, algorithm: &Value) -> Result<Normalized, Error> {
    normalize_in(op, algorithm, "")
}

fn normalize_in(op: &str, algorithm: &Value, context: &str) -> Result<Normalized, Error> {
    let name = name_of(algorithm, context)?;
    let name = canonical(&name).ok_or_else(|| unrecognized(context))?;
    let (dict, list) = dictionary(op, name).ok_or_else(|| {
        Error::not_supported(format!("{context}{name}: Unsupported operation: {op}")).of_method()
    })?;
    let mut members = Vec::new();
    for member in list {
        let value = algorithm.get(member.name);
        let at = format!("{context}{dict}: {}: ", member.name);
        let param = match (member.kind, value) {
            (_, Value::Absent) if !member.required => continue,
            (MemberKind::Buffer, Value::Bytes(b)) | (MemberKind::BigInteger, Value::Bytes(b)) => {
                Param::Bytes(b.clone())
            }
            (MemberKind::Buffer, Value::Absent) => {
                return Err(Error::type_error(format!("{at}Missing required property")));
            }
            (MemberKind::Buffer, _) => {
                return Err(Error::type_error(format!("{at}Not a BufferSource")));
            }
            (MemberKind::BigInteger, _) => {
                return Err(Error::type_error(format!(
                    "{at}Missing or not a Uint8Array"
                )));
            }
            (MemberKind::Number, Value::Num(n)) if n.is_nan() => {
                return Err(Error::type_error(format!("{at}Is not a number")));
            }
            (MemberKind::Number, Value::Num(n)) => {
                // Web IDL's [EnforceRange]: finite, truncated, in range.
                let n = n.trunc();
                if !n.is_finite() || n < 0.0 || n > member.max {
                    return Err(Error::type_error(format!("{at}Outside of numeric range")));
                }
                Param::Num(n as u32)
            }
            (MemberKind::Number, _) => {
                return Err(Error::type_error(format!("{at}Missing required property")));
            }
            (MemberKind::Str, Value::Str(s)) => Param::Str(s.clone()),
            (MemberKind::Str, _) => {
                return Err(Error::type_error(format!("{at}Missing or not a string")));
            }
            (MemberKind::Key, Value::Key(k)) => Param::Key(*k),
            // Absent, the reference's words for a required member (probed:
            // EcdhKeyDeriveParams' public).
            (MemberKind::Key, Value::Absent) => {
                return Err(Error::type_error(format!("{at}Missing required property")));
            }
            (MemberKind::Key, _) => {
                return Err(Error::type_error(format!("{at}Missing or not a CryptoKey")));
            }
            (MemberKind::Algorithm, Value::Str(_) | Value::Dict(_)) => {
                let context = format!("{context}{dict}: {}: ", member.name);
                Param::Hash(normalize_in("digest", value, &context)?.name)
            }
            (MemberKind::Algorithm, _) => {
                return Err(Error::type_error(format!(
                    "{at}Missing or not an AlgorithmIdentifier"
                )));
            }
        };
        members.push((member.name, param));
    }
    Ok(Normalized { name, members })
}
