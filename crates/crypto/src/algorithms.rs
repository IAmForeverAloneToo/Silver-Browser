//! The operations, by algorithm, each over aws-lc-rs, with the reference
//! browser's errors (Chrome 157, probed 2026-11-22; docs/design/
//! webcrypto.md).

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::{aead, digest, hkdf, hmac, kem, pbkdf2, rand, rsa, signature};

use crate::jwk::{self, Jwk};
use crate::native;
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
    Ec {
        name: &'static str,
        curve: &'static str,
    },
    Okp {
        name: &'static str,
    },
    /// ChaCha20-Poly1305's: a 256-bit key, its algorithm its name alone.
    Chacha,
    /// ML-DSA-44, -65 or -87's: its algorithm its name alone.
    MlDsa {
        name: &'static str,
    },
    /// ML-KEM-768 or -1024's: its algorithm its name alone.
    MlKem {
        name: &'static str,
    },
}

impl KeyAlgorithm {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            KeyAlgorithm::Aes { name, .. }
            | KeyAlgorithm::Kdf { name }
            | KeyAlgorithm::Rsa { name, .. }
            | KeyAlgorithm::Ec { name, .. }
            | KeyAlgorithm::Okp { name }
            | KeyAlgorithm::MlDsa { name }
            | KeyAlgorithm::MlKem { name } => name,
            KeyAlgorithm::Hmac { .. } => "HMAC",
            KeyAlgorithm::Chacha => CHACHA,
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
            KeyAlgorithm::Kdf { .. }
            | KeyAlgorithm::Okp { .. }
            | KeyAlgorithm::Chacha
            | KeyAlgorithm::MlDsa { .. }
            | KeyAlgorithm::MlKem { .. } => vec![name],
            KeyAlgorithm::Ec { curve, .. } => vec![
                name,
                ("namedCurve", AlgorithmValue::Str((*curve).to_owned())),
            ],
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

/// ChaCha20-Poly1305's name.
pub(crate) const CHACHA: &str = "ChaCha20-Poly1305";

pub(crate) fn digest_algorithm(name: &str) -> &'static digest::Algorithm {
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
        (CHACHA, _) => &[Encrypt, Decrypt, WrapKey, UnwrapKey],
        ("HMAC", _) => &[Sign, Verify],
        ("HKDF" | "PBKDF2", _) => &[DeriveKey, DeriveBits],
        ("RSASSA-PKCS1-v1_5" | "RSA-PSS", KeyType::Public) => &[Verify],
        ("RSASSA-PKCS1-v1_5" | "RSA-PSS", _) => &[Sign],
        ("ECDSA", KeyType::Public) => &[Verify],
        ("ECDSA", _) => &[Sign],
        ("ECDH" | "X25519", KeyType::Public) => &[],
        ("ECDH" | "X25519", _) => &[DeriveKey, DeriveBits],
        ("Ed25519" | "ML-DSA-44" | "ML-DSA-65" | "ML-DSA-87", KeyType::Public) => &[Verify],
        ("Ed25519" | "ML-DSA-44" | "ML-DSA-65" | "ML-DSA-87", _) => &[Sign],
        ("ML-KEM-768" | "ML-KEM-1024", KeyType::Public) => &[EncapsulateKey, EncapsulateBits],
        ("ML-KEM-768" | "ML-KEM-1024", KeyType::Private) => &[DecapsulateKey, DecapsulateBits],
        ("RSA-OAEP", KeyType::Public) => &[Encrypt, WrapKey],
        ("RSA-OAEP", _) => &[Decrypt, UnwrapKey],
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
        Material::RsaPublic { .. }
        | Material::RsaPrivate(_)
        | Material::EcPublic { .. }
        | Material::EcPrivate { .. }
        | Material::OkpPublic { .. }
        | Material::OkpPrivate { .. }
        | Material::PqPublic { .. }
        | Material::PqPrivate { .. } => &[],
    }
}

/// The range of key sizes aws-lc-rs's RSA key pairs and signing take
/// (its `KeyPair`: 2048 to 8192 bits); outside it, the library's calls.
const SAFE_PRIVATE: std::ops::RangeInclusive<u32> = 2048..=8192;

/// The hash's name as `native` names it.
fn native_hash(hash: &str) -> native::Hash {
    match hash {
        "SHA-1" => native::Hash::Sha1,
        "SHA-384" => native::Hash::Sha384,
        "SHA-512" => native::Hash::Sha512,
        _ => native::Hash::Sha256,
    }
}

/// The JWK `alg` of RSA algorithm `name` with `hash` (probed: RS1 to
/// RS512, PS1 to PS512, RSA-OAEP and RSA-OAEP-256 to RSA-OAEP-512).
fn rsa_jwk_alg(name: &str, hash: &str) -> &'static str {
    match (name, hash) {
        ("RSA-OAEP", "SHA-1") => "RSA-OAEP",
        ("RSA-OAEP", "SHA-384") => "RSA-OAEP-384",
        ("RSA-OAEP", "SHA-512") => "RSA-OAEP-512",
        ("RSA-OAEP", _) => "RSA-OAEP-256",
        ("RSA-PSS", "SHA-1") => "PS1",
        ("RSA-PSS", "SHA-384") => "PS384",
        ("RSA-PSS", "SHA-512") => "PS512",
        ("RSA-PSS", _) => "PS256",
        (_, "SHA-1") => "RS1",
        (_, "SHA-384") => "RS384",
        (_, "SHA-512") => "RS512",
        _ => "RS256",
    }
}

/// A digest's length in bytes.
fn digest_len(hash: &str) -> u32 {
    match hash {
        "SHA-1" => 20,
        "SHA-384" => 48,
        "SHA-512" => 64,
        _ => 32,
    }
}

/// RSA-PSS's route for a key of `bits` bits and a salt of `salt` bytes:
/// aws-lc-rs signs and verifies SHA-256, SHA-384 and SHA-512 at 2048 to
/// 8192 bits with a salt as long as the digest alone (its `RSA_PSS_*`,
/// `RSA_PSS_SALTLEN_DIGEST`); anything else, the library.
pub(crate) fn pss_route(
    hash: &str,
    bits: u32,
    salt: u32,
) -> Option<(
    &'static signature::RsaSignatureEncoding,
    &'static signature::RsaParameters,
)> {
    let pair: (
        &'static signature::RsaSignatureEncoding,
        &'static signature::RsaParameters,
    ) = match hash {
        "SHA-256" => (
            &signature::RSA_PSS_SHA256,
            &signature::RSA_PSS_2048_8192_SHA256,
        ),
        "SHA-384" => (
            &signature::RSA_PSS_SHA384,
            &signature::RSA_PSS_2048_8192_SHA384,
        ),
        "SHA-512" => (
            &signature::RSA_PSS_SHA512,
            &signature::RSA_PSS_2048_8192_SHA512,
        ),
        _ => return None,
    };
    (SAFE_PRIVATE.contains(&bits) && salt == digest_len(hash)).then_some(pair)
}

/// RSA-PSS's signature of `data` with the private key `parts` and a salt
/// of `salt` bytes, on its route.
fn pss_sign(
    parts: &native::RsaPrivateParts,
    hash: &str,
    salt: u32,
    data: &[u8],
) -> Result<Vec<u8>, Error> {
    if let Some((encoding, _)) = pss_route(hash, bit_length(&parts.n), salt) {
        let pair = safe_key_pair(parts).map_err(|_| Error::operation(""))?;
        let mut out = vec![0u8; pair.public_modulus_len()];
        pair.sign(encoding, &rand::SystemRandom::new(), data, &mut out)
            .map_err(|_| Error::operation(""))?;
        return Ok(out);
    }
    let key = native::rsa_private_from_parts(parts).map_err(|_| Error::operation(""))?;
    let digest = digest::digest(digest_algorithm(hash), data);
    native::rsa_sign_pss(&key, native_hash(hash), digest.as_ref(), salt)
        .map_err(|_| Error::operation(""))
}

/// RSASSA-PKCS1-v1_5 signing's route for a key of `bits` bits: aws-lc-rs
/// signs SHA-256, SHA-384 and SHA-512 at 2048 to 8192 bits (its
/// `RSA_PKCS1_SHA*`, no SHA-1); anything else, the library.
pub(crate) fn pkcs1_sign_route(
    hash: &str,
    bits: u32,
) -> Option<&'static signature::RsaSignatureEncoding> {
    let encoding = match hash {
        "SHA-256" => &signature::RSA_PKCS1_SHA256,
        "SHA-384" => &signature::RSA_PKCS1_SHA384,
        "SHA-512" => &signature::RSA_PKCS1_SHA512,
        _ => return None,
    };
    SAFE_PRIVATE.contains(&bits).then_some(encoding)
}

/// The safe API's key pair of `parts`, for the tests.
#[cfg(test)]
pub(crate) fn safe_key_pair_for_tests(parts: &native::RsaPrivateParts) -> rsa::KeyPair {
    safe_key_pair(parts).unwrap()
}

/// A modulus's length in bits, for the tests.
#[cfg(test)]
pub(crate) fn bit_length_for_tests(n: &[u8]) -> u32 {
    bit_length(n)
}

/// aws-lc-rs's key pair of `parts`, for a key inside its range.
fn safe_key_pair(parts: &native::RsaPrivateParts) -> Result<rsa::KeyPair, Error> {
    let components = rsa::KeyPairComponents {
        public_key: rsa::PublicKeyComponents {
            n: parts.n.as_slice(),
            e: parts.e.as_slice(),
        },
        d: parts.d.as_slice(),
        p: parts.p.as_slice(),
        q: parts.q.as_slice(),
        dP: parts.dp.as_slice(),
        dQ: parts.dq.as_slice(),
        qInv: parts.qi.as_slice(),
    };
    rsa::KeyPair::from_components(&components).map_err(|_| Error::data(""))
}

/// RSASSA-PKCS1-v1_5's signature of `data` with the private key `parts`,
/// on its route.
fn pkcs1_sign(parts: &native::RsaPrivateParts, hash: &str, data: &[u8]) -> Result<Vec<u8>, Error> {
    if let Some(encoding) = pkcs1_sign_route(hash, bit_length(&parts.n)) {
        let pair = safe_key_pair(parts).map_err(|_| Error::operation(""))?;
        let mut out = vec![0u8; pair.public_modulus_len()];
        pair.sign(encoding, &rand::SystemRandom::new(), data, &mut out)
            .map_err(|_| Error::operation(""))?;
        return Ok(out);
    }
    let key = native::rsa_private_from_parts(parts).map_err(|_| Error::operation(""))?;
    let digest = digest::digest(digest_algorithm(hash), data);
    native::rsa_sign_pkcs1(&key, native_hash(hash), digest.as_ref())
        .map_err(|_| Error::operation(""))
}

/// A PKCS #8 `PrivateKeyInfo` (RFC 5208) around a PKCS #1
/// `RSAPrivateKey`: version 0, rsaEncryption with NULL parameters, the
/// key as an OCTET STRING. Written here, never read (the lead's ruling of
/// 2026-11-23): the library reads every PKCS #8 this crate takes.
pub(crate) fn pkcs8_wrap(pkcs1: &[u8]) -> Vec<u8> {
    const HEAD: [u8; 18] = [
        0x02, 0x01, 0x00, // version 0
        0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05,
        0x00, // rsaEncryption, NULL
    ];
    let mut body = HEAD.to_vec();
    body.push(0x04);
    der_length(&mut body, pkcs1.len());
    body.extend_from_slice(pkcs1);
    let mut out = vec![0x30];
    der_length(&mut out, body.len());
    out.extend_from_slice(&body);
    out
}

/// A DER length, short or long form.
fn der_length(out: &mut Vec<u8>, len: usize) {
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

/// A private key's parts, by the library from a PKCS #8, aws-lc-rs's
/// own parse required too inside its range.
fn pkcs8_parts(der: &[u8]) -> Result<native::RsaPrivateParts, Error> {
    let key = native::rsa_private_from_pkcs8(der).map_err(|_| Error::data(""))?;
    let parts = key.parts();
    if SAFE_PRIVATE.contains(&bit_length(&parts.n)) {
        rsa::KeyPair::from_pkcs8(der).map_err(|_| Error::data(""))?;
    }
    Ok(parts)
}

/// A private key's PKCS #8: aws-lc-rs's inside its range, the library's
/// PKCS #1 in the wrapper written here outside it.
fn pkcs8_of(parts: &native::RsaPrivateParts) -> Result<Vec<u8>, Error> {
    let failed = || Error::operation("");
    if SAFE_PRIVATE.contains(&bit_length(&parts.n)) {
        let pair = safe_key_pair(parts).map_err(|_| failed())?;
        let der = AsDer::<aws_lc_rs::encoding::Pkcs8V1Der>::as_der(&pair).map_err(|_| failed())?;
        return Ok(der.as_ref().to_vec());
    }
    let key = native::rsa_private_from_parts(parts).map_err(|_| failed())?;
    let mut pkcs1 = native::rsa_private_pkcs1(&key).map_err(|_| failed())?;
    let out = pkcs8_wrap(&pkcs1);
    pkcs1.iter_mut().for_each(|b| *b = 0);
    std::hint::black_box(&pkcs1);
    Ok(out)
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
                KeyAlgorithm::Rsa {
                    name: "RSASSA-PKCS1-v1_5",
                    hash,
                    ..
                } => match &key.material {
                    Material::RsaPrivate(parts) => {
                        Ok(Outcome::Bytes(pkcs1_sign(parts, hash, &data)?))
                    }
                    _ => Err(Error::operation("")),
                },
                KeyAlgorithm::Rsa {
                    name: "RSA-PSS",
                    hash,
                    ..
                } => match &key.material {
                    Material::RsaPrivate(parts) => {
                        let salt = alg.num("saltLength").unwrap_or(0);
                        Ok(Outcome::Bytes(pss_sign(parts, hash, salt, &data)?))
                    }
                    _ => Err(Error::operation("")),
                },
                KeyAlgorithm::Okp { name: "Ed25519" } => match &key.material {
                    Material::OkpPrivate { d, x } => {
                        let pair = signature::Ed25519KeyPair::from_seed_and_public_key(d, x)
                            .map_err(|_| Error::operation(""))?;
                        Ok(Outcome::Bytes(pair.sign(&data).as_ref().to_vec()))
                    }
                    _ => Err(Error::operation("")),
                },
                KeyAlgorithm::MlDsa { name } => match &key.material {
                    Material::PqPrivate { seed, .. } => {
                        Ok(Outcome::Bytes(ml_dsa_sign(name, seed, &alg, &data)?))
                    }
                    _ => Err(Error::operation("")),
                },
                KeyAlgorithm::Ec {
                    name: "ECDSA",
                    curve,
                } => match &key.material {
                    Material::EcPrivate { d, point } => Ok(Outcome::Bytes(ecdsa_sign(
                        curve,
                        alg.hash().unwrap_or("SHA-256"),
                        d,
                        point,
                        &data,
                    )?)),
                    _ => Err(Error::operation("")),
                },
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
            Ok(Outcome::Bool(verify(key, &alg, &signature, &data)?))
        }
        Call::WrapKey {
            format,
            key,
            wrapping_key,
            algorithm,
        } => wrap_key(keys, &format, key, wrapping_key, &algorithm),
        Call::UnwrapKey {
            format,
            wrapped,
            unwrapping_key,
            algorithm,
            unwrapped_algorithm,
            extractable,
            usages,
        } => unwrap_key(
            keys,
            format,
            &wrapped,
            unwrapping_key,
            &algorithm,
            unwrapped_algorithm,
            extractable,
            usage_set(&usages),
        ),
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
            let base = key_for(keys, key, alg.name, Usage::DeriveBits)?;
            Ok(Outcome::Bytes(derive_bits(keys, base, &alg, length)?))
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
            let derived_length = normalize("get key length", &derived)?;
            let bits = {
                // The base key's algorithm and usage before the derived
                // key's length, as the spec and the reference check them
                // (WPT: a 192-bit AES key from a key without deriveKey is
                // an InvalidAccessError).
                let base = key_for(keys, key, alg.name, Usage::DeriveKey)?;
                let length = key_length(&derived_length)?;
                derive_bits(keys, base, &alg, length)?
            };
            // As raw-secret, which every secret's algorithm takes
            // (ChaCha20-Poly1305 refuses raw; the reference derives it).
            let info = import(
                keys,
                "raw-secret",
                &Value::Bytes(bits),
                &derived_import,
                extractable,
                &usage_set(&usages),
            )?;
            Ok(Outcome::Key(info))
        }
        Call::EncapsulateBits { algorithm, key } => {
            let alg = normalize("encapsulate", &algorithm)?;
            let key = key_for(keys, key, alg.name, Usage::EncapsulateBits)?;
            let Material::PqPublic { public } = &key.material else {
                return Err(Error::operation(""));
            };
            let (ciphertext, shared) = ml_kem_encapsulate(alg.name, public)?;
            Ok(Outcome::Encapsulated {
                ciphertext,
                shared: Box::new(Outcome::Bytes(shared)),
            })
        }
        Call::EncapsulateKey {
            algorithm,
            key,
            shared_algorithm,
            extractable,
            usages,
        } => {
            let alg = normalize("encapsulate", &algorithm)?;
            let shared_import = normalize("importKey", &shared_algorithm)?;
            let key = key_for(keys, key, alg.name, Usage::EncapsulateKey)?;
            let Material::PqPublic { public } = &key.material else {
                return Err(Error::operation(""));
            };
            let (ciphertext, shared) = ml_kem_encapsulate(alg.name, public)?;
            let shared = shared_key(keys, shared, &shared_import, extractable, &usages)?;
            Ok(Outcome::Encapsulated {
                ciphertext,
                shared: Box::new(shared),
            })
        }
        Call::DecapsulateBits {
            algorithm,
            key,
            ciphertext,
        } => {
            let alg = normalize("decapsulate", &algorithm)?;
            let key = key_for(keys, key, alg.name, Usage::DecapsulateBits)?;
            let Material::PqPrivate { seed, .. } = &key.material else {
                return Err(Error::operation(""));
            };
            Ok(Outcome::Bytes(ml_kem_decapsulate(
                alg.name,
                seed,
                &ciphertext,
            )?))
        }
        Call::DecapsulateKey {
            algorithm,
            key,
            ciphertext,
            shared_algorithm,
            extractable,
            usages,
        } => {
            let alg = normalize("decapsulate", &algorithm)?;
            let shared_import = normalize("importKey", &shared_algorithm)?;
            let shared = {
                let key = key_for(keys, key, alg.name, Usage::DecapsulateKey)?;
                let Material::PqPrivate { seed, .. } = &key.material else {
                    return Err(Error::operation(""));
                };
                ml_kem_decapsulate(alg.name, seed, &ciphertext)?
            };
            shared_key(keys, shared, &shared_import, extractable, &usages)
        }
        Call::GetPublicKey { key, usages } => public_key_of(keys, key, &usages),
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
        // 256 bits, whatever length was asked (probed).
        CHACHA => Ok(Some(256)),
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
    crypt_with(key, alg, data, encrypt)
}

