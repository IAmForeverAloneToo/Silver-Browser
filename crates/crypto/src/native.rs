//! The few calls into aws-lc's C library that aws-lc-rs's safe API lacks
//! (G255; the lead's ruling of 2026-11-23): AES-GCM with any IV and tag
//! length the reference allows; RSA's keys (generated, made from their
//! parts, a PKCS #8 parsed, their parts and PKCS #1 read, each checked)
//! and RSASSA-PKCS1-v1_5's and RSA-PSS's signing and verification outside
//! aws-lc-rs's ranges. Every unsafe call of the crate is here.
//!
//! The rules this module keeps: each C object is owned by a wrapper that
//! frees it on every path (`Context`, `Rsa`, `Bignum`, `Pkey`); a private
//! key's parts and PKCS #1 are overwritten as they go; every return code
//! is checked, a failure an `Err(Failed)` and never a panic across the
//! boundary; every length a script controls is checked in Rust before
//! the call that takes it (against `c_int`'s range, and, for an IV and a
//! tag, against what the reference allows, so that its errors come
//! first); a tag is checked by the library (`EVP_DecryptFinal_ex`,
//! constant time), never by a comparison here.

#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

use std::os::raw::{c_int, c_void};
use std::ptr;

use aws_lc_sys as sys;

/// A call into the library failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Failed;

/// A length as the library's `int`, or `Failed` past its range.
fn c_len(len: usize) -> Result<c_int, Failed> {
    c_int::try_from(len).map_err(|_| Failed)
}

/// An `EVP_CIPHER_CTX`, freed as it goes.
struct Context(*mut sys::EVP_CIPHER_CTX);

impl Context {
    fn new() -> Result<Context, Failed> {
        // SAFETY: `EVP_CIPHER_CTX_new` takes nothing and returns a new
        // context or null; null is refused below.
        let ctx = unsafe { sys::EVP_CIPHER_CTX_new() };
        if ctx.is_null() {
            return Err(Failed);
        }
        Ok(Context(ctx))
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `EVP_CIPHER_CTX_new`, is not null,
        // and is freed once, here.
        unsafe { sys::EVP_CIPHER_CTX_free(self.0) };
    }
}

/// Whether the return code of the call named `site` says it worked (1).
/// A test build can make any one call fail (`tests::failing`), so that
/// each check is seen to turn a failure into `Failed`.
fn ok(site: &'static str, code: c_int) -> Result<(), Failed> {
    #[cfg(test)]
    let code = if tests::FAIL.get() == Some(site) {
        0
    } else {
        code
    };
    #[cfg(not(test))]
    let _ = site;
    if code == 1 { Ok(()) } else { Err(Failed) }
}

/// Counts, in a test build, each time an operation reaches the library,
/// so that a test can say what is refused never does.
fn reached() {
    #[cfg(test)]
    tests::REACHED.set(tests::REACHED.get() + 1);
}

/// The GCM cipher for a key of `len` bytes.
fn gcm_cipher(len: usize) -> Result<*const sys::EVP_CIPHER, Failed> {
    reached();
    // SAFETY: each returns a pointer to a static cipher description,
    // never null, which is not freed.
    let cipher = unsafe {
        match len {
            16 => sys::EVP_aes_128_gcm(),
            32 => sys::EVP_aes_256_gcm(),
            _ => return Err(Failed),
        }
    };
    if cipher.is_null() {
        Err(Failed)
    } else {
        Ok(cipher)
    }
}

/// AES-GCM's tag lengths, in bytes, that the reference allows (its 32,
/// 64, 96, 104, 112, 120 and 128 bits).
pub(crate) fn gcm_tag_allowed(bytes: usize) -> bool {
    matches!(bytes, 4 | 8 | 12 | 13 | 14 | 15 | 16)
}

/// Checks, by their lengths, what the caller must have checked already,
/// so that nothing out of range reaches the library.
fn gcm_lengths(key: usize, iv: usize, aad: usize, data: usize, tag: usize) -> Result<(), Failed> {
    if iv == 0 {
        return Err(Failed);
    }
    if !gcm_tag_allowed(tag) {
        return Err(Failed);
    }
    if !matches!(key, 16 | 32) {
        return Err(Failed);
    }
    c_len(iv)?;
    c_len(aad)?;
    c_len(data)?;
    Ok(())
}

