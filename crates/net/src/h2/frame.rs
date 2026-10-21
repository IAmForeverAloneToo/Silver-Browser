//! HTTP/2's frames (RFC 9113 sections 4 and 6): read from bytes that
//! may hold part of a frame, each checked as section 6 says before it
//! is a value, and written back. What a frame's place in a stream's
//! life allows is the connection's to judge (`conn`); this judges each
//! frame alone.

/// The largest frame payload either end takes until it says otherwise
/// (`SETTINGS_MAX_FRAME_SIZE`'s initial value), which this end keeps.
pub const MAX_FRAME_SIZE: usize = 16_384;

/// The largest frame payload the protocol allows a peer to announce.
pub const MAX_FRAME_SIZE_LIMIT: usize = (1 << 24) - 1;

/// A frame header's length.
pub const HEADER_LENGTH: usize = 9;

/// The client connection preface (3.4).
pub const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// An error code (section 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ErrorCode(pub u32);

impl ErrorCode {
    pub const NO_ERROR: ErrorCode = ErrorCode(0x0);
    pub const PROTOCOL_ERROR: ErrorCode = ErrorCode(0x1);
    pub const INTERNAL_ERROR: ErrorCode = ErrorCode(0x2);
    pub const FLOW_CONTROL_ERROR: ErrorCode = ErrorCode(0x3);
    pub const SETTINGS_TIMEOUT: ErrorCode = ErrorCode(0x4);
    pub const STREAM_CLOSED: ErrorCode = ErrorCode(0x5);
    pub const FRAME_SIZE_ERROR: ErrorCode = ErrorCode(0x6);
    pub const REFUSED_STREAM: ErrorCode = ErrorCode(0x7);
    pub const CANCEL: ErrorCode = ErrorCode(0x8);
    pub const COMPRESSION_ERROR: ErrorCode = ErrorCode(0x9);
    pub const CONNECT_ERROR: ErrorCode = ErrorCode(0xa);
    pub const ENHANCE_YOUR_CALM: ErrorCode = ErrorCode(0xb);
    pub const INADEQUATE_SECURITY: ErrorCode = ErrorCode(0xc);
    pub const HTTP_1_1_REQUIRED: ErrorCode = ErrorCode(0xd);

    /// The code's name, as section 7 gives it, or its number.
    pub fn name(self) -> String {
        let names = [
            "NO_ERROR",
            "PROTOCOL_ERROR",
            "INTERNAL_ERROR",
            "FLOW_CONTROL_ERROR",
            "SETTINGS_TIMEOUT",
            "STREAM_CLOSED",
            "FRAME_SIZE_ERROR",
            "REFUSED_STREAM",
            "CANCEL",
            "COMPRESSION_ERROR",
            "CONNECT_ERROR",
            "ENHANCE_YOUR_CALM",
            "INADEQUATE_SECURITY",
            "HTTP_1_1_REQUIRED",
        ];
        match names.get(self.0 as usize) {
            Some(name) => (*name).to_owned(),
            None => format!("{:#x}", self.0),
        }
    }
}

/// A setting's identifier (6.5.2, RFC 9218 2.1).
pub mod setting {
    pub const HEADER_TABLE_SIZE: u16 = 0x1;
    pub const ENABLE_PUSH: u16 = 0x2;
    pub const MAX_CONCURRENT_STREAMS: u16 = 0x3;
    pub const INITIAL_WINDOW_SIZE: u16 = 0x4;
    pub const MAX_FRAME_SIZE: u16 = 0x5;
    pub const MAX_HEADER_LIST_SIZE: u16 = 0x6;
    pub const NO_RFC7540_PRIORITIES: u16 = 0x9;
}

