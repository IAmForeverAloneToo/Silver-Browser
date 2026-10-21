//! HTTP/2 (M38, docs/design/http.md): its frames (RFC 9113) and its
//! header compression (RFC 7541), written in the crate as HTTP/1.1 is.
//! `hpack` and `frame` take bytes and give values, and back; `conn` is
//! the client's connection, `session` its protocol without a socket.

pub(crate) mod conn;
pub mod frame;
pub mod hpack;
mod huffman_table;
pub(crate) mod session;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod session_tests;

/// For the `h2_client` fuzz target (M38): a session with three streams
/// open (a `GET`, a `HEAD`, a `POST` with a body larger than a window)
/// fed `data` as the server's bytes, in pieces, its fetches taking what
/// comes; then ended. Nothing panics; no stream hears anything after its
/// end, and every stream has ended once the session has.
#[doc(hidden)]
pub fn fuzz_session(data: &[u8]) {
    use std::sync::mpsc::{Receiver, channel};

    use conn::{Event, Request};
    use session::{Ending, Session};

    let mut session = Session::new();
    session.take_output();
    let mut streams: Vec<(Receiver<Event>, bool)> = Vec::new();
    for (token, method) in ["GET", "HEAD", "POST"].into_iter().enumerate() {
        let (sender, events) = channel();
        session.open(
            token as u64,
            Request {
                fields: vec![
                    (b":method".to_vec(), method.as_bytes().to_vec()),
                    (b":scheme".to_vec(), b"https".to_vec()),
                    (b":authority".to_vec(), b"a.test".to_vec()),
                    (b":path".to_vec(), b"/".to_vec()),
                ],
                body: (method == "POST").then(|| vec![7; 100_000]),
                idempotent: method == "GET",
            },
            sender,
        );
        streams.push((events, false));
    }
    let take = |session: &mut Session, streams: &mut Vec<(Receiver<Event>, bool)>| {
        for (token, (events, ended)) in streams.iter_mut().enumerate() {
            while let Ok(event) = events.try_recv() {
                assert!(!*ended, "an event after the stream's end: {event:?}");
                match event {
                    Event::Data(bytes) => session.consumed(token as u64, bytes.len()),
                    Event::End | Event::Retry | Event::Failed(_) => *ended = true,
                    Event::Head { .. } => {}
                }
            }
        }
        session.take_output();
    };
    let mut ending = Ending::error(frame::ErrorCode::NO_ERROR, "done");
    for piece in data.split(|&byte| byte == 0xf7) {
        if let Err(error) = session.receive(piece) {
            ending = error;
            break;
        }
        take(&mut session, &mut streams);
    }
    session.end(&ending);
    take(&mut session, &mut streams);
    assert!(session.is_idle());
    assert!(
        streams.iter().all(|(_, ended)| *ended),
        "a stream left open"
    );
}
