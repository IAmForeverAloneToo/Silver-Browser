//! The operations, by algorithm, each over aws-lc-rs, with the reference
//! browser's errors (Chrome 157, probed 2026-11-22; docs/design/
//! webcrypto.md).

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::{aead, digest, hkdf, hmac, pbkdf2, rand, rsa, signature};

use crate::jwk::{self, Jwk};
use crate::normalize::{Normalized, normalize};
use crate::{
    AlgorithmValue, Call, Error, JwkValue, Key, KeyInfo, KeyType, Keys, Material, Outcome, Usage,
    Value, usage_set,
};

/// A key's algorithm, with its parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeyAlgorithm {
    Aes {
        name: &'static str,
        length: u32,
    },
    Hmac {
        hash: &'static str,
        length: u32,
    },
    Kdf {
        name: &'static str,
    },
    Rsa {
        name: &'static str,
        hash: &'static str,
        modulus_length: u32,
        public_exponent: Vec<u8>,
    },
}

impl KeyAlgorithm {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            KeyAlgorithm::Aes { name, .. }
            | KeyAlgorithm::Kdf { name }
            | KeyAlgorithm::Rsa { name, .. } => name,
            KeyAlgorithm::Hmac { .. } => "HMAC",
        }
    }

    /// The `algorithm` attribute's members, in the reference's order.
    pub(crate) fn members(&self) -> Vec<(&'static str, AlgorithmValue)> {
        let name = ("name", AlgorithmValue::Str(self.name().to_owned()));
        match self {
            KeyAlgorithm::Aes { length, .. } => {
                vec![name, ("length", AlgorithmValue::Num(*length))]
            }
            KeyAlgorithm::Hmac { hash, length } => vec![
                name,
                ("length", AlgorithmValue::Num(*length)),
                ("hash", AlgorithmValue::Named((*hash).to_owned())),
            ],
            KeyAlgorithm::Kdf { .. } => vec![name],
            KeyAlgorithm::Rsa {
                hash,
                modulus_length,
                public_exponent,
                ..
            } => vec![
                name,
                ("hash", AlgorithmValue::Named((*hash).to_owned())),
                ("modulusLength", AlgorithmValue::Num(*modulus_length)),
                (
                    "publicExponent",
                    AlgorithmValue::Bytes(public_exponent.clone()),
                ),
            ],
        }
    }
}

fn digest_algorithm(name: &str) -> &'static digest::Algorithm {
    match name {
        "SHA-1" => &digest::SHA1_FOR_LEGACY_USE_ONLY,
        "SHA-384" => &digest::SHA384,
        "SHA-512" => &digest::SHA512,
        _ => &digest::SHA256,
    }
}

fn hmac_algorithm(hash: &str) -> hmac::Algorithm {
    match hash {
        "SHA-1" => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
        "SHA-384" => hmac::HMAC_SHA384,
        "SHA-512" => hmac::HMAC_SHA512,
        _ => hmac::HMAC_SHA256,
    }
}

/// A hash's block size in bits: an HMAC key's length when none is given.
fn block_bits(hash: &str) -> u32 {
    match hash {
        "SHA-384" | "SHA-512" => 1024,
        _ => 512,
    }
}

/// The usages an algorithm's key of `kind` may have.
fn allowed(name: &str, kind: KeyType) -> &'static [Usage] {
    use Usage::*;
    match (name, kind) {
        ("AES-GCM" | "AES-CBC" | "AES-CTR", _) => &[Encrypt, Decrypt, WrapKey, UnwrapKey],
        ("AES-KW", _) => &[WrapKey, UnwrapKey],
        ("HMAC", _) => &[Sign, Verify],
        ("HKDF" | "PBKDF2", _) => &[DeriveKey, DeriveBits],
        ("RSASSA-PKCS1-v1_5" | "RSA-PSS", KeyType::Public) => &[Verify],
        ("RSASSA-PKCS1-v1_5" | "RSA-PSS", _) => &[Sign],
        _ => &[],
    }
}