/// A frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Data {
        stream: u32,
        end_stream: bool,
        data: Vec<u8>,
        /// The payload's length, padding included, which flow control
        /// counts.
        flow: usize,
    },
    Headers {
        stream: u32,
        end_stream: bool,
        end_headers: bool,
        block: Vec<u8>,
    },
    /// A deprecated priority signal, read and set aside.
    Priority {
        stream: u32,
    },
    RstStream {
        stream: u32,
        code: ErrorCode,
    },
    Settings {
        ack: bool,
        values: Vec<(u16, u32)>,
    },
    PushPromise {
        stream: u32,
        promised: u32,
        end_headers: bool,
        block: Vec<u8>,
    },
    Ping {
        ack: bool,
        data: [u8; 8],
    },
    GoAway {
        last_stream: u32,
        code: ErrorCode,
        debug: Vec<u8>,
    },
    WindowUpdate {
        stream: u32,
        increment: u32,
    },
    Continuation {
        stream: u32,
        end_headers: bool,
        block: Vec<u8>,
    },
    /// A frame of a type this end does not know, which it ignores (5.5).
    Unknown {
        kind: u8,
        stream: u32,
    },
}

/// Why a frame could not be read: an error of the connection, or of
/// one stream alone (5.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    Connection(ErrorCode, &'static str),
    Stream(u32, ErrorCode, &'static str),
}

const DATA: u8 = 0x0;
const HEADERS: u8 = 0x1;
const PRIORITY: u8 = 0x2;
const RST_STREAM: u8 = 0x3;
const SETTINGS: u8 = 0x4;
const PUSH_PROMISE: u8 = 0x5;
const PING: u8 = 0x6;
const GOAWAY: u8 = 0x7;
const WINDOW_UPDATE: u8 = 0x8;
const CONTINUATION: u8 = 0x9;

const END_STREAM: u8 = 0x1;
const ACK: u8 = 0x1;
const END_HEADERS: u8 = 0x4;
const PADDED: u8 = 0x8;
const PRIORITY_FLAG: u8 = 0x20;