/// AES-GCM's encryption with any IV and tag length: the ciphertext, then
/// the tag of `tag` bytes.
pub(crate) fn gcm_seal(
    key: &[u8],
    iv: &[u8],
    aad: &[u8],
    plain: &[u8],
    tag: usize,
) -> Result<Vec<u8>, Failed> {
    gcm_lengths(key.len(), iv.len(), aad.len(), plain.len(), tag)?;
    let cipher = gcm_cipher(key.len())?;
    let ctx = Context::new()?;
    // GCM's output is as long as its input; the final call writes
    // nothing, the room for a block given all the same.
    let mut out = vec![0u8; plain.len() + 16 + tag];
    let mut written: c_int = 0;
    let mut last: c_int = 0;
    // SAFETY: `ctx.0` is a live context; `cipher` a static cipher; the
    // key, IV and AAD pointers are valid for the lengths given, which
    // were checked against `c_int` and the cipher's key size above; `out`
    // has room for the input and a block; the out-pointers are live
    // `c_int`s. Every return code is checked.
    unsafe {
        ok(
            "EncryptInit cipher",
            sys::EVP_EncryptInit_ex(ctx.0, cipher, ptr::null_mut(), ptr::null(), ptr::null()),
        )?;
        ok(
            "SetIvLen",
            sys::EVP_CIPHER_CTX_ctrl(
                ctx.0,
                sys::EVP_CTRL_GCM_SET_IVLEN,
                c_len(iv.len())?,
                ptr::null_mut(),
            ),
        )?;
        ok(
            "EncryptInit key",
            sys::EVP_EncryptInit_ex(
                ctx.0,
                ptr::null(),
                ptr::null_mut(),
                key.as_ptr(),
                iv.as_ptr(),
            ),
        )?;
        if !aad.is_empty() {
            let mut aad_out: c_int = 0;
            ok(
                "EncryptUpdate aad",
                sys::EVP_EncryptUpdate(
                    ctx.0,
                    ptr::null_mut(),
                    &mut aad_out,
                    aad.as_ptr(),
                    c_len(aad.len())?,
                ),
            )?;
        }
        ok(
            "EncryptUpdate",
            sys::EVP_EncryptUpdate(
                ctx.0,
                out.as_mut_ptr(),
                &mut written,
                plain.as_ptr(),
                c_len(plain.len())?,
            ),
        )?;
        let at = usize::try_from(written).map_err(|_| Failed)?;
        if at > plain.len() + 16 {
            return Err(Failed);
        }
        ok(
            "EncryptFinal",
            sys::EVP_EncryptFinal_ex(ctx.0, out.as_mut_ptr().add(at), &mut last),
        )?;
    }
    let len = usize::try_from(written).map_err(|_| Failed)?
        + usize::try_from(last).map_err(|_| Failed)?;
    if len != plain.len() {
        return Err(Failed);
    }
    let mut tag_bytes = vec![0u8; tag];
    // SAFETY: `ctx.0` is live and finished; `tag_bytes` has room for the
    // `tag` bytes asked for, a length checked against the allowed set.
    unsafe {
        ok(
            "GetTag",
            sys::EVP_CIPHER_CTX_ctrl(
                ctx.0,
                sys::EVP_CTRL_GCM_GET_TAG,
                c_len(tag)?,
                tag_bytes.as_mut_ptr().cast::<c_void>(),
            ),
        )?;
    }
    out.truncate(len);
    out.extend_from_slice(&tag_bytes);
    Ok(out)
}