fn check_usages(name: &str, kind: KeyType, usages: &[Usage]) -> Result<(), Error> {
    let allowed = allowed(name, kind);
    if usages.iter().any(|u| !allowed.contains(u)) {
        return Err(Error::syntax(
            "Cannot create a key using the specified key usages.",
        ));
    }
    Ok(())
}

fn empty_usages(kind: KeyType, usages: &[Usage]) -> Result<(), Error> {
    if usages.is_empty() && matches!(kind, KeyType::Secret | KeyType::Private) {
        return Err(Error::syntax("Usages cannot be empty when creating a key."));
    }
    Ok(())
}

fn random(len: usize) -> Result<Vec<u8>, Error> {
    let mut bytes = vec![0u8; len];
    rand::fill(&mut bytes).map_err(|_| Error::operation(""))?;
    Ok(bytes)
}

/// The key `id` for an operation of algorithm `name` that wants `usage`.
fn key_for<'k>(keys: &'k Keys, id: u32, name: &str, usage: Usage) -> Result<&'k Key, Error> {
    let key = keys.get(id)?;
    if key.algorithm.name() != name {
        return Err(Error::invalid_access(
            "key.algorithm does not match that of operation",
        ));
    }
    if !key.usages.contains(&usage) {
        return Err(Error::invalid_access(
            "key.usages does not permit this operation",
        ));
    }
    Ok(key)
}

fn secret(key: &Key) -> &[u8] {
    match &key.material {
        Material::Secret(bytes) => bytes,
        Material::RsaPublic { .. } => &[],
    }
}

pub(crate) fn run(keys: &mut Keys, call: Call) -> Result<Outcome, Error> {
    match call {
        Call::Digest { algorithm, data } => {
            let alg = normalize("digest", &algorithm)?;
            let out = digest::digest(digest_algorithm(alg.name), &data);
            Ok(Outcome::Bytes(out.as_ref().to_vec()))
        }
        Call::Encrypt {
            algorithm,
            key,
            data,
        } => crypt(keys, &normalize("encrypt", &algorithm)?, key, &data, true),
        Call::Decrypt {
            algorithm,
            key,
            data,
        } => crypt(keys, &normalize("decrypt", &algorithm)?, key, &data, false),
        Call::Sign {
            algorithm,
            key,
            data,
        } => {
            let alg = normalize("sign", &algorithm)?;
            let key = key_for(keys, key, alg.name, Usage::Sign)?;
            match &key.algorithm {
                KeyAlgorithm::Hmac { hash, .. } => {
                    let k = hmac::Key::new(hmac_algorithm(hash), secret(key));
                    Ok(Outcome::Bytes(hmac::sign(&k, &data).as_ref().to_vec()))
                }
                _ => Err(Error::not_supported("Unsupported operation")),
            }
        }
        Call::Verify {
            algorithm,
            key,
            signature,
            data,
        } => {
            let alg = normalize("verify", &algorithm)?;
            let key = key_for(keys, key, alg.name, Usage::Verify)?;
            Ok(Outcome::Bool(verify(key, &signature, &data)))
        }
        Call::GenerateKey {
            algorithm,
            extractable,
            usages,
        } => {
            let alg = normalize("generateKey", &algorithm)?;
            generate(keys, &alg, extractable, &usage_set(&usages))
        }
        Call::ImportKey {
            format,
            key_data,
            algorithm,
            extractable,
            usages,
        } => {
            let alg = normalize("importKey", &algorithm)?;
            let info = import(
                keys,
                &format,
                &key_data,
                &alg,
                extractable,
                &usage_set(&usages),
            )?;
            Ok(Outcome::Key(info))
        }
        Call::ExportKey { format, key } => export(keys.get(key)?, &format),
        Call::DeriveBits {
            algorithm,
            key,
            length,
        } => {
            let alg = normalize("deriveBits", &algorithm)?;
            let key = key_for(keys, key, alg.name, Usage::DeriveBits)?;
            Ok(Outcome::Bytes(derive_bits(key, &alg, length)?))
        }
        Call::DeriveKey {
            algorithm,
            key,
            derived,
            extractable,
            usages,
        } => {
            let alg = normalize("deriveBits", &algorithm)?;
            let derived_import = normalize("importKey", &derived)?;
            let length = key_length(&normalize("get key length", &derived)?)?;
            let bits = {
                let key = key_for(keys, key, alg.name, Usage::DeriveKey)?;
                derive_bits(key, &alg, length)?
            };
            let info = import(
                keys,
                "raw",
                &Value::Bytes(bits),
                &derived_import,
                extractable,
                &usage_set(&usages),
            )?;
            Ok(Outcome::Key(info))
        }
    }
}

