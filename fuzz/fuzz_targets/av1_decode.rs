//! AV1 (M40) from arbitrary bytes, through `rav1d`'s C interface in
//! `media`'s one module that writes `unsafe`: the input cut into temporal
//! units, each a little-endian 16-bit length and its bytes. Nothing may
//! panic, and each picture is at most `MAX_VIDEO_SIZE` a side, its pixels
//! exactly its size, opaque.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use media::{Error, MAX_VIDEO_SIZE, PictureDecoder};

fuzz_target!(|data: &[u8]| {
    let Ok(mut decoder) = PictureDecoder::av1() else {
        return;
    };
    let mut rest = data;
    let mut timestamp = 0;
    while rest.len() >= 2 {
        let len = usize::from(u16::from_le_bytes([rest[0], rest[1]])).min(rest.len() - 2);
        let unit = &rest[2..2 + len];
        rest = &rest[2 + len..];
        timestamp += 1;
        match decoder.decode_bytes(unit, timestamp) {
            Ok(pictures) => {
                for p in pictures {
                    assert!(p.width <= MAX_VIDEO_SIZE && p.height <= MAX_VIDEO_SIZE);
                    assert_eq!(p.rgba.len(), p.width as usize * p.height as usize * 4);
                    assert!(p.rgba.chunks(4).all(|px| px[3] == 255));
                }
            }
            Err(e) => assert_ne!(e, Error::Panicked, "the AV1 decoder panicked"),
        }
    }
    if let Err(e) = decoder.drain() {
        assert_ne!(e, Error::Panicked, "the AV1 decoder panicked draining");
    }
});
