//! The WebSocket protocol's client side (G74, RFC 6455, RFC 7692), with
//! no I/O of its own: the opening handshake's key, its request's
//! fields and the check of the server's answer; the frames, read from
//! the bytes as they come and written masked; a message put together
//! from its fragments; `permessage-deflate`, inflated within a bound;
//! and the closing handshake's code and reason.
//!
//! Every message is bounded ([`MAX_MESSAGE`]), what is inflated with it,
//! so that a server's frame header or a deflate stream cannot make the
//! browser hold more than that (a decompression bomb is refused where it
//! passes the bound, before it is held).

use std::fmt;

use flate2::{Decompress, FlushDecompress, Status};

pub use crate::transport::websocket::{CLOSING_TIMEOUT, Socket, SocketEvent};

/// The most bytes one message may hold, inflated: past it the connection
/// is failed with 1009 (message too big). The reference holds no fixed
/// limit; a page's message past this one is out of all its use.
pub const MAX_MESSAGE: usize = 64 << 20;

/// The opening handshake's version (RFC 6455 section 4.1).
pub const VERSION: &str = "13";

/// The GUID the server's accept is hashed with (section 1.3).
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// The extension offered, as the reference offers it.
pub const OFFERED_EXTENSIONS: &str = "permessage-deflate; client_max_window_bits";

/// What the protocol refused: why the connection fails, and the close
/// code it fails with (section 7.4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub code: u16,
    pub reason: &'static str,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.reason, self.code)
    }
}

impl std::error::Error for Failure {}

fn fail(code: u16, reason: &'static str) -> Failure {
    Failure { code, reason }
}

/// A new handshake key: 16 random bytes in base64 (section 4.1).
pub fn new_key() -> String {
    let mut bytes = [0u8; 16];
    let _ = crate::fill_random(&mut bytes);
    crate::csp::base64(&bytes)
}

/// The accept a server answers `key` with (section 4.2.2).
pub fn accept_for(key: &str) -> String {
    let joined = format!("{key}{GUID}");
    let digest = aws_lc_rs::digest::digest(
        &aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY,
        joined.as_bytes(),
    );
    crate::csp::base64(digest.as_ref())
}

/// What the server agreed to: its subprotocol, its extensions as it
/// named them, and how `permessage-deflate` runs, if it does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Agreed {
    pub protocol: String,
    pub extensions: String,
    pub deflate: Option<DeflateParams>,
}

/// `permessage-deflate`'s parameters as agreed (RFC 7692 section 7).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeflateParams {
    /// The server starts each message's compression afresh.
    pub server_no_context_takeover: bool,
    pub client_no_context_takeover: bool,
}

/// Checks the server's answer to the opening handshake (section 4.1,
/// the client's requirements): status 101, `Upgrade: websocket`,
/// `Connection: upgrade`, the accept for `key`, a subprotocol among
/// those `offered` (none if none were), and no extension but
/// `permessage-deflate` with parameters it may have. `field` gives a
/// header field's value, the values of one name joined by commas.
pub fn check_answer(
    status: u16,
    field: &dyn Fn(&str) -> Option<String>,
    key: &str,
    offered: &[String],
) -> Result<Agreed, &'static str> {
    if status != 101 {
        return Err("Unexpected response code");
    }
    let has_token = |name: &str, token: &str| {
        field(name).is_some_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case(token))
        })
    };
    if !has_token("upgrade", "websocket") {
        return Err("'Upgrade' header is missing");
    }
    if !has_token("connection", "upgrade") {
        return Err("'Connection' header is missing");
    }
    match field("sec-websocket-accept") {
        Some(accept) if accept.trim() == accept_for(key) => {}
        Some(_) => return Err("Incorrect 'Sec-WebSocket-Accept' header value"),
        None => return Err("'Sec-WebSocket-Accept' header is missing"),
    }
    let protocol = field("sec-websocket-protocol").map(|p| p.trim().to_owned());
    let protocol = match protocol {
        Some(p) if p.contains(',') => {
            return Err("'Sec-WebSocket-Protocol' header must not appear more than once");
        }
        Some(p) if !offered.contains(&p) => {
            return Err("'Sec-WebSocket-Protocol' header value is not one of those offered");
        }
        Some(p) => p,
        None if !offered.is_empty() => {
            return Err(
                "Sent non-empty 'Sec-WebSocket-Protocol' header but no response was received",
            );
        }
        None => String::new(),
    };
    let mut agreed = Agreed {
        protocol,
        ..Agreed::default()
    };
    if let Some(extensions) = field("sec-websocket-extensions") {
        let extensions = extensions.trim().to_owned();
        for extension in extensions.split(',').filter(|e| !e.trim().is_empty()) {
            let mut parts = extension.split(';').map(str::trim);
            let name = parts.next().unwrap_or_default();
            if !name.eq_ignore_ascii_case("permessage-deflate") || agreed.deflate.is_some() {
                return Err("Found an unsupported extension in 'Sec-WebSocket-Extensions' header");
            }
            let mut params = DeflateParams::default();
            let mut seen: Vec<String> = Vec::new();
            for param in parts {
                let (name, value) = match param.split_once('=') {
                    Some((n, v)) => (
                        n.trim().to_ascii_lowercase(),
                        Some(v.trim().trim_matches('"')),
                    ),
                    None => (param.to_ascii_lowercase(), None),
                };
                if seen.contains(&name) {
                    return Err("Received duplicate permessage-deflate extension parameter");
                }
                seen.push(name.clone());
                match (name.as_str(), value) {
                    ("server_no_context_takeover", None) => {
                        params.server_no_context_takeover = true
                    }
                    ("client_no_context_takeover", None) => {
                        params.client_no_context_takeover = true
                    }
                    ("server_max_window_bits", Some(v)) | ("client_max_window_bits", Some(v))
                        if window_bits(v).is_some() => {}
                    _ => {
                        return Err(
                            "Received an unexpected permessage-deflate extension parameter",
                        );
                    }
                }
            }
            agreed.deflate = Some(params);
        }
        agreed.extensions = extensions;
    }
    Ok(agreed)
}