/// An encryption or decryption with `key`, its usages checked already
/// (`crypt`'s, or wrapKey's and unwrapKey's own).
fn crypt_with(key: &Key, alg: &Normalized, data: &[u8], encrypt: bool) -> Result<Outcome, Error> {
    match alg.name {
        "AES-CBC" => aes_cbc(secret(key), alg, data, encrypt),
        CHACHA => chacha(secret(key), alg, data, encrypt),
        "AES-CTR" => aes_ctr(secret(key), alg, data),
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
            // aws-lc-rs's AEAD takes a 96-bit IV and a 128-bit tag alone;
            // any other, the library's own GCM (`native`).
            let Ok(nonce) = <[u8; 12]>::try_from(iv) else {
                return native_gcm(secret(key), iv, aad, data, tag, encrypt);
            };
            if tag != 128 {
                return native_gcm(secret(key), iv, aad, data, tag, encrypt);
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
        "RSA-OAEP" => oaep(key, alg, data, encrypt),
        _ => Err(Error::not_supported("Unsupported operation")),
    }
}

/// AES's cipher of a key of `len` bytes (128 or 256 bits: the
/// reference refuses 192).
/// ChaCha20-Poly1305 (RFC 8439), aws-lc-rs's: a 96-bit IV and a 128-bit
/// tag alone, each refused otherwise in the reference's words (probed:
/// its IV's says 16 bytes, as Chrome 158's does), the tag after the data.
fn chacha(key: &[u8], alg: &Normalized, data: &[u8], encrypt: bool) -> Result<Outcome, Error> {
    if alg.num("tagLength").is_some_and(|tag| tag != 128) {
        return Err(Error::operation(
            "The tag length is invalid: Must be 128 bits",
        ));
    }
    let Ok(nonce) = <[u8; 12]>::try_from(alg.bytes("iv").unwrap_or(&[])) else {
        return Err(Error::operation(
            "The \"iv\" has an unexpected length -- must be 16 bytes",
        ));
    };
    let aad = aead::Aad::from(alg.bytes("additionalData").unwrap_or(&[]));
    let failed = |_| Error::operation("");
    let sealing = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::CHACHA20_POLY1305, key).map_err(failed)?,
    );
    let nonce = aead::Nonce::assume_unique_for_key(nonce);
    let mut buffer = data.to_vec();
    if encrypt {
        sealing
            .seal_in_place_append_tag(nonce, aad, &mut buffer)
            .map_err(failed)?;
        return Ok(Outcome::Bytes(buffer));
    }
    if buffer.len() < 16 {
        return Err(Error::operation("The provided data is too small"));
    }
    let plain = sealing
        .open_in_place(nonce, aad, &mut buffer)
        .map_err(failed)?;
    Ok(Outcome::Bytes(plain.to_vec()))
}

fn aes_cipher(len: usize) -> Result<&'static aws_lc_rs::cipher::Algorithm, Error> {
    match len {
        16 => Ok(&aws_lc_rs::cipher::AES_128),
        32 => Ok(&aws_lc_rs::cipher::AES_256),
        _ => Err(Error::operation("")),
    }
}

/// AES-CBC with PKCS #7 padding, aws-lc-rs's; the reference's errors.
fn aes_cbc(key: &[u8], alg: &Normalized, data: &[u8], encrypt: bool) -> Result<Outcome, Error> {
    use aws_lc_rs::cipher::{
        DecryptionContext, EncryptionContext, PaddedBlockDecryptingKey, PaddedBlockEncryptingKey,
        UnboundCipherKey,
    };
    use aws_lc_rs::iv::FixedLength;
    let Ok(iv) = <[u8; 16]>::try_from(alg.bytes("iv").unwrap_or(&[])) else {
        return Err(Error::operation(
            "The \"iv\" has an unexpected length -- must be 16 bytes",
        ));
    };
    let failed = |_| Error::operation("");
    let unbound = UnboundCipherKey::new(aes_cipher(key.len())?, key).map_err(failed)?;
    let mut buffer = data.to_vec();
    if encrypt {
        let sealing = PaddedBlockEncryptingKey::cbc_pkcs7(unbound).map_err(failed)?;
        sealing
            .less_safe_encrypt(&mut buffer, EncryptionContext::Iv128(FixedLength::from(iv)))
            .map_err(failed)?;
        Ok(Outcome::Bytes(buffer))
    } else {
        let opening = PaddedBlockDecryptingKey::cbc_pkcs7(unbound).map_err(failed)?;
        let plain = opening
            .decrypt(&mut buffer, DecryptionContext::Iv128(FixedLength::from(iv)))
            .map_err(failed)?
            .to_vec();
        Ok(Outcome::Bytes(plain))
    }
}

/// AES-CTR, its counter's low `length` bits counting and wrapping, the
/// rest fixed (the specification's, Chrome's): aws-lc-rs's CTR, which
/// counts all 128 bits, run on each stretch before the low bits wrap,
/// from that stretch's counter. Encryption and decryption are one.
fn aes_ctr(key: &[u8], alg: &Normalized, data: &[u8]) -> Result<Outcome, Error> {
    use aws_lc_rs::cipher::{EncryptingKey, EncryptionContext, UnboundCipherKey};
    use aws_lc_rs::iv::FixedLength;
    let Ok(counter) = <[u8; 16]>::try_from(alg.bytes("counter").unwrap_or(&[])) else {
        return Err(Error::operation(
            "The \"counter\" has an unexpected length -- must be 16 bytes",
        ));
    };
    let length = alg.num("length").unwrap_or(0);
    if !(1..=128).contains(&length) {
        return Err(Error::operation(
            "The \"length\" member must be >= 1 and <= 128",
        ));
    }
    let blocks = data.len().div_ceil(16) as u128;
    if length < 128 && blocks > 1u128 << length {
        return Err(Error::data(
            "The input is too large for the counter length.",
        ));
    }
    let failed = |_| Error::operation("");
    let unbound = UnboundCipherKey::new(aes_cipher(key.len())?, key).map_err(failed)?;
    let sealing = EncryptingKey::ctr(unbound).map_err(failed)?;
    let mask: u128 = if length == 128 {
        u128::MAX
    } else {
        (1u128 << length) - 1
    };
    let mut counter = u128::from_be_bytes(counter);
    let mut out = data.to_vec();
    let mut at = 0;
    while at < out.len() {
        let low = counter & mask;
        // Blocks before the low bits wrap; all that is left at 128 bits,
        // where aws-lc-rs's own count wraps as the specification's does.
        let room = (mask - low).checked_add(1).map_or(usize::MAX, |b| {
            usize::try_from(b.saturating_mul(16)).unwrap_or(usize::MAX)
        });
        let end = out.len().min(at.saturating_add(room));
        sealing
            .less_safe_encrypt(
                &mut out[at..end],
                EncryptionContext::Iv128(FixedLength::from(counter.to_be_bytes())),
            )
            .map_err(failed)?;
        let used = (end - at).div_ceil(16) as u128;
        counter = (counter & !mask) | (low.wrapping_add(used) & mask);
        at = end;
    }
    Ok(Outcome::Bytes(out))
}

/// AES-KW (RFC 3394), aws-lc-rs's: the reference's errors (probed) for
/// what is too short, then for what is not a multiple of 8 bytes.
fn aes_kw(key: &[u8], data: &[u8], wrap: bool) -> Result<Vec<u8>, Error> {
    use aws_lc_rs::key_wrap::{AES_128, AES_256, AesKek, KeyWrap};
    if data.len() < if wrap { 16 } else { 24 } {
        return Err(Error::operation("The provided data is too small"));
    }
    if !data.len().is_multiple_of(8) {
        return Err(Error::data(
            "The AES-KW input data length is invalid: not a multiple of 8 bytes",
        ));
    }
    let cipher = match key.len() {
        16 => &AES_128,
        32 => &AES_256,
        _ => return Err(Error::operation("")),
    };
    let kek = AesKek::new(cipher, key).map_err(|_| Error::operation(""))?;
    let mut out = vec![0u8; data.len() + 8];
    let len = if wrap {
        kek.wrap(data, &mut out).map(|o| o.len())
    } else {
        kek.unwrap(data, &mut out).map(|o| o.len())
    }
    .map_err(|_| Error::operation(""))?;
    out.truncate(len);
    Ok(out)
}

/// An exported JWK's text, as the reference wraps it (probed: compact,
/// its members in their export order).
pub(crate) fn jwk_text(members: &[(String, JwkValue)]) -> String {
    use devtools::Json;
    Json::object(members.iter().map(|(name, value)| {
        let value = match value {
            JwkValue::Str(s) => Json::String(s.clone()),
            JwkValue::Bool(b) => Json::Bool(*b),
            JwkValue::List(items) => {
                Json::Array(items.iter().map(|s| Json::String(s.clone())).collect())
            }
        };
        (name.as_str(), value)
    }))
    .write()
}

/// An unwrapped JWK's text as the dictionary importKey takes, or the
/// reference's DataError for anything but a JSON object.
fn jwk_value(bytes: &[u8]) -> Result<Value, Error> {
    use devtools::Json;
    let refused = || Error::data("JWK input could not be parsed to a JSON dictionary");
    let text = std::str::from_utf8(bytes).map_err(|_| refused())?;
    fn value(json: &Json) -> Value {
        match json {
            Json::Null => Value::Absent,
            Json::Bool(b) => Value::Bool(*b),
            Json::Number(n) => Value::Num(*n),
            Json::String(s) => Value::Str(s.clone()),
            Json::Array(items) => Value::List(items.iter().map(value).collect()),
            Json::Object(members) => {
                // A name given twice is its last, as JSON.parse keeps.
                let mut out: Vec<(String, Value)> = Vec::new();
                for (name, member) in members {
                    out.retain(|(n, _)| n != name);
                    out.push((name.clone(), value(member)));
                }
                Value::Dict(out)
            }
        }
    }
    match Json::parse(text).map_err(|_| refused())? {
        object @ Json::Object(_) => Ok(value(&object)),
        _ => Err(refused()),
    }
}

/// wrapKey: the key exported in `format` (a JWK as its text) and
/// encrypted, or wrapped by AES-KW, with the wrapping key, in the
/// reference's order of checks.
fn wrap_key(
    keys: &Keys,
    format: &str,
    key: u32,
    wrapping_key: u32,
    algorithm: &Value,
) -> Result<Outcome, Error> {
    let alg = normalize("wrapKey", algorithm)?;
    let wrapping = key_for(keys, wrapping_key, alg.name, Usage::WrapKey)?;
    let key = keys.get(key)?;
    exportable(key)?;
    if !key.extractable {
        return Err(Error::invalid_access("key is not extractable"));
    }
    let mut bytes = match export(key, format)? {
        Outcome::Bytes(bytes) => bytes,
        Outcome::Jwk(members) => jwk_text(&members).into_bytes(),
        _ => return Err(Error::operation("")),
    };
    let out = if alg.name == "AES-KW" {
        aes_kw(secret(wrapping), &bytes, true).map(Outcome::Bytes)
    } else {
        crypt_with(wrapping, &alg, &bytes, true)
    };
    bytes.iter_mut().for_each(|b| *b = 0);
    std::hint::black_box(&bytes);
    out
}

