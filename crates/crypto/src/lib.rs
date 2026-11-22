//! Web Crypto (G255, docs/design/webcrypto.md): the `SubtleCrypto`
//! operations and the keys they work on, engine-neutral, every primitive
//! aws-lc's (no cryptography written here): through aws-lc-rs's safe
//! API, or, where it has none, the C library's own calls (`native`).
//!
//! A realm's keys live in its [`Keys`], by number; script holds the
//! number alone, never a key's material, and an export or a wrap asks
//! the store, which checks `extractable` itself. The bindings convert the
//! arguments as Web IDL says, reading an algorithm's members in the
//! order [`members`] gives (the reference browser's), and hand them over
//! as [`Value`]s; [`run`] normalizes the algorithm, with the reference's
//! errors, and does the operation.

// Unsafe is denied everywhere but `native`, the one module of calls into
// aws-lc's C library (the lead's ruling for G255).
#![deny(unsafe_code)]

mod algorithms;
mod jwk;
mod native;
mod normalize;

use std::collections::HashMap;

pub use normalize::{Member, MemberKind, members};

/// A Web Crypto error: the `DOMException`'s name (or `TypeError`) and
/// message, the reference browser's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub name: &'static str,
    pub message: String,
    /// Whether the message is the method's (`Failed to execute ...`), as
    /// the reference words the arguments' errors and a few others.
    pub of_method: bool,
}

impl Error {
    pub(crate) fn new(name: &'static str, message: impl Into<String>) -> Error {
        Error {
            name,
            message: message.into(),
            of_method: false,
        }
    }

    /// An error the reference words as the method's.
    pub(crate) fn of_method(mut self) -> Error {
        self.of_method = true;
        self
    }

    pub(crate) fn type_error(message: impl Into<String>) -> Error {
        Error::new("TypeError", message).of_method()
    }

    pub(crate) fn operation(message: impl Into<String>) -> Error {
        Error::new("OperationError", message)
    }

    pub(crate) fn data(message: impl Into<String>) -> Error {
        Error::new("DataError", message)
    }

    pub(crate) fn syntax(message: impl Into<String>) -> Error {
        Error::new("SyntaxError", message)
    }

    pub(crate) fn invalid_access(message: impl Into<String>) -> Error {
        Error::new("InvalidAccessError", message)
    }

    pub(crate) fn not_supported(message: impl Into<String>) -> Error {
        Error::new("NotSupportedError", message)
    }
}

/// An argument as the bindings converted it.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Not given (`undefined`).
    Absent,
    Str(String),
    Num(f64),
    Bool(bool),
    /// A `BufferSource`'s bytes, copied.
    Bytes(Vec<u8>),
    /// A `CryptoKey`, by its number.
    Key(u32),
    /// A dictionary's members, as read, in order: an algorithm, a JWK.
    Dict(Vec<(String, Value)>),
    List(Vec<Value>),
    /// Of a type the member's kind does not take.
    Other,
}

impl Value {
    /// Member `name` of a dictionary, or `Absent`.
    pub fn get(&self, name: &str) -> &Value {
        match self {
            Value::Dict(members) => members
                .iter()
                .find(|(n, _)| n == name)
                .map_or(&Value::Absent, |(_, v)| v),
            _ => &Value::Absent,
        }
    }
}

/// A key's usages, in the order the reference lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Usage {
    Encrypt,
    Decrypt,
    Sign,
    Verify,
    DeriveKey,
    DeriveBits,
    WrapKey,
    UnwrapKey,
}

impl Usage {
    pub const ALL: [Usage; 8] = [
        Usage::Encrypt,
        Usage::Decrypt,
        Usage::Sign,
        Usage::Verify,
        Usage::DeriveKey,
        Usage::DeriveBits,
        Usage::WrapKey,
        Usage::UnwrapKey,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Usage::Encrypt => "encrypt",
            Usage::Decrypt => "decrypt",
            Usage::Sign => "sign",
            Usage::Verify => "verify",
            Usage::DeriveKey => "deriveKey",
            Usage::DeriveBits => "deriveBits",
            Usage::WrapKey => "wrapKey",
            Usage::UnwrapKey => "unwrapKey",
        }
    }

    pub fn parse(name: &str) -> Option<Usage> {
        Usage::ALL.into_iter().find(|u| u.name() == name)
    }
}

