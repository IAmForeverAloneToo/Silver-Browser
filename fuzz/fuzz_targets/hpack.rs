//! HPACK (M38): any bytes as a server's header blocks, decoded in turn
//! by one decoder, never panic, keep its table within what this end
//! announced, and keep each decoded list within its bound; and any
//! header list encoded is decoded the same, block after block, by a
//! decoder whose table follows the encoder's.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use net::h2::hpack::{Decoder, Encoder, field_size};

/// The table size and list bound the first bytes choose.
fn bounds(data: &[u8]) -> (usize, usize) {
    let table = usize::from(data.first().copied().unwrap_or(0)) * 32;
    let list = 64 + usize::from(data.get(1).copied().unwrap_or(0)) * 64;
    (table, list)
}

fuzz_target!(|data: &[u8]| {
    let (table, list) = bounds(data);
    let rest = data.get(2..).unwrap_or_default();

    // A server's blocks: the input split at 0xfe bytes.
    let mut decoder = Decoder::new(table);
    for block in rest.split(|&byte| byte == 0xfe) {
        if let Ok(decoded) = decoder.decode(block, list) {
            let size: usize = decoded
                .fields
                .iter()
                .map(|(name, value)| field_size(name, value))
                .sum();
            assert!(size <= list);
        }
        assert!(decoder.table_size() <= table);
    }

    // Header lists: the input's lines, each `name:value`, split into
    // blocks at empty lines, encoded and decoded the same.
    let mut encoder = Encoder::default();
    encoder.set_max_size(table);
    let mut decoder = Decoder::new(table);
    for block in rest
        .split(|&byte| byte == b'\n')
        .collect::<Vec<_>>()
        .split(|line| line.is_empty())
    {
        let fields: Vec<(Vec<u8>, Vec<u8>)> = block
            .iter()
            .map(|line| match line.iter().position(|&byte| byte == b':') {
                Some(at) => (line[..at].to_ascii_lowercase(), line[at + 1..].to_vec()),
                None => (line.to_ascii_lowercase(), Vec::new()),
            })
            .collect();
        let bytes = encoder.encode(
            fields
                .iter()
                .map(|(name, value)| (name.as_slice(), value.as_slice())),
        );
        let decoded = decoder
            .decode(&bytes, usize::MAX)
            .expect("our own block decodes");
        assert_eq!(decoded.fields, fields);
        assert!(decoder.table_size() <= table);
    }
});