fn u32_at(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Reads the frame at the start of `bytes`, of a payload at most
/// `max_size`: the frame and how many bytes it took, or `None` while
/// part of it has yet to arrive.
pub fn read(bytes: &[u8], max_size: usize) -> Result<Option<(Frame, usize)>, FrameError> {
    if bytes.len() < HEADER_LENGTH {
        return Ok(None);
    }
    let length =
        (usize::from(bytes[0]) << 16) | (usize::from(bytes[1]) << 8) | usize::from(bytes[2]);
    let kind = bytes[3];
    let flags = bytes[4];
    let stream = u32_at(&bytes[5..9]) & 0x7fff_ffff;
    if length > max_size {
        return Err(FrameError::Connection(
            ErrorCode::FRAME_SIZE_ERROR,
            "a frame larger than allowed",
        ));
    }
    if bytes.len() < HEADER_LENGTH + length {
        return Ok(None);
    }
    let payload = &bytes[HEADER_LENGTH..HEADER_LENGTH + length];
    let frame = parse(kind, flags, stream, payload)?;
    Ok(Some((frame, HEADER_LENGTH + length)))
}

/// A connection error of `code`.
fn connection(code: ErrorCode, why: &'static str) -> FrameError {
    FrameError::Connection(code, why)
}

/// The payload without its padding, for a frame with `PADDED` set.
fn unpad(flags: u8, payload: &[u8]) -> Result<&[u8], FrameError> {
    if flags & PADDED == 0 {
        return Ok(payload);
    }
    let (&pad, rest) = payload.split_first().ok_or(connection(
        ErrorCode::FRAME_SIZE_ERROR,
        "padding without its length",
    ))?;
    let pad = usize::from(pad);
    if pad > rest.len() {
        return Err(connection(
            ErrorCode::PROTOCOL_ERROR,
            "padding longer than the frame",
        ));
    }
    Ok(&rest[..rest.len() - pad])
}

fn parse(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Result<Frame, FrameError> {
    let on_stream = |what: &'static str| match stream {
        0 => Err(connection(ErrorCode::PROTOCOL_ERROR, what)),
        _ => Ok(()),
    };
    let on_connection = |what: &'static str| match stream {
        0 => Ok(()),
        _ => Err(connection(ErrorCode::PROTOCOL_ERROR, what)),
    };
    Ok(match kind {
        DATA => {
            on_stream("DATA on the connection")?;
            Frame::Data {
                stream,
                end_stream: flags & END_STREAM != 0,
                data: unpad(flags, payload)?.to_vec(),
                flow: payload.len(),
            }
        }
        HEADERS => {
            on_stream("HEADERS on the connection")?;
            let mut block = unpad(flags, payload)?;
            if flags & PRIORITY_FLAG != 0 {
                if block.len() < 5 {
                    return Err(connection(
                        ErrorCode::FRAME_SIZE_ERROR,
                        "HEADERS too short for its priority",
                    ));
                }
                block = &block[5..];
            }
            Frame::Headers {
                stream,
                end_stream: flags & END_STREAM != 0,
                end_headers: flags & END_HEADERS != 0,
                block: block.to_vec(),
            }
        }
        PRIORITY => {
            on_stream("PRIORITY on the connection")?;
            if payload.len() != 5 {
                return Err(FrameError::Stream(
                    stream,
                    ErrorCode::FRAME_SIZE_ERROR,
                    "PRIORITY not five bytes",
                ));
            }
            Frame::Priority { stream }
        }
        RST_STREAM => {
            on_stream("RST_STREAM on the connection")?;
            if payload.len() != 4 {
                return Err(connection(
                    ErrorCode::FRAME_SIZE_ERROR,
                    "RST_STREAM not four bytes",
                ));
            }
            Frame::RstStream {
                stream,
                code: ErrorCode(u32_at(payload)),
            }
        }
        SETTINGS => {
            on_connection("SETTINGS on a stream")?;
            let ack = flags & ACK != 0;
            if ack && !payload.is_empty() {
                return Err(connection(
                    ErrorCode::FRAME_SIZE_ERROR,
                    "a SETTINGS acknowledgement with a payload",
                ));
            }
            if !payload.len().is_multiple_of(6) {
                return Err(connection(
                    ErrorCode::FRAME_SIZE_ERROR,
                    "SETTINGS not of six-byte settings",
                ));
            }
            let values = payload
                .as_chunks::<6>()
                .0
                .iter()
                .map(|chunk| {
                    (
                        u16::from_be_bytes([chunk[0], chunk[1]]),
                        u32_at(&chunk[2..]),
                    )
                })
                .collect();
            Frame::Settings { ack, values }
        }
        PUSH_PROMISE => {
            on_stream("PUSH_PROMISE on the connection")?;
            let block = unpad(flags, payload)?;
            if block.len() < 4 {
                return Err(connection(
                    ErrorCode::FRAME_SIZE_ERROR,
                    "PUSH_PROMISE without its stream",
                ));
            }
            Frame::PushPromise {
                stream,
                promised: u32_at(block) & 0x7fff_ffff,
                end_headers: flags & END_HEADERS != 0,
                block: block[4..].to_vec(),
            }
        }
        PING => {
            on_connection("PING on a stream")?;
            let data: [u8; 8] = payload
                .try_into()
                .map_err(|_| connection(ErrorCode::FRAME_SIZE_ERROR, "PING not eight bytes"))?;
            Frame::Ping {
                ack: flags & ACK != 0,
                data,
            }
        }
        GOAWAY => {
            on_connection("GOAWAY on a stream")?;
            if payload.len() < 8 {
                return Err(connection(ErrorCode::FRAME_SIZE_ERROR, "GOAWAY too short"));
            }
            Frame::GoAway {
                last_stream: u32_at(payload) & 0x7fff_ffff,
                code: ErrorCode(u32_at(&payload[4..])),
                debug: payload[8..].to_vec(),
            }
        }
        WINDOW_UPDATE => {
            if payload.len() != 4 {
                return Err(connection(
                    ErrorCode::FRAME_SIZE_ERROR,
                    "WINDOW_UPDATE not four bytes",
                ));
            }
            let increment = u32_at(payload) & 0x7fff_ffff;
            if increment == 0 {
                return Err(match stream {
                    0 => connection(ErrorCode::PROTOCOL_ERROR, "a window update of nothing"),
                    _ => FrameError::Stream(
                        stream,
                        ErrorCode::PROTOCOL_ERROR,
                        "a window update of nothing",
                    ),
                });
            }
            Frame::WindowUpdate { stream, increment }
        }
        CONTINUATION => {
            on_stream("CONTINUATION on the connection")?;
            Frame::Continuation {
                stream,
                end_headers: flags & END_HEADERS != 0,
                block: payload.to_vec(),
            }
        }
        kind => Frame::Unknown { kind, stream },
    })
}

