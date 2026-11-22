//! JSON Web Keys (RFC 7517) as Web Crypto imports and exports them
//! (section 14.3), with the reference browser's messages: a member's
//! base64url (RFC 4648 section 5, no padding), and the checks of `kty`,
//! `use`, `key_ops`, `ext` and `alg` against the call.

use crate::{Error, JwkValue, Usage, Value};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// `bytes` in base64url, unpadded.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (u32::from(b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[(n >> (18 - 6 * i)) as usize & 63]));
        }
    }
    out
}

/// Unpadded base64url's bytes; `None` for any other character, padding
/// among them, or a length no encoding has.
pub(crate) fn decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut n = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let v = ALPHABET.iter().position(|&a| a == c)? as u32;
        n = (n << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((n >> bits) as u8);
            n &= (1 << bits) - 1;
        }
    }
    // A last group of one character holds no byte; leftover bits must be
    // zero, as a canonical encoding's are.
    (text.len() % 4 != 1 && n == 0).then_some(out)
}

/// A JWK to import: its members as the bindings read them.
pub(crate) struct Jwk<'a>(pub(crate) &'a Value);

impl Jwk<'_> {
    pub(crate) fn str(&self, name: &str) -> Option<&str> {
        match self.0.get(name) {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Member `name`, required, as bytes.
    pub(crate) fn bytes(&self, name: &str) -> Result<Vec<u8>, Error> {
        let text = self.str(name).ok_or_else(|| {
            Error::data(format!("The required JWK member \"{name}\" was missing"))
        })?;
        decode(text).ok_or_else(|| {
            Error::data(format!(
                "The JWK member \"{name}\" could not be base64url decoded or contained padding"
            ))
        })
    }

    /// The checks every import makes (section 14.3.9's steps), given the
    /// key type `kty` the algorithm wants and the `alg` it names, if
    /// any.
    pub(crate) fn check(
        &self,
        kty: &str,
        alg: Option<&str>,
        extractable: bool,
        usages: &[Usage],
    ) -> Result<(), Error> {
        match self.str("kty") {
            None => return Err(Error::data("The required JWK member \"kty\" was missing")),
            Some(k) if k != kty => {
                return Err(Error::data(format!(
                    "The JWK \"kty\" member was not \"{kty}\""
                )));
            }
            Some(_) => {}
        }
        if let Some(want) = alg
            && let Some(given) = self.str("alg")
            && given != want
        {
            return Err(Error::data(
                "The JWK \"alg\" member was inconsistent with that specified by the Web Crypto call",
            ));
        }
        if let Some(given) = self.str("use") {
            let fits = match given {
                "enc" => usages.iter().all(|u| {
                    matches!(
                        u,
                        Usage::Encrypt | Usage::Decrypt | Usage::WrapKey | Usage::UnwrapKey
                    )
                }),
                "sig" => usages
                    .iter()
                    .all(|u| matches!(u, Usage::Sign | Usage::Verify)),
                _ => false,
            };
            if !fits {
                return Err(Error::data(
                    "The JWK \"use\" member was inconsistent with that specified by the Web Crypto call. The JWK usage must be a superset of those requested",
                ));
            }
        }
        if let Value::List(ops) = self.0.get("key_ops") {
            let ops: Vec<&str> = ops
                .iter()
                .filter_map(|o| match o {
                    Value::Str(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect();
            if !usages.iter().all(|u| ops.contains(&u.name())) {
                return Err(Error::data(
                    "The JWK \"key_ops\" member was inconsistent with that specified by the Web Crypto call. The JWK usage must be a superset of those requested",
                ));
            }
        }
        if let Value::Bool(false) = self.0.get("ext")
            && extractable
        {
            return Err(Error::data(
                "The \"ext\" member of the JWK dictionary is inconsistent what that specified by the Web Crypto call",
            ));
        }
        Ok(())
    }
}

/// A JWK to export: `members` with `ext` and `key_ops`, in the
/// reference's order (alphabetical: probed, `alg,ext,k,key_ops,kty`).
pub(crate) fn exported(
    mut members: Vec<(&str, JwkValue)>,
    extractable: bool,
    usages: &[Usage],
) -> Vec<(String, JwkValue)> {
    members.push(("ext", JwkValue::Bool(extractable)));
    members.push((
        "key_ops",
        JwkValue::List(usages.iter().map(|u| u.name().to_owned()).collect()),
    ));
    members.sort_by(|a, b| a.0.cmp(b.0));
    members
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}
