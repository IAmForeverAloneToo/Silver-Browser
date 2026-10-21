//! The HTTP/2 session driven frame by frame (M38): what it sends within
//! the server's windows, what it takes of the server's, and the frames
//! that end a stream or the connection.

use std::sync::mpsc::{Receiver, channel};

use super::conn::{Event, H2_STREAM_WINDOW, Request, StreamError};
use super::frame::{self, ErrorCode, Frame, setting};
use super::hpack::{Decoder, Encoder};
use super::session::{DEFAULT_WINDOW, Session};

/// The frames in `bytes`, after the preface if it begins them.
fn frames(bytes: &[u8]) -> Vec<Frame> {
    let mut bytes = bytes.strip_prefix(frame::PREFACE).unwrap_or(bytes);
    let mut out = Vec::new();
    while let Some((frame, length)) = frame::read(bytes, usize::MAX).unwrap() {
        out.push(frame);
        bytes = &bytes[length..];
    }
    out
}

/// A session past the server's settings, its own first frames taken.
fn begun(server_settings: &[(u16, u32)]) -> Session {
    let mut session = Session::new();
    session.take_output();
    let settings = Frame::Settings {
        ack: false,
        values: server_settings.to_vec(),
    };
    session.receive(&settings.to_bytes()).unwrap();
    session.take_output();
    session
}

/// Opens a `method` stream with `body`; its events.
fn open(session: &mut Session, token: u64, method: &str, body: Option<Vec<u8>>) -> Receiver<Event> {
    let (sender, events) = channel();
    session.open(
        token,
        Request {
            fields: vec![
                (b":method".to_vec(), method.as_bytes().to_vec()),
                (b":scheme".to_vec(), b"https".to_vec()),
                (b":authority".to_vec(), b"a.test".to_vec()),
                (b":path".to_vec(), b"/".to_vec()),
            ],
            body,
            idempotent: method == "GET",
        },
        sender,
    );
    events
}

/// The server's head on `stream`, from `encoder`.
fn head(encoder: &mut Encoder, stream: u32, fields: &[(&str, &str)], end: bool) -> Vec<u8> {
    let block = encoder.encode(
        fields
            .iter()
            .map(|(name, value)| (name.as_bytes(), value.as_bytes())),
    );
    let mut out = Vec::new();
    frame::write_headers(&mut out, stream, &block, end, frame::MAX_FRAME_SIZE);
    out
}

fn data(stream: u32, bytes: &[u8], end: bool) -> Vec<u8> {
    Frame::Data {
        stream,
        end_stream: end,
        data: bytes.to_vec(),
        flow: bytes.len(),
    }
    .to_bytes()
}

/// How many bytes of `DATA` `bytes` holds.
fn data_sent(bytes: &[u8]) -> usize {
    frames(bytes)
        .iter()
        .map(|frame| match frame {
            Frame::Data { data, .. } => data.len(),
            _ => 0,
        })
        .sum()
}

#[test]
fn the_session_begins_with_its_settings_and_a_larger_window() {
    let mut session = Session::new();
    let output = session.take_output();
    assert!(output.starts_with(frame::PREFACE));
    let first = frames(&output);
    let Frame::Settings { ack: false, values } = &first[0] else {
        panic!("{first:?}");
    };
    assert!(values.contains(&(setting::ENABLE_PUSH, 0)));
    assert!(values.contains(&(setting::INITIAL_WINDOW_SIZE, H2_STREAM_WINDOW)));
    assert!(matches!(first[1], Frame::WindowUpdate { stream: 0, .. }));
    // A server whose first frame is not its settings is refused.
    let mut session = Session::new();
    let ping = Frame::Ping {
        ack: false,
        data: [0; 8],
    };
    assert!(session.receive(&ping.to_bytes()).is_err());
}