/// A derived key type's length in bits ("get key length"), or `None`
/// for a KDF's, which takes none.
fn key_length(alg: &Normalized) -> Result<Option<u32>, Error> {
    match alg.name {
        "AES-GCM" | "AES-CBC" | "AES-CTR" | "AES-KW" => {
            let length = alg.num("length").unwrap_or(0);
            aes_length(length)?;
            Ok(Some(length))
        }
        "HMAC" => {
            let hash = alg.hash().unwrap_or("SHA-256");
            match alg.num("length") {
                Some(0) => Err(Error::operation("HMAC key length must not be zero")),
                Some(n) => Ok(Some(n)),
                None => Ok(Some(block_bits(hash))),
            }
        }
        _ => Ok(None),
    }
}

fn aes_length(bits: u32) -> Result<(), Error> {
    match bits {
        128 | 256 => Ok(()),
        192 => Err(Error::operation("192-bit AES keys are not supported")),
        _ => Err(Error::operation("AES key length must be 128 or 256 bits")),
    }
}

fn crypt(
    keys: &Keys,
    alg: &Normalized,
    key: u32,
    data: &[u8],
    encrypt: bool,
) -> Result<Outcome, Error> {
    let usage = if encrypt {
        Usage::Encrypt
    } else {
        Usage::Decrypt
    };
    let key = key_for(keys, key, alg.name, usage)?;
    match alg.name {
        "AES-GCM" => {
            let iv = alg.bytes("iv").unwrap_or(&[]);
            let aad = alg.bytes("additionalData").unwrap_or(&[]);
            let tag = alg.num("tagLength").unwrap_or(128);
            if !matches!(tag, 32 | 64 | 96 | 104 | 112 | 120 | 128) {
                return Err(Error::operation(
                    "The tag length is invalid: Must be 32, 64, 96, 104, 112, 120, or 128 bits",
                ));
            }
            if iv.is_empty() {
                return Err(Error::operation(""));
            }
            // aws-lc-rs's AEAD takes a 96-bit IV and a 128-bit tag alone
            // (docs/design/webcrypto.md, section 2b).
            let nonce: [u8; 12] = iv
                .try_into()
                .map_err(|_| Error::not_supported("AES-GCM: an IV of 96 bits alone"))?;
            if tag != 128 {
                return Err(Error::not_supported("AES-GCM: a tag of 128 bits alone"));
            }
            let algorithm = if secret(key).len() == 16 {
                &aead::AES_128_GCM
            } else {
                &aead::AES_256_GCM
            };
            let unbound =
                aead::UnboundKey::new(algorithm, secret(key)).map_err(|_| Error::operation(""))?;
            let sealing = aead::LessSafeKey::new(unbound);
            let nonce = aead::Nonce::assume_unique_for_key(nonce);
            let mut buffer = data.to_vec();
            if encrypt {
                sealing
                    .seal_in_place_append_tag(nonce, aead::Aad::from(aad), &mut buffer)
                    .map_err(|_| Error::operation(""))?;
                Ok(Outcome::Bytes(buffer))
            } else {
                let plain = sealing
                    .open_in_place(nonce, aead::Aad::from(aad), &mut buffer)
                    .map_err(|_| Error::operation(""))?;
                Ok(Outcome::Bytes(plain.to_vec()))
            }
        }
        _ => Err(Error::not_supported("Unsupported operation")),
    }
}