/// A `*_max_window_bits` value: 8 to 15, no leading zero.
fn window_bits(value: &str) -> Option<u8> {
    if value.starts_with('0') || value.is_empty() || value.len() > 2 {
        return None;
    }
    value
        .parse::<u8>()
        .ok()
        .filter(|bits| (8..=15).contains(bits))
}

/// A frame's opcode (section 5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}

impl Opcode {
    fn of(bits: u8) -> Option<Opcode> {
        Some(match bits {
            0 => Opcode::Continuation,
            1 => Opcode::Text,
            2 => Opcode::Binary,
            8 => Opcode::Close,
            9 => Opcode::Ping,
            10 => Opcode::Pong,
            _ => return None,
        })
    }

    fn bits(self) -> u8 {
        match self {
            Opcode::Continuation => 0,
            Opcode::Text => 1,
            Opcode::Binary => 2,
            Opcode::Close => 8,
            Opcode::Ping => 9,
            Opcode::Pong => 10,
        }
    }

    fn is_control(self) -> bool {
        matches!(self, Opcode::Close | Opcode::Ping | Opcode::Pong)
    }
}

/// A frame the client sends: masked with a fresh key (section 5.3).
pub fn encode(opcode: Opcode, payload: &[u8]) -> Vec<u8> {
    let mut mask = [0u8; 4];
    let _ = crate::fill_random(&mut mask);
    encode_masked(opcode, payload, mask)
}