/// A set of usages, kept in the reference's order, each once.
pub(crate) fn usage_set(usages: &[Usage]) -> Vec<Usage> {
    let mut set: Vec<Usage> = usages.to_vec();
    set.sort();
    set.dedup();
    set
}

/// A key's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyType {
    Secret,
    Public,
    Private,
}

impl KeyType {
    pub fn name(self) -> &'static str {
        match self {
            KeyType::Secret => "secret",
            KeyType::Public => "public",
            KeyType::Private => "private",
        }
    }
}

/// A key's `algorithm` attribute's members, in the reference's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlgorithmValue {
    Str(String),
    Num(u32),
    Bytes(Vec<u8>),
    /// A nested algorithm (`hash`), its name.
    Named(String),
}

/// What script may know of a key: everything but its material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    pub id: u32,
    pub kind: KeyType,
    pub extractable: bool,
    pub algorithm: Vec<(&'static str, AlgorithmValue)>,
    pub usages: Vec<Usage>,
}

/// A JWK member's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JwkValue {
    Str(String),
    Bool(bool),
    List(Vec<String>),
}

/// An operation's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Bytes(Vec<u8>),
    Bool(bool),
    Key(KeyInfo),
    /// A generated key pair: its public key, then its private one.
    Pair(KeyInfo, KeyInfo),
    /// An exported JWK, its members in the reference's order.
    Jwk(Vec<(String, JwkValue)>),
}

/// A `SubtleCrypto` call, its arguments converted.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Digest {
        algorithm: Value,
        data: Vec<u8>,
    },
    Encrypt {
        algorithm: Value,
        key: u32,
        data: Vec<u8>,
    },
    Decrypt {
        algorithm: Value,
        key: u32,
        data: Vec<u8>,
    },
    Sign {
        algorithm: Value,
        key: u32,
        data: Vec<u8>,
    },
    Verify {
        algorithm: Value,
        key: u32,
        signature: Vec<u8>,
        data: Vec<u8>,
    },
    GenerateKey {
        algorithm: Value,
        extractable: bool,
        usages: Vec<Usage>,
    },
    ImportKey {
        format: String,
        /// The bytes, or a JWK's members.
        key_data: Value,
        algorithm: Value,
        extractable: bool,
        usages: Vec<Usage>,
    },
    ExportKey {
        format: String,
        key: u32,
    },
    DeriveBits {
        algorithm: Value,
        key: u32,
        /// `null` is `None`.
        length: Option<u32>,
    },
    DeriveKey {
        algorithm: Value,
        key: u32,
        derived: Value,
        extractable: bool,
        usages: Vec<Usage>,
    },
}

impl Call {
    /// The method's name, for the messages.
    pub fn method(&self) -> &'static str {
        match self {
            Call::Digest { .. } => "digest",
            Call::Encrypt { .. } => "encrypt",
            Call::Decrypt { .. } => "decrypt",
            Call::Sign { .. } => "sign",
            Call::Verify { .. } => "verify",
            Call::GenerateKey { .. } => "generateKey",
            Call::ImportKey { .. } => "importKey",
            Call::ExportKey { .. } => "exportKey",
            Call::DeriveBits { .. } => "deriveBits",
            Call::DeriveKey { .. } => "deriveKey",
        }
    }
}

/// A key's material, which never leaves the store but by an export the
/// key's `extractable` allows.
#[derive(Clone)]
pub(crate) enum Material {
    /// A symmetric key's bytes (AES, HMAC) or a KDF's secret.
    Secret(Vec<u8>),
    /// An RSA public key: its modulus and exponent, big-endian.
    RsaPublic { n: Vec<u8>, e: Vec<u8> },
    /// An RSA private key's parts, overwritten as they go.
    RsaPrivate(Box<native::RsaPrivateParts>),
}

