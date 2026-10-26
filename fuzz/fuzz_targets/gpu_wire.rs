//! Any bytes as the GPU process's protocols (M41, D38): the browser's
//! messages with the GPU process, read as frames and whole, and the
//! client protocol's batches of requests and replies. Nothing panics; a
//! message or batch decoded is encoded again, and that encoding decodes
//! to one encoded the same.

#![no_main]
#![forbid(unsafe_code)]

use std::io::Cursor;

use engine::gpu::wire::{FromGpu, Reply, Request, ToGpu};
use engine::ipc::{MAX_MESSAGE, ReadError, Wire, decode_all, encode, read_message};
use libfuzzer_sys::fuzz_target;

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
    frames::<ToGpu>(data);
    frames::<FromGpu>(data);
    if let Ok(batch) = decode_all::<Vec<Request>>(data) {
        stable(&batch);
    }
    if let Ok(batch) = decode_all::<Vec<Reply>>(data) {
        stable(&batch);
    }
});