/// unwrapKey: the wrapped bytes decrypted, or unwrapped by AES-KW, then
/// imported as `format` (a JWK from its text), in the reference's order.
#[allow(clippy::too_many_arguments)]
fn unwrap_key(
    keys: &mut Keys,
    format: String,
    wrapped: &[u8],
    unwrapping_key: u32,
    algorithm: &Value,
    unwrapped_algorithm: Value,
    extractable: bool,
    usages: Vec<Usage>,
) -> Result<Outcome, Error> {
    let alg = normalize("unwrapKey", algorithm)?;
    normalize("importKey", &unwrapped_algorithm)?;
    let unwrapping = key_for(keys, unwrapping_key, alg.name, Usage::UnwrapKey)?;
    let mut bytes = if alg.name == "AES-KW" {
        aes_kw(secret(unwrapping), wrapped, false)?
    } else {
        match crypt_with(unwrapping, &alg, wrapped, false)? {
            Outcome::Bytes(bytes) => bytes,
            _ => return Err(Error::operation("")),
        }
    };
    let key_data = if format == "jwk" {
        jwk_value(&bytes)
    } else {
        Ok(Value::Bytes(bytes.clone()))
    };
    bytes.iter_mut().for_each(|b| *b = 0);
    std::hint::black_box(&bytes);
    run(
        keys,
        Call::ImportKey {
            format,
            key_data: key_data?,
            algorithm: unwrapped_algorithm,
            extractable,
            usages,
        },
    )
}

/// A curve Web Crypto names: its coordinates' length in bytes, and the
/// aws-lc-rs algorithms that parse and make its keys.
struct Curve {
    name: &'static str,
    coord: usize,
    ecdh: &'static aws_lc_rs::agreement::Algorithm,
    parse: &'static signature::EcdsaVerificationAlgorithm,
    pair: &'static signature::EcdsaSigningAlgorithm,
}

const CURVES: [Curve; 3] = [
    Curve {
        name: "P-256",
        coord: 32,
        ecdh: &aws_lc_rs::agreement::ECDH_P256,
        parse: &signature::ECDSA_P256_SHA256_FIXED,
        pair: &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
    },
    Curve {
        name: "P-384",
        coord: 48,
        ecdh: &aws_lc_rs::agreement::ECDH_P384,
        parse: &signature::ECDSA_P384_SHA384_FIXED,
        pair: &signature::ECDSA_P384_SHA384_FIXED_SIGNING,
    },
    Curve {
        name: "P-521",
        coord: 66,
        ecdh: &aws_lc_rs::agreement::ECDH_P521,
        parse: &signature::ECDSA_P521_SHA512_FIXED,
        pair: &signature::ECDSA_P521_SHA512_FIXED_SIGNING,
    },
];

fn curve_of(name: &str) -> &'static Curve {
    CURVES.iter().find(|c| c.name == name).unwrap_or(&CURVES[0])
}

/// The curve an EC algorithm names, or the reference's NotSupportedError.
fn named_curve(alg: &Normalized, context: &str) -> Result<&'static str, Error> {
    let given = alg.str("namedCurve").unwrap_or("");
    CURVES
        .iter()
        .find(|c| c.name == given)
        .map(|c| c.name)
        .ok_or_else(|| {
            Error::not_supported(format!("{context}: Unrecognized namedCurve")).of_method()
        })
}

/// The reason ECDSA with a curve and hash aws-lc-rs pairs in no way is
/// refused: a known gap (the lead's ruling, 2026-11-23), a
/// NotSupportedError on purpose. Proton's OpenPGP.js rethrows a DataError
/// or an OperationError from ECDSA on P-256 and P-384 and fails the
/// message, but falls back to its own code for any other name.
pub(crate) const ECDSA_GAP: &str = "ECDSA with this curve and hash is not supported";

/// A public key's point, uncompressed, on `curve`, from any form
/// aws-lc-rs takes (an SPKI, or an uncompressed or compressed point),
/// checked by it; `None` for one it refuses. The point is the end of its
/// own SPKI's encoding of the key, the fixed-length BIT STRING it writes.
fn ec_point(curve: &Curve, bytes: &[u8]) -> Option<Vec<u8>> {
    let parsed = signature::ParsedPublicKey::new(curve.parse, bytes).ok()?;
    let der = AsDer::<aws_lc_rs::encoding::PublicKeyX509Der>::as_der(&parsed).ok()?;
    let der = der.as_ref();
    let len = 2 * curve.coord + 1;
    let point = der.get(der.len().checked_sub(len)?..)?;
    (point.first() == Some(&4)).then(|| point.to_vec())
}

/// A private key's scalar and point, from the private key `pair` aws-lc-rs
/// parsed or made.
fn ec_parts(pair: &signature::EcdsaKeyPair) -> Option<(Vec<u8>, Vec<u8>)> {
    use aws_lc_rs::encoding::{AsBigEndian, EcPrivateKeyBin};
    use signature::KeyPair;
    let d = AsBigEndian::<EcPrivateKeyBin>::as_be_bytes(&pair.private_key()).ok()?;
    Some((d.as_ref().to_vec(), pair.public_key().as_ref().to_vec()))
}

/// The scalar `d` is `point`'s, both on `curve`: aws-lc-rs's own point of
/// it compared. On purpose, the check `EcdsaKeyPair::
/// from_private_key_and_public_key` makes today but does not document
/// (the lead's ruling on key material a page gives, 2026-11-23); no test
/// sees it while that stands, the JWK test pins the outcome.
fn ec_pair(curve: &Curve, d: &[u8], point: &[u8]) -> Option<signature::EcdsaKeyPair> {
    use signature::KeyPair;
    let pair =
        signature::EcdsaKeyPair::from_private_key_and_public_key(curve.pair, d, point).ok()?;
    let private = aws_lc_rs::agreement::PrivateKey::from_private_key(curve.ecdh, d).ok()?;
    let public = private.compute_public_key().ok()?;
    (public.as_ref() == point && pair.public_key().as_ref() == point).then_some(pair)
}

fn generate_ec(
    keys: &mut Keys,
    alg: &Normalized,
    extractable: bool,
    usages: &[Usage],
) -> Result<Outcome, Error> {
    let curve = curve_of(named_curve(alg, "EcKeyGenParams")?);
    let public_allowed = allowed(alg.name, KeyType::Public);
    let private_allowed = allowed(alg.name, KeyType::Private);
    if usages
        .iter()
        .any(|u| !public_allowed.contains(u) && !private_allowed.contains(u))
    {
        return Err(Error::syntax(
            "Cannot create a key using the specified key usages.",
        ));
    }
    let pair = signature::EcdsaKeyPair::generate(curve.pair).map_err(|_| Error::operation(""))?;
    let (d, point) = ec_parts(&pair).ok_or_else(|| Error::operation(""))?;
    let public_usages: Vec<Usage> = usages
        .iter()
        .copied()
        .filter(|u| public_allowed.contains(u))
        .collect();
    let private_usages: Vec<Usage> = usages
        .iter()
        .copied()
        .filter(|u| private_allowed.contains(u))
        .collect();
    empty_usages(KeyType::Private, &private_usages)?;
    let algorithm = KeyAlgorithm::Ec {
        name: alg.name,
        curve: curve.name,
    };
    let public = Key {
        kind: KeyType::Public,
        extractable: true,
        algorithm: algorithm.clone(),
        usages: public_usages,
        material: Material::EcPublic {
            point: point.clone(),
        },
    };
    let private = Key {
        kind: KeyType::Private,
        extractable,
        algorithm,
        usages: private_usages,
        material: Material::EcPrivate { d, point },
    };
    Ok(Outcome::Pair(keys.insert(public), keys.insert(private)))
}

/// An EC key's import, as the reference checks it (probed): raw and SPKI
/// public, PKCS #8 private, JWK either.
fn import_ec(
    keys: &mut Keys,
    format: &str,
    data: &Value,
    alg: &Normalized,
    extractable: bool,
    usages: &[Usage],
) -> Result<KeyInfo, Error> {
    let curve = curve_of(named_curve(alg, "EcKeyImportParams")?);
    let bytes = || match data {
        Value::Bytes(bytes) => Ok(bytes.as_slice()),
        _ => Err(Error::type_error(
            "Key data must be a BufferSource for non-JWK formats",
        )),
    };
    let other_curve =
        || Error::data("The imported EC key specifies a different curve than requested");
    let invalid = || Error::data("The imported EC key is invalid");
    let (kind, material) = match format {
        "raw" | "raw-public" | "spki" => {
            check_usages(alg.name, KeyType::Public, usages)?;
            let given = bytes()?;
            let point = match ec_point(curve, given) {
                Some(point) => point,
                None if format == "spki" && CURVES.iter().any(|c| ec_point(c, given).is_some()) => {
                    return Err(other_curve());
                }
                None => return Err(Error::data("")),
            };
            (KeyType::Public, Material::EcPublic { point })
        }
        "pkcs8" => {
            check_usages(alg.name, KeyType::Private, usages)?;
            let given = bytes()?;
            let pair = match signature::EcdsaKeyPair::from_pkcs8(curve.pair, given) {
                Ok(pair) => pair,
                Err(_)
                    if CURVES
                        .iter()
                        .any(|c| signature::EcdsaKeyPair::from_pkcs8(c.pair, given).is_ok()) =>
                {
                    return Err(other_curve());
                }
                Err(_) => return Err(Error::data("")),
            };
            let (d, point) = ec_parts(&pair).ok_or_else(|| Error::data(""))?;
            (KeyType::Private, Material::EcPrivate { d, point })
        }
        "jwk" => {
            let Value::Dict(_) = data else {
                return Err(Error::type_error(
                    "Key data must be an object for JWK import",
                ));
            };
            let jwk = jwk::Jwk(data);
            let want = (alg.name == "ECDSA").then_some(match curve.name {
                "P-384" => "ES384",
                "P-521" => "ES512",
                _ => "ES256",
            });
            jwk.check("EC", want, extractable, usages)?;
            match jwk.str("crv") {
                None => return Err(Error::data("The required JWK member \"crv\" was missing")),
                Some(crv) if crv != curve.name => {
                    return Err(Error::data(
                        "The JWK's \"crv\" member specifies a different curve than requested",
                    ));
                }
                Some(_) => {}
            }
            let private = jwk.str("d").is_some();
            check_usages(
                alg.name,
                if private {
                    KeyType::Private
                } else {
                    KeyType::Public
                },
                usages,
            )?;
            let x = jwk.bytes("x")?;
            let y = jwk.bytes("y")?;
            if x.len() != curve.coord || y.len() != curve.coord {
                return Err(invalid());
            }
            let point = [&[4u8][..], &x, &y].concat();
            let point = ec_point(curve, &point).ok_or_else(invalid)?;
            if private {
                let d = jwk.bytes("d")?;
                if d.len() != curve.coord || ec_pair(curve, &d, &point).is_none() {
                    return Err(invalid());
                }
                (KeyType::Private, Material::EcPrivate { d, point })
            } else {
                (KeyType::Public, Material::EcPublic { point })
            }
        }
        _ => {
            return Err(Error::not_supported(
                "Unsupported import key format for algorithm",
            ));
        }
    };
    if kind == KeyType::Private {
        empty_usages(KeyType::Private, usages)?;
    }
    let key = Key {
        kind,
        extractable,
        algorithm: KeyAlgorithm::Ec {
            name: alg.name,
            curve: curve.name,
        },
        usages: usages.to_vec(),
        material,
    };
    Ok(keys.insert(key))
}

fn export_ec(key: &Key, curve: &str, material: &Material, format: &str) -> Result<Outcome, Error> {
    let curve = curve_of(curve);
    let wrong = || Error::invalid_access("The key is not of the expected type");
    let point = match material {
        Material::EcPublic { point } | Material::EcPrivate { point, .. } => point,
        _ => return Err(Error::operation("")),
    };
    let private = matches!(material, Material::EcPrivate { .. });
    match format {
        "raw" | "raw-public" | "spki" if private => Err(wrong()),
        "raw" | "raw-public" => Ok(Outcome::Bytes(point.clone())),
        "spki" => {
            let parsed = signature::ParsedPublicKey::new(curve.parse, point)
                .map_err(|_| Error::operation(""))?;
            let der = AsDer::<aws_lc_rs::encoding::PublicKeyX509Der>::as_der(&parsed)
                .map_err(|_| Error::operation(""))?;
            Ok(Outcome::Bytes(der.as_ref().to_vec()))
        }
        "pkcs8" => {
            let Material::EcPrivate { d, .. } = material else {
                return Err(wrong());
            };
            let private = aws_lc_rs::agreement::PrivateKey::from_private_key(curve.ecdh, d)
                .map_err(|_| Error::operation(""))?;
            let der = AsDer::<aws_lc_rs::encoding::Pkcs8V1Der>::as_der(&private)
                .map_err(|_| Error::operation(""))?;
            Ok(Outcome::Bytes(der.as_ref().to_vec()))
        }
        "jwk" => {
            let c = curve.coord;
            let mut members = vec![
                ("crv", JwkValue::Str(curve.name.into())),
                ("kty", JwkValue::Str("EC".into())),
                ("x", JwkValue::Str(jwk::encode(&point[1..1 + c]))),
                ("y", JwkValue::Str(jwk::encode(&point[1 + c..]))),
            ];
            if let Material::EcPrivate { d, .. } = material {
                members.push(("d", JwkValue::Str(jwk::encode(d))));
            }
            Ok(Outcome::Jwk(jwk::exported(
                members,
                key.extractable,
                &key.usages,
            )))
        }
        _ => Err(Error::not_supported(
            "Unsupported export key format for algorithm",
        )),
    }
}

/// ECDSA's signature (r and s, each the curve's length), aws-lc-rs's for
/// the curve and hash pairs it signs.
/// The curve and hash pairs aws-lc-rs signs ECDSA with: `supports`'s
/// table too.
fn ecdsa_signing(curve: &str, hash: &str) -> Option<&'static signature::EcdsaSigningAlgorithm> {
    Some(match (curve, hash) {
        ("P-256", "SHA-256") => &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
        ("P-384", "SHA-384") => &signature::ECDSA_P384_SHA384_FIXED_SIGNING,
        ("P-521", "SHA-256") => &signature::ECDSA_P521_SHA256_FIXED_SIGNING,
        ("P-521", "SHA-384") => &signature::ECDSA_P521_SHA384_FIXED_SIGNING,
        ("P-521", "SHA-512") => &signature::ECDSA_P521_SHA512_FIXED_SIGNING,
        _ => return None,
    })
}

/// The curve and hash pairs aws-lc-rs verifies ECDSA with, and whether
/// as the fixed form (else its ASN.1 form): `supports`'s table too.
fn ecdsa_verifying(
    curve: &str,
    hash: &str,
) -> Option<(&'static signature::EcdsaVerificationAlgorithm, bool)> {
    use signature::{
        ECDSA_P256_SHA1_ASN1, ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA384_ASN1,
        ECDSA_P256_SHA512_ASN1, ECDSA_P384_SHA256_ASN1, ECDSA_P384_SHA384_FIXED,
        ECDSA_P384_SHA512_ASN1, ECDSA_P521_SHA1_FIXED, ECDSA_P521_SHA256_FIXED,
        ECDSA_P521_SHA384_FIXED, ECDSA_P521_SHA512_FIXED,
    };
    Some(match (curve, hash) {
        ("P-256", "SHA-256") => (&ECDSA_P256_SHA256_FIXED, true),
        ("P-256", "SHA-1") => (&ECDSA_P256_SHA1_ASN1, false),
        ("P-256", "SHA-384") => (&ECDSA_P256_SHA384_ASN1, false),
        ("P-256", "SHA-512") => (&ECDSA_P256_SHA512_ASN1, false),
        ("P-384", "SHA-384") => (&ECDSA_P384_SHA384_FIXED, true),
        ("P-384", "SHA-256") => (&ECDSA_P384_SHA256_ASN1, false),
        ("P-384", "SHA-512") => (&ECDSA_P384_SHA512_ASN1, false),
        ("P-521", "SHA-1") => (&ECDSA_P521_SHA1_FIXED, true),
        ("P-521", "SHA-256") => (&ECDSA_P521_SHA256_FIXED, true),
        ("P-521", "SHA-384") => (&ECDSA_P521_SHA384_FIXED, true),
        ("P-521", "SHA-512") => (&ECDSA_P521_SHA512_FIXED, true),
        _ => return None,
    })
}