fn verify(key: &Key, signature: &[u8], data: &[u8]) -> bool {
    match (&key.algorithm, &key.material) {
        (KeyAlgorithm::Hmac { hash, .. }, Material::Secret(bytes)) => {
            let k = hmac::Key::new(hmac_algorithm(hash), bytes);
            hmac::verify(&k, data, signature).is_ok()
        }
        (
            KeyAlgorithm::Rsa {
                name: "RSASSA-PKCS1-v1_5",
                hash,
                ..
            },
            Material::RsaPublic { n, e },
        ) => {
            let parameters: &signature::RsaParameters = match *hash {
                "SHA-1" => &signature::RSA_PKCS1_1024_8192_SHA1_FOR_LEGACY_USE_ONLY,
                "SHA-384" => &signature::RSA_PKCS1_2048_8192_SHA384,
                "SHA-512" => &signature::RSA_PKCS1_1024_8192_SHA512_FOR_LEGACY_USE_ONLY,
                _ => &signature::RSA_PKCS1_1024_8192_SHA256_FOR_LEGACY_USE_ONLY,
            };
            let components = signature::RsaPublicKeyComponents { n, e };
            components.verify(parameters, data, signature).is_ok()
        }
        _ => false,
    }
}

fn generate(
    keys: &mut Keys,
    alg: &Normalized,
    extractable: bool,
    usages: &[Usage],
) -> Result<Outcome, Error> {
    match alg.name {
        "AES-GCM" | "AES-CBC" | "AES-CTR" | "AES-KW" => {
            check_usages(alg.name, KeyType::Secret, usages)?;
            let length = alg.num("length").unwrap_or(0);
            aes_length(length)?;
            empty_usages(KeyType::Secret, usages)?;
            let key = Key {
                kind: KeyType::Secret,
                extractable,
                algorithm: KeyAlgorithm::Aes {
                    name: alg.name,
                    length,
                },
                usages: usages.to_vec(),
                material: Material::Secret(random(length as usize / 8)?),
            };
            Ok(Outcome::Key(keys.insert(key)))
        }
        "HMAC" => {
            check_usages("HMAC", KeyType::Secret, usages)?;
            let hash = alg.hash().unwrap_or("SHA-256");
            let length = match alg.num("length") {
                Some(0) => return Err(Error::operation("HMAC key length must not be zero")),
                Some(n) => n,
                None => block_bits(hash),
            };
            empty_usages(KeyType::Secret, usages)?;
            let mut bytes = random(length.div_ceil(8) as usize)?;
            if length % 8 != 0
                && let Some(last) = bytes.last_mut()
            {
                *last &= 0xffu8 << (8 - length % 8);
            }
            let key = Key {
                kind: KeyType::Secret,
                extractable,
                algorithm: KeyAlgorithm::Hmac { hash, length },
                usages: usages.to_vec(),
                material: Material::Secret(bytes),
            };
            Ok(Outcome::Key(keys.insert(key)))
        }
        _ => Err(Error::not_supported("Unsupported operation")),
    }
}