#[test]
fn a_body_is_sent_within_the_servers_windows() {
    let mut session = begun(&[]);
    let _events = open(&mut session, 1, "POST", Some(vec![1; 200_000]));
    // The default windows: 65,535 bytes, the stream's and the
    // connection's alike.
    let sent = data_sent(&session.take_output());
    assert_eq!(sent, DEFAULT_WINDOW as usize);
    assert_eq!(session.send_window(), 0);
    // The stream's window alone grows: the connection's still holds it.
    let update = Frame::WindowUpdate {
        stream: 1,
        increment: 100_000,
    };
    session.receive(&update.to_bytes()).unwrap();
    assert_eq!(data_sent(&session.take_output()), 0);
    // Then the connection's: as much as the stream's has room for.
    let update = Frame::WindowUpdate {
        stream: 0,
        increment: 1_000_000,
    };
    session.receive(&update.to_bytes()).unwrap();
    assert_eq!(data_sent(&session.take_output()), 100_000);
    // And a smaller initial window from a setting moves the stream's.
    let mut session = begun(&[(setting::INITIAL_WINDOW_SIZE, 1_000)]);
    let _events = open(&mut session, 1, "POST", Some(vec![1; 5_000]));
    assert_eq!(data_sent(&session.take_output()), 1_000);
}

#[test]
fn a_response_comes_whole_and_its_windows_are_given_back_as_it_is_taken() {
    let mut session = begun(&[]);
    let events = open(&mut session, 7, "GET", None);
    let mut encoder = Encoder::default();
    let mut bytes = head(
        &mut encoder,
        1,
        &[(":status", "200"), ("content-length", "6")],
        false,
    );
    bytes.extend(data(1, b"hel", false));
    bytes.extend(data(1, b"lo!", true));
    session.receive(&bytes).unwrap();
    assert!(matches!(
        events.recv().unwrap(),
        Event::Head { status: 200, .. }
    ));
    assert!(matches!(events.recv().unwrap(), Event::Data(d) if d == b"hel"));
    assert!(matches!(events.recv().unwrap(), Event::Data(d) if d == b"lo!"));
    assert!(matches!(events.recv().unwrap(), Event::End));
    assert!(session.is_idle());
    // A large body: no window given back until half is taken.
    let events = open(&mut session, 8, "GET", None);
    session.take_output();
    let mut bytes = head(&mut encoder, 3, &[(":status", "200")], false);
    for _ in 0..200 {
        bytes.extend(data(3, &[0; 16_384], false));
    }
    session.receive(&bytes).unwrap();
    assert!(
        !frames(&session.take_output())
            .iter()
            .any(|frame| matches!(frame, Frame::WindowUpdate { .. })),
        "nothing taken yet"
    );
    session.consumed(8, H2_STREAM_WINDOW as usize / 2);
    assert!(
        frames(&session.take_output())
            .iter()
            .any(|frame| matches!(frame, Frame::WindowUpdate { stream: 3, .. }))
    );
    drop(events);
}

#[test]
fn a_server_past_this_ends_windows_is_refused() {
    let mut session = begun(&[]);
    let events = open(&mut session, 1, "GET", None);
    let mut encoder = Encoder::default();
    session
        .receive(&head(&mut encoder, 1, &[(":status", "200")], false))
        .unwrap();
    let _ = events.recv();
    // Past the stream's window: the stream is reset.
    let mut sent = 0;
    while sent <= H2_STREAM_WINDOW as usize {
        session.receive(&data(1, &[0; 16_384], false)).unwrap();
        sent += 16_384;
    }
    let mut failed = false;
    while let Ok(event) = events.try_recv() {
        failed |= matches!(
            event,
            Event::Failed(StreamError::Reset(ErrorCode::FLOW_CONTROL_ERROR))
        );
    }
    assert!(failed);
}

#[test]
fn a_body_past_its_length_is_refused_as_it_comes() {
    let mut session = begun(&[]);
    let events = open(&mut session, 1, "GET", None);
    let mut encoder = Encoder::default();
    let mut bytes = head(
        &mut encoder,
        1,
        &[(":status", "200"), ("content-length", "2")],
        false,
    );
    // More than its length, before its end.
    bytes.extend(data(1, b"three", false));
    session.receive(&bytes).unwrap();
    let _head = events.recv().unwrap();
    assert!(matches!(
        events.recv().unwrap(),
        Event::Failed(StreamError::Malformed(_))
    ));
    assert!(frames(&session.take_output()).iter().any(|frame| matches!(
        frame,
        Frame::RstStream {
            stream: 1,
            code: ErrorCode::PROTOCOL_ERROR
        }
    )));
}

