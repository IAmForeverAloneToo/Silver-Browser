//! The server-sent events parser (G74) on any bytes a server could
//! send, cut into pieces of any size. It never panics, parsing in
//! pieces says what parsing the bytes whole says (a CR, a byte order
//! mark or a character cut between pieces included), and no event's
//! data passes the bound.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use net::event_stream::{MAX_EVENT, Parser, Said};

fuzz_target!(|data: &[u8]| {
    let Some((&piece, bytes)) = data.split_first() else {
        return;
    };
    let mut whole = Parser::new("");
    let mut said = Vec::new();
    let whole_result = whole.feed(bytes, &mut said);
    for s in &said {
        if let Said::Event { data, .. } = s {
            assert!(data.len() <= MAX_EVENT);
        }
    }
    let mut pieces = Parser::new("");
    let mut in_pieces = Vec::new();
    let mut pieces_result = Ok(());
    for chunk in bytes.chunks(usize::from(piece).max(1)) {
        pieces_result = pieces.feed(chunk, &mut in_pieces);
        if pieces_result.is_err() {
            break;
        }
    }
    assert_eq!(whole_result, pieces_result, "the same outcome");
    if whole_result.is_ok() {
        assert_eq!(said, in_pieces, "in pieces as whole");
        assert_eq!(whole.last_event_id(), pieces.last_event_id());
    }
});