/// AES-GCM's decryption with any IV and tag length: `sealed` is the
/// ciphertext and its tag of `tag` bytes. The tag is checked by the
/// library; a wrong one is `Failed`.
pub(crate) fn gcm_open(
    key: &[u8],
    iv: &[u8],
    aad: &[u8],
    sealed: &[u8],
    tag: usize,
) -> Result<Vec<u8>, Failed> {
    if sealed.len() < tag {
        return Err(Failed);
    }
    let (data, tag_bytes) = sealed.split_at(sealed.len() - tag);
    gcm_lengths(key.len(), iv.len(), aad.len(), data.len(), tag)?;
    let cipher = gcm_cipher(key.len())?;
    let ctx = Context::new()?;
    let mut out = vec![0u8; data.len() + 16];
    let mut tag_copy = tag_bytes.to_vec();
    let mut written: c_int = 0;
    let mut last: c_int = 0;
    // SAFETY: as in `gcm_seal`: a live context, a static cipher, pointers
    // valid for the checked lengths, `out` with room for the input and a
    // block; `tag_copy` holds the `tag` bytes the library reads. Every
    // return code is checked; `EVP_DecryptFinal_ex` checks the tag.
    unsafe {
        ok(
            "DecryptInit cipher",
            sys::EVP_DecryptInit_ex(ctx.0, cipher, ptr::null_mut(), ptr::null(), ptr::null()),
        )?;
        ok(
            "SetIvLen",
            sys::EVP_CIPHER_CTX_ctrl(
                ctx.0,
                sys::EVP_CTRL_GCM_SET_IVLEN,
                c_len(iv.len())?,
                ptr::null_mut(),
            ),
        )?;
        ok(
            "DecryptInit key",
            sys::EVP_DecryptInit_ex(
                ctx.0,
                ptr::null(),
                ptr::null_mut(),
                key.as_ptr(),
                iv.as_ptr(),
            ),
        )?;
        if !aad.is_empty() {
            let mut aad_out: c_int = 0;
            ok(
                "DecryptUpdate aad",
                sys::EVP_DecryptUpdate(
                    ctx.0,
                    ptr::null_mut(),
                    &mut aad_out,
                    aad.as_ptr(),
                    c_len(aad.len())?,
                ),
            )?;
        }
        ok(
            "DecryptUpdate",
            sys::EVP_DecryptUpdate(
                ctx.0,
                out.as_mut_ptr(),
                &mut written,
                data.as_ptr(),
                c_len(data.len())?,
            ),
        )?;
        ok(
            "SetTag",
            sys::EVP_CIPHER_CTX_ctrl(
                ctx.0,
                sys::EVP_CTRL_GCM_SET_TAG,
                c_len(tag)?,
                tag_copy.as_mut_ptr().cast::<c_void>(),
            ),
        )?;
        let at = usize::try_from(written).map_err(|_| Failed)?;
        if at > data.len() + 16 {
            return Err(Failed);
        }
        ok(
            "DecryptFinal",
            sys::EVP_DecryptFinal_ex(ctx.0, out.as_mut_ptr().add(at), &mut last),
        )?;
    }
    let len = usize::try_from(written).map_err(|_| Failed)?
        + usize::try_from(last).map_err(|_| Failed)?;
    if len != data.len() {
        return Err(Failed);
    }
    out.truncate(len);
    Ok(out)
}

/// An `RSA`, freed as it goes.
struct Rsa(*mut sys::RSA);

impl Drop for Rsa {
    fn drop(&mut self) {
        // SAFETY: the pointer came from the library, is not null, and is
        // freed once, here.
        unsafe { sys::RSA_free(self.0) };
    }
}

/// An RSA public key's parts, big-endian, without leading zeros.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RsaParts {
    pub(crate) n: Vec<u8>,
    pub(crate) e: Vec<u8>,
}

/// A `BIGNUM`'s bytes; empty for none.
fn bignum_bytes(bn: *const sys::BIGNUM) -> Vec<u8> {
    if bn.is_null() {
        return Vec::new();
    }
    // SAFETY: `bn` is a live BIGNUM the key owns, not null; the buffer is
    // `BN_num_bytes` long, which `BN_bn2bin` fills exactly.
    unsafe {
        let len = sys::BN_num_bytes(bn) as usize;
        let mut out = vec![0u8; len];
        let wrote = sys::BN_bn2bin(bn, out.as_mut_ptr());
        out.truncate(wrote.min(len));
        out
    }
}

/// The public parts of the key `rsa` holds.
fn parts_of(rsa: &Rsa) -> RsaParts {
    let mut n = ptr::null();
    let mut e = ptr::null();
    let mut d = ptr::null();
    // SAFETY: `rsa.0` is a live key; the getter writes borrowed pointers
    // (or null) into the live locals given, which are read only while
    // `rsa` lives, below.
    unsafe { sys::RSA_get0_key(rsa.0, &mut n, &mut e, &mut d) };
    RsaParts {
        n: bignum_bytes(n),
        e: bignum_bytes(e),
    }
}

/// A PKCS #1 `RSAPublicKey` (RFC 8017 A.1.1) parsed by the library and
/// checked by it (`RSA_check_key`): a key it can use, or `Failed`.
fn rsa_public_key(der: &[u8]) -> Result<Rsa, Failed> {
    reached();
    // SAFETY: the pointer and length are `der`'s own; the result is a new
    // key or null, owned at once by its wrapper.
    let rsa = unsafe { sys::RSA_public_key_from_bytes(der.as_ptr(), der.len()) };
    if rsa.is_null() {
        return Err(Failed);
    }
    checked(Rsa(rsa))
}