impl Drop for Material {
    /// A secret's bytes are overwritten as it goes.
    fn drop(&mut self) {
        if let Material::Secret(bytes) = self {
            bytes.iter_mut().for_each(|b| *b = 0);
            std::hint::black_box(&bytes);
        }
    }
}

impl std::fmt::Debug for Material {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never a key's bytes, in a log or a panic.
        f.write_str("Material(..)")
    }
}

/// A key in the store.
#[derive(Debug, Clone)]
pub(crate) struct Key {
    pub(crate) kind: KeyType,
    pub(crate) extractable: bool,
    pub(crate) algorithm: algorithms::KeyAlgorithm,
    pub(crate) usages: Vec<Usage>,
    pub(crate) material: Material,
}

/// A realm's keys, by number.
#[derive(Debug, Default)]
pub struct Keys {
    keys: HashMap<u32, Key>,
    next: u32,
}

impl Keys {
    pub(crate) fn insert(&mut self, key: Key) -> KeyInfo {
        self.next += 1;
        let id = self.next;
        let info = KeyInfo {
            id,
            kind: key.kind,
            extractable: key.extractable,
            algorithm: key.algorithm.members(),
            usages: key.usages.clone(),
        };
        self.keys.insert(id, key);
        info
    }

    pub(crate) fn get(&self, id: u32) -> Result<&Key, Error> {
        self.keys
            .get(&id)
            .ok_or_else(|| Error::invalid_access("The key is not of this realm."))
    }

