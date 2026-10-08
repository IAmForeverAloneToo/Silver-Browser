//! Images (M23) from arbitrary bytes: each decoder, frame composition and
//! the EXIF reader. Nothing may panic, a decoder's panics included, and
//! what decodes must keep its bounds: sides and pixels within
//! `MAX_IMAGE_SIDE` and `MAX_IMAGE_PIXELS`, at most `MAX_FRAMES` frames,
//! each exactly its image's size, premultiplied, no delay under 11ms.

#![no_main]

use images::{Error, MAX_FRAMES, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let header = images::read_header(data);
    assert_ne!(
        header,
        Err(Error::Panicked),
        "a decoder panicked reading a header"
    );
    let image = images::decode(data);
    assert_ne!(image, Err(Error::Panicked), "a decoder panicked");
    let Ok(image) = image else { return };
    let header = header.expect("what decodes has a header");
    assert_eq!((header.width, header.height), (image.width, image.height));
    assert!(image.width <= MAX_IMAGE_SIDE && image.height <= MAX_IMAGE_SIDE);
    assert!(u64::from(image.width) * u64::from(image.height) <= MAX_IMAGE_PIXELS);
    assert!(!image.frames.is_empty() && image.frames.len() <= MAX_FRAMES);
    let size = image.width as usize * image.height as usize * 4;
    for frame in &image.frames {
        assert_eq!(frame.pixels.len(), size);
        assert!(frame.delay > 10);
        for p in frame.pixels.as_chunks::<4>().0 {
            assert!(p[..3].iter().all(|&c| c <= p[3]), "not premultiplied");
        }
    }
});
