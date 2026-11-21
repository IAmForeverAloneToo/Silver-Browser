//! A WebSocket connection against a server of the test's own on
//! 127.0.0.1: the handshake's fields, messages both ways, `Sent`, pings,
//! both closing handshakes, a dropped connection, a breach of the
//! protocol, and a page gone.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::fetch::{Client, Destination, Request};
use crate::websocket::{Agreed, Socket, SocketEvent, accept_for};
use crate::{Origin, Url};

const WAIT: Duration = Duration::from_secs(10);

/// The request's head as the server read it, and the stream after it.
fn handshake_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8];
    while !head.ends_with(b"\r\n\r\n") {
        assert_eq!(stream.read(&mut byte).expect("reads"), 1, "the head ended");
        head.push(byte[0]);
    }
    String::from_utf8(head).expect("ASCII")
}

fn field<'h>(head: &'h str, name: &str) -> Option<&'h str> {
    head.lines().find_map(|line| {
        let (n, value) = line.split_once(':')?;
        n.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

/// The 101 for `head`, with `protocol` chosen, if one.
fn answer(head: &str, protocol: Option<&str>) -> Vec<u8> {
    let key = field(head, "Sec-WebSocket-Key").expect("a key");
    let mut out = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n",
        accept_for(key)
    );
    if let Some(protocol) = protocol {
        out.push_str(&format!("Sec-WebSocket-Protocol: {protocol}\r\n"));
    }
    out.push_str("\r\n");
    out.into_bytes()
}

/// A client's frame, read whole and unmasked: its first byte and its
/// payload.
fn read_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut two = [0u8; 2];
    stream.read_exact(&mut two).expect("a frame");
    assert_eq!(two[1] & 0x80, 0x80, "a client's frames are masked");
    let len = match two[1] & 0x7f {
        126 => {
            let mut b = [0u8; 2];
            stream.read_exact(&mut b).expect("length");
            u64::from(u16::from_be_bytes(b))
        }
        127 => {
            let mut b = [0u8; 8];
            stream.read_exact(&mut b).expect("length");
            u64::from_be_bytes(b)
        }
        n => u64::from(n),
    };
    let mut mask = [0u8; 4];
    stream.read_exact(&mut mask).expect("mask");
    let mut payload = vec![0u8; len as usize];
    stream.read_exact(&mut payload).expect("payload");
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= mask[i % 4];
    }
    (two[0], payload)
}

/// A server's frame: final, unmasked.
fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() < 126);
    let mut out = vec![0x80 | opcode, payload.len() as u8];
    out.extend_from_slice(payload);
    out
}

/// A server for one connection, running `script` on it once it has read
/// the handshake's head, and its URL (`ws:` made `http:`).
fn serve(script: impl FnOnce(TcpStream, String) + Send + 'static) -> (Url, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
    let port = listener.local_addr().expect("an address").port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("a connection");
        stream.set_read_timeout(Some(WAIT)).expect("a timeout");
        let head = handshake_head(&mut stream);
        script(stream, head);
    });
    let url = Url::parse(&format!("http://127.0.0.1:{port}/chat?room=1")).expect("a URL");
    (url, server)
}

fn request(url: Url) -> Request {
    let mut request = Request::get(url, Destination::Empty);
    request.origin = Some(Origin::Tuple {
        scheme: "https".to_owned(),
        host: crate::Host::Domain("example.com".to_owned()),
        port: None,
    });
    request.credentials = crate::fetch::Credentials::Include;
    request
}

fn open(url: Url, protocols: &[&str]) -> (Result<(Agreed, Socket), String>, Receiver<SocketEvent>) {
    let (tx, rx) = mpsc::channel();
    let protocols: Vec<String> = protocols.iter().map(|p| (*p).to_owned()).collect();
    let opened = Client::default().open_websocket(
        &request(url),
        &protocols,
        Box::new(move |event| {
            let _ = tx.send(event);
        }),
    );
    (opened, rx)
}

fn next(rx: &Receiver<SocketEvent>) -> SocketEvent {
    rx.recv_timeout(WAIT).expect("an event in time")
}

#[test]
fn the_handshake_sends_the_reference_browsers_fields_in_its_order() {
    let (heads, got) = mpsc::channel();
    let (url, server) = serve(move |mut stream, head| {
        stream.write_all(&answer(&head, Some("chat"))).unwrap();
        heads.send(head).unwrap();
        let _ = stream.shutdown(Shutdown::Both);
    });
    let (opened, _rx) = open(url, &["chat", "superchat"]);
    let (agreed, _socket) = opened.expect("opens");
    assert_eq!(agreed.protocol, "chat");
    let head = got.recv_timeout(WAIT).unwrap();
    server.join().unwrap();
    assert!(head.starts_with("GET /chat?room=1 HTTP/1.1\r\n"), "{head}");
    let names: Vec<&str> = head
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':').map(|(name, _)| name))
        .collect();
    assert_eq!(
        names,
        [
            "Host",
            "Connection",
            "Pragma",
            "Cache-Control",
            "User-Agent",
            "Upgrade",
            "Origin",
            "Sec-WebSocket-Version",
            "Accept-Encoding",
            "Accept-Language",
            "Sec-WebSocket-Key",
            "Sec-WebSocket-Extensions",
            "Sec-WebSocket-Protocol",
        ]
    );
    assert_eq!(field(&head, "Origin"), Some("https://example.com"));
    assert_eq!(
        field(&head, "Sec-WebSocket-Protocol"),
        Some("chat, superchat")
    );
    assert_eq!(
        field(&head, "Sec-WebSocket-Extensions"),
        Some("permessage-deflate; client_max_window_bits")
    );
}