fn import(
    keys: &mut Keys,
    format: &str,
    data: &Value,
    alg: &Normalized,
    extractable: bool,
    usages: &[Usage],
) -> Result<KeyInfo, Error> {
    let raw = || match data {
        Value::Bytes(bytes) => Ok(bytes.clone()),
        _ => Err(Error::type_error(
            "Key data must be a BufferSource for non-JWK formats",
        )),
    };
    let jwk = || match data {
        Value::Dict(_) => Ok(Jwk(data)),
        _ => Err(Error::type_error(
            "Key data must be an object for JWK import",
        )),
    };
    let secret_key = |algorithm: KeyAlgorithm, bytes: Vec<u8>| Key {
        kind: KeyType::Secret,
        extractable,
        algorithm,
        usages: usages.to_vec(),
        material: Material::Secret(bytes),
    };
    match alg.name {
        "AES-GCM" | "AES-CBC" | "AES-CTR" | "AES-KW" => {
            check_usages(alg.name, KeyType::Secret, usages)?;
            let bytes = match format {
                "raw" | "raw-secret" => raw()?,
                "jwk" => {
                    let jwk = jwk()?;
                    jwk.check("oct", None, extractable, usages)?;
                    let bytes = jwk.bytes("k")?;
                    let suffix = &alg.name[4..];
                    let suffix = if suffix == "KW" {
                        "KW".to_owned()
                    } else {
                        suffix.to_owned()
                    };
                    if let Some(given) = jwk.str("alg") {
                        let want = format!("A{}{suffix}", bytes.len() * 8);
                        if given != want {
                            return Err(Error::data(
                                "The JWK \"k\" member did not include the right length of key data for the given algorithm.",
                            ));
                        }
                    }
                    bytes
                }
                _ => {
                    return Err(Error::not_supported(
                        "Unsupported import key format for algorithm",
                    ));
                }
            };
            match bytes.len() {
                16 | 32 => {}
                24 => return Err(Error::operation("192-bit AES keys are not supported")),
                _ => return Err(Error::data("AES key data must be 128 or 256 bits")),
            }
            empty_usages(KeyType::Secret, usages)?;
            let length = bytes.len() as u32 * 8;
            Ok(keys.insert(secret_key(
                KeyAlgorithm::Aes {
                    name: alg.name,
                    length,
                },
                bytes,
            )))
        }
        "HMAC" => {
            check_usages("HMAC", KeyType::Secret, usages)?;
            let hash = alg.hash().unwrap_or("SHA-256");
            let bytes = match format {
                "raw" | "raw-secret" => raw()?,
                "jwk" => {
                    let jwk = jwk()?;
                    let want = format!("HS{}", &hash[4..]);
                    let want = if hash == "SHA-1" {
                        "HS1".to_owned()
                    } else {
                        want
                    };
                    jwk.check("oct", Some(&want), extractable, usages)?;
                    jwk.bytes("k")?
                }
                _ => {
                    return Err(Error::not_supported(
                        "Unsupported import key format for algorithm",
                    ));
                }
            };
            if bytes.is_empty() {
                return Err(Error::data("HMAC key data must not be empty"));
            }
            let length = alg.num("length").unwrap_or(bytes.len() as u32 * 8);
            empty_usages(KeyType::Secret, usages)?;
            Ok(keys.insert(secret_key(KeyAlgorithm::Hmac { hash, length }, bytes)))
        }
        "HKDF" | "PBKDF2" => {
            check_usages(alg.name, KeyType::Secret, usages)?;
            if format != "raw" {
                return Err(Error::not_supported(
                    "Unsupported import key format for algorithm",
                ));
            }
            if extractable {
                return Err(Error::syntax("KDF keys must set extractable=false"));
            }
            let bytes = raw()?;
            empty_usages(KeyType::Secret, usages)?;
            Ok(keys.insert(secret_key(KeyAlgorithm::Kdf { name: alg.name }, bytes)))
        }
        "RSASSA-PKCS1-v1_5" => {
            let hash = alg.hash().unwrap_or("SHA-256");
            let (n, e) = match format {
                // An SPKI's modulus and exponent come with the aws-lc
                // module of the next push (docs/design/webcrypto.md,
                // section 7): aws-lc-rs's getters need its `ring-io`,
                // whose `untrusted` would be a second version in the tree.
                "spki" => {
                    return Err(Error::not_supported(
                        "RSASSA-PKCS1-v1_5: an SPKI's import comes with the next push",
                    ));
                }
                "jwk" => {
                    let jwk = jwk()?;
                    let want = match hash {
                        "SHA-1" => "RS1",
                        "SHA-384" => "RS384",
                        "SHA-512" => "RS512",
                        _ => "RS256",
                    };
                    jwk.check("RSA", Some(want), extractable, usages)?;
                    if jwk.str("d").is_some() {
                        return Err(Error::not_supported(
                            "RSASSA-PKCS1-v1_5: a private key's import comes later",
                        ));
                    }
                    let n = strip_zeros(jwk.bytes("n")?);
                    let e = strip_zeros(jwk.bytes("e")?);
                    (n, e)
                }
                _ => {
                    return Err(Error::not_supported(
                        "Unsupported import key format for algorithm",
                    ));
                }
            };
            check_usages(alg.name, KeyType::Public, usages)?;
            if n.is_empty() || e.is_empty() {
                return Err(Error::data("The RSA key is not valid"));
            }
            let key = Key {
                kind: KeyType::Public,
                extractable: true,
                algorithm: KeyAlgorithm::Rsa {
                    name: alg.name,
                    hash,
                    modulus_length: bit_length(&n),
                    public_exponent: e.clone(),
                },
                usages: usages.to_vec(),
                material: Material::RsaPublic { n, e },
            };
            // A public key is extractable whatever the call asked.
            let _ = extractable;
            Ok(keys.insert(key))
        }
        _ => Err(Error::not_supported("Unsupported operation")),
    }
}

