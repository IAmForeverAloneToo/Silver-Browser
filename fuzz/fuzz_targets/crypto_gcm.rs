//! The aws-lc C library's AES-GCM (G255, `crypto`'s `native`) on any
//! key, IV, tag length, data and associated data: at a 96-bit IV and a
//! 128-bit tag, the same ciphertext and tag as aws-lc-rs's safe API, and
//! the same accepting and refusing of a sealed input, a byte flipped or
//! not; at any other, what it seals opens again, and a byte flipped
//! anywhere is refused. It never panics.

#![no_main]
#![forbid(unsafe_code)]

use aws_lc_rs::aead;
use crypto::fuzzing::{gcm_open, gcm_seal};
use libfuzzer_sys::fuzz_target;

const TAGS: [usize; 7] = [4, 8, 12, 13, 14, 15, 16];

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let key_len = if data[0] & 1 == 0 { 16 } else { 32 };
    let iv_len = usize::from(data[1] % 40) + 1;
    let tag = TAGS[usize::from(data[2]) % TAGS.len()];
    let aad_len = usize::from(data[3] % 24);
    let rest = &data[4..];
    if rest.len() < key_len + iv_len + aad_len {
        return;
    }
    let (key, rest) = rest.split_at(key_len);
    let (iv, rest) = rest.split_at(iv_len);
    let (aad, plain) = rest.split_at(aad_len);
    let sealed = gcm_seal(key, iv, aad, plain, tag).expect("a valid input seals");
    assert_eq!(sealed.len(), plain.len() + tag);
    assert_eq!(gcm_open(key, iv, aad, &sealed, tag).as_deref(), Some(plain));
    let mut flipped = sealed.clone();
    let at = usize::from(data[1]) % flipped.len().max(1);
    if let Some(byte) = flipped.get_mut(at) {
        *byte ^= 0x80;
        assert_eq!(
            gcm_open(key, iv, aad, &flipped, tag),
            None,
            "a flipped byte refused"
        );
    }
    if iv_len == 12 && tag == 16 {
        let algorithm = if key_len == 16 {
            &aead::AES_128_GCM
        } else {
            &aead::AES_256_GCM
        };
        let k = aead::LessSafeKey::new(aead::UnboundKey::new(algorithm, key).unwrap());
        let nonce = || aead::Nonce::try_assume_unique_for_key(iv).unwrap();
        let mut theirs = plain.to_vec();
        k.seal_in_place_append_tag(nonce(), aead::Aad::from(aad), &mut theirs)
            .unwrap();
        assert_eq!(sealed, theirs, "the same ciphertext and tag");
        let mut opened = flipped.clone();
        let theirs_open = k
            .open_in_place(nonce(), aead::Aad::from(aad), &mut opened)
            .is_ok();
        assert!(!theirs_open, "aws-lc-rs refuses the flipped byte too");
    }
});