/// `rsa`, if the library's check of it (`RSA_check_key`) passes.
fn checked(rsa: Rsa) -> Result<Rsa, Failed> {
    // SAFETY: `rsa.0` is a live key, only read.
    ok("RsaCheck", unsafe { sys::RSA_check_key(rsa.0) })?;
    Ok(rsa)
}

/// The parts of a PKCS #1 `RSAPublicKey`, parsed and checked by the
/// library: what aws-lc-rs's `PublicKey::as_ref` gives of an SPKI, or a
/// JWK's `n` and `e` encoded (a key the library refuses is `Failed`).
pub(crate) fn rsa_public_parts(der: &[u8]) -> Result<RsaParts, Failed> {
    Ok(parts_of(&rsa_public_key(der)?))
}

/// The library's name of a digest.
fn nid(hash: Hash) -> c_int {
    match hash {
        Hash::Sha1 => sys::NID_sha1,
        Hash::Sha256 => sys::NID_sha256,
        Hash::Sha384 => sys::NID_sha384,
        Hash::Sha512 => sys::NID_sha512,
    }
}

/// A `BIGNUM` made here, freed as it goes.
struct Bignum(*mut sys::BIGNUM);

impl Drop for Bignum {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `BN_new`, is not null, and is freed
        // once, here.
        unsafe { sys::BN_free(self.0) };
    }
}

/// An RSA private key the library made, parsed or checked.
pub(crate) struct RsaPrivate(Rsa);

impl RsaPrivate {
    /// All its parts.
    pub(crate) fn parts(&self) -> RsaPrivateParts {
        private_parts_of(&self.0)
    }
}

/// An RSA private key's parts, big-endian, without leading zeros: what
/// the store keeps of it (the lead's ruling, 2026-11-23), overwritten as
/// it goes and never printed.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct RsaPrivateParts {
    pub(crate) n: Vec<u8>,
    pub(crate) e: Vec<u8>,
    pub(crate) d: Vec<u8>,
    pub(crate) p: Vec<u8>,
    pub(crate) q: Vec<u8>,
    pub(crate) dp: Vec<u8>,
    pub(crate) dq: Vec<u8>,
    pub(crate) qi: Vec<u8>,
}

impl Drop for RsaPrivateParts {
    fn drop(&mut self) {
        for part in [
            &mut self.d,
            &mut self.p,
            &mut self.q,
            &mut self.dp,
            &mut self.dq,
            &mut self.qi,
        ] {
            part.iter_mut().for_each(|b| *b = 0);
            std::hint::black_box(&part);
        }
    }
}

impl std::fmt::Debug for RsaPrivateParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never a private part, in a log or a panic.
        f.write_str("RsaPrivateParts(..)")
    }
}

/// The parts of the private key `rsa` holds.
fn private_parts_of(rsa: &Rsa) -> RsaPrivateParts {
    let (mut n, mut e, mut d) = (ptr::null(), ptr::null(), ptr::null());
    let (mut p, mut q) = (ptr::null(), ptr::null());
    let (mut dp, mut dq, mut qi) = (ptr::null(), ptr::null(), ptr::null());
    // SAFETY: `rsa.0` is a live key; the getters write borrowed pointers
    // (or null) into the live locals given, which are read only while
    // `rsa` lives, below.
    unsafe {
        sys::RSA_get0_key(rsa.0, &mut n, &mut e, &mut d);
        sys::RSA_get0_factors(rsa.0, &mut p, &mut q);
        sys::RSA_get0_crt_params(rsa.0, &mut dp, &mut dq, &mut qi);
    }
    RsaPrivateParts {
        n: bignum_bytes(n),
        e: bignum_bytes(e),
        d: bignum_bytes(d),
        p: bignum_bytes(p),
        q: bignum_bytes(q),
        dp: bignum_bytes(dp),
        dq: bignum_bytes(dq),
        qi: bignum_bytes(qi),
    }
}

/// A number the library holds, from big-endian bytes.
fn bignum(bytes: &[u8]) -> Result<Bignum, Failed> {
    // SAFETY: the pointer and length are `bytes`' own; with no number
    // given, the result is a new one or null.
    let bn = unsafe { sys::BN_bin2bn(bytes.as_ptr(), bytes.len(), ptr::null_mut()) };
    if bn.is_null() {
        return Err(Failed);
    }
    Ok(Bignum(bn))
}

