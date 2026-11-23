//! The operations, by algorithm, each over aws-lc-rs, with the reference
//! browser's errors (Chrome 157, probed 2026-11-22; docs/design/
//! webcrypto.md).

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::{aead, digest, hkdf, hmac, pbkdf2, rand, rsa, signature};

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
        ("HMAC", _) => &[Sign, Verify],
        ("HKDF" | "PBKDF2", _) => &[DeriveKey, DeriveBits],
        ("RSASSA-PKCS1-v1_5" | "RSA-PSS", KeyType::Public) => &[Verify],
        ("RSASSA-PKCS1-v1_5" | "RSA-PSS", _) => &[Sign],
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
        Material::RsaPublic { .. } | Material::RsaPrivate(_) => &[],
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

/// An RSA public key's SPKI, aws-lc-rs's encoding of it.
fn spki_of(n: &[u8], e: &[u8]) -> Result<Vec<u8>, Error> {
    let key =
        rsa::PublicKey::from_der(&rsa_public_key_der(n, e)).map_err(|_| Error::operation(""))?;
    let der = key.as_der().map_err(|_| Error::operation(""))?;
    Ok(der.as_ref().to_vec())
}

/// The reason an RSA-OAEP key outside aws-lc-rs's range is refused: a
/// known gap (the lead's ruling, 2026-11-23), where the reference works.
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
                return Err(Error::operation(OAEP_GAP));
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
                return Err(Error::operation(OAEP_GAP));
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
        _ => Err(Error::not_supported("Unsupported operation")),
    }
}

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