fn strip_zeros(mut bytes: Vec<u8>) -> Vec<u8> {
    let zeros = bytes.iter().take_while(|&&b| b == 0).count();
    bytes.drain(..zeros);
    bytes
}

fn bit_length(n: &[u8]) -> u32 {
    match n.first() {
        Some(&first) => (n.len() as u32 - 1) * 8 + (8 - first.leading_zeros()),
        None => 0,
    }
}

/// An RSA public key's PKCS #1 `RSAPublicKey` (RFC 8017 appendix A.1.1),
/// for aws-lc-rs to read and write as SPKI: written here, never read.
fn rsa_public_key_der(n: &[u8], e: &[u8]) -> Vec<u8> {
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
    fn integer(out: &mut Vec<u8>, value: &[u8]) {
        out.push(0x02);
        let pad = value.first().is_some_and(|&b| b & 0x80 != 0);
        length(out, value.len() + usize::from(pad));
        if pad {
            out.push(0);
        }
        out.extend_from_slice(value);
    }
    let mut body = Vec::new();
    integer(&mut body, n);
    integer(&mut body, e);
    let mut out = vec![0x30];
    length(&mut out, body.len());
    out.extend_from_slice(&body);
    out
}

fn export(key: &Key, format: &str) -> Result<Outcome, Error> {
    if !key.extractable {
        return Err(Error::invalid_access("key is not extractable").of_method());
    }
    let unsupported = || Error::not_supported("Unsupported export key format for algorithm");
    match (&key.algorithm, &key.material) {
        (KeyAlgorithm::Aes { name, length }, Material::Secret(bytes)) => match format {
            "raw" | "raw-secret" => Ok(Outcome::Bytes(bytes.clone())),
            "jwk" => {
                let suffix = &name[4..];
                Ok(Outcome::Jwk(jwk::exported(
                    vec![
                        ("alg", JwkValue::Str(format!("A{length}{suffix}"))),
                        ("k", JwkValue::Str(jwk::encode(bytes))),
                        ("kty", JwkValue::Str("oct".into())),
                    ],
                    key.extractable,
                    &key.usages,
                )))
            }
            _ => Err(unsupported()),
        },
        (KeyAlgorithm::Hmac { hash, .. }, Material::Secret(bytes)) => match format {
            "raw" | "raw-secret" => Ok(Outcome::Bytes(bytes.clone())),
            "jwk" => {
                let alg = if *hash == "SHA-1" {
                    "HS1".to_owned()
                } else {
                    format!("HS{}", &hash[4..])
                };
                Ok(Outcome::Jwk(jwk::exported(
                    vec![
                        ("alg", JwkValue::Str(alg)),
                        ("k", JwkValue::Str(jwk::encode(bytes))),
                        ("kty", JwkValue::Str("oct".into())),
                    ],
                    key.extractable,
                    &key.usages,
                )))
            }
            _ => Err(unsupported()),
        },
        (KeyAlgorithm::Rsa { hash, .. }, Material::RsaPublic { n, e }) => match format {
            "spki" => {
                let key = rsa::PublicKey::from_der(&rsa_public_key_der(n, e))
                    .map_err(|_| Error::operation(""))?;
                let der = key.as_der().map_err(|_| Error::operation(""))?;
                Ok(Outcome::Bytes(der.as_ref().to_vec()))
            }
            "jwk" => {
                let alg = match *hash {
                    "SHA-1" => "RS1",
                    "SHA-384" => "RS384",
                    "SHA-512" => "RS512",
                    _ => "RS256",
                };
                Ok(Outcome::Jwk(jwk::exported(
                    vec![
                        ("alg", JwkValue::Str(alg.into())),
                        ("e", JwkValue::Str(jwk::encode(e))),
                        ("kty", JwkValue::Str("RSA".into())),
                        ("n", JwkValue::Str(jwk::encode(n))),
                    ],
                    key.extractable,
                    &key.usages,
                )))
            }
            _ => Err(unsupported()),
        },
        _ => Err(unsupported()),
    }
}