/// The private key of `parts`, made and checked by the library
/// (`RSA_new_private_key` ends with `RSA_check_key`): `Failed` for parts
/// that are not one key.
pub(crate) fn rsa_private_from_parts(parts: &RsaPrivateParts) -> Result<RsaPrivate, Failed> {
    reached();
    let [n, e, d, p, q, dp, dq, qi] = [
        &parts.n, &parts.e, &parts.d, &parts.p, &parts.q, &parts.dp, &parts.dq, &parts.qi,
    ]
    .map(|part| bignum(part));
    let (n, e, d, p, q, dp, dq, qi) = (n?, e?, d?, p?, q?, dp?, dq?, qi?);
    // SAFETY: each number is live; the library copies them into a new
    // key (or returns null), which its wrapper owns at once.
    let rsa = unsafe { sys::RSA_new_private_key(n.0, e.0, d.0, p.0, q.0, dp.0, dq.0, qi.0) };
    if rsa.is_null() {
        return Err(Failed);
    }
    // On purpose, the check aws-lc already makes (`RSA_new_private_key` in
    // crypto/fipsmodule/rsa/rsa.c ends with `RSA_check_key`): key material
    // a page gives crosses here, and that is a detail of a dependency we
    // will upgrade (the lead's ruling, 2026-11-23). No test can see it
    // while aws-lc's own check stands; a test pins the outcome.
    Ok(RsaPrivate(checked(Rsa(rsa))?))
}

/// An `EVP_PKEY`, freed as it goes.
struct Pkey(*mut sys::EVP_PKEY);

impl Drop for Pkey {
    fn drop(&mut self) {
        // SAFETY: the pointer came from the library, is not null, and is
        // freed once, here.
        unsafe { sys::EVP_PKEY_free(self.0) };
    }
}

/// A PKCS #8 `PrivateKeyInfo` holding an RSA key, parsed and checked by
/// the library (its `RSA_parse_private_key` ends with `RSA_check_key`),
/// nothing after it: `Failed` for anything else.
pub(crate) fn rsa_private_from_pkcs8(der: &[u8]) -> Result<RsaPrivate, Failed> {
    reached();
    let mut cbs = sys::CBS {
        data: der.as_ptr(),
        len: der.len(),
    };
    // SAFETY: `cbs` describes `der`, which lives through the call; the
    // result is a new key or null, owned at once by its wrapper.
    let pkey = unsafe { sys::EVP_parse_private_key(&mut cbs) };
    if pkey.is_null() {
        return Err(Failed);
    }
    let pkey = Pkey(pkey);
    if cbs.len != 0 {
        return Err(Failed);
    }
    // SAFETY: `pkey.0` is a live key; `get1` returns a new reference to
    // its RSA, or null for a key of another kind, which its wrapper owns
    // and frees.
    let rsa = unsafe { sys::EVP_PKEY_get1_RSA(pkey.0) };
    // Never a null key past here; no test can see this while the check
    // below refuses one too (aws-lc's `RSA_check_key` begins with
    // `GUARD_PTR`), but that is the dependency's, not ours.
    if rsa.is_null() {
        return Err(Failed);
    }
    // On purpose, the check aws-lc already makes (`RSA_parse_private_key`
    // in crypto/rsa_extra/rsa_asn1.c ends with `RSA_check_key`), as above.
    Ok(RsaPrivate(checked(Rsa(rsa))?))
}

/// A private key's PKCS #1 `RSAPrivateKey`, written by the library: what
/// a PKCS #8 export wraps (`algorithms::pkcs8_wrap`).
pub(crate) fn rsa_private_pkcs1(key: &RsaPrivate) -> Result<Vec<u8>, Failed> {
    let mut out: *mut u8 = ptr::null_mut();
    let mut len = 0usize;
    // SAFETY: the key is live and only read; the out-pointers are live
    // locals the library writes a new buffer and its length into.
    ok("RsaToBytes", unsafe {
        sys::RSA_private_key_to_bytes(&mut out, &mut len, key.0.0)
    })?;
    if out.is_null() {
        return Err(Failed);
    }
    // SAFETY: `out` holds the `len` bytes the library wrote; they are
    // copied, then overwritten and freed once, here.
    let bytes = unsafe {
        let bytes = std::slice::from_raw_parts(out, len).to_vec();
        sys::OPENSSL_cleanse(out.cast(), len);
        sys::OPENSSL_free(out.cast());
        bytes
    };
    Ok(bytes)
}

