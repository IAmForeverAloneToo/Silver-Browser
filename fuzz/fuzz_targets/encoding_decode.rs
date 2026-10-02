//! Every decoder over arbitrary bytes. The first byte picks the cuts:
//! the rest is decoded whole, and again in pieces cut at positions taken
//! from it, and the two must agree, in both error modes. A fatal decode
//! that succeeds must give what a replacing one gives, and UTF-8 must
//! give what std's lossy decoding gives.

#![no_main]

use encoding::{DecoderMode, Encoding};
use libfuzzer_sys::fuzz_target;

fn decode(encoding: Encoding, mode: DecoderMode, bytes: &[u8], cuts: &[usize]) -> (String, bool) {
    let mut out = String::new();
    let mut decoder = encoding
        .new_decoder(mode)
        .expect("only decodable encodings are tried");
    let mut start = 0;
    for &cut in cuts {
        if decoder.decode(&bytes[start..cut], false, &mut out).is_err() {
            return (out, false);
        }
        start = cut;
    }
    let ok = decoder.decode(&bytes[start..], true, &mut out).is_ok();
    (out, ok)
}

fuzz_target!(|data: &[u8]| {
    let Some((&first, bytes)) = data.split_first() else {
        return;
    };
    // Up to seven cuts, at positions spread by the first byte's bits.
    let mut cuts: Vec<usize> = (0..first % 8)
        .map(|i| (usize::from(first) * 31 + i as usize * 97) % (bytes.len() + 1))
        .collect();
    cuts.sort_unstable();
    for encoding in Encoding::all().filter(|e| e.can_decode()) {
        for mode in [DecoderMode::Replacement, DecoderMode::Fatal] {
            let whole = decode(encoding, mode, bytes, &[]);
            let pieces = decode(encoding, mode, bytes, &cuts);
            assert_eq!(whole, pieces, "{encoding:?} {mode:?} cut at {cuts:?}");
        }
        let (lossy, ok) = decode(encoding, DecoderMode::Replacement, bytes, &[]);
        assert!(ok, "replacing never fails");
        let (fatal, fatal_ok) = decode(encoding, DecoderMode::Fatal, bytes, &[]);
        if fatal_ok {
            assert_eq!(fatal, lossy, "{encoding:?}");
        }
    }
    let (utf8, _) = decode(encoding::UTF_8, DecoderMode::Replacement, bytes, &[]);
    assert_eq!(utf8, String::from_utf8_lossy(bytes));
});