#[test]
fn messages_go_both_ways_and_the_clients_close_is_clean() {
    let (url, server) = serve(|mut stream, head| {
        // The 101 and a first message in one write: read past the head
        // by the handshake's reader, then handed to the connection.
        let mut first = answer(&head, None);
        first.extend_from_slice(&frame(0x1, b"welcome"));
        stream.write_all(&first).unwrap();
        assert_eq!(read_frame(&mut stream), (0x81, b"hello".to_vec()));
        assert_eq!(read_frame(&mut stream), (0x82, vec![1, 2, 3]));
        stream.write_all(&frame(0x1, b"hi")).unwrap();
        stream.write_all(&frame(0x2, &[9, 8])).unwrap();
        let (first, payload) = read_frame(&mut stream);
        assert_eq!(first, 0x88);
        assert_eq!(payload, [&[0x03, 0xe8][..], b"done"].concat());
        stream.write_all(&frame(0x8, &payload)).unwrap();
        let _ = stream.shutdown(Shutdown::Both);
    });
    let (opened, rx) = open(url, &[]);
    let (_, socket) = opened.expect("opens");
    assert_eq!(next(&rx), SocketEvent::Text("welcome".to_owned()));
    socket.send_text("hello");
    socket.send_binary(&[1, 2, 3]);
    let mut sent = 0;
    while sent < 8 {
        match next(&rx) {
            SocketEvent::Sent(n) => sent += n,
            other => panic!("{other:?} before the sends were written"),
        }
    }
    assert_eq!(sent, 8, "the payloads' bytes, frames' headers left out");
    assert_eq!(next(&rx), SocketEvent::Text("hi".to_owned()));
    assert_eq!(next(&rx), SocketEvent::Binary(vec![9, 8]));
    socket.close(Some(1000), "done");
    assert_eq!(
        next(&rx),
        SocketEvent::Closed {
            clean: true,
            code: 1000,
            reason: "done".to_owned(),
            failed: false
        }
    );
    server.join().unwrap();
}

#[test]
fn the_servers_close_is_echoed_and_clean() {
    let (url, server) = serve(|mut stream, head| {
        stream.write_all(&answer(&head, None)).unwrap();
        let close = [&[0x03, 0xe9][..], b"away"].concat();
        stream.write_all(&frame(0x8, &close)).unwrap();
        assert_eq!(read_frame(&mut stream), (0x88, close));
        let _ = stream.shutdown(Shutdown::Both);
    });
    let (opened, rx) = open(url, &[]);
    let (_, socket) = opened.expect("opens");
    assert_eq!(next(&rx), SocketEvent::Closing);
    assert_eq!(
        next(&rx),
        SocketEvent::Closed {
            clean: true,
            code: 1001,
            reason: "away".to_owned(),
            failed: false
        }
    );
    // Sends once closed go nowhere.
    socket.send_text("late");
    server.join().unwrap();
}

#[test]
fn a_ping_is_answered_with_its_payload() {
    let (url, server) = serve(|mut stream, head| {
        stream.write_all(&answer(&head, None)).unwrap();
        stream.write_all(&frame(0x9, b"are you there")).unwrap();
        assert_eq!(read_frame(&mut stream), (0x8a, b"are you there".to_vec()));
        let _ = stream.shutdown(Shutdown::Both);
    });
    let (opened, rx) = open(url, &[]);
    let _socket = opened.expect("opens").1;
    server.join().unwrap();
    assert_eq!(
        next(&rx),
        SocketEvent::Closed {
            clean: false,
            code: 1006,
            reason: String::new(),
            failed: false
        }
    );
}

#[test]
fn a_breach_of_the_protocol_fails_the_connection_with_its_code() {
    let (url, server) = serve(|mut stream, head| {
        stream.write_all(&answer(&head, None)).unwrap();
        // A server's frame masked (section 5.1).
        stream
            .write_all(&[0x81, 0x82, 0, 0, 0, 0, b'h', b'i'])
            .unwrap();
        let (first, payload) = read_frame(&mut stream);
        assert_eq!(first, 0x88);
        assert_eq!(&payload[..2], &1002u16.to_be_bytes());
    });
    let (opened, rx) = open(url, &[]);
    let _socket = opened.expect("opens").1;
    assert_eq!(
        next(&rx),
        SocketEvent::Closed {
            clean: false,
            code: 1006,
            reason: String::new(),
            failed: true
        }
    );
    server.join().unwrap();
}

#[test]
fn a_page_gone_closes_with_going_away() {
    let (url, server) = serve(|mut stream, head| {
        stream.write_all(&answer(&head, None)).unwrap();
        let (first, payload) = read_frame(&mut stream);
        assert_eq!(first, 0x88);
        assert_eq!(payload, 1001u16.to_be_bytes());
    });
    let (opened, _rx) = open(url, &[]);
    drop(opened.expect("opens"));
    server.join().unwrap();
}

#[test]
fn an_answer_that_is_no_switch_fails_as_the_reference_says() {
    let (url, server) = serve(|mut stream, _| {
        stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
    });
    let (opened, _rx) = open(url, &[]);
    assert_eq!(
        opened.err().as_deref(),
        Some("Error during WebSocket handshake: Unexpected response code: 404")
    );
    server.join().unwrap();

    let (url, server) = serve(|mut stream, _| {
        stream
            .write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: wrong\r\n\r\n")
            .unwrap();
    });
    let (opened, _rx) = open(url, &[]);
    assert!(
        opened
            .err()
            .is_some_and(|why| why.starts_with("Error during WebSocket handshake: ")),
    );
    server.join().unwrap();
}