/// A new RSA key of `bits` bits (a multiple of 8, 512 to 8192: the
/// reference refuses to make a smaller one) and public exponent `e` (3 or
/// 65537), made by the library.
pub(crate) fn rsa_generate(bits: u32, e: u64) -> Result<RsaPrivate, Failed> {
    if !(512..=8192).contains(&bits) || !bits.is_multiple_of(8) || !matches!(e, 3 | 65537) {
        return Err(Failed);
    }
    reached();
    // SAFETY: `BN_new` takes nothing and returns a new number or null.
    let bn = unsafe { sys::BN_new() };
    if bn.is_null() {
        return Err(Failed);
    }
    let bn = Bignum(bn);
    // SAFETY: `bn.0` is a live number, written once.
    ok("BnSet", unsafe { sys::BN_set_u64(bn.0, e) })?;
    // SAFETY: `RSA_new` takes nothing and returns a new key or null.
    let rsa = unsafe { sys::RSA_new() };
    if rsa.is_null() {
        return Err(Failed);
    }
    let rsa = Rsa(rsa);
    // SAFETY: `rsa.0` and `bn.0` are live; `bits` was checked to be at
    // most 8192, within `c_int`; no callback is given.
    ok("RsaGenerate", unsafe {
        sys::RSA_generate_key_ex(rsa.0, bits as c_int, bn.0, ptr::null_mut())
    })?;
    Ok(RsaPrivate(rsa))
}

/// RSASSA-PKCS1-v1_5's signature of `digest` (by `hash`) with `key`, made
/// by the library; `Failed` where the digest does not fit the key.
pub(crate) fn rsa_sign_pkcs1(
    key: &RsaPrivate,
    hash: Hash,
    digest: &[u8],
) -> Result<Vec<u8>, Failed> {
    // SAFETY: `key.0.0` is a live key, only read.
    let size = unsafe { sys::RSA_size(key.0.0) } as usize;
    let mut out = vec![0u8; size];
    let mut len: std::os::raw::c_uint = 0;
    // SAFETY: the key is live; the digest pointer is valid for its
    // length; `out` has `RSA_size` bytes, the most the call writes; `len`
    // is a live local.
    ok("RsaSign", unsafe {
        sys::RSA_sign(
            nid(hash),
            digest.as_ptr(),
            digest.len(),
            out.as_mut_ptr(),
            &mut len,
            key.0.0,
        )
    })?;
    let len = usize::try_from(len).map_err(|_| Failed)?;
    if len > size {
        return Err(Failed);
    }
    out.truncate(len);
    Ok(out)
}

/// The library's description of a digest, a static one.
fn md(hash: Hash) -> *const sys::EVP_MD {
    // SAFETY: each takes nothing and returns a pointer to a static digest
    // description, never freed.
    unsafe {
        match hash {
            Hash::Sha1 => sys::EVP_sha1(),
            Hash::Sha256 => sys::EVP_sha256(),
            Hash::Sha384 => sys::EVP_sha384(),
            Hash::Sha512 => sys::EVP_sha512(),
        }
    }
}

/// RSA-PSS's signature of `digest` (by `hash`, MGF1 with it too) with
/// `key` and a salt of `salt` bytes, made by the library; `Failed` for a
/// salt the key cannot hold or past `c_int`.
pub(crate) fn rsa_sign_pss(
    key: &RsaPrivate,
    hash: Hash,
    digest: &[u8],
    salt: u32,
) -> Result<Vec<u8>, Failed> {
    let salt = c_int::try_from(salt).map_err(|_| Failed)?;
    let md = md(hash);
    // SAFETY: `key.0.0` is a live key, only read.
    let size = unsafe { sys::RSA_size(key.0.0) } as usize;
    let mut out = vec![0u8; size];
    let mut len = 0usize;
    // SAFETY: the key is live; `out` has `size` bytes, the most given; the
    // digest pointer is valid for its length; `md` is static; `len` is a
    // live local.
    ok("RsaSignPss", unsafe {
        sys::RSA_sign_pss_mgf1(
            key.0.0,
            &mut len,
            out.as_mut_ptr(),
            size,
            digest.as_ptr(),
            digest.len(),
            md,
            md,
            salt,
        )
    })?;
    if len > size {
        return Err(Failed);
    }
    out.truncate(len);
    Ok(out)
}