fn ecdsa_sign(
    curve: &str,
    hash: &str,
    d: &[u8],
    point: &[u8],
    data: &[u8],
) -> Result<Vec<u8>, Error> {
    let algorithm = ecdsa_signing(curve, hash).ok_or_else(|| Error::not_supported(ECDSA_GAP))?;
    let pair = signature::EcdsaKeyPair::from_private_key_and_public_key(algorithm, d, point)
        .map_err(|_| Error::operation(""))?;
    let signature = pair
        .sign(&rand::SystemRandom::new(), data)
        .map_err(|_| Error::operation(""))?;
    Ok(signature.as_ref().to_vec())
}

/// An INTEGER's DER, of a big-endian unsigned value.
fn der_integer(out: &mut Vec<u8>, value: &[u8]) {
    let value: &[u8] = match value.iter().position(|&b| b != 0) {
        Some(at) => &value[at..],
        None => &[0],
    };
    out.push(0x02);
    let pad = value[0] & 0x80 != 0;
    der_length(out, value.len() + usize::from(pad));
    if pad {
        out.push(0);
    }
    out.extend_from_slice(value);
}

/// ECDSA's verification of a signature of r and s: aws-lc-rs's, as the
/// fixed form where it has it, as its DER (written here, never read)
/// where it has the ASN.1 form alone; a signature of another length false.
fn ecdsa_verify(
    curve: &str,
    hash: &str,
    point: &[u8],
    signature: &[u8],
    data: &[u8],
) -> Result<bool, Error> {
    use signature::UnparsedPublicKey;
    let (algorithm, fixed) =
        ecdsa_verifying(curve, hash).ok_or_else(|| Error::not_supported(ECDSA_GAP))?;
    let c = curve_of(curve).coord;
    if signature.len() != 2 * c {
        return Ok(false);
    }
    let key = UnparsedPublicKey::new(algorithm, point);
    if fixed {
        return Ok(key.verify(data, signature).is_ok());
    }
    let mut body = Vec::new();
    der_integer(&mut body, &signature[..c]);
    der_integer(&mut body, &signature[c..]);
    let mut der = vec![0x30];
    der_length(&mut der, body.len());
    der.extend_from_slice(&body);
    Ok(key.verify(data, &der).is_ok())
}

/// ECDH's shared secret with the `public` key, its first `length` bits
/// (all the curve's, by default), the reference's errors in its order.
/// The public key an ECDH or X25519 derivation's `public` member names,
/// checked as the derivation checks it (the reference's words), and the
/// most bits it agrees on: `supports`'s check too.
fn agreement_public<'k>(keys: &'k Keys, alg: &Normalized) -> Result<(&'k Key, u32), Error> {
    let public = keys.get(alg.key("public").ok_or_else(|| Error::operation(""))?)?;
    if alg.name == "X25519" {
        let prefix = "The public parameter for X25519 key derivation";
        let KeyAlgorithm::Okp { name: "X25519" } = &public.algorithm else {
            return Err(Error::invalid_access(format!(
                "{prefix} must be for X25519"
            )));
        };
        let Material::OkpPublic { .. } = &public.material else {
            return Err(Error::invalid_access(format!(
                "{prefix} is not a public key"
            )));
        };
        return Ok((public, 256));
    }
    let prefix = "The public parameter for ECDH key derivation";
    let KeyAlgorithm::Ec {
        name: "ECDH",
        curve,
    } = &public.algorithm
    else {
        return Err(Error::invalid_access(format!("{prefix} must be for ECDH")));
    };
    let Material::EcPublic { .. } = &public.material else {
        return Err(Error::invalid_access(format!(
            "{prefix} is not a public EC key"
        )));
    };
    Ok((public, 8 * curve_of(curve).coord as u32))
}

fn ecdh_bits(
    keys: &Keys,
    key: &Key,
    alg: &Normalized,
    length: Option<u32>,
) -> Result<Vec<u8>, Error> {
    let (KeyAlgorithm::Ec { curve, .. }, Material::EcPrivate { d, .. }) =
        (&key.algorithm, &key.material)
    else {
        return Err(Error::operation(""));
    };
    let (public, most) = agreement_public(keys, alg)?;
    let (KeyAlgorithm::Ec { curve: theirs, .. }, Material::EcPublic { point }) =
        (&public.algorithm, &public.material)
    else {
        return Err(Error::operation(""));
    };
    if theirs != curve {
        return Err(Error::invalid_access(
            "The public parameter for ECDH key derivation is for a different named curve",
        ));
    }
    let c = curve_of(curve);
    let length = length.unwrap_or(most);
    if length > most {
        return Err(Error::operation(format!(
            "Length specified for ECDH key derivation is too large. Maximum allowed is {most} bits"
        )));
    }
    let private = aws_lc_rs::agreement::PrivateKey::from_private_key(c.ecdh, d)
        .map_err(|_| Error::operation(""))?;
    let peer = aws_lc_rs::agreement::UnparsedPublicKey::new(c.ecdh, point);
    let mut secret =
        aws_lc_rs::agreement::agree(&private, peer, Error::operation(""), |s| Ok(s.to_vec()))?;
    secret.truncate(length.div_ceil(8) as usize);
    if !length.is_multiple_of(8)
        && let Some(last) = secret.last_mut()
    {
        *last &= 0xffu8 << (8 - length % 8);
    }
    Ok(secret)
}

/// The SPKI and PKCS #8 prefixes of an X25519 key (RFC 8410), written
/// here: aws-lc-rs encodes no X25519 key in either. Never read.
const X25519_SPKI: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x03, 0x21, 0x00,
];
const X25519_PKCS8: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x04, 0x22, 0x04, 0x20,
];

/// An Ed25519 or X25519 key's 32 public bytes from an SPKI aws-lc-rs takes
/// as one, strict DER of the one length an RFC 8410 SPKI has: its last 32
/// bytes, the BIT STRING's.
fn okp_spki(name: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() != 44 {
        return None;
    }
    let taken = if name == "Ed25519" {
        signature::ParsedPublicKey::new(&signature::ED25519, bytes).is_ok()
    } else {
        let unparsed =
            aws_lc_rs::agreement::UnparsedPublicKey::new(&aws_lc_rs::agreement::X25519, bytes);
        aws_lc_rs::agreement::ParsedPublicKey::try_from(&unparsed).is_ok_and(|p| {
            matches!(
                p.format(),
                aws_lc_rs::agreement::ParsedPublicKeyFormat::X509
            )
        })
    };
    taken.then(|| bytes[12..].to_vec())
}

/// An Ed25519 seed's public key, aws-lc-rs's.
fn ed25519_public(d: &[u8]) -> Option<Vec<u8>> {
    use signature::KeyPair;
    let pair = signature::Ed25519KeyPair::from_seed_unchecked(d).ok()?;
    Some(pair.public_key().as_ref().to_vec())
}

/// An X25519 scalar's public key, aws-lc-rs's.
fn x25519_public(d: &[u8]) -> Option<Vec<u8>> {
    let private =
        aws_lc_rs::agreement::PrivateKey::from_private_key(&aws_lc_rs::agreement::X25519, d)
            .ok()?;
    Some(private.compute_public_key().ok()?.as_ref().to_vec())
}

fn generate_okp(
    keys: &mut Keys,
    alg: &Normalized,
    extractable: bool,
    usages: &[Usage],
) -> Result<Outcome, Error> {
    use aws_lc_rs::encoding::{AsBigEndian, Curve25519SeedBin};
    let public_allowed = allowed(alg.name, KeyType::Public);
    let private_allowed = allowed(alg.name, KeyType::Private);
    if usages
        .iter()
        .any(|u| !public_allowed.contains(u) && !private_allowed.contains(u))
    {
        return Err(Error::syntax(
            "Cannot create a key using the specified key usages.",
        ));
    }
    let failed = || Error::operation("");
    let (d, x) = if alg.name == "Ed25519" {
        use signature::KeyPair;
        let pair = signature::Ed25519KeyPair::generate().map_err(|_| failed())?;
        let seed = pair.seed().map_err(|_| failed())?;
        let d = AsBigEndian::<Curve25519SeedBin>::as_be_bytes(&seed).map_err(|_| failed())?;
        (d.as_ref().to_vec(), pair.public_key().as_ref().to_vec())
    } else {
        let private = aws_lc_rs::agreement::PrivateKey::generate(&aws_lc_rs::agreement::X25519)
            .map_err(|_| failed())?;
        let d = AsBigEndian::<Curve25519SeedBin>::as_be_bytes(&private).map_err(|_| failed())?;
        let x = private.compute_public_key().map_err(|_| failed())?;
        (d.as_ref().to_vec(), x.as_ref().to_vec())
    };
    let public_usages: Vec<Usage> = usages
        .iter()
        .copied()
        .filter(|u| public_allowed.contains(u))
        .collect();
    let private_usages: Vec<Usage> = usages
        .iter()
        .copied()
        .filter(|u| private_allowed.contains(u))
        .collect();
    empty_usages(KeyType::Private, &private_usages)?;
    let algorithm = KeyAlgorithm::Okp { name: alg.name };
    let public = Key {
        kind: KeyType::Public,
        extractable: true,
        algorithm: algorithm.clone(),
        usages: public_usages,
        material: Material::OkpPublic { x: x.clone() },
    };
    let private = Key {
        kind: KeyType::Private,
        extractable,
        algorithm,
        usages: private_usages,
        material: Material::OkpPrivate { d, x },
    };
    Ok(Outcome::Pair(keys.insert(public), keys.insert(private)))
}

/// An Ed25519 or X25519 key's import, as the reference checks it (probed).
fn import_okp(
    keys: &mut Keys,
    format: &str,
    data: &Value,
    alg: &Normalized,
    extractable: bool,
    usages: &[Usage],
) -> Result<KeyInfo, Error> {
    let name = alg.name;
    let ed = name == "Ed25519";
    let bytes = || match data {
        Value::Bytes(bytes) => Ok(bytes.as_slice()),
        _ => Err(Error::type_error(
            "Key data must be a BufferSource for non-JWK formats",
        )),
    };
    let public_ok =
        |x: &[u8]| !ed || signature::ParsedPublicKey::new(&signature::ED25519, x).is_ok();
    let (kind, material) = match format {
        "raw" | "raw-public" => {
            check_usages(name, KeyType::Public, usages)?;
            let x = bytes()?;
            if x.len() != 32 {
                return Err(Error::data(format!("{name} key data must be 256 bits")));
            }
            if !public_ok(x) {
                return Err(Error::data(""));
            }
            (KeyType::Public, Material::OkpPublic { x: x.to_vec() })
        }
        "spki" => {
            check_usages(name, KeyType::Public, usages)?;
            let x = okp_spki(name, bytes()?).ok_or_else(|| Error::data(""))?;
            (KeyType::Public, Material::OkpPublic { x })
        }
        "pkcs8" => {
            check_usages(name, KeyType::Private, usages)?;
            if !ed {
                let d = native::x25519_private_from_pkcs8(bytes()?).map_err(|_| Error::data(""))?;
                let x = x25519_public(&d).ok_or_else(|| Error::data(""))?;
                return import_okp_key(
                    keys,
                    name,
                    KeyType::Private,
                    Material::OkpPrivate { d, x },
                    extractable,
                    usages,
                );
            }
            use aws_lc_rs::encoding::{AsBigEndian, Curve25519SeedBin};
            use signature::KeyPair;
            let pair = signature::Ed25519KeyPair::from_pkcs8_maybe_unchecked(bytes()?)
                .map_err(|_| Error::data(""))?;
            let seed = pair.seed().map_err(|_| Error::data(""))?;
            let d = AsBigEndian::<Curve25519SeedBin>::as_be_bytes(&seed)
                .map_err(|_| Error::data(""))?;
            let x = pair.public_key().as_ref().to_vec();
            (
                KeyType::Private,
                Material::OkpPrivate {
                    d: d.as_ref().to_vec(),
                    x,
                },
            )
        }
        "jwk" => {
            let Value::Dict(_) = data else {
                return Err(Error::type_error(
                    "Key data must be an object for JWK import",
                ));
            };
            let jwk = jwk::Jwk(data);
            jwk.check("OKP", None, extractable, usages)?;
            // Ed25519's `alg`, if given, Ed25519 or EdDSA (probed).
            if ed
                && jwk
                    .str("alg")
                    .is_some_and(|a| a != "Ed25519" && a != "EdDSA")
            {
                return Err(Error::data(
                    "The JWK \"alg\" member was inconsistent with that specified by the Web Crypto call",
                ));
            }
            match jwk.str("crv") {
                None => return Err(Error::data("The required JWK member \"crv\" was missing")),
                Some(crv) if crv != name => {
                    return Err(Error::data(
                        "The JWK's \"crv\" member specifies a different curve than requested",
                    ));
                }
                Some(_) => {}
            }
            let private = jwk.str("d").is_some();
            check_usages(
                name,
                if private {
                    KeyType::Private
                } else {
                    KeyType::Public
                },
                usages,
            )?;
            let x = jwk.bytes("x")?;
            if x.len() != 32 || !public_ok(&x) {
                return Err(Error::data(""));
            }
            if private {
                let d = jwk.bytes("d")?;
                let public = if ed {
                    ed25519_public(&d)
                } else {
                    x25519_public(&d)
                };
                if d.len() != 32 || public.as_deref() != Some(x.as_slice()) {
                    return Err(Error::data(""));
                }
                (KeyType::Private, Material::OkpPrivate { d, x })
            } else {
                (KeyType::Public, Material::OkpPublic { x })
            }
        }
        _ => {
            return Err(Error::not_supported(
                "Unsupported import key format for algorithm",
            ));
        }
    };
    import_okp_key(keys, name, kind, material, extractable, usages)
}

fn import_okp_key(
    keys: &mut Keys,
    name: &'static str,
    kind: KeyType,
    material: Material,
    extractable: bool,
    usages: &[Usage],
) -> Result<KeyInfo, Error> {
    if kind == KeyType::Private {
        empty_usages(KeyType::Private, usages)?;
    }
    let key = Key {
        kind,
        extractable,
        algorithm: KeyAlgorithm::Okp { name },
        usages: usages.to_vec(),
        material,
    };
    Ok(keys.insert(key))
}