/// Writes a frame header.
fn header(out: &mut Vec<u8>, length: usize, kind: u8, flags: u8, stream: u32) {
    out.extend_from_slice(&(length as u32).to_be_bytes()[1..]);
    out.push(kind);
    out.push(flags);
    out.extend_from_slice(&(stream & 0x7fff_ffff).to_be_bytes());
}

impl Frame {
    /// Writes the frame, unpadded; a header block or data larger than
    /// a frame takes is the caller's to split.
    pub fn write(&self, out: &mut Vec<u8>) {
        let flag = |on: bool, bit: u8| if on { bit } else { 0 };
        match self {
            Frame::Data {
                stream,
                end_stream,
                data,
                ..
            } => {
                header(
                    out,
                    data.len(),
                    DATA,
                    flag(*end_stream, END_STREAM),
                    *stream,
                );
                out.extend_from_slice(data);
            }
            Frame::Headers {
                stream,
                end_stream,
                end_headers,
                block,
            } => {
                let flags = flag(*end_stream, END_STREAM) | flag(*end_headers, END_HEADERS);
                header(out, block.len(), HEADERS, flags, *stream);
                out.extend_from_slice(block);
            }
            Frame::Priority { stream } => {
                header(out, 5, PRIORITY, 0, *stream);
                out.extend_from_slice(&[0, 0, 0, 0, 15]);
            }
            Frame::RstStream { stream, code } => {
                header(out, 4, RST_STREAM, 0, *stream);
                out.extend_from_slice(&code.0.to_be_bytes());
            }
            Frame::Settings { ack, values } => {
                header(out, values.len() * 6, SETTINGS, flag(*ack, ACK), 0);
                for (id, value) in values {
                    out.extend_from_slice(&id.to_be_bytes());
                    out.extend_from_slice(&value.to_be_bytes());
                }
            }
            Frame::PushPromise {
                stream,
                promised,
                end_headers,
                block,
            } => {
                header(
                    out,
                    block.len() + 4,
                    PUSH_PROMISE,
                    flag(*end_headers, END_HEADERS),
                    *stream,
                );
                out.extend_from_slice(&(promised & 0x7fff_ffff).to_be_bytes());
                out.extend_from_slice(block);
            }
            Frame::Ping { ack, data } => {
                header(out, 8, PING, flag(*ack, ACK), 0);
                out.extend_from_slice(data);
            }
            Frame::GoAway {
                last_stream,
                code,
                debug,
            } => {
                header(out, 8 + debug.len(), GOAWAY, 0, 0);
                out.extend_from_slice(&(last_stream & 0x7fff_ffff).to_be_bytes());
                out.extend_from_slice(&code.0.to_be_bytes());
                out.extend_from_slice(debug);
            }
            Frame::WindowUpdate { stream, increment } => {
                header(out, 4, WINDOW_UPDATE, 0, *stream);
                out.extend_from_slice(&(increment & 0x7fff_ffff).to_be_bytes());
            }
            Frame::Continuation {
                stream,
                end_headers,
                block,
            } => {
                header(
                    out,
                    block.len(),
                    CONTINUATION,
                    flag(*end_headers, END_HEADERS),
                    *stream,
                );
                out.extend_from_slice(block);
            }
            Frame::Unknown { kind, stream } => header(out, 0, *kind, 0, *stream),
        }
    }

    /// The frame's bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }
}

/// Writes a header block as a HEADERS frame and the CONTINUATION frames
/// it needs, none larger than `max_size`.
pub fn write_headers(
    out: &mut Vec<u8>,
    stream: u32,
    block: &[u8],
    end_stream: bool,
    max_size: usize,
) {
    let mut pieces = block.chunks(max_size.max(1));
    let first = pieces.next().unwrap_or_default();
    let rest: Vec<&[u8]> = pieces.collect();
    Frame::Headers {
        stream,
        end_stream,
        end_headers: rest.is_empty(),
        block: first.to_vec(),
    }
    .write(out);
    for (index, piece) in rest.iter().enumerate() {
        Frame::Continuation {
            stream,
            end_headers: index + 1 == rest.len(),
            block: piece.to_vec(),
        }
        .write(out);
    }
}
