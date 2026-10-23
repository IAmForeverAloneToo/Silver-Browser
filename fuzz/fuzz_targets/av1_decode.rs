//! AV1 (M40) from arbitrary bytes, through `rav1d`'s C interface in
//! `media`'s one module that writes `unsafe`: the input cut into temporal
//! units, each a little-endian 16-bit length and its bytes. Nothing may
//! panic outside `media`'s guard, a decoder that panicked is refused
//! after, and each picture is at most `MAX_VIDEO_SIZE` a side, its pixels
//! exactly its size, opaque.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use media::{Error, MAX_VIDEO_SIZE, PictureDecoder};

/// A decoder's panic is contained by `media`, as it is in the browser
/// (release builds unwind): the target's hook does not abort on it, as
/// libFuzzer's would, and it comes back as `Error::Panicked`. A panic
/// outside `media`'s guard still ends the run. With
/// `SILVER_FUZZ_PANICS` set to a folder, each input that made a decoder
/// panic is written there.
fn contain_decoder_panics() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| std::panic::set_hook(Box::new(|_| {})));
    // rav1d's panics are never contained: they end the run, as they end
    // the browser's renderer.
    media::abort_on_decoder_panic();
}

fn keep_panicking_input(data: &[u8]) {
    if let Some(folder) = std::env::var_os("SILVER_FUZZ_PANICS") {
        let name = format!(
            "{:016x}",
            data.iter()
                .fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(*b))
                    .wrapping_mul(0x100000001b3))
        );
        let _ = std::fs::write(std::path::Path::new(&folder).join(name), data);
    }
}

fuzz_target!(|data: &[u8]| {
    contain_decoder_panics();
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
            Err(Error::Panicked) => {
                keep_panicking_input(data);
                assert_eq!(
                    decoder.decode_bytes(unit, timestamp).err(),
                    Some(Error::Panicked)
                );
                return;
            }
            Err(_) => {}
        }
    }
    if let Err(Error::Panicked) = decoder.drain() {
        keep_panicking_input(data);
    }
});