fn export_okp(key: &Key, name: &str, material: &Material, format: &str) -> Result<Outcome, Error> {
    let wrong = || Error::invalid_access("The key is not of the expected type");
    let failed = || Error::operation("");
    let ed = name == "Ed25519";
    let (d, x) = match material {
        Material::OkpPublic { x } => (None, x),
        Material::OkpPrivate { d, x } => (Some(d), x),
        _ => return Err(failed()),
    };
    match format {
        "raw" | "raw-public" | "spki" if d.is_some() => Err(wrong()),
        "pkcs8" if d.is_none() => Err(wrong()),
        "raw" | "raw-public" => Ok(Outcome::Bytes(x.clone())),
        "spki" if ed => {
            let parsed =
                signature::ParsedPublicKey::new(&signature::ED25519, x).map_err(|_| failed())?;
            let der = AsDer::<aws_lc_rs::encoding::PublicKeyX509Der>::as_der(&parsed)
                .map_err(|_| failed())?;
            Ok(Outcome::Bytes(der.as_ref().to_vec()))
        }
        "spki" => Ok(Outcome::Bytes([&X25519_SPKI[..], x].concat())),
        "pkcs8" if ed => {
            let pair =
                signature::Ed25519KeyPair::from_seed_and_public_key(d.ok_or_else(failed)?, x)
                    .map_err(|_| failed())?;
            let der =
                AsDer::<aws_lc_rs::encoding::Pkcs8V1Der>::as_der(&pair).map_err(|_| failed())?;
            Ok(Outcome::Bytes(der.as_ref().to_vec()))
        }
        "pkcs8" => Ok(Outcome::Bytes(
            [&X25519_PKCS8[..], d.ok_or_else(failed)?].concat(),
        )),
        "jwk" => {
            let mut members = vec![
                ("crv", JwkValue::Str(name.into())),
                ("kty", JwkValue::Str("OKP".into())),
                ("x", JwkValue::Str(jwk::encode(x))),
            ];
            if ed {
                members.push(("alg", JwkValue::Str("Ed25519".into())));
            }
            if let Some(d) = d {
                members.push(("d", JwkValue::Str(jwk::encode(d))));
            }
            Ok(Outcome::Jwk(jwk::exported(
                members,
                key.extractable,
                &key.usages,
            )))
        }
        _ => Err(Error::not_supported(
            "Unsupported export key format for algorithm",
        )),
    }
}

/// ML-DSA's known gap (G255 step 5's ruling): a non-empty context,
/// which aws-lc-rs's stable API cannot pass.
pub(crate) const ML_DSA_CONTEXT_GAP: &str = "ML-DSA with a non-empty context is not supported";

/// ML-DSA `name`'s signing and verifying algorithms (aws-lc-rs's).
fn ml_dsa(
    name: &str,
) -> Option<(
    &'static signature::PqdsaSigningAlgorithm,
    &'static signature::PqdsaVerificationAlgorithm,
)> {
    match name {
        "ML-DSA-44" => Some((&signature::ML_DSA_44_SIGNING, &signature::ML_DSA_44)),
        "ML-DSA-65" => Some((&signature::ML_DSA_65_SIGNING, &signature::ML_DSA_65)),
        "ML-DSA-87" => Some((&signature::ML_DSA_87_SIGNING, &signature::ML_DSA_87)),
        _ => None,
    }
}

/// ML-KEM `name`'s parameter set: aws-lc-rs's, and the library's own
/// for a key made from its seed (`native`).
fn ml_kem(name: &str) -> Option<(&'static kem::Algorithm<kem::AlgorithmId>, native::MlKem)> {
    match name {
        "ML-KEM-768" => Some((&kem::ML_KEM_768, native::MlKem::K768)),
        "ML-KEM-1024" => Some((&kem::ML_KEM_1024, native::MlKem::K1024)),
        _ => None,
    }
}

/// A post-quantum algorithm's keys (ML-DSA's, step 5b, and ML-KEM's,
/// 5c), which share their forms: a seed kept as the private key, its
/// raw-seed and its PKCS#8 of the seed alone, the public key raw and in
/// its SPKI, and an AKP JWK. Its OID is 2.16.840.1.101.3.4.`arc`.`last`.
#[derive(Clone, Copy)]
struct Pq {
    name: &'static str,
    arc: u8,
    last: u8,
    seed_len: usize,
    public_len: usize,
    kem: bool,
}

/// The post-quantum algorithm named `name`.
fn pq(name: &str) -> Option<Pq> {
    let (name, arc, last, seed_len, public_len, kem) = match name {
        "ML-DSA-44" => ("ML-DSA-44", 0x03, 0x11, 32, 1312, false),
        "ML-DSA-65" => ("ML-DSA-65", 0x03, 0x12, 32, 1952, false),
        "ML-DSA-87" => ("ML-DSA-87", 0x03, 0x13, 32, 2592, false),
        "ML-KEM-768" => ("ML-KEM-768", 0x04, 0x02, 64, 1184, true),
        "ML-KEM-1024" => ("ML-KEM-1024", 0x04, 0x03, 64, 1568, true),
        _ => return None,
    };
    Some(Pq {
        name,
        arc,
        last,
        seed_len,
        public_len,
        kem,
    })
}

impl Pq {
    fn key_algorithm(self) -> KeyAlgorithm {
        if self.kem {
            KeyAlgorithm::MlKem { name: self.name }
        } else {
            KeyAlgorithm::MlDsa { name: self.name }
        }
    }

    /// Its AlgorithmIdentifier: the OID, no parameters (RFC 9881, the
    /// ML-KEM draft's alike).
    fn algorithm_id(self) -> [u8; 13] {
        [
            0x30, 0x0b, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, self.arc, self.last,
        ]
    }

    /// A public key's SPKI.
    fn spki(self, public: &[u8]) -> Vec<u8> {
        let mut bits = vec![0x03];
        der_length(&mut bits, public.len() + 1);
        bits.push(0);
        bits.extend_from_slice(public);
        let id = self.algorithm_id();
        let mut out = vec![0x30];
        der_length(&mut out, id.len() + bits.len());
        out.extend_from_slice(&id);
        out.extend_from_slice(&bits);
        out
    }

    /// A private key's PKCS#8, of its seed alone (the `seed` choice,
    /// `[0] IMPLICIT OCTET STRING`): the only form the reference writes
    /// or reads (probed: 54 bytes for ML-DSA, 86 for ML-KEM; the
    /// expanded key and the pair of both refused).
    fn pkcs8(self, seed: &[u8]) -> Vec<u8> {
        let n = self.seed_len as u8;
        let mut out = vec![0x30, 20 + n, 0x02, 0x01, 0x00];
        out.extend_from_slice(&self.algorithm_id());
        out.extend_from_slice(&[0x04, n + 2, 0x80, n]);
        out.extend_from_slice(seed);
        out
    }

    /// The public key its seed makes (FIPS 204's ML-DSA.KeyGen_internal,
    /// FIPS 203's ML-KEM.KeyGen_internal); `None` for a seed of another
    /// length.
    fn public_of(self, seed: &[u8]) -> Option<Vec<u8>> {
        if seed.len() != self.seed_len {
            return None;
        }
        if let Some((_, kind)) = ml_kem(self.name) {
            let (_secret, public) = native::ml_kem_from_seed(kind, seed).ok()?;
            return Some(public);
        }
        use signature::KeyPair;
        let (signing, _) = ml_dsa(self.name)?;
        let pair = signature::PqdsaKeyPair::from_seed(signing, seed).ok()?;
        Some(pair.public_key().as_ref().to_vec())
    }

    /// Whether `public` is one of its public keys: its length, and for
    /// ML-KEM FIPS 203's check that each coefficient is below q (the
    /// reference refuses a key that fails it, probed), which the library
    /// makes as it encapsulates (FIPS 203 section 7.2's input check), not
    /// as it takes the key: a throwaway encapsulation makes it.
    fn public_ok(self, public: &[u8]) -> bool {
        public.len() == self.public_len
            && ml_kem(self.name).is_none_or(|(algorithm, _)| {
                kem::EncapsulationKey::new(algorithm, public)
                    .is_ok_and(|key| key.encapsulate().is_ok())
            })
    }
}

/// ML-DSA's context, checked as the reference checks it (probed): over
/// 255 bytes an OperationError; any other non-empty, the known gap.
fn ml_dsa_context(alg: &Normalized) -> Result<(), Error> {
    let context = alg.bytes("context").unwrap_or(&[]);
    if context.len() > 255 {
        return Err(Error::operation(""));
    }
    if !context.is_empty() {
        return Err(Error::not_supported(ML_DSA_CONTEXT_GAP));
    }
    Ok(())
}

/// An ML-DSA signature of `data` by the key of `seed`: FIPS 204's
/// hedged signing, so never the same twice (as the reference's).
fn ml_dsa_sign(name: &str, seed: &[u8], alg: &Normalized, data: &[u8]) -> Result<Vec<u8>, Error> {
    ml_dsa_context(alg)?;
    let failed = || Error::operation("");
    let (signing, _) = ml_dsa(name).ok_or_else(failed)?;
    let pair = signature::PqdsaKeyPair::from_seed(signing, seed).map_err(|_| failed())?;
    let mut out = vec![0u8; signing.signature_len()];
    let len = pair.sign(data, &mut out).map_err(|_| failed())?;
    out.truncate(len);
    Ok(out)
}

/// An ML-KEM encapsulation to the public key `public`: its ciphertext
/// and shared secret (32 bytes), aws-lc-rs's.
fn ml_kem_encapsulate(name: &str, public: &[u8]) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let failed = || Error::operation("");
    let (algorithm, _) = ml_kem(name).ok_or_else(failed)?;
    let key = kem::EncapsulationKey::new(algorithm, public).map_err(|_| failed())?;
    let (ciphertext, shared) = key.encapsulate().map_err(|_| failed())?;
    Ok((ciphertext.as_ref().to_vec(), shared.as_ref().to_vec()))
}