/// RSA-PSS's verification, as `rsa_verify_pkcs1`'s: `Ok(false)` for a
/// signature that does not verify (a salt past what the key holds among
/// them, as the reference answers), `Failed` for a key the library
/// refuses or a salt past `c_int`.
pub(crate) fn rsa_verify_pss(
    der: &[u8],
    hash: Hash,
    digest: &[u8],
    salt: u32,
    signature: &[u8],
) -> Result<bool, Failed> {
    let salt = c_int::try_from(salt).map_err(|_| Failed)?;
    let rsa = rsa_public_key(der)?;
    let md = md(hash);
    // SAFETY: `rsa.0` is a live, checked key; the digest and signature
    // pointers are valid for their lengths; `md` is static; it writes
    // nothing.
    let code = unsafe {
        sys::RSA_verify_pss_mgf1(
            rsa.0,
            digest.as_ptr(),
            digest.len(),
            md,
            md,
            salt,
            signature.as_ptr(),
            signature.len(),
        )
    };
    Ok(code == 1)
}

/// The digests RSASSA-PKCS1-v1_5 signs, as the library names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

/// RSASSA-PKCS1-v1_5's verification of `signature` over `digest` (the
/// message's, by `hash`) with a key aws-lc-rs's safe API does not take:
/// `Ok(false)` for a signature that does not verify, `Failed` for a key
/// the library refuses (never a false verify for want of a key).
pub(crate) fn rsa_verify_pkcs1(
    der: &[u8],
    hash: Hash,
    digest: &[u8],
    signature: &[u8],
) -> Result<bool, Failed> {
    let rsa = rsa_public_key(der)?;
    let nid = nid(hash);
    // SAFETY: `rsa.0` is a live, checked key; the digest and signature
    // pointers are valid for the lengths given (`usize`s, the library's
    // own type here); it writes nothing.
    let code = unsafe {
        sys::RSA_verify(
            nid,
            digest.as_ptr(),
            digest.len(),
            signature.as_ptr(),
            signature.len(),
            rsa.0,
        )
    };
    Ok(code == 1)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use aws_lc_rs::aead;
    use std::cell::Cell;

    thread_local! {
        /// The call whose return code a test makes a failure.
        pub(super) static FAIL: Cell<Option<&'static str>> = const { Cell::new(None) };
        /// How many times an operation reached the library.
        pub(super) static REACHED: Cell<usize> = const { Cell::new(0) };
    }

    /// Runs `f` with the call named `site` failing.
    pub(crate) fn failing<T>(site: &'static str, f: impl FnOnce() -> T) -> T {
        FAIL.set(Some(site));
        let out = f();
        FAIL.set(None);
        out
    }

    const SEAL_CALLS: [&str; 7] = [
        "EncryptInit cipher",
        "SetIvLen",
        "EncryptInit key",
        "EncryptUpdate aad",
        "EncryptUpdate",
        "EncryptFinal",
        "GetTag",
    ];
    const OPEN_CALLS: [&str; 7] = [
        "DecryptInit cipher",
        "SetIvLen",
        "DecryptInit key",
        "DecryptUpdate aad",
        "DecryptUpdate",
        "SetTag",
        "DecryptFinal",
    ];

    #[test]
    fn a_failed_call_is_an_error_whichever_it_is() {
        let (key, iv) = ([7u8; 16], [1u8; 8]);
        let sealed = gcm_seal(&key, &iv, b"aad", b"some data", 12).unwrap();
        for site in SEAL_CALLS {
            let out = failing(site, || gcm_seal(&key, &iv, b"aad", b"some data", 12));
            assert_eq!(out, Err(Failed), "{site} failing");
        }
        for site in OPEN_CALLS {
            let out = failing(site, || gcm_open(&key, &iv, b"aad", &sealed, 12));
            assert_eq!(out, Err(Failed), "{site} failing");
        }
        // And with none failing, both work.
        assert_eq!(
            gcm_open(&key, &iv, b"aad", &sealed, 12).unwrap(),
            b"some data"
        );
    }

    #[test]
    fn nothing_out_of_range_reaches_the_library() {
        let key = [2u8; 16];
        REACHED.set(0);
        assert_eq!(
            gcm_seal(&key, b"", b"", b"x", 16),
            Err(Failed),
            "an empty IV"
        );
        assert_eq!(
            gcm_open(&key, b"", b"", &[0; 20], 16),
            Err(Failed),
            "an empty IV"
        );
        assert_eq!(
            gcm_seal(&key, b"iv", b"", b"x", 5),
            Err(Failed),
            "a 40-bit tag"
        );
        assert_eq!(
            gcm_open(&key, b"iv", b"", &[0; 20], 5),
            Err(Failed),
            "a 40-bit tag"
        );
        assert_eq!(
            gcm_seal(&[2; 24], b"iv", b"", b"x", 16),
            Err(Failed),
            "a 192-bit key"
        );
        assert_eq!(
            gcm_open(&[2; 24], b"iv", b"", &[0; 20], 16),
            Err(Failed),
            "a 192-bit key"
        );
        assert_eq!(
            gcm_open(&key, b"iv", b"", b"ab", 4),
            Err(Failed),
            "shorter than its tag"
        );
        assert_eq!(REACHED.get(), 0, "a refused input reached the library");
        gcm_seal(&key, b"iv", b"", b"x", 16).unwrap();
        assert_eq!(REACHED.get(), 1, "the counter counts");
    }

    #[test]
    fn a_length_past_c_int_is_refused_before_any_call() {
        let past = c_int::MAX as usize + 1;
        assert_eq!(gcm_lengths(16, 12, 0, 0, 16), Ok(()));
        assert_eq!(gcm_lengths(16, past, 0, 0, 16), Err(Failed), "the IV");
        assert_eq!(gcm_lengths(16, 12, past, 0, 16), Err(Failed), "the AAD");
        assert_eq!(gcm_lengths(16, 12, 0, past, 16), Err(Failed), "the data");
        let max = c_int::MAX as usize;
        assert_eq!(gcm_lengths(32, max, max, max, 4), Ok(()));
    }

    #[test]
    fn gcm_at_96_and_128_bits_is_aws_lc_rs_s() {
        let key = [9u8; 32];
        let iv = [3u8; 12];
        let plain = b"the same bytes either way";
        let ours = gcm_seal(&key, &iv, b"aad", plain, 16).unwrap();
        let sealing =
            aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_256_GCM, &key).unwrap());
        let mut theirs = plain.to_vec();
        sealing
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(iv),
                aead::Aad::from(b"aad"),
                &mut theirs,
            )
            .unwrap();
        assert_eq!(ours, theirs);
        assert_eq!(gcm_open(&key, &iv, b"aad", &ours, 16).unwrap(), plain);
    }

    #[test]
    fn gcm_takes_any_iv_and_a_short_tag_and_checks_it() {
        let key = [1u8; 16];
        for iv_len in [1, 8, 16, 60] {
            for tag in [4, 12, 16] {
                let iv = vec![5u8; iv_len];
                let sealed = gcm_seal(&key, &iv, b"", b"hello", tag).unwrap();
                assert_eq!(sealed.len(), 5 + tag);
                assert_eq!(gcm_open(&key, &iv, b"", &sealed, tag).unwrap(), b"hello");
                let mut bad = sealed.clone();
                *bad.last_mut().unwrap() ^= 1;
                assert_eq!(gcm_open(&key, &iv, b"", &bad, tag), Err(Failed));
            }
        }
        assert_eq!(gcm_seal(&key, b"", b"", b"x", 16), Err(Failed));
        assert_eq!(gcm_seal(&key, b"iv", b"", b"x", 5), Err(Failed));
        assert_eq!(gcm_open(&key, b"iv", b"", b"ab", 4), Err(Failed));
    }

    #[test]
    fn an_rsa_public_keys_parts_are_the_librarys_reading() {
        // RSAPublicKey { n = 0x00c3 (as 0xc3), e = 3 }.
        let der = [0x30, 0x07, 0x02, 0x02, 0x00, 0xc3, 0x02, 0x01, 0x03];
        let parts = rsa_public_parts(&der).unwrap();
        assert_eq!((parts.n, parts.e), (vec![0xc3], vec![3]));
        assert_eq!(rsa_public_parts(b"not a key"), Err(Failed));
        // An even modulus the library parses but refuses as a key.
        let even = [0x30, 0x07, 0x02, 0x02, 0x00, 0xc4, 0x02, 0x01, 0x03];
        assert_eq!(rsa_public_parts(&even), Err(Failed));
        assert_eq!(
            failing("RsaCheck", || rsa_public_parts(&der)),
            Err(Failed),
            "the check's failure"
        );
    }
}