    /// How many keys the store holds.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// Runs `call` against `keys`, with the reference browser's errors.
pub fn run(keys: &mut Keys, call: Call) -> Result<Outcome, Error> {
    let method = call.method();
    algorithms::run(keys, call).map_err(|mut error| {
        if error.of_method {
            error.message = format!(
                "Failed to execute '{method}' on 'SubtleCrypto': {}",
                error.message
            );
        }
        error
    })
}

/// The lead's addition to ruling (a): a signature the library made, with
/// a key of any size it makes, verifies true through the page's own
/// calls, on whichever route the key takes; its message changed, false.
/// `choice` picks the key and the hash. Never a silent false.
#[cfg(any(test, feature = "fuzzing"))]
fn rsa_signed_natively_verifies(choice: u8, message: &[u8]) {
    use native::{Hash, RsaPrivate};
    /// Each side of aws-lc-rs's ranges, both exponents.
    const SIZES: [(u32, u64); 9] = [
        (512, 65537),
        (520, 3),
        (1016, 65537),
        (1024, 3),
        (1032, 65537),
        (2040, 3),
        (2048, 65537),
        (2056, 3),
        (3072, 65537),
    ];
    const HASHES: [(&str, Hash); 4] = [
        ("SHA-1", Hash::Sha1),
        ("SHA-256", Hash::Sha256),
        ("SHA-384", Hash::Sha384),
        ("SHA-512", Hash::Sha512),
    ];
    thread_local! {
        static POOL: Vec<RsaPrivate> = SIZES
            .iter()
            .map(|&(bits, e)| native::rsa_generate(bits, e).expect("the library makes the key"))
            .collect();
    }
    let (name, hash) = HASHES[usize::from(choice) % HASHES.len()];
    let at = usize::from(choice / 4) % SIZES.len();
    let digest = aws_lc_rs::digest::digest(algorithms::digest_algorithm(name), message);
    let Some((signature, parts)) = POOL.with(|pool| {
        let key = &pool[at];
        native::rsa_sign_pkcs1(key, hash, digest.as_ref())
            .ok()
            .map(|signature| (signature, key.parts()))
    }) else {
        // The digest does not fit the key: nothing was signed.
        return;
    };
    let mut keys = Keys::default();
    let member = |b: &[u8]| Value::Str(jwk::encode(b));
    let imported = run(
        &mut keys,
        Call::ImportKey {
            format: "jwk".into(),
            key_data: Value::Dict(vec![
                ("kty".into(), Value::Str("RSA".into())),
                ("n".into(), member(&parts.n)),
                ("e".into(), member(&parts.e)),
            ]),
            algorithm: Value::Dict(vec![
                ("name".into(), Value::Str("RSASSA-PKCS1-v1_5".into())),
                ("hash".into(), Value::Str(name.into())),
            ]),
            extractable: true,
            usages: vec![Usage::Verify],
        },
    );
    let Ok(Outcome::Key(key)) = imported else {
        panic!("a key the library made imports: {imported:?}");
    };
    let verify = |keys: &mut Keys, data: Vec<u8>| {
        run(
            keys,
            Call::Verify {
                algorithm: Value::Str("RSASSA-PKCS1-v1_5".into()),
                key: key.id,
                signature: signature.clone(),
                data,
            },
        )
    };
    let bits = SIZES[at].0;
    assert_eq!(
        verify(&mut keys, message.to_vec()),
        Ok(Outcome::Bool(true)),
        "{bits} bits, {name}: the library's own signature"
    );
    let mut other = message.to_vec();
    other.push(0);
    assert_eq!(
        verify(&mut keys, other),
        Ok(Outcome::Bool(false)),
        "{bits} bits, {name}: another message"
    );
    // The private key through the page's calls, on its signing route: the
    // library's own bytes (PKCS #1 v1.5's signatures are deterministic),
    // or, where the digest does not fit, the same refusal.
    let private = run(
        &mut keys,
        Call::ImportKey {
            format: "jwk".into(),
            key_data: Value::Dict(
                [
                    ("kty", Value::Str("RSA".into())),
                    ("n", member(&parts.n)),
                    ("e", member(&parts.e)),
                    ("d", member(&parts.d)),
                    ("p", member(&parts.p)),
                    ("q", member(&parts.q)),
                    ("dp", member(&parts.dp)),
                    ("dq", member(&parts.dq)),
                    ("qi", member(&parts.qi)),
                ]
                .into_iter()
                .map(|(n, v)| (n.to_owned(), v))
                .collect(),
            ),
            algorithm: Value::Dict(vec![
                ("name".into(), Value::Str("RSASSA-PKCS1-v1_5".into())),
                ("hash".into(), Value::Str(name.into())),
            ]),
            extractable: false,
            usages: vec![Usage::Sign],
        },
    );
    let Ok(Outcome::Key(private)) = private else {
        panic!("{bits} bits: the library's private key imports: {private:?}");
    };
    let signed = run(
        &mut keys,
        Call::Sign {
            algorithm: Value::Str("RSASSA-PKCS1-v1_5".into()),
            key: private.id,
            data: message.to_vec(),
        },
    );
    assert_eq!(
        signed,
        Ok(Outcome::Bytes(signature)),
        "{bits} bits, {name}: the page's signature is the library's"
    );
}

/// `native`'s AES-GCM, for the fuzz target that holds it to aws-lc-rs's
/// (`fuzz_targets/crypto_gcm.rs`), and the property that a signature the
/// library made verifies: in no build but the fuzz crate's.
#[cfg(feature = "fuzzing")]
pub mod fuzzing {
    /// A signature the library made verifies true, another message false,
    /// with keys each side of aws-lc-rs's ranges (`choice` picks the key
    /// and the hash); it panics otherwise.
    pub fn rsa_signed_natively_verifies(choice: u8, message: &[u8]) {
        crate::rsa_signed_natively_verifies(choice, message);
    }

    /// AES-GCM's encryption through the library's own calls: the
    /// ciphertext and its tag, or `None` for what it refuses.
    pub fn gcm_seal(
        key: &[u8],
        iv: &[u8],
        aad: &[u8],
        plain: &[u8],
        tag: usize,
    ) -> Option<Vec<u8>> {
        crate::native::gcm_seal(key, iv, aad, plain, tag).ok()
    }

    /// Its decryption, the tag checked by the library.
    pub fn gcm_open(
        key: &[u8],
        iv: &[u8],
        aad: &[u8],
        sealed: &[u8],
        tag: usize,
    ) -> Option<Vec<u8>> {
        crate::native::gcm_open(key, iv, aad, sealed, tag).ok()
    }
}

#[cfg(test)]
mod tests;