#[test]
fn a_head_requests_response_carries_no_body_whatever_its_length() {
    let mut session = begun(&[]);
    let events = open(&mut session, 1, "HEAD", None);
    let mut encoder = Encoder::default();
    session
        .receive(&head(
            &mut encoder,
            1,
            &[(":status", "200"), ("content-length", "1000")],
            true,
        ))
        .unwrap();
    assert!(matches!(
        events.recv().unwrap(),
        Event::Head { status: 200, .. }
    ));
    assert!(matches!(events.recv().unwrap(), Event::End));
}

#[test]
fn frames_that_end_the_connection() {
    let push = Frame::PushPromise {
        stream: 1,
        promised: 2,
        end_headers: true,
        block: vec![0x82],
    }
    .to_bytes();
    let headers_on_an_idle_stream = head(&mut Encoder::default(), 5, &[(":status", "200")], true);
    let window_past = Frame::WindowUpdate {
        stream: 0,
        increment: 0x7fff_ffff,
    }
    .to_bytes();
    for (what, bytes) in [
        ("a push", push),
        ("headers on an idle stream", headers_on_an_idle_stream),
        ("a window past its bound", window_past),
    ] {
        let mut session = begun(&[]);
        let _events = open(&mut session, 1, "GET", None);
        assert!(session.receive(&bytes).is_err(), "{what}");
    }
    // Too many pings in a second.
    let mut session = begun(&[]);
    let ping = Frame::Ping {
        ack: false,
        data: [1; 8],
    }
    .to_bytes();
    let mut ended = false;
    for _ in 0..1_100 {
        if let Err(ending) = session.receive(&ping) {
            assert_eq!(ending.code, ErrorCode::ENHANCE_YOUR_CALM);
            ended = true;
            break;
        }
    }
    assert!(ended);
}

#[test]
fn a_goaway_retries_the_streams_it_did_not_process() {
    let mut session = begun(&[]);
    let first = open(&mut session, 1, "GET", None);
    let second = open(&mut session, 2, "GET", None);
    let away = Frame::GoAway {
        last_stream: 1,
        code: ErrorCode::NO_ERROR,
        debug: Vec::new(),
    };
    session.receive(&away.to_bytes()).unwrap();
    assert!(session.going_away());
    assert!(matches!(second.recv().unwrap(), Event::Retry));
    assert!(first.try_recv().is_err(), "stream 1 goes on");
    // A new stream is not opened on it.
    let third = open(&mut session, 3, "GET", None);
    assert!(matches!(third.recv().unwrap(), Event::Retry));
    // The session ended: a stream that heard nothing of a GET retries.
    session.end(&super::session::Ending::error(
        ErrorCode::NO_ERROR,
        "closed",
    ));
    assert!(matches!(first.recv().unwrap(), Event::Retry));
}

#[test]
fn a_cancelled_stream_is_reset_and_a_shared_table_kept() {
    let mut session = begun(&[]);
    let _events = open(&mut session, 1, "GET", None);
    session.take_output();
    session.cancel(1);
    assert_eq!(
        frames(&session.take_output()),
        [Frame::RstStream {
            stream: 1,
            code: ErrorCode::CANCEL
        }]
    );
    // Its head, coming after, is decoded all the same: the next uses
    // the table it filled.
    let mut encoder = Encoder::default();
    session
        .receive(&head(
            &mut encoder,
            1,
            &[(":status", "200"), ("x-kept", "1")],
            true,
        ))
        .unwrap();
    let events = open(&mut session, 2, "GET", None);
    session
        .receive(&head(
            &mut encoder,
            3,
            &[(":status", "200"), ("x-kept", "1")],
            true,
        ))
        .unwrap();
    let Event::Head { fields, .. } = events.recv().unwrap() else {
        panic!("a head");
    };
    assert_eq!(fields, [(b"x-kept".to_vec(), b"1".to_vec())]);
    // The request's headers decode as a server reads them.
    let mut decoder = Decoder::default();
    let mut session = Session::new();
    let _ = open(&mut session, 1, "GET", None);
    let request = frames(&session.take_output());
    let Some(Frame::Headers { block, .. }) = request.last() else {
        panic!("{request:?}");
    };
    let fields = decoder.decode(block, usize::MAX).unwrap().fields;
    assert_eq!(fields[0], (b":method".to_vec(), b"GET".to_vec()));
}
