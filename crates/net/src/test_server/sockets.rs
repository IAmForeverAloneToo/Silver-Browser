//! A WebSocket server for tests (G74): a listener on 127.0.0.1 whose
//! connections a test drives by hand, frame by frame, so that the
//! engine's tests need no network types of their own (T34).

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::websocket::accept_for;

/// How long a peer waits for a byte before the test fails.
const WAIT: Duration = Duration::from_secs(10);

/// A listener on 127.0.0.1, on a port of its own.
pub struct SocketListener {
    listener: TcpListener,
}

impl SocketListener {
    pub fn bind() -> SocketListener {
        SocketListener {
            listener: TcpListener::bind("127.0.0.1:0").expect("binds"),
        }
    }

    pub fn port(&self) -> u16 {
        self.listener.local_addr().expect("an address").port()
    }

    /// The next connection, waited for.
    pub fn accept(&self) -> SocketPeer {
        let (stream, _) = self.listener.accept().expect("a connection");
        stream.set_read_timeout(Some(WAIT)).expect("a timeout");
        SocketPeer { stream }
    }

    /// Whether no connection waits to be accepted now.
    pub fn nothing_waiting(&self) -> bool {
        self.listener.set_nonblocking(true).expect("non-blocking");
        let waiting = self.listener.accept().err().map(|e| e.kind());
        self.listener.set_nonblocking(false).expect("blocking");
        waiting == Some(ErrorKind::WouldBlock)
    }
}

/// A port nothing listens on (one bound, then let go).
pub fn closed_port() -> u16 {
    SocketListener::bind().port()
}

/// One connection, as its server sees it.
pub struct SocketPeer {
    stream: TcpStream,
}

impl SocketPeer {
    /// The request's head, through its blank line.
    pub fn head(&mut self) -> String {
        let mut head = Vec::new();
        let mut byte = [0u8];
        while !head.ends_with(b"\r\n\r\n") {
            assert_eq!(self.stream.read(&mut byte).expect("reads"), 1);
            head.push(byte[0]);
        }
        String::from_utf8(head).expect("ASCII")
    }

    /// Answers the handshake whose head is `head` with a 101, naming
    /// `protocol` if there is one.
    pub fn switch(&mut self, head: &str, protocol: Option<&str>) {
        let key = head
            .lines()
            .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
            .expect("a key");
        let protocol = protocol
            .map(|protocol| format!("Sec-WebSocket-Protocol: {protocol}\r\n"))
            .unwrap_or_default();
        let answer = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n{protocol}\r\n",
            accept_for(key)
        );
        self.write(answer.as_bytes());
    }

    /// Writes `bytes` as they are: frames, or an HTTP answer.
    pub fn write(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).expect("writes");
    }

    /// Writes `bytes`, whether or not the client is still there.
    pub fn try_write(&mut self, bytes: &[u8]) {
        let _ = self.stream.write_all(bytes);
    }

    /// The client's next frame, unmasked, of any length: its first byte
    /// and its payload.
    pub fn frame(&mut self) -> (u8, Vec<u8>) {
        let mut two = [0u8; 2];
        self.stream.read_exact(&mut two).expect("a frame");
        let len = match two[1] & 0x7f {
            126 => {
                let mut b = [0u8; 2];
                self.stream.read_exact(&mut b).expect("its length");
                usize::from(u16::from_be_bytes(b))
            }
            127 => {
                let mut b = [0u8; 8];
                self.stream.read_exact(&mut b).expect("its length");
                usize::try_from(u64::from_be_bytes(b)).expect("a length")
            }
            n => usize::from(n),
        };
        let mut mask = [0u8; 4];
        self.stream.read_exact(&mut mask).expect("its mask");
        let mut payload = vec![0u8; len];
        self.stream.read_exact(&mut payload).expect("its payload");
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
        (two[0], payload)
    }

    /// Ends the connection both ways.
    pub fn shutdown(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

/// A server frame (unmasked) of `opcode` and a short `payload`.
pub fn server_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() < 126, "a short frame");
    let mut out = vec![0x80 | opcode, payload.len() as u8];
    out.extend_from_slice(payload);
    out
}

/// A WebSocket server for one connection: its handshake's head given to
/// `script` with the peer after the 101 (naming `protocol` if there is
/// one); its port and its thread.
pub fn serve_socket(
    protocol: Option<&'static str>,
    script: impl FnOnce(String, SocketPeer) + Send + 'static,
) -> (u16, JoinHandle<()>) {
    let listener = SocketListener::bind();
    let port = listener.port();
    let server = thread::spawn(move || {
        let mut peer = listener.accept();
        let head = peer.head();
        peer.switch(&head, protocol);
        script(head, peer);
    });
    (port, server)
}
