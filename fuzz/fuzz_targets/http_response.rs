//! The HTTP/1.1 response reader on any bytes, cut into pieces of any
//! size and read through a buffer of any size. It never panics, reading
//! in pieces gives what reading the bytes whole gives, and it never
//! reads past its limits: the status lines and fields, the body, the
//! chunked coding's framing, and at most one buffer's fill beyond.

#![no_main]

use std::io::{BufReader, Read};

use libfuzzer_sys::fuzz_target;
use net::http::{Limits, read_response};

const LIMITS: Limits = Limits {
    header_bytes: 1024,
    body_bytes: 4096,
};

/// Hands out `bytes` in pieces whose sizes cycle through `sizes`, and
/// counts what it handed out.
struct Pieces<'a> {
    bytes: &'a [u8],
    sizes: &'a [u8],
    next: usize,
    pulled: usize,
}

impl Read for Pieces<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let size = match self.sizes {
            [] => buf.len(),
            sizes => usize::from(sizes[self.next % sizes.len()] % 17) + 1,
        };
        self.next += 1;
        let n = size.min(buf.len()).min(self.bytes.len());
        buf[..n].copy_from_slice(&self.bytes[..n]);
        self.bytes = &self.bytes[n..];
        self.pulled += n;
        Ok(n)
    }
}

fuzz_target!(|data: &[u8]| {
    // A control byte (HEAD or not, and the buffer's size), up to eight
    // piece sizes, and the response.
    let Some((&control, rest)) = data.split_first() else {
        return;
    };
    let to_head = control & 1 != 0;
    let capacity = usize::from(control >> 1) % 64 + 1;
    let (sizes, bytes) = rest.split_at(rest.len().min(8));

    let whole = read_response(&mut BufReader::new(bytes), to_head, LIMITS);
    let pieces = Pieces {
        bytes,
        sizes,
        next: 0,
        pulled: 0,
    };
    let mut reader = BufReader::with_capacity(capacity, pieces);
    let cut = read_response(&mut reader, to_head, LIMITS);
    assert_eq!(format!("{whole:?}"), format!("{cut:?}"));

    if let Ok(response) = &whole {
        assert!(response.body.len() <= LIMITS.body_bytes);
    }
    let pulled = reader.get_ref().pulled;
    assert!(
        pulled <= LIMITS.header_bytes + 2 * LIMITS.body_bytes + capacity,
        "{pulled} bytes read"
    );
});
