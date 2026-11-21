//! The WebSocket reader (G74) on any bytes a server could send, cut into
//! pieces of any size, with `permessage-deflate` agreed or not. It never
//! panics, reading in pieces gives what reading the bytes whole gives,
//! and no message it gives, inflated or not, passes the bound: a frame
//! header or a deflate stream never makes it hold more.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use net::websocket::{DeflateParams, Event, MAX_MESSAGE, Reader};

fn reader(flags: u8) -> Reader {
    Reader::new((flags & 1 != 0).then_some(DeflateParams {
        server_no_context_takeover: flags & 2 != 0,
        client_no_context_takeover: false,
    }))
}

fn size(event: &Event) -> usize {
    match event {
        Event::Text(t) => t.len(),
        Event::Binary(b) | Event::Ping(b) | Event::Pong(b) => b.len(),
        Event::Close(c) => c.as_ref().map_or(0, |(_, r)| r.len()),
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&flags, rest)) = data.split_first() else {
        return;
    };
    let Some((&piece, bytes)) = rest.split_first() else {
        return;
    };
    let whole = reader(flags).read(bytes);
    if let Ok(events) = &whole {
        assert!(events.iter().all(|e| size(e) <= MAX_MESSAGE));
    }
    let mut pieces = reader(flags);
    let mut events = Vec::new();
    let mut failed = None;
    for chunk in bytes.chunks(usize::from(piece).max(1)) {
        match pieces.read(chunk) {
            Ok(more) => events.extend(more),
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    match (&whole, failed) {
        (Ok(all), None) => assert_eq!(all, &events, "in pieces as whole"),
        (Err(a), Some(b)) => assert_eq!(a.code, b.code, "the same failure"),
        (Ok(_), Some(e)) => panic!("the pieces failed where the whole did not: {e}"),
        // Whole bytes may fail on a frame the pieces had not finished.
        (Err(_), None) => {}
    }
});
