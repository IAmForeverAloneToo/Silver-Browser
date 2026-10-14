//! Any bytes as what a renderer or the browser sends over its pipe
//! (M34.2): read as frames, each decoded as a message of either side,
//! and as one message without its frame. Nothing panics; a frame's
//! length past the limit stops the reading; a message decoded is encoded
//! again, and that encoding decodes to a message encoded the same.

#![no_main]
#![forbid(unsafe_code)]

use std::io::Cursor;

use engine::ipc::{
    MAX_MESSAGE, ReadError, ToBrowser, ToRenderer, Wire, decode_all, encode, read_message,
};
use libfuzzer_sys::fuzz_target;

/// A message decoded encodes to bytes that decode to a message encoded
/// the same (URLs are made canonical by the first decoding).
fn stable<M: Wire>(message: &M) {
    let once = encode(message);
    let again = decode_all::<M>(&once).expect("an encoding decodes");
    assert_eq!(encode(&again), once);
}

fn frames<M: Wire>(data: &[u8]) {
    let mut reader = Cursor::new(data);
    loop {
        match read_message::<M>(&mut reader) {
            Ok(Some(message)) => stable(&message),
            Ok(None) => break,
            Err(ReadError::TooLong(len)) => {
                assert!(len > MAX_MESSAGE);
                break;
            }
            Err(_) => break,
        }
    }
}

fuzz_target!(|data: &[u8]| {
    frames::<ToBrowser>(data);
    frames::<ToRenderer>(data);
    if let Ok(message) = decode_all::<ToBrowser>(data) {
        stable(&message);
    }
    if let Ok(message) = decode_all::<ToRenderer>(data) {
        stable(&message);
    }
});