/// [`encode`] with the mask given: for the tests.
pub fn encode_masked(opcode: Opcode, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let len = payload.len();
    let mut out = Vec::with_capacity(len + 14);
    out.push(0x80 | opcode.bits());
    if len < 126 {
        out.push(0x80 | len as u8);
    } else if len <= usize::from(u16::MAX) {
        out.push(0x80 | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

/// A close frame's payload: `code` and `reason`, or none for no code
/// (section 5.5.1).
pub fn close_payload(code: Option<u16>, reason: &str) -> Vec<u8> {
    match code {
        Some(code) => {
            let mut out = code.to_be_bytes().to_vec();
            out.extend_from_slice(reason.as_bytes());
            out
        }
        None => Vec::new(),
    }
}

/// What the reader made of the bytes so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A whole message, its fragments put together and inflated.
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    /// The server's close: its code and reason, or none.
    Close(Option<(u16, String)>),
}

/// Reads frames from a server's bytes as they come (section 5): one
/// message at a time put together from its fragments, control frames
/// between them, each bounded, `permessage-deflate`'s inflated. Any
/// breach fails the connection with its code.
pub struct Reader {
    buffer: Vec<u8>,
    /// The message being put together: its opcode, whether compressed,
    /// and its bytes so far (compressed ones before inflating).
    partial: Option<(Opcode, bool, Vec<u8>)>,
    inflate: Option<(Decompress, DeflateParams)>,
    closed: bool,
}

impl Reader {
    /// A reader for a connection whose server agreed to `deflate`.
    pub fn new(deflate: Option<DeflateParams>) -> Reader {
        Reader {
            buffer: Vec::new(),
            partial: None,
            inflate: deflate.map(|params| (Decompress::new(false), params)),
            closed: false,
        }
    }

    /// Takes `bytes`, and gives the events they complete; past the first
    /// failure nothing more is read.
    pub fn read(&mut self, bytes: &[u8]) -> Result<Vec<Event>, Failure> {
        if self.closed {
            return Ok(Vec::new());
        }
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        loop {
            match self.frame()? {
                Some(Some(event)) => {
                    let close = matches!(event, Event::Close(_));
                    events.push(event);
                    if close {
                        self.closed = true;
                        self.buffer.clear();
                        break;
                    }
                }
                Some(None) => {}
                None => break,
            }
        }
        Ok(events)
    }

    /// One frame off the buffer: `None` until it is all there; inside,
    /// the event it completes, if any.
    fn frame(&mut self) -> Result<Option<Option<Event>>, Failure> {
        let b = &self.buffer;
        if b.len() < 2 {
            return Ok(None);
        }
        let fin = b[0] & 0x80 != 0;
        let rsv1 = b[0] & 0x40 != 0;
        if b[0] & 0x30 != 0 {
            return Err(fail(1002, "One or more reserved bits are on"));
        }
        let opcode = Opcode::of(b[0] & 0x0f).ok_or(fail(1002, "Unrecognized frame opcode"))?;
        if b[1] & 0x80 != 0 {
            return Err(fail(
                1002,
                "A server must not mask any frames that it sends to the client",
            ));
        }
        let (len, header) = match b[1] & 0x7f {
            126 => {
                if b.len() < 4 {
                    return Ok(None);
                }
                let len = usize::from(u16::from_be_bytes([b[2], b[3]]));
                if len < 126 {
                    return Err(fail(
                        1002,
                        "The minimal number of bytes MUST be used to encode the length",
                    ));
                }
                (len, 4)
            }
            127 => {
                if b.len() < 10 {
                    return Ok(None);
                }
                let mut eight = [0u8; 8];
                eight.copy_from_slice(&b[2..10]);
                let len = u64::from_be_bytes(eight);
                if len >> 63 != 0 {
                    return Err(fail(
                        1002,
                        "The most significant bit of the length must be 0",
                    ));
                }
                if len <= u64::from(u16::MAX) {
                    return Err(fail(
                        1002,
                        "The minimal number of bytes MUST be used to encode the length",
                    ));
                }
                if len > MAX_MESSAGE as u64 {
                    return Err(fail(1009, "Message too big"));
                }
                (len as usize, 10)
            }
            n => (usize::from(n), 2),
        };
        if opcode.is_control() {
            if !fin {
                return Err(fail(1002, "Received fragmented control frame"));
            }
            if len > 125 {
                return Err(fail(1002, "Received control frame having too long payload"));
            }
            if rsv1 {
                return Err(fail(1002, "One or more reserved bits are on"));
            }
        }
        if b.len() < header + len {
            // A frame that would pass the bound is refused before its
            // bytes are held.
            if self.held() + len > MAX_MESSAGE {
                return Err(fail(1009, "Message too big"));
            }
            return Ok(None);
        }
        let payload: Vec<u8> = self.buffer[header..header + len].to_vec();
        self.buffer.drain(..header + len);
        if opcode.is_control() {
            return Ok(Some(Some(match opcode {
                Opcode::Ping => Event::Ping(payload),
                Opcode::Pong => Event::Pong(payload),
                _ => Event::Close(close_of(&payload)?),
            })));
        }
        match (opcode, &mut self.partial) {
            (Opcode::Continuation, None) => {
                return Err(fail(1002, "Received unexpected continuation frame"));
            }
            (Opcode::Continuation, Some(_)) if rsv1 => {
                return Err(fail(1002, "One or more reserved bits are on"));
            }
            (Opcode::Continuation, Some((_, _, bytes))) => {
                if bytes.len() + payload.len() > MAX_MESSAGE {
                    return Err(fail(1009, "Message too big"));
                }
                bytes.extend_from_slice(&payload);
            }
            (_, Some(_)) => {
                return Err(fail(
                    1002,
                    "Received start of new message but previous message is unfinished",
                ));
            }
            (opcode, None) => {
                if rsv1 && self.inflate.is_none() {
                    return Err(fail(1002, "One or more reserved bits are on"));
                }
                self.partial = Some((opcode, rsv1, payload));
            }
        }
        if !fin {
            return Ok(Some(None));
        }
        let (opcode, compressed, bytes) = self.partial.take().expect("a message under way");
        let bytes = if compressed {
            self.inflated(&bytes)?
        } else {
            bytes
        };
        Ok(Some(Some(match opcode {
            Opcode::Text => Event::Text(
                String::from_utf8(bytes)
                    .map_err(|_| fail(1007, "Could not decode a text frame as UTF-8."))?,
            ),
            _ => Event::Binary(bytes),
        })))
    }

    /// The bytes held now: the buffer's and the message's so far.
    fn held(&self) -> usize {
        self.buffer.len() + self.partial.as_ref().map_or(0, |(_, _, b)| b.len())
    }

    /// A compressed message inflated (RFC 7692 section 7.2.2): its bytes
    /// and the tail its sender left off, never past [`MAX_MESSAGE`].
    fn inflated(&mut self, bytes: &[u8]) -> Result<Vec<u8>, Failure> {
        let (inflate, params) = self
            .inflate
            .as_mut()
            .ok_or(fail(1002, "One or more reserved bits are on"))?;
        let mut input = bytes.to_vec();
        input.extend_from_slice(&[0x00, 0x00, 0xff, 0xff]);
        let mut out: Vec<u8> = Vec::new();
        let mut taken = 0usize;
        loop {
            if out.len() > MAX_MESSAGE {
                return Err(fail(1009, "Message too big"));
            }
            // Room for a slice more, one byte past the bound at most, so
            // that a stream that passes it is seen to.
            let room = (MAX_MESSAGE + 1 - out.len()).min(64 << 10);
            out.reserve(room);
            let (before_in, before_out) = (inflate.total_in(), inflate.total_out());
            let status = inflate
                .decompress_vec(&input[taken..], &mut out, FlushDecompress::Sync)
                .map_err(|_| fail(1002, "Decompression failed"))?;
            let consumed = (inflate.total_in() - before_in) as usize;
            let produced = (inflate.total_out() - before_out) as usize;
            taken += consumed;
            if out.len() > MAX_MESSAGE {
                return Err(fail(1009, "Message too big"));
            }
            let spare = out.len() < out.capacity();
            if status == Status::StreamEnd
                || (taken >= input.len() && spare)
                || (consumed == 0 && produced == 0)
            {
                break;
            }
        }
        if params.server_no_context_takeover {
            inflate.reset(false);
        }
        Ok(out)
    }
}

/// A close frame's code and reason (section 5.5.1, 7.4): none for an
/// empty payload; a code a server may not send, or a reason not UTF-8,
/// fails.
fn close_of(payload: &[u8]) -> Result<Option<(u16, String)>, Failure> {
    match payload.len() {
        0 => Ok(None),
        1 => Err(fail(
            1002,
            "Received a broken close frame containing an invalid size body",
        )),
        _ => {
            let code = u16::from_be_bytes([payload[0], payload[1]]);
            let allowed = matches!(code, 1000..=1003 | 1007..=1014 | 3000..=4999);
            if !allowed {
                return Err(fail(
                    1002,
                    "Received a broken close frame containing a reserved status code",
                ));
            }
            let reason = String::from_utf8(payload[2..].to_vec()).map_err(|_| {
                fail(
                    1007,
                    "Received a broken close frame containing invalid UTF-8",
                )
            })?;
            Ok(Some((code, reason)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};

    /// A server's frame: unmasked.
    fn server(first: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![first];
        let len = payload.len();
        if len < 126 {
            out.push(len as u8);
        } else if len <= 65535 {
            out.push(126);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            out.push(127);
            out.extend_from_slice(&(len as u64).to_be_bytes());
        }
        out.extend_from_slice(payload);
        out
    }

    fn deflated(bytes: &[u8]) -> Vec<u8> {
        let mut c = Compress::new(Compression::default(), false);
        let mut out = Vec::with_capacity(bytes.len() + 64);
        c.compress_vec(bytes, &mut out, FlushCompress::Sync)
            .expect("compresses");
        assert!(out.ends_with(&[0, 0, 0xff, 0xff]));
        out.truncate(out.len() - 4);
        out
    }

    /// The sample nonce of RFC 6455 section 1.3, sent as the opening
    /// handshake's Sec-WebSocket-Key.
    const RFC_NONCE: &str = "dGhlIHNhbXBsZSBub25jZQ==";

    /// RFC 6455 section 1.3's example.
    #[test]
    fn the_accept_is_the_rfcs() {
        assert_eq!(accept_for(RFC_NONCE), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
        assert_eq!(new_key().len(), 24);
        assert_ne!(new_key(), new_key());
    }

    #[test]
    fn an_answer_is_checked_as_the_client_must() {
        let fields = |extra: &'static [(&'static str, &'static str)]| {
            move |name: &str| -> Option<String> {
                let base = [
                    ("upgrade", "WebSocket"),
                    ("connection", "keep-alive, Upgrade"),
                    ("sec-websocket-accept", "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="),
                ];
                extra
                    .iter()
                    .chain(base.iter())
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        let agreed = check_answer(101, &fields(&[]), RFC_NONCE, &[]).expect("agreed");
        assert_eq!(agreed, Agreed::default());
        assert!(check_answer(200, &fields(&[]), RFC_NONCE, &[]).is_err());
        let bad = fields(&[("sec-websocket-accept", "nope")]);
        assert!(check_answer(101, &bad, RFC_NONCE, &[]).is_err());
        let offered = vec!["chat".to_owned(), "superchat".to_owned()];
        let chosen = fields(&[("sec-websocket-protocol", "superchat")]);
        assert_eq!(
            check_answer(101, &chosen, RFC_NONCE, &offered)
                .expect("ok")
                .protocol,
            "superchat"
        );
        assert!(check_answer(101, &fields(&[]), RFC_NONCE, &offered).is_err());
        let other = fields(&[("sec-websocket-protocol", "other")]);
        assert!(check_answer(101, &other, RFC_NONCE, &offered).is_err());
        let deflate = fields(&[(
            "sec-websocket-extensions",
            "permessage-deflate; server_no_context_takeover; client_max_window_bits=15",
        )]);
        let agreed = check_answer(101, &deflate, RFC_NONCE, &[]).expect("ok");
        assert_eq!(
            agreed.deflate,
            Some(DeflateParams {
                server_no_context_takeover: true,
                client_no_context_takeover: false
            })
        );
        let unknown = fields(&[("sec-websocket-extensions", "x-webkit-deflate-frame")]);
        assert!(check_answer(101, &unknown, RFC_NONCE, &[]).is_err());
        let bits = fields(&[(
            "sec-websocket-extensions",
            "permessage-deflate; server_max_window_bits=7",
        )]);
        assert!(check_answer(101, &bits, RFC_NONCE, &[]).is_err());
    }

    #[test]
    fn frames_are_read_as_they_come_and_put_together() {
        let mut r = Reader::new(None);
        let mut bytes = server(0x01, b"Hel");
        bytes.extend(server(0x89, b"p"));
        bytes.extend(server(0x80, b"lo"));
        // A byte at a time.
        let mut events = Vec::new();
        for b in &bytes {
            events.extend(r.read(&[*b]).expect("reads"));
        }
        assert_eq!(
            events,
            [Event::Ping(b"p".to_vec()), Event::Text("Hello".to_owned())]
        );
        let big = vec![7u8; 70_000];
        assert_eq!(
            r.read(&server(0x82, &big)).expect("reads"),
            [Event::Binary(big)]
        );
        let mut close = 1000u16.to_be_bytes().to_vec();
        close.extend_from_slice(b"bye");
        assert_eq!(
            r.read(&server(0x88, &close)).expect("reads"),
            [Event::Close(Some((1000, "bye".to_owned())))]
        );
        assert_eq!(r.read(&server(0x81, b"after")).expect("reads"), []);
    }

    #[test]
    fn a_frame_the_protocol_forbids_fails_with_its_code() {
        let cases: [(Vec<u8>, u16); 9] = [
            (vec![0x81, 0x80, 0, 0, 0, 0], 1002), // masked by the server
            (vec![0xc1, 0x00], 1002),             // RSV1 without deflate
            (vec![0x83, 0x00], 1002),             // reserved opcode
            (vec![0x09, 0x00], 1002),             // fragmented ping
            (server(0x89, &[0u8; 126]), 1002),    // long control frame
            (vec![0x80, 0x00], 1002),             // continuation of nothing
            (server(0x81, &[0xff, 0xfe]), 1007),  // not UTF-8
            (vec![0x81, 126, 0, 5], 1002),        // length not minimal
            (server(0x88, &[0x03, 0xec]), 1002),  // close code 1004
        ];
        for (bytes, code) in cases {
            let err = Reader::new(None).read(&bytes).expect_err("fails");
            assert_eq!(err.code, code, "{bytes:?}");
        }
        // A masked frame whose mask and payload would read as two text
        // frames were the mask let by: refused, as masked.
        let masked = [0x81, 0x82, b'h', b'i', 0x81, 0x02, b'o', b'k'];
        assert_eq!(
            Reader::new(None).read(&masked).expect_err("masked").code,
            1002
        );
        let mut header = vec![0x82, 127];
        header.extend_from_slice(&((MAX_MESSAGE as u64) + 1).to_be_bytes());
        assert_eq!(
            Reader::new(None).read(&header).expect_err("too big").code,
            1009
        );
        // Fragments that pass the bound together.
        let mut r = Reader::new(None);
        let chunk = vec![0u8; 1 << 20];
        let mut first = true;
        let mut failed = None;
        for _ in 0..(MAX_MESSAGE / chunk.len() + 2) {
            let frame = server(if first { 0x02 } else { 0x00 }, &chunk);
            first = false;
            if let Err(e) = r.read(&frame) {
                failed = Some(e.code);
                break;
            }
        }
        assert_eq!(failed, Some(1009));
    }

    #[test]
    fn deflated_messages_are_inflated_with_or_without_context() {
        let params = DeflateParams::default();
        let mut r = Reader::new(Some(params));
        for text in ["Hello", "Hello again, Hello again"] {
            let mut c = Compress::new(Compression::default(), false);
            let mut out = Vec::with_capacity(256);
            c.compress_vec(text.as_bytes(), &mut out, FlushCompress::Sync)
                .expect("compresses");
            out.truncate(out.len() - 4);
            let mut fresh = Reader::new(Some(DeflateParams {
                server_no_context_takeover: true,
                client_no_context_takeover: false,
            }));
            assert_eq!(
                fresh.read(&server(0xc1, &out)).expect("reads"),
                [Event::Text(text.to_owned())]
            );
        }
        // Context kept across messages: the second refers to the first.
        let mut c = Compress::new(Compression::default(), false);
        let mut first = Vec::with_capacity(256);
        c.compress_vec(b"abcabcabc", &mut first, FlushCompress::Sync)
            .expect("compresses");
        first.truncate(first.len() - 4);
        let mut second = Vec::with_capacity(256);
        c.compress_vec(b"abcabcabc", &mut second, FlushCompress::Sync)
            .expect("compresses");
        second.truncate(second.len() - 4);
        assert_eq!(
            r.read(&server(0xc1, &first)).expect("reads"),
            [Event::Text("abcabcabc".into())]
        );
        assert_eq!(
            r.read(&server(0xc1, &second)).expect("reads"),
            [Event::Text("abcabcabc".into())]
        );
        assert!(!deflated(b"x").is_empty());
    }

    /// A deflate stream that inflates past the bound is refused before
    /// it is held: a decompression bomb.
    #[test]
    fn a_decompression_bomb_is_refused() {
        let zeros = vec![0u8; MAX_MESSAGE + 1024];
        let bomb = deflated(&zeros);
        assert!(bomb.len() < 1 << 20, "{}", bomb.len());
        let mut r = Reader::new(Some(DeflateParams::default()));
        assert_eq!(
            r.read(&server(0xc2, &bomb)).expect_err("refused").code,
            1009
        );
    }

    #[test]
    fn client_frames_are_masked() {
        let frame = encode_masked(Opcode::Text, b"Hello", [0x37, 0xfa, 0x21, 0x3d]);
        // RFC 6455 section 5.7's example.
        assert_eq!(
            frame,
            [
                0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58
            ]
        );
        let long = encode(Opcode::Binary, &[1u8; 300]);
        assert_eq!(&long[..4], &[0x82, 0x80 | 126, 0x01, 0x2c]);
        assert_eq!(close_payload(Some(1000), "ok"), [0x03, 0xe8, b'o', b'k']);
    }
}