fn derive_bits(key: &Key, alg: &Normalized, length: Option<u32>) -> Result<Vec<u8>, Error> {
    let secret = secret(key);
    match alg.name {
        "HKDF" => {
            let length = length.ok_or_else(|| {
                Error::operation("No length was specified for the HKDF Derive Bits operation.")
            })?;
            if length % 8 != 0 {
                return Err(Error::operation(
                    "The length provided for HKDF is not a multiple of 8 bits.",
                ));
            }
            let algorithm = match alg.hash().unwrap_or("SHA-256") {
                "SHA-1" => hkdf::HKDF_SHA1_FOR_LEGACY_USE_ONLY,
                "SHA-384" => hkdf::HKDF_SHA384,
                "SHA-512" => hkdf::HKDF_SHA512,
                _ => hkdf::HKDF_SHA256,
            };
            struct Len(usize);
            impl hkdf::KeyType for Len {
                fn len(&self) -> usize {
                    self.0
                }
            }
            let bytes = length as usize / 8;
            let salt = hkdf::Salt::new(algorithm, alg.bytes("salt").unwrap_or(&[]));
            let prk = salt.extract(secret);
            let info = [alg.bytes("info").unwrap_or(&[])];
            let mut out = vec![0u8; bytes];
            prk.expand(&info, Len(bytes))
                .and_then(|okm| okm.fill(&mut out))
                .map_err(|_| Error::operation("The length provided for HKDF is too large."))?;
            Ok(out)
        }
        "PBKDF2" => {
            let length = length.ok_or_else(|| {
                Error::operation("No length was specified for the PBKDF2 Derive Bits operation.")
            })?;
            if length % 8 != 0 {
                return Err(Error::operation(
                    "The length provided for PBKDF2 is not a multiple of 8 bits.",
                ));
            }
            let iterations = std::num::NonZeroU32::new(alg.num("iterations").unwrap_or(0))
                .ok_or_else(|| Error::operation("PBKDF2 requires iterations > 0"))?;
            let algorithm = match alg.hash().unwrap_or("SHA-256") {
                "SHA-1" => pbkdf2::PBKDF2_HMAC_SHA1,
                "SHA-384" => pbkdf2::PBKDF2_HMAC_SHA384,
                "SHA-512" => pbkdf2::PBKDF2_HMAC_SHA512,
                _ => pbkdf2::PBKDF2_HMAC_SHA256,
            };
            let mut out = vec![0u8; length as usize / 8];
            pbkdf2::derive(
                algorithm,
                iterations,
                alg.bytes("salt").unwrap_or(&[]),
                secret,
                &mut out,
            );
            Ok(out)
        }
        _ => Err(Error::not_supported("Unsupported operation")),
    }
}