/// An ML-KEM decapsulation of `ciphertext` with the key of `seed`: the
/// key expanded from its seed (`native`, the expanded bytes overwritten
/// as they go) and decapsulated by aws-lc-rs. A ciphertext of another
/// length is a DataError with no message (the reference's, probed); a
/// wrong one gives FIPS 203's implicit rejection, never an error.
fn ml_kem_decapsulate(name: &str, seed: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, Error> {
    let failed = || Error::operation("");
    let (algorithm, kind) = ml_kem(name).ok_or_else(failed)?;
    if ciphertext.len() != kind.ciphertext_len() {
        return Err(Error::data(""));
    }
    let (secret, _) = native::ml_kem_from_seed(kind, seed).map_err(|_| failed())?;
    let key = kem::DecapsulationKey::new(algorithm, secret.bytes()).map_err(|_| failed())?;
    let shared = key
        .decapsulate(kem::Ciphertext::from(ciphertext))
        .map_err(|_| failed())?;
    Ok(shared.as_ref().to_vec())
}

/// A post-quantum key pair, made from random bytes as FIPS 204's
/// ML-DSA.KeyGen and FIPS 203's ML-KEM.KeyGen make it, the seed kept
/// (its raw-seed and PKCS#8).
fn generate_pq(
    keys: &mut Keys,
    alg: &Normalized,
    extractable: bool,
    usages: &[Usage],
) -> Result<Outcome, Error> {
    let public_allowed = allowed(alg.name, KeyType::Public);
    let private_allowed = allowed(alg.name, KeyType::Private);
    if usages
        .iter()
        .any(|u| !public_allowed.contains(u) && !private_allowed.contains(u))
    {
        return Err(Error::syntax(
            "Cannot create a key using the specified key usages.",
        ));
    }
    let failed = || Error::operation("");
    let pq = pq(alg.name).ok_or_else(failed)?;
    let seed = random(pq.seed_len)?;
    let public = pq.public_of(&seed).ok_or_else(failed)?;
    let public_usages: Vec<Usage> = usages
        .iter()
        .copied()
        .filter(|u| public_allowed.contains(u))
        .collect();
    let private_usages: Vec<Usage> = usages
        .iter()
        .copied()
        .filter(|u| private_allowed.contains(u))
        .collect();
    empty_usages(KeyType::Private, &private_usages)?;
    let algorithm = pq.key_algorithm();
    let public_key = Key {
        kind: KeyType::Public,
        extractable: true,
        algorithm: algorithm.clone(),
        usages: public_usages,
        material: Material::PqPublic {
            public: public.clone(),
        },
    };
    let private_key = Key {
        kind: KeyType::Private,
        extractable,
        algorithm,
        usages: private_usages,
        material: Material::PqPrivate { seed, public },
    };
    Ok(Outcome::Pair(
        keys.insert(public_key),
        keys.insert(private_key),
    ))
}

/// A post-quantum key's import, as the reference checks it (probed):
/// the public key raw (`raw-public`) or in an SPKI, the private key its
/// seed (`raw-seed`) or the seed's PKCS#8, or a JWK (`AKP`: `pub`, and
/// the seed as `priv`, the two agreeing); a length or encoding refused a
/// DataError with no message, as the reference's.
fn import_pq(
    keys: &mut Keys,
    format: &str,
    data: &Value,
    alg: &Normalized,
    extractable: bool,
    usages: &[Usage],
) -> Result<KeyInfo, Error> {
    let name = alg.name;
    let pq = pq(name).ok_or_else(|| Error::operation(""))?;
    let bad = || Error::data("");
    let bytes = || match data {
        Value::Bytes(bytes) => Ok(bytes.as_slice()),
        _ => Err(Error::type_error(
            "Key data must be a BufferSource for non-JWK formats",
        )),
    };
    let private = |seed: &[u8]| {
        let public = pq.public_of(seed).ok_or_else(bad)?;
        Ok::<_, Error>((
            KeyType::Private,
            Material::PqPrivate {
                seed: seed.to_vec(),
                public,
            },
        ))
    };
    let (kind, material) = match format {
        "raw-public" => {
            check_usages(name, KeyType::Public, usages)?;
            let public = bytes()?;
            if !pq.public_ok(public) {
                return Err(bad());
            }
            (
                KeyType::Public,
                Material::PqPublic {
                    public: public.to_vec(),
                },
            )
        }
        "spki" => {
            check_usages(name, KeyType::Public, usages)?;
            let der = bytes()?;
            let prefix = pq.spki(&vec![0; pq.public_len]);
            let prefix = &prefix[..prefix.len() - pq.public_len];
            if der.len() != prefix.len() + pq.public_len
                || !der.starts_with(prefix)
                || !pq.public_ok(&der[prefix.len()..])
            {
                return Err(bad());
            }
            (
                KeyType::Public,
                Material::PqPublic {
                    public: der[prefix.len()..].to_vec(),
                },
            )
        }
        "raw-seed" => {
            check_usages(name, KeyType::Private, usages)?;
            private(bytes()?)?
        }
        "pkcs8" => {
            check_usages(name, KeyType::Private, usages)?;
            let der = bytes()?;
            let prefix = pq.pkcs8(&[]);
            if der.len() != prefix.len() + pq.seed_len || !der.starts_with(&prefix) {
                return Err(bad());
            }
            private(&der[prefix.len()..])?
        }
        "jwk" => {
            let Value::Dict(_) = data else {
                return Err(Error::type_error(
                    "Key data must be an object for JWK import",
                ));
            };
            let jwk = jwk::Jwk(data);
            jwk.check("AKP", Some(name), extractable, usages)?;
            let has_private = jwk.str("priv").is_some();
            check_usages(
                name,
                if has_private {
                    KeyType::Private
                } else {
                    KeyType::Public
                },
                usages,
            )?;
            let public = jwk.bytes("pub")?;
            if !pq.public_ok(&public) {
                return Err(bad());
            }
            if has_private {
                let mut seed = jwk.bytes("priv")?;
                let made = private(&seed);
                seed.iter_mut().for_each(|b| *b = 0);
                std::hint::black_box(&seed);
                let made = made?;
                match &made.1 {
                    Material::PqPrivate {
                        public: from_seed, ..
                    } if *from_seed == public => made,
                    _ => return Err(bad()),
                }
            } else {
                (KeyType::Public, Material::PqPublic { public })
            }
        }
        _ => {
            return Err(Error::not_supported(
                "Unsupported import key format for algorithm",
            ));
        }
    };
    if kind == KeyType::Private {
        empty_usages(KeyType::Private, usages)?;
    }
    Ok(keys.insert(Key {
        kind,
        extractable,
        algorithm: pq.key_algorithm(),
        usages: usages.to_vec(),
        material,
    }))
}

fn export_pq(key: &Key, name: &str, material: &Material, format: &str) -> Result<Outcome, Error> {
    let wrong = || Error::invalid_access("The key is not of the expected type");
    let failed = || Error::operation("");
    let pq = pq(name).ok_or_else(failed)?;
    let (seed, public) = match material {
        Material::PqPublic { public } => (None, public),
        Material::PqPrivate { seed, public } => (Some(seed), public),
        _ => return Err(failed()),
    };
    match (format, seed) {
        ("raw-public" | "spki", Some(_)) | ("raw-seed" | "pkcs8", None) => Err(wrong()),
        ("raw-public", None) => Ok(Outcome::Bytes(public.clone())),
        ("spki", None) => Ok(Outcome::Bytes(pq.spki(public))),
        ("raw-seed", Some(seed)) => Ok(Outcome::Bytes(seed.clone())),
        ("pkcs8", Some(seed)) => Ok(Outcome::Bytes(pq.pkcs8(seed))),
        ("jwk", _) => {
            let mut members = vec![
                ("alg", JwkValue::Str(name.into())),
                ("kty", JwkValue::Str("AKP".into())),
                ("pub", JwkValue::Str(jwk::encode(public))),
            ];
            if let Some(seed) = seed {
                members.push(("priv", JwkValue::Str(jwk::encode(seed))));
            }
            Ok(Outcome::Jwk(jwk::exported(
                members,
                key.extractable,
                &key.usages,
            )))
        }
        _ => Err(Error::not_supported(
            "Unsupported export key format for algorithm",
        )),
    }
}

/// X25519's shared secret with the `public` key, cut as ECDH's (an
/// all-zero secret, from a small-order point, the library's refusal: an
/// OperationError, as the reference's).
fn x25519_bits(
    keys: &Keys,
    key: &Key,
    alg: &Normalized,
    length: Option<u32>,
) -> Result<Vec<u8>, Error> {
    let Material::OkpPrivate { d, .. } = &key.material else {
        return Err(Error::operation(""));
    };
    let (public, most) = agreement_public(keys, alg)?;
    let Material::OkpPublic { x } = &public.material else {
        return Err(Error::operation(""));
    };
    let length = length.unwrap_or(most);
    if length > most {
        return Err(Error::operation(
            "The length provided for X25519 is too large.",
        ));
    }
    let x25519 = &aws_lc_rs::agreement::X25519;
    let private = aws_lc_rs::agreement::PrivateKey::from_private_key(x25519, d)
        .map_err(|_| Error::operation(""))?;
    let peer = aws_lc_rs::agreement::UnparsedPublicKey::new(x25519, x);
    let mut secret =
        aws_lc_rs::agreement::agree(&private, peer, Error::operation(""), |s| Ok(s.to_vec()))?;
    secret.truncate(length.div_ceil(8) as usize);
    if !length.is_multiple_of(8)
        && let Some(last) = secret.last_mut()
    {
        *last &= 0xffu8 << (8 - length % 8);
    }
    Ok(secret)
}

/// A key's record for a structured clone: JSON of its import's format,
/// bytes (base64url), algorithm, extractable and usages.
pub(crate) fn key_record(key: &Key) -> Option<Vec<u8>> {
    use devtools::Json;
    let format = match &key.material {
        Material::Secret(_) => "raw-secret",
        Material::RsaPublic { .. }
        | Material::EcPublic { .. }
        | Material::OkpPublic { .. }
        | Material::PqPublic { .. } => "spki",
        Material::RsaPrivate(_)
        | Material::EcPrivate { .. }
        | Material::OkpPrivate { .. }
        | Material::PqPrivate { .. } => "pkcs8",
    };
    // A secret's bytes as they are (a KDF's no export gives); any other
    // key's export, whatever its `extractable`.
    let mut bytes = match &key.material {
        Material::Secret(bytes) => bytes.clone(),
        _ => {
            let mut whole = key.clone();
            whole.extractable = true;
            match export(&whole, format).ok()? {
                Outcome::Bytes(bytes) => bytes,
                _ => return None,
            }
        }
    };
    let name = Json::String(key.algorithm.name().to_owned());
    let algorithm = match &key.algorithm {
        KeyAlgorithm::Hmac { hash, length } => Json::object([
            ("name", name),
            ("hash", Json::String((*hash).to_owned())),
            ("length", Json::Number(f64::from(*length))),
        ]),
        KeyAlgorithm::Rsa { hash, .. } => {
            Json::object([("name", name), ("hash", Json::String((*hash).to_owned()))])
        }
        KeyAlgorithm::Ec { curve, .. } => Json::object([
            ("name", name),
            ("namedCurve", Json::String((*curve).to_owned())),
        ]),
        KeyAlgorithm::Aes { .. }
        | KeyAlgorithm::Kdf { .. }
        | KeyAlgorithm::Okp { .. }
        | KeyAlgorithm::Chacha
        | KeyAlgorithm::MlDsa { .. }
        | KeyAlgorithm::MlKem { .. } => Json::object([("name", name)]),
    };
    let record = Json::object([
        ("f", Json::String(format.to_owned())),
        ("d", Json::String(jwk::encode(&bytes))),
        ("a", algorithm),
        ("e", Json::Bool(key.extractable)),
        (
            "u",
            Json::Array(
                key.usages
                    .iter()
                    .map(|u| Json::String(u.name().to_owned()))
                    .collect(),
            ),
        ),
    ])
    .write();
    bytes.iter_mut().for_each(|b| *b = 0);
    std::hint::black_box(&bytes);
    Some(record.into_bytes())
}

/// A key made again from its record, by importKey and every check of its.
pub(crate) fn key_from_record(keys: &mut Keys, record: &[u8]) -> Result<KeyInfo, Error> {
    use devtools::Json;
    let refused = || Error::data("The CryptoKey could not be deserialized");
    let text = std::str::from_utf8(record).map_err(|_| refused())?;
    let json = Json::parse(text).map_err(|_| refused())?;
    let text_of = |name: &str| json.get(name).and_then(Json::as_str).ok_or_else(refused);
    let format = text_of("f")?;
    if !matches!(format, "raw" | "raw-secret" | "spki" | "pkcs8") {
        return Err(refused());
    }
    let mut data = jwk::decode(text_of("d")?).ok_or_else(refused)?;
    let Some(Json::Object(members)) = json.get("a") else {
        return Err(refused());
    };
    let algorithm = Value::Dict(
        members
            .iter()
            .map(|(name, value)| {
                let value = match value {
                    Json::String(s) => Value::Str(s.clone()),
                    Json::Number(n) => Value::Num(*n),
                    _ => Value::Other,
                };
                (name.clone(), value)
            })
            .collect(),
    );
    let extractable = json.get("e").and_then(Json::as_bool).ok_or_else(refused)?;
    let usages: Vec<Usage> = json
        .get("u")
        .and_then(Json::as_array)
        .ok_or_else(refused)?
        .iter()
        .map(|u| u.as_str().and_then(Usage::parse).ok_or_else(refused))
        .collect::<Result<_, _>>()?;
    let out = run(
        keys,
        Call::ImportKey {
            format: format.to_owned(),
            key_data: Value::Bytes(data.clone()),
            algorithm,
            extractable,
            usages,
        },
    );
    data.iter_mut().for_each(|b| *b = 0);
    std::hint::black_box(&data);
    match out {
        Ok(Outcome::Key(info)) => Ok(info),
        _ => Err(refused()),
    }
}

/// An RSA public key's SPKI, aws-lc-rs's encoding of it.
fn spki_of(n: &[u8], e: &[u8]) -> Result<Vec<u8>, Error> {
    let key =
        rsa::PublicKey::from_der(&rsa_public_key_der(n, e)).map_err(|_| Error::operation(""))?;
    let der = key.as_der().map_err(|_| Error::operation(""))?;
    Ok(der.as_ref().to_vec())
}

/// The reason an RSA-OAEP key outside aws-lc-rs's range is refused: a
/// known gap (the lead's ruling, 2026-11-23), where the reference works;
/// a NotSupportedError, as ECDSA's gap, for the fallbacks keyed on it.
pub(crate) const OAEP_GAP: &str =
    "RSA-OAEP with a key under 2048 or over 8192 bits is not supported";

/// RSA-OAEP's encryption with a public key or decryption with a private
/// one, the hash's MGF1, the label given or none (an empty one is none):
/// aws-lc-rs's, for a key of 2048 to 8192 bits alone; any other an
/// OperationError (a known gap). Any failure else an OperationError with
/// no message, as the reference's.
fn oaep(key: &Key, alg: &Normalized, data: &[u8], encrypt: bool) -> Result<Outcome, Error> {
    use rsa::{
        OAEP_SHA1_MGF1SHA1, OAEP_SHA256_MGF1SHA256, OAEP_SHA384_MGF1SHA384, OAEP_SHA512_MGF1SHA512,
        OaepPrivateDecryptingKey, OaepPublicEncryptingKey, PrivateDecryptingKey,
        PublicEncryptingKey,
    };
    let KeyAlgorithm::Rsa { hash, .. } = &key.algorithm else {
        return Err(Error::operation(""));
    };
    let algorithm = match *hash {
        "SHA-1" => &OAEP_SHA1_MGF1SHA1,
        "SHA-384" => &OAEP_SHA384_MGF1SHA384,
        "SHA-512" => &OAEP_SHA512_MGF1SHA512,
        _ => &OAEP_SHA256_MGF1SHA256,
    };
    // An empty label is none: aws-lc-rs sets none for it.
    let label = alg.bytes("label");
    let failed = || Error::operation("");
    match (&key.material, encrypt) {
        (Material::RsaPublic { n, e }, true) => {
            if !SAFE_PRIVATE.contains(&bit_length(n)) {
                return Err(Error::not_supported(OAEP_GAP));
            }
            let public = PublicEncryptingKey::from_der(&spki_of(n, e)?).map_err(|_| failed())?;
            let sealing = OaepPublicEncryptingKey::new(public).map_err(|_| failed())?;
            let mut out = vec![0u8; sealing.ciphertext_size()];
            let written = sealing
                .encrypt(algorithm, data, &mut out, label)
                .map_err(|_| failed())?
                .len();
            out.truncate(written);
            Ok(Outcome::Bytes(out))
        }
        (Material::RsaPrivate(parts), false) => {
            if !SAFE_PRIVATE.contains(&bit_length(&parts.n)) {
                return Err(Error::not_supported(OAEP_GAP));
            }
            let mut pkcs8 = pkcs8_of(parts)?;
            let private = PrivateDecryptingKey::from_pkcs8(&pkcs8).map_err(|_| failed());
            pkcs8.iter_mut().for_each(|b| *b = 0);
            std::hint::black_box(&pkcs8);
            let opening = OaepPrivateDecryptingKey::new(private?).map_err(|_| failed())?;
            let mut out = vec![0u8; opening.min_output_size()];
            let plain = opening
                .decrypt(algorithm, data, &mut out, label)
                .map_err(|_| failed())?
                .to_vec();
            out.iter_mut().for_each(|b| *b = 0);
            Ok(Outcome::Bytes(plain))
        }
        _ => Err(failed()),
    }
}

/// AES-GCM through the library's own calls: an IV of any length, a tag of
/// any length the reference allows (checked already).
fn native_gcm(
    key: &[u8],
    iv: &[u8],
    aad: &[u8],
    data: &[u8],
    tag_bits: u32,
    encrypt: bool,
) -> Result<Outcome, Error> {
    let tag = tag_bits as usize / 8;
    let out = if encrypt {
        native::gcm_seal(key, iv, aad, data, tag)
    } else {
        native::gcm_open(key, iv, aad, data, tag)
    };
    out.map(Outcome::Bytes).map_err(|_| Error::operation(""))
}

/// Where an RSA operation runs (the lead's ruling (a), G255): aws-lc-rs's
/// safe API for a key inside the range its parameters take, the
/// library's own calls (`native`) for any other.
#[derive(Debug)]
pub(crate) enum Route {
    Safe(&'static signature::RsaParameters),
    Native(native::Hash),
}

/// RSASSA-PKCS1-v1_5 verification's route for a key of `bits` bits:
/// aws-lc-rs verifies SHA-1, SHA-256 and SHA-512 at 1024 to 8192 bits,
/// SHA-384 at 2048 to 8192 (its `RSA_PKCS1_*` parameters); the reference
/// verifies from 512 bits to 16384.
pub(crate) fn pkcs1_verify_route(hash: &str, bits: u32) -> Route {
    let (safe, native, low): (&'static signature::RsaParameters, _, u32) = match hash {
        "SHA-1" => (
            &signature::RSA_PKCS1_1024_8192_SHA1_FOR_LEGACY_USE_ONLY,
            native::Hash::Sha1,
            1024,
        ),
        "SHA-384" => (
            &signature::RSA_PKCS1_2048_8192_SHA384,
            native::Hash::Sha384,
            2048,
        ),
        "SHA-512" => (
            &signature::RSA_PKCS1_1024_8192_SHA512_FOR_LEGACY_USE_ONLY,
            native::Hash::Sha512,
            1024,
        ),
        _ => (
            &signature::RSA_PKCS1_1024_8192_SHA256_FOR_LEGACY_USE_ONLY,
            native::Hash::Sha256,
            1024,
        ),
    };
    if (low..=8192).contains(&bits) {
        Route::Safe(safe)
    } else {
        Route::Native(native)
    }
}

fn verify(key: &Key, alg: &Normalized, signature: &[u8], data: &[u8]) -> Result<bool, Error> {
    match (&key.algorithm, &key.material) {
        (KeyAlgorithm::Hmac { hash, .. }, Material::Secret(bytes)) => {
            let k = hmac::Key::new(hmac_algorithm(hash), bytes);
            Ok(hmac::verify(&k, data, signature).is_ok())
        }
        (
            KeyAlgorithm::Rsa {
                name: "RSASSA-PKCS1-v1_5",
                hash,
                ..
            },
            Material::RsaPublic { n, e },
        ) => match pkcs1_verify_route(hash, bit_length(n)) {
            Route::Safe(parameters) => {
                let components = signature::RsaPublicKeyComponents { n, e };
                Ok(components.verify(parameters, data, signature).is_ok())
            }
            Route::Native(native_hash) => {
                let digest = digest::digest(digest_algorithm(hash), data);
                native::rsa_verify_pkcs1(
                    &rsa_public_key_der(n, e),
                    native_hash,
                    digest.as_ref(),
                    signature,
                )
                .map_err(|_| Error::operation(""))
            }
        },
        (
            KeyAlgorithm::Rsa {
                name: "RSA-PSS",
                hash,
                ..
            },
            Material::RsaPublic { n, e },
        ) => {
            let salt = alg.num("saltLength").unwrap_or(0);
            match pss_route(hash, bit_length(n), salt) {
                Some((_, parameters)) => {
                    let components = signature::RsaPublicKeyComponents { n, e };
                    Ok(components.verify(parameters, data, signature).is_ok())
                }
                None => {
                    let digest = digest::digest(digest_algorithm(hash), data);
                    native::rsa_verify_pss(
                        &rsa_public_key_der(n, e),
                        native_hash(hash),
                        digest.as_ref(),
                        salt,
                        signature,
                    )
                    .map_err(|_| Error::operation(""))
                }
            }
        }
        (
            KeyAlgorithm::Ec {
                name: "ECDSA",
                curve,
            },
            Material::EcPublic { point },
        ) => ecdsa_verify(
            curve,
            alg.hash().unwrap_or("SHA-256"),
            point,
            signature,
            data,
        ),
        // A signature of another length aws-lc-rs refuses: false.
        (KeyAlgorithm::Okp { name: "Ed25519" }, Material::OkpPublic { x }) => {
            Ok(signature::UnparsedPublicKey::new(&signature::ED25519, x)
                .verify(data, signature)
                .is_ok())
        }
        (KeyAlgorithm::MlDsa { name }, Material::PqPublic { public }) => {
            ml_dsa_context(alg)?;
            let (_, verifying) = ml_dsa(name).ok_or_else(|| Error::operation(""))?;
            Ok(signature::UnparsedPublicKey::new(verifying, public)
                .verify(data, signature)
                .is_ok())
        }
        // Nothing else verifies (the usages refuse it first).
        _ => Err(Error::operation("")),
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
        CHACHA => {
            check_usages(CHACHA, KeyType::Secret, usages)?;
            empty_usages(KeyType::Secret, usages)?;
            Ok(Outcome::Key(keys.insert(Key {
                kind: KeyType::Secret,
                extractable,
                algorithm: KeyAlgorithm::Chacha,
                usages: usages.to_vec(),
                material: Material::Secret(random(32)?),
            })))
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
        "RSASSA-PKCS1-v1_5" | "RSA-PSS" | "RSA-OAEP" => {
            generate_rsa(keys, alg, extractable, usages)
        }
        "ECDSA" | "ECDH" => generate_ec(keys, alg, extractable, usages),
        "Ed25519" | "X25519" => generate_okp(keys, alg, extractable, usages),
        "ML-DSA-44" | "ML-DSA-65" | "ML-DSA-87" | "ML-KEM-768" | "ML-KEM-1024" => {
            generate_pq(keys, alg, extractable, usages)
        }
        _ => Err(Error::not_supported("Unsupported operation")),
    }
}

/// RSA generation's checks that need no key, the modulus then the
/// exponent (probed order): generateKey's and `supports`'s alike.
fn rsa_params(alg: &Normalized) -> Result<(u32, u64), Error> {
    let bits = alg.num("modulusLength").unwrap_or(0);
    if !bits.is_multiple_of(8) || !(256..=8192).contains(&bits) {
        return Err(Error::operation(
            "The modulus length must be a multiple of 8 bits and >= 256 and <= 8192",
        ));
    }
    let exponent = strip_zeros(alg.bytes("publicExponent").unwrap_or_default().to_vec());
    let e: u64 = match exponent.as_slice() {
        [3] => 3,
        [1, 0, 1] => 65537,
        _ => {
            return Err(Error::operation(
                "The \"publicExponent\" must be either 3 or 65537",
            ));
        }
    };
    Ok((bits, e))
}

/// The largest RSA modulus the library parses (aws-lc's
/// OPENSSL_RSA_MAX_MODULUS_BITS): a PSS salt past what it fits, with
/// SHA-1's 20 bytes, fits no key.
const RSA_MOST_BITS: u32 = 16384;

/// The least modulus the library makes a key of: a smaller one is an
/// OperationError at generation (the reference's too, probed).
const RSA_LEAST_GENERATED: u32 = 512;

/// An RSA key pair, in the reference's order of checks (probed): the
/// usages' kinds, the modulus, the exponent, the generation (the
/// reference makes no key under 512 bits: an OperationError), then a
/// private key with no usage.
fn generate_rsa(
    keys: &mut Keys,
    alg: &Normalized,
    extractable: bool,
    usages: &[Usage],
) -> Result<Outcome, Error> {
    let public_allowed = allowed(alg.name, KeyType::Public);
    let private_allowed = allowed(alg.name, KeyType::Private);
    if usages
        .iter()
        .any(|u| !public_allowed.contains(u) && !private_allowed.contains(u))
    {
        return Err(Error::syntax(
            "Cannot create a key using the specified key usages.",
        ));
    }
    let hash = alg.hash().unwrap_or("SHA-256");
    let (bits, e) = rsa_params(alg)?;
    let parts = if e == 65537 && matches!(bits, 2048 | 3072 | 4096 | 8192) {
        let size = match bits {
            2048 => rsa::KeySize::Rsa2048,
            3072 => rsa::KeySize::Rsa3072,
            4096 => rsa::KeySize::Rsa4096,
            _ => rsa::KeySize::Rsa8192,
        };
        let pair = rsa::KeyPair::generate(size).map_err(|_| Error::operation(""))?;
        let der = AsDer::<aws_lc_rs::encoding::Pkcs8V1Der>::as_der(&pair)
            .map_err(|_| Error::operation(""))?;
        native::rsa_private_from_pkcs8(der.as_ref())
            .map_err(|_| Error::operation(""))?
            .parts()
    } else {
        native::rsa_generate(bits, e)
            .map_err(|_| Error::operation(""))?
            .parts()
    };
    let public_usages: Vec<Usage> = usages
        .iter()
        .copied()
        .filter(|u| public_allowed.contains(u))
        .collect();
    let private_usages: Vec<Usage> = usages
        .iter()
        .copied()
        .filter(|u| private_allowed.contains(u))
        .collect();
    empty_usages(KeyType::Private, &private_usages)?;
    let algorithm = KeyAlgorithm::Rsa {
        name: alg.name,
        hash,
        modulus_length: bit_length(&parts.n),
        public_exponent: parts.e.clone(),
    };
    let public = Key {
        kind: KeyType::Public,
        extractable: true,
        algorithm: algorithm.clone(),
        usages: public_usages,
        material: Material::RsaPublic {
            n: parts.n.clone(),
            e: parts.e.clone(),
        },
    };
    let private = Key {
        kind: KeyType::Private,
        extractable,
        algorithm,
        usages: private_usages,
        material: Material::RsaPrivate(Box::new(parts)),
    };
    Ok(Outcome::Pair(keys.insert(public), keys.insert(private)))
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
        CHACHA => {
            check_usages(CHACHA, KeyType::Secret, usages)?;
            let bytes = match format {
                "raw-secret" => raw()?,
                "jwk" => {
                    let jwk = jwk()?;
                    jwk.check("oct", Some("C20P"), extractable, usages)?;
                    jwk.bytes("k")?
                }
                _ => {
                    return Err(Error::not_supported(
                        "Unsupported import key format for algorithm",
                    ));
                }
            };
            if bytes.len() != 32 {
                return Err(Error::data("ChaCha20-Poly1305 key data must be 256 bits"));
            }
            empty_usages(KeyType::Secret, usages)?;
            Ok(keys.insert(secret_key(KeyAlgorithm::Chacha, bytes)))
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
            let bits = bytes.len() as u64 * 8;
            let length = match alg.num("length") {
                // More than 7 bits short of its data, or past it: refused
                // (the spec's, and the reference's words, probed).
                Some(length) if u64::from(length) > bits || u64::from(length) + 8 <= bits => {
                    return Err(Error::data(
                        "The optional HMAC key length must be shorter than the key data, and by no more than 7 bits.",
                    ));
                }
                Some(length) => length,
                None => bits as u32,
            };
            empty_usages(KeyType::Secret, usages)?;
            Ok(keys.insert(secret_key(KeyAlgorithm::Hmac { hash, length }, bytes)))
        }
        "HKDF" | "PBKDF2" => {
            check_usages(alg.name, KeyType::Secret, usages)?;
            if !matches!(format, "raw" | "raw-secret") {
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
        "ECDSA" | "ECDH" => import_ec(keys, format, data, alg, extractable, usages),
        "Ed25519" | "X25519" => import_okp(keys, format, data, alg, extractable, usages),
        "ML-DSA-44" | "ML-DSA-65" | "ML-DSA-87" | "ML-KEM-768" | "ML-KEM-1024" => {
            import_pq(keys, format, data, alg, extractable, usages)
        }
        "RSASSA-PKCS1-v1_5" | "RSA-PSS" | "RSA-OAEP" => {
            let hash = alg.hash().unwrap_or("SHA-256");
            let (n, e) = match format {
                // The SPKI read by aws-lc-rs, its PKCS #1 key by the
                // library (`native`): never DER read here.
                "spki" => {
                    let key = rsa::PublicKey::from_der(&raw()?).map_err(|_| Error::data(""))?;
                    let parts =
                        native::rsa_public_parts(key.as_ref()).map_err(|_| Error::data(""))?;
                    (parts.n, parts.e)
                }
                "pkcs8" => {
                    check_usages(alg.name, KeyType::Private, usages)?;
                    let parts = pkcs8_parts(&raw()?)?;
                    return import_private(keys, alg.name, hash, parts, extractable, usages);
                }
                "jwk" => {
                    let jwk = jwk()?;
                    jwk.check(
                        "RSA",
                        Some(rsa_jwk_alg(alg.name, hash)),
                        extractable,
                        usages,
                    )?;
                    if jwk.str("d").is_some() {
                        check_usages(alg.name, KeyType::Private, usages)?;
                        let parts = jwk_private_parts(&jwk)?;
                        return import_private(keys, alg.name, hash, parts, extractable, usages);
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
            // A key the library refuses is refused here, so that whatever
            // imports can be verified with (or its verification refused),
            // never answered false for want of a key.
            if native::rsa_public_parts(&rsa_public_key_der(&n, &e)).is_err() {
                return Err(Error::data("The RSA key is not valid"));
            }
            let key = Key {
                kind: KeyType::Public,
                extractable,
                algorithm: KeyAlgorithm::Rsa {
                    name: alg.name,
                    hash,
                    modulus_length: bit_length(&n),
                    public_exponent: e.clone(),
                },
                usages: usages.to_vec(),
                material: Material::RsaPublic { n, e },
            };
            Ok(keys.insert(key))
        }
        _ => Err(Error::not_supported("Unsupported operation")),
    }
}

/// A private JWK's parts, in the reference's order of its errors
/// (probed: `n`, `e`, `d`, then `p`, `q`, `dp`, `dq`, `qi`), the library's
/// check and, inside its range, aws-lc-rs's required of them.
fn jwk_private_parts(jwk: &jwk::Jwk<'_>) -> Result<native::RsaPrivateParts, Error> {
    let mut parts = native::RsaPrivateParts::default();
    for (name, part) in [
        ("n", &mut parts.n),
        ("e", &mut parts.e),
        ("d", &mut parts.d),
        ("p", &mut parts.p),
        ("q", &mut parts.q),
        ("dp", &mut parts.dp),
        ("dq", &mut parts.dq),
        ("qi", &mut parts.qi),
    ] {
        *part = strip_zeros(jwk.bytes(name)?);
    }
    let key = native::rsa_private_from_parts(&parts).map_err(|_| Error::data(""))?;
    if SAFE_PRIVATE.contains(&bit_length(&parts.n)) {
        safe_key_pair(&parts)?;
    }
    // The library's own reading of them, leading zeros and all settled.
    Ok(key.parts())
}

/// A private key, imported.
fn import_private(
    keys: &mut Keys,
    name: &'static str,
    hash: &'static str,
    parts: native::RsaPrivateParts,
    extractable: bool,
    usages: &[Usage],
) -> Result<KeyInfo, Error> {
    empty_usages(KeyType::Private, usages)?;
    let key = Key {
        kind: KeyType::Private,
        extractable,
        algorithm: KeyAlgorithm::Rsa {
            name,
            hash,
            modulus_length: bit_length(&parts.n),
            public_exponent: parts.e.clone(),
        },
        usages: usages.to_vec(),
        material: Material::RsaPrivate(Box::new(parts)),
    };
    Ok(keys.insert(key))
}

fn strip_zeros(mut bytes: Vec<u8>) -> Vec<u8> {
    let zeros = bytes.iter().take_while(|&&b| b == 0).count();
    bytes.drain(..zeros);
    bytes
}

pub(crate) fn bit_length(n: &[u8]) -> u32 {
    match n.first() {
        Some(&first) => (n.len() as u32 - 1) * 8 + (8 - first.leading_zeros()),
        None => 0,
    }
}

/// An RSA public key's PKCS #1 `RSAPublicKey` (RFC 8017 appendix A.1.1),
/// for aws-lc-rs to read and write as SPKI: written here, never read.
pub(crate) fn rsa_public_key_der(n: &[u8], e: &[u8]) -> Vec<u8> {
    fn integer(out: &mut Vec<u8>, value: &[u8]) {
        out.push(0x02);
        let pad = value.first().is_some_and(|&b| b & 0x80 != 0);
        der_length(out, value.len() + usize::from(pad));
        if pad {
            out.push(0);
        }
        out.extend_from_slice(value);
    }
    let mut body = Vec::new();
    integer(&mut body, n);
    integer(&mut body, e);
    let mut out = vec![0x30];
    der_length(&mut out, body.len());
    out.extend_from_slice(&body);
    out
}

/// Whether `key`'s algorithm has an export at all: a KDF's has none, and
/// says so before its `extractable` is looked at, as the spec orders
/// exportKey's and wrapKey's checks (probed: "HKDF: Unsupported
/// operation: exportKey", with the method's prefix in exportKey alone).
fn exportable(key: &Key) -> Result<(), Error> {
    let name = key.algorithm.name();
    if has_export(name) {
        Ok(())
    } else {
        Err(Error::not_supported(format!(
            "{name}: Unsupported operation: exportKey"
        )))
    }
}

/// Whether algorithm `name`'s keys export at all (a KDF's do not).
fn has_export(name: &str) -> bool {
    !matches!(name, "HKDF" | "PBKDF2")
}

/// An import's checks of its algorithm that need no key data, run by
/// the import itself and by `supports`: an EC algorithm's curve.
fn import_params(alg: &Normalized) -> Result<(), Error> {
    match alg.name {
        "ECDSA" | "ECDH" => named_curve(alg, "EcKeyImportParams").map(|_| ()),
        // A length of 0 fits no key data (more than 7 bits short of any).
        "HMAC" if alg.num("length") == Some(0) => Err(Error::data("")),
        _ => Ok(()),
    }
}

/// An algorithm identifier's name: the string, or its `name` member.
pub(crate) fn algorithm_name(algorithm: &Value) -> Option<&str> {
    match algorithm {
        Value::Str(name) => Some(name),
        _ => match algorithm.get("name") {
            Value::Str(name) => Some(name),
            _ => None,
        },
    }
}

/// Whether algorithm `name`'s keys come in pairs, whose private key
/// getPublicKey takes.
fn asymmetric(name: &str) -> bool {
    matches!(
        name,
        "RSASSA-PKCS1-v1_5"
            | "RSA-PSS"
            | "RSA-OAEP"
            | "ECDSA"
            | "ECDH"
            | "Ed25519"
            | "X25519"
            | "ML-DSA-44"
            | "ML-DSA-65"
            | "ML-DSA-87"
            | "ML-KEM-768"
            | "ML-KEM-1024"
    )
}

/// getPublicKey: a private key's public key, of the usages asked
/// (probed: the reference's order and words).
fn public_key_of(keys: &mut Keys, key: u32, usages: &[Usage]) -> Result<Outcome, Error> {
    let key = keys.get(key)?;
    match key.kind {
        KeyType::Secret => {
            return Err(Error::not_supported(
                "getPublicKey can only be called with keys from an asymmetric algorithm",
            )
            .of_method());
        }
        KeyType::Public => {
            return Err(Error::invalid_access(
                "getPublicKey can only be called with a private key",
            )
            .of_method());
        }
        KeyType::Private => {}
    }
    check_usages(key.algorithm.name(), KeyType::Public, usages)?;
    let material = match &key.material {
        Material::RsaPrivate(parts) => Material::RsaPublic {
            n: parts.n.clone(),
            e: parts.e.clone(),
        },
        Material::EcPrivate { point, .. } => Material::EcPublic {
            point: point.clone(),
        },
        Material::OkpPrivate { x, .. } => Material::OkpPublic { x: x.clone() },
        Material::PqPrivate { public, .. } => Material::PqPublic {
            public: public.clone(),
        },
        _ => return Err(Error::operation("")),
    };
    let public = Key {
        kind: KeyType::Public,
        extractable: true,
        algorithm: key.algorithm.clone(),
        usages: usage_set(usages),
        material,
    };
    Ok(Outcome::Key(keys.insert(public)))
}

/// The shared key an encapsulation's or decapsulation's secret makes:
/// imported as raw-secret, as the reference imports it (probed: an
/// AES key of 256 bits whatever length was asked, HMAC's of 256), the
/// secret overwritten after.
fn shared_key(
    keys: &mut Keys,
    mut shared: Vec<u8>,
    algorithm: &Normalized,
    extractable: bool,
    usages: &[Usage],
) -> Result<Outcome, Error> {
    let info = import(
        keys,
        "raw-secret",
        &Value::Bytes(shared.clone()),
        algorithm,
        extractable,
        &usage_set(usages),
    );
    shared.iter_mut().for_each(|b| *b = 0);
    std::hint::black_box(&shared);
    Ok(Outcome::Key(info?))
}

fn export(key: &Key, format: &str) -> Result<Outcome, Error> {
    exportable(key).map_err(Error::of_method)?;
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
        (KeyAlgorithm::Chacha, Material::Secret(bytes)) => match format {
            "raw-secret" => Ok(Outcome::Bytes(bytes.clone())),
            "jwk" => Ok(Outcome::Jwk(jwk::exported(
                vec![
                    ("alg", JwkValue::Str("C20P".into())),
                    ("k", JwkValue::Str(jwk::encode(bytes))),
                    ("kty", JwkValue::Str("oct".into())),
                ],
                key.extractable,
                &key.usages,
            ))),
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
        (
            KeyAlgorithm::Okp { name },
            material @ (Material::OkpPublic { .. } | Material::OkpPrivate { .. }),
        ) => export_okp(key, name, material, format),
        (
            KeyAlgorithm::MlDsa { name } | KeyAlgorithm::MlKem { name },
            material @ (Material::PqPublic { .. } | Material::PqPrivate { .. }),
        ) => export_pq(key, name, material, format),
        (
            KeyAlgorithm::Ec { curve, .. },
            material @ (Material::EcPublic { .. } | Material::EcPrivate { .. }),
        ) => export_ec(key, curve, material, format),
        (KeyAlgorithm::Rsa { name, hash, .. }, Material::RsaPrivate(parts)) => match format {
            "pkcs8" => Ok(Outcome::Bytes(pkcs8_of(parts)?)),
            "jwk" => Ok(Outcome::Jwk(jwk::exported(
                vec![
                    ("alg", JwkValue::Str(rsa_jwk_alg(name, hash).into())),
                    ("d", JwkValue::Str(jwk::encode(&parts.d))),
                    ("dp", JwkValue::Str(jwk::encode(&parts.dp))),
                    ("dq", JwkValue::Str(jwk::encode(&parts.dq))),
                    ("e", JwkValue::Str(jwk::encode(&parts.e))),
                    ("kty", JwkValue::Str("RSA".into())),
                    ("n", JwkValue::Str(jwk::encode(&parts.n))),
                    ("p", JwkValue::Str(jwk::encode(&parts.p))),
                    ("q", JwkValue::Str(jwk::encode(&parts.q))),
                    ("qi", JwkValue::Str(jwk::encode(&parts.qi))),
                ],
                key.extractable,
                &key.usages,
            ))),
            "spki" => Err(Error::invalid_access("The key is not of the expected type")),
            _ => Err(unsupported()),
        },
        (KeyAlgorithm::Rsa { name, hash, .. }, Material::RsaPublic { n, e }) => match format {
            "pkcs8" => Err(Error::invalid_access("The key is not of the expected type")),
            "spki" => Ok(Outcome::Bytes(spki_of(n, e)?)),
            "jwk" => Ok(Outcome::Jwk(jwk::exported(
                vec![
                    ("alg", JwkValue::Str(rsa_jwk_alg(name, hash).into())),
                    ("e", JwkValue::Str(jwk::encode(e))),
                    ("kty", JwkValue::Str("RSA".into())),
                    ("n", JwkValue::Str(jwk::encode(n))),
                ],
                key.extractable,
                &key.usages,
            ))),
            _ => Err(unsupported()),
        },
        _ => Err(unsupported()),
    }
}

/// deriveBits' checks of HKDF's and PBKDF2's parameters, which need no
/// key, in the reference's order and words (probed: enforce-range,
/// dict-int): deriveBits' and `supports`'s alike. X25519's most, 256
/// bits, is `supports`'s alone here: the operation looks at its keys
/// first (`x25519_bits`).
fn derive_params(alg: &Normalized, length: Option<u32>) -> Result<(), Error> {
    let name = alg.name;
    match name {
        "HKDF" | "PBKDF2" => {
            let length = length.ok_or_else(|| {
                Error::operation(format!(
                    "No length was specified for the {name} Derive Bits operation."
                ))
            })?;
            if length % 8 != 0 {
                return Err(Error::operation(match name {
                    "HKDF" => "The length provided for HKDF is not a multiple of 8 bits.",
                    _ => "Length for PBKDF2 key derivation must be a multiple of 8 bits.",
                }));
            }
            // RFC 5869's most, 255 hash lengths: checked before any
            // buffer is made (a wrapped -8 asks 512 MiB).
            let hash_bits = 8 * digest_algorithm(alg.hash().unwrap_or("SHA-256")).output_len();
            if name == "HKDF" && u64::from(length) > 255 * hash_bits as u64 {
                return Err(Error::operation(
                    "The length provided for HKDF is too large.",
                ));
            }
            if name == "PBKDF2" && alg.num("iterations").unwrap_or(0) == 0 {
                return Err(Error::operation("PBKDF2 requires iterations > 0"));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// `SubtleCrypto.supports(operation, algorithm, lengthOrAdditionalAlgorithm)`
/// (Chrome 157 ships it, probed: supports): whether `algorithm` normalizes
/// for `op` and passes the operation's own checks that need no key, the
/// operations' code run or their checks shared, so that it never says yes
/// to what the operation refuses. `extra` is a length (`Value::Num`), a
/// derived key's algorithm (deriveKey's), or `Value::Absent`.
pub(crate) fn supports(keys: &Keys, op: &str, algorithm: &Value, extra: &Value) -> bool {
    let length = match extra {
        Value::Num(n) => Some(*n as u32),
        _ => None,
    };
    // getPublicKey takes no algorithm: any private key of an asymmetric
    // one, whatever else the dictionary says.
    if op == "getPublicKey" {
        return algorithm_name(algorithm)
            .and_then(crate::normalize::canonical)
            .is_some_and(asymmetric);
    }
    let normalized_as = match op {
        "encrypt" | "decrypt" | "sign" | "verify" | "digest" | "generateKey" | "importKey"
        | "deriveBits" | "wrapKey" | "unwrapKey" => op,
        "exportKey" => "importKey",
        "deriveKey" => "deriveBits",
        "encapsulateKey" | "encapsulateBits" => "encapsulate",
        "decapsulateKey" | "decapsulateBits" => "decapsulate",
        _ => return false,
    };
    let Ok(alg) = normalize(normalized_as, algorithm) else {
        return false;
    };
    match op {
        "generateKey" => match alg.name {
            "RSASSA-PKCS1-v1_5" | "RSA-PSS" | "RSA-OAEP" => {
                rsa_params(&alg).is_ok_and(|(bits, _)| bits >= RSA_LEAST_GENERATED)
            }
            // The rest are made here as generateKey makes them, all the
            // usages the algorithm allows asked for, and thrown away.
            name => {
                let mut usages: Vec<Usage> = allowed(name, KeyType::Secret).to_vec();
                for kind in [KeyType::Public, KeyType::Private] {
                    for usage in allowed(name, kind) {
                        if !usages.contains(usage) {
                            usages.push(*usage);
                        }
                    }
                }
                generate(&mut Keys::default(), &alg, false, &usages).is_ok()
            }
        },
        "importKey" => import_params(&alg).is_ok(),
        // The shared key's algorithm, which the operation needs: one that
        // 32 bytes of secret import as (run on a throwaway store).
        "encapsulateKey" | "decapsulateKey" => match extra {
            Value::Absent => false,
            _ => normalize("importKey", extra).is_ok_and(|shared| {
                let usages = allowed(shared.name, KeyType::Secret);
                import(
                    &mut Keys::default(),
                    "raw-secret",
                    &Value::Bytes(vec![0; 32]),
                    &shared,
                    false,
                    usages,
                )
                .is_ok()
            }),
        },
        "exportKey" => import_params(&alg).is_ok() && has_export(alg.name),
        "encrypt" | "decrypt" => cipher_params(&alg),
        // The wrapped key's algorithm, if given: one that imports, and,
        // to be wrapped, exports.
        "wrapKey" | "unwrapKey" => {
            cipher_params(&alg)
                && match extra {
                    Value::Absent => true,
                    _ => normalize("importKey", extra).is_ok_and(|wrapped| {
                        import_params(&wrapped).is_ok()
                            && (op == "unwrapKey"
                                || has_export(wrapped.name)
                                    && (alg.name != "AES-KW" || kw_wraps(&wrapped)))
                    }),
                }
        }
        "deriveBits" => derive_supported(keys, &alg, length),
        // ECDSA's hash where some curve pairs with it; RSA-PSS's salt
        // where some key the store takes fits it.
        "sign" | "verify" if alg.name == "ECDSA" => {
            let hash = alg.hash().unwrap_or("SHA-256");
            CURVES.iter().any(|c| {
                if op == "sign" {
                    ecdsa_signing(c.name, hash).is_some()
                } else {
                    ecdsa_verifying(c.name, hash).is_some()
                }
            })
        }
        // ML-DSA's context, as signing and verifying check it.
        "sign" | "verify" if ml_dsa(alg.name).is_some() => ml_dsa_context(&alg).is_ok(),
        "sign" | "verify" if alg.name == "RSA-PSS" => {
            alg.num("saltLength").unwrap_or(0) <= RSA_MOST_BITS / 8 - 20 - 2
        }
        // The derived key's type must be an algorithm: deriveKey takes
        // no length (fuzzed: crypto_supports, CI 45419454311).
        "deriveKey" => {
            let derived_length = match extra {
                Value::Str(_) | Value::Dict(_) => {
                    let (Ok(_), Ok(by_length)) = (
                        normalize("importKey", extra),
                        normalize("get key length", extra),
                    ) else {
                        return false;
                    };
                    match key_length(&by_length) {
                        Ok(length) => length,
                        Err(_) => return false,
                    }
                }
                _ => return false,
            };
            derive_supported(keys, &alg, derived_length)
        }
        _ => true,
    }
}

/// Whether AES-KW wraps some key of `wrapped`'s algorithm: its export
/// must be a multiple of 8 bytes, 16 or more. An EC or OKP key's every
/// encoding has a length its curve and usages fix (none of P-256's is
/// one, P-384's SPKI is), so a throwaway key of the curve is exported
/// in each format with each usages it may have; a secret's or an RSA
/// key's lengths vary with the key, and some fits. (An unwrap takes any
/// bytes, a JWK's text padded among them.)
fn kw_wraps(wrapped: &Normalized) -> bool {
    if !matches!(wrapped.name, "ECDSA" | "ECDH" | "Ed25519" | "X25519") {
        return true;
    }
    let mut keys = Keys::default();
    let mut usages: Vec<Usage> = allowed(wrapped.name, KeyType::Public).to_vec();
    usages.extend_from_slice(allowed(wrapped.name, KeyType::Private));
    let Ok(Outcome::Pair(public, private)) = generate(&mut keys, wrapped, true, &usages) else {
        return false;
    };
    [public, private].iter().any(|info| {
        let Ok(made) = keys.get(info.id) else {
            return false;
        };
        let all = &info.usages;
        // Each subset of its usages: a JWK's key_ops are the key's.
        (0..1u32 << all.len()).any(|mask| {
            let mut key = made.clone();
            key.usages = (0..all.len())
                .filter(|i| mask & (1 << i) != 0)
                .map(|i| all[i])
                .collect();
            if key.kind == KeyType::Private && key.usages.is_empty() {
                return false;
            }
            ["raw", "spki", "pkcs8", "jwk"].iter().any(|format| {
                let len = match export(&key, format) {
                    Ok(Outcome::Bytes(bytes)) => bytes.len(),
                    Ok(Outcome::Jwk(members)) => jwk_text(&members).len(),
                    _ => return false,
                };
                len >= 16 && len.is_multiple_of(8)
            })
        })
    })
}

/// An encryption's checks that need no key: the AES ciphers' and
/// ChaCha20's run on a throwaway key and no data; RSA-OAEP's and AES-KW's
/// all need the key.
fn cipher_params(alg: &Normalized) -> bool {
    match alg.name {
        // A 256-bit throwaway key: AES-256's, and ChaCha20's.
        "AES-CBC" | "AES-CTR" | "AES-GCM" | CHACHA => {
            let key = Key {
                kind: KeyType::Secret,
                extractable: false,
                algorithm: KeyAlgorithm::Aes {
                    name: alg.name,
                    length: 256,
                },
                usages: vec![],
                material: Material::Secret(vec![0; 32]),
            };
            crypt_with(&key, alg, &[], true).is_ok()
        }
        _ => true,
    }
}

/// A derivation's checks that need no base key: its parameters, and an
/// agreement's public key, which bounds its length.
fn derive_supported(keys: &Keys, alg: &Normalized, length: Option<u32>) -> bool {
    derive_params(alg, length).is_ok()
        && match alg.name {
            "ECDH" | "X25519" => agreement_public(keys, alg)
                .is_ok_and(|(_, most)| length.is_none_or(|length| length <= most)),
            _ => true,
        }
}

fn derive_bits(
    keys: &Keys,
    key: &Key,
    alg: &Normalized,
    length: Option<u32>,
) -> Result<Vec<u8>, Error> {
    if alg.name == "ECDH" {
        return ecdh_bits(keys, key, alg, length);
    }
    if alg.name == "X25519" {
        return x25519_bits(keys, key, alg, length);
    }
    let secret = secret(key);
    match alg.name {
        "HKDF" => {
            derive_params(alg, length)?;
            let length = length.unwrap_or(0);
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
            let too_large = |_| Error::operation("The length provided for HKDF is too large.");
            // The length checked (RFC 5869's most, 255 hash lengths)
            // before a buffer of it is made: a wrapped -8 asks 512 MiB.
            let okm = prk.expand(&info, Len(bytes)).map_err(too_large)?;
            let mut out = vec![0u8; bytes];
            okm.fill(&mut out).map_err(too_large)?;
            Ok(out)
        }
        "PBKDF2" => {
            derive_params(alg, length)?;
            let length = length.unwrap_or(0);
            let iterations = std::num::NonZeroU32::new(alg.num("iterations").unwrap_or(0))
                .ok_or_else(|| Error::operation(""))?;
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
