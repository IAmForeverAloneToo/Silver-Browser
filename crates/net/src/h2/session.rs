//! A client's HTTP/2 session, bytes in and bytes out (M38): the streams,
//! their windows and the connection's, the header blocks and their
//! bounds, the rules each frame is held to. It owns no socket and no
//! thread: the connection's thread (`conn`) feeds it what it reads and
//! writes what it gives, and the tests and the fuzzer drive it directly.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use super::conn::{
    Event, H2_CONNECTION_WINDOW, H2_STREAM_WINDOW, MAX_H2_CONTROL_FRAMES, MAX_H2_STREAMS,
    MAX_HEADER_LIST_SIZE, Request, StreamError,
};
use super::frame::{self, ErrorCode, Frame, FrameError, setting};
use super::hpack::{self, Decoder, Encoder};

/// A window's initial size until a setting says otherwise.
pub(crate) const DEFAULT_WINDOW: i64 = 65_535;
/// The largest a window may grow (6.9.1).
pub(crate) const MAX_WINDOW: i64 = (1 << 31) - 1;

/// A stream the session keeps.
struct Stream {
    token: u64,
    events: Sender<Event>,
    /// What the server lets this end send, and what this end lets it.
    send_window: i64,
    recv_window: i64,
    /// Bytes the fetch took since the window was last given back.
    unacked: usize,
    /// The request's body not yet sent, and whether it was sent whole.
    body: VecDeque<u8>,
    sent_end: bool,
    /// Whether the final head came, its `content-length`, and how much
    /// body came.
    head: bool,
    content_length: Option<u64>,
    received: u64,
    /// Whether the response may have a body: not for a `HEAD`, a `204`
    /// or a `304`.
    bodiless: bool,
    is_head: bool,
    idempotent: bool,
    /// Whether anything of the response came.
    heard: bool,
}

/// A header block arriving across `CONTINUATION` frames.
struct Block {
    stream: u32,
    end_stream: bool,
    bytes: Vec<u8>,
}

/// Why a session ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ending {
    pub(crate) code: ErrorCode,
    pub(crate) why: String,
    /// Whether to tell the server with a `GOAWAY`.
    pub(crate) tell: bool,
}

impl Ending {
    pub(crate) fn error(code: ErrorCode, why: impl Into<String>) -> Ending {
        Ending {
            code,
            why: why.into(),
            tell: true,
        }
    }
}

/// The session.
pub(crate) struct Session {
    /// Bytes read and not yet framed, and bytes framed for the socket.
    input: Vec<u8>,
    output: Vec<u8>,
    encoder: Encoder,
    decoder: Decoder,
    streams: HashMap<u32, Stream>,
    tokens: HashMap<u64, u32>,
    next_id: u32,
    /// The server's settings.
    peer_frame_size: usize,
    peer_window: i64,
    got_settings: bool,
    send_window: i64,
    recv_window: i64,
    unacked: usize,
    block: Option<Block>,
    /// The last id a `GOAWAY` from the server said it processed.
    going_away: Option<u32>,
    /// The most streams the server allows, when it last said so and
    /// the connection's owner has not yet taken it.
    limit: Option<usize>,
    /// Control frames in the current second.
    control: (Instant, u32),
}

impl Default for Session {
    fn default() -> Session {
        Session::new()
    }
}

impl Session {
    /// A session whose preface, settings and raised connection window
    /// are waiting to be written (3.4, 6.5, 6.9).
    pub(crate) fn new() -> Session {
        let mut output = frame::PREFACE.to_vec();
        Frame::Settings {
            ack: false,
            values: vec![
                (setting::ENABLE_PUSH, 0),
                (setting::MAX_CONCURRENT_STREAMS, MAX_H2_STREAMS as u32),
                (setting::INITIAL_WINDOW_SIZE, H2_STREAM_WINDOW),
                (setting::MAX_HEADER_LIST_SIZE, MAX_HEADER_LIST_SIZE as u32),
                (setting::NO_RFC7540_PRIORITIES, 1),
            ],
        }
        .write(&mut output);
        Frame::WindowUpdate {
            stream: 0,
            increment: H2_CONNECTION_WINDOW - DEFAULT_WINDOW as u32,
        }
        .write(&mut output);
        Session {
            input: Vec::new(),
            output,
            encoder: Encoder::default(),
            decoder: Decoder::default(),
            streams: HashMap::new(),
            tokens: HashMap::new(),
            next_id: 1,
            peer_frame_size: frame::MAX_FRAME_SIZE,
            peer_window: DEFAULT_WINDOW,
            got_settings: false,
            send_window: DEFAULT_WINDOW,
            recv_window: i64::from(H2_CONNECTION_WINDOW),
            unacked: 0,
            block: None,
            going_away: None,
            limit: None,
            control: (Instant::now(), 0),
        }
    }

    /// The bytes to write, taken.
    pub(crate) fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }

    /// Whether bytes wait to be written.
    pub(crate) fn has_output(&self) -> bool {
        !self.output.is_empty()
    }

    /// Whether no stream is open.
    pub(crate) fn is_idle(&self) -> bool {
        self.streams.is_empty()
    }

    /// Whether the server said it goes away.
    pub(crate) fn going_away(&self) -> bool {
        self.going_away.is_some()
    }

    /// The most streams the server allows, if it said so since this was
    /// last asked.
    pub(crate) fn take_limit(&mut self) -> Option<usize> {
        self.limit.take()
    }

    /// What this end may still send on the connection, for the tests.
    #[cfg(test)]
    pub(crate) fn send_window(&self) -> i64 {
        self.send_window
    }

    /// Opens a stream: its header block written, its body to follow.
    pub(crate) fn open(&mut self, token: u64, request: Request, events: Sender<Event>) {
        if self.going_away.is_some() || self.next_id > MAX_WINDOW as u32 {
            let _ = events.send(Event::Retry);
            return;
        }
        let id = self.next_id;
        self.next_id += 2;
        let method = request
            .fields
            .iter()
            .find(|(name, _)| name == b":method")
            .map(|(_, value)| value.clone())
            .unwrap_or_default();
        let block = self.encoder.encode(
            request
                .fields
                .iter()
                .map(|(name, value)| (name.as_slice(), value.as_slice())),
        );
        let body: VecDeque<u8> = request.body.unwrap_or_default().into();
        frame::write_headers(
            &mut self.output,
            id,
            &block,
            body.is_empty(),
            self.peer_frame_size,
        );
        self.tokens.insert(token, id);
        self.streams.insert(
            id,
            Stream {
                token,
                events,
                send_window: self.peer_window,
                recv_window: i64::from(H2_STREAM_WINDOW),
                unacked: 0,
                sent_end: body.is_empty(),
                body,
                head: false,
                content_length: None,
                received: 0,
                bodiless: false,
                is_head: method == b"HEAD",
                idempotent: request.idempotent,
                heard: false,
            },
        );
        self.send_bodies();
    }

    /// Cancels the stream of `token`: reset, its bytes given back.
    pub(crate) fn cancel(&mut self, token: u64) {
        if let Some(id) = self.tokens.remove(&token) {
            if let Some(stream) = self.streams.remove(&id) {
                self.unacked += stream.unacked;
            }
            Frame::RstStream {
                stream: id,
                code: ErrorCode::CANCEL,
            }
            .write(&mut self.output);
            self.give_back();
        }
    }

    /// The fetch of `token` took `bytes` of its body.
    pub(crate) fn consumed(&mut self, token: u64, bytes: usize) {
        self.unacked += bytes;
        if let Some(stream) = self
            .tokens
            .get(&token)
            .and_then(|id| self.streams.get_mut(id))
        {
            stream.unacked += bytes;
        }
        self.give_back();
    }

    /// Gives back the windows of what the fetches took, once half of a
    /// window is to be given.
    fn give_back(&mut self) {
        if self.unacked >= H2_CONNECTION_WINDOW as usize / 2 {
            Frame::WindowUpdate {
                stream: 0,
                increment: self.unacked as u32,
            }
            .write(&mut self.output);
            self.recv_window += self.unacked as i64;
            self.unacked = 0;
        }
        for (&id, stream) in &mut self.streams {
            if stream.unacked >= H2_STREAM_WINDOW as usize / 2 {
                Frame::WindowUpdate {
                    stream: id,
                    increment: stream.unacked as u32,
                }
                .write(&mut self.output);
                stream.recv_window += stream.unacked as i64;
                stream.unacked = 0;
            }
        }
    }

    /// Sends what the windows let of the requests' bodies.
    pub(crate) fn send_bodies(&mut self) {
        let mut ids: Vec<u32> = self.streams.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let Some(stream) = self.streams.get_mut(&id) else {
                continue;
            };
            while !stream.sent_end {
                let room = stream
                    .send_window
                    .min(self.send_window)
                    .min(self.peer_frame_size as i64)
                    .max(0) as usize;
                let size = room.min(stream.body.len());
                if size == 0 && !stream.body.is_empty() {
                    break;
                }
                let data: Vec<u8> = stream.body.drain(..size).collect();
                let end = stream.body.is_empty();
                stream.send_window -= size as i64;
                self.send_window -= size as i64;
                Frame::Data {
                    stream: id,
                    end_stream: end,
                    data,
                    flow: size,
                }
                .write(&mut self.output);
                stream.sent_end = end;
            }
        }
    }

    /// Takes `bytes` read from the server, and handles every whole frame.
    pub(crate) fn receive(&mut self, bytes: &[u8]) -> Result<(), Ending> {
        self.input.extend_from_slice(bytes);
        let mut at = 0;
        let result = loop {
            match frame::read(&self.input[at..], frame::MAX_FRAME_SIZE) {
                Ok(Some((frame, length))) => {
                    at += length;
                    if let Err(ending) = self.frame(frame) {
                        break Err(ending);
                    }
                }
                Ok(None) => break Ok(()),
                Err(FrameError::Connection(code, why)) => break Err(Ending::error(code, why)),
                Err(FrameError::Stream(id, code, why)) => {
                    let header = &self.input[at..];
                    let length = usize::from(header[0]) << 16
                        | usize::from(header[1]) << 8
                        | usize::from(header[2]);
                    at += frame::HEADER_LENGTH + length;
                    self.reset(id, code, StreamError::Malformed(why));
                }
            }
        };
        self.input.drain(..at);
        self.give_back();
        self.send_bodies();
        result
    }

    /// Counts a control frame: past the rate, the session ends.
    fn control(&mut self) -> Result<(), Ending> {
        let now = Instant::now();
        if now.duration_since(self.control.0) >= Duration::from_secs(1) {
            self.control = (now, 0);
        }
        self.control.1 += 1;
        if self.control.1 > MAX_H2_CONTROL_FRAMES {
            return Err(Ending::error(
                ErrorCode::ENHANCE_YOUR_CALM,
                "too many control frames",
            ));
        }
        Ok(())
    }

    /// Resets stream `id` with `code`, its fetch told `error`.
    fn reset(&mut self, id: u32, code: ErrorCode, error: StreamError) {
        if let Some(stream) = self.streams.remove(&id) {
            self.tokens.remove(&stream.token);
            self.unacked += stream.unacked;
            let _ = stream.events.send(Event::Failed(error));
        }
        Frame::RstStream { stream: id, code }.write(&mut self.output);
    }

    /// Whether `id` is a stream this end never opened: an idle one.
    fn idle(&self, id: u32) -> bool {
        id.is_multiple_of(2) || id >= self.next_id
    }

    fn frame(&mut self, frame: Frame) -> Result<(), Ending> {
        if !self.got_settings && !matches!(frame, Frame::Settings { ack: false, .. }) {
            return Err(Ending::error(
                ErrorCode::PROTOCOL_ERROR,
                "the server's first frame was not its settings",
            ));
        }
        // A header block's frames come one after another (6.10).
        if self.block.is_some() && !matches!(frame, Frame::Continuation { .. }) {
            return Err(Ending::error(
                ErrorCode::PROTOCOL_ERROR,
                "a frame inside a header block",
            ));
        }
        match frame {
            Frame::Settings { ack: true, .. } => {}
            Frame::Settings { ack: false, values } => {
                self.control()?;
                self.settings(&values)?;
                self.got_settings = true;
                Frame::Settings {
                    ack: true,
                    values: Vec::new(),
                }
                .write(&mut self.output);
            }
            Frame::Ping { ack, data } => {
                if !ack {
                    self.control()?;
                    Frame::Ping { ack: true, data }.write(&mut self.output);
                }
            }
            Frame::GoAway {
                last_stream, code, ..
            } => {
                self.going_away = Some(last_stream);
                let above: Vec<u32> = self
                    .streams
                    .keys()
                    .copied()
                    .filter(|&id| id > last_stream)
                    .collect();
                for id in above {
                    if let Some(stream) = self.streams.remove(&id) {
                        self.tokens.remove(&stream.token);
                        let _ = stream.events.send(Event::Retry);
                    }
                }
                if code != ErrorCode::NO_ERROR && self.streams.is_empty() {
                    return Err(Ending {
                        code,
                        why: "the server went away".to_owned(),
                        tell: false,
                    });
                }
            }
            Frame::WindowUpdate {
                stream: 0,
                increment,
            } => {
                self.send_window += i64::from(increment);
                if self.send_window > MAX_WINDOW {
                    return Err(Ending::error(
                        ErrorCode::FLOW_CONTROL_ERROR,
                        "the connection's window past its bound",
                    ));
                }
            }
            Frame::WindowUpdate {
                stream: id,
                increment,
            } => {
                if self.idle(id) {
                    return Err(Ending::error(
                        ErrorCode::PROTOCOL_ERROR,
                        "a window update on an idle stream",
                    ));
                }
                if let Some(stream) = self.streams.get_mut(&id) {
                    stream.send_window += i64::from(increment);
                    if stream.send_window > MAX_WINDOW {
                        self.reset(
                            id,
                            ErrorCode::FLOW_CONTROL_ERROR,
                            StreamError::Reset(ErrorCode::FLOW_CONTROL_ERROR),
                        );
                    }
                }
            }
            Frame::RstStream { stream: id, code } => {
                if self.idle(id) {
                    return Err(Ending::error(
                        ErrorCode::PROTOCOL_ERROR,
                        "a reset of an idle stream",
                    ));
                }
                if let Some(stream) = self.streams.remove(&id) {
                    self.tokens.remove(&stream.token);
                    self.unacked += stream.unacked;
                    let event = match code {
                        ErrorCode::REFUSED_STREAM => Event::Retry,
                        code => Event::Failed(StreamError::Reset(code)),
                    };
                    let _ = stream.events.send(event);
                }
            }
            Frame::PushPromise { .. } => {
                return Err(Ending::error(
                    ErrorCode::PROTOCOL_ERROR,
                    "a push this end did not allow",
                ));
            }
            Frame::Headers {
                stream,
                end_stream,
                end_headers,
                block,
            } => {
                if self.idle(stream) {
                    return Err(Ending::error(
                        ErrorCode::PROTOCOL_ERROR,
                        "headers on a stream this end did not open",
                    ));
                }
                self.block = Some(Block {
                    stream,
                    end_stream,
                    bytes: block,
                });
                self.check_block()?;
                if end_headers {
                    self.header_block()?;
                }
            }
            Frame::Continuation {
                stream,
                end_headers,
                block,
            } => {
                let Some(open) = self.block.as_mut().filter(|open| open.stream == stream) else {
                    return Err(Ending::error(
                        ErrorCode::PROTOCOL_ERROR,
                        "a continuation of no header block",
                    ));
                };
                open.bytes.extend_from_slice(&block);
                self.check_block()?;
                if end_headers {
                    self.header_block()?;
                }
            }
            Frame::Data {
                stream: id,
                end_stream,
                data,
                flow,
            } => self.data(id, end_stream, data, flow)?,
            Frame::Priority { .. } | Frame::Unknown { .. } => {}
        }
        Ok(())
    }

    /// A header block arriving is held to twice the list's bound, its
    /// compressed size (the decoder holds the rest).
    fn check_block(&self) -> Result<(), Ending> {
        match &self.block {
            Some(block) if block.bytes.len() > 2 * MAX_HEADER_LIST_SIZE => Err(Ending::error(
                ErrorCode::ENHANCE_YOUR_CALM,
                "a header block too large",
            )),
            _ => Ok(()),
        }
    }

    fn settings(&mut self, values: &[(u16, u32)]) -> Result<(), Ending> {
        for &(id, value) in values {
            match id {
                setting::HEADER_TABLE_SIZE => self.encoder.set_max_size(value as usize),
                setting::ENABLE_PUSH if value > 1 => {
                    return Err(Ending::error(
                        ErrorCode::PROTOCOL_ERROR,
                        "ENABLE_PUSH past 1",
                    ));
                }
                setting::MAX_CONCURRENT_STREAMS => self.limit = Some(value as usize),
                setting::INITIAL_WINDOW_SIZE => {
                    if i64::from(value) > MAX_WINDOW {
                        return Err(Ending::error(
                            ErrorCode::FLOW_CONTROL_ERROR,
                            "an initial window past its bound",
                        ));
                    }
                    // Every stream's window moves by the difference
                    // (6.9.2).
                    let delta = i64::from(value) - self.peer_window;
                    self.peer_window = i64::from(value);
                    for stream in self.streams.values_mut() {
                        stream.send_window += delta;
                        if stream.send_window > MAX_WINDOW {
                            return Err(Ending::error(
                                ErrorCode::FLOW_CONTROL_ERROR,
                                "a stream's window past its bound",
                            ));
                        }
                    }
                }
                setting::MAX_FRAME_SIZE => {
                    let size = value as usize;
                    if !(frame::MAX_FRAME_SIZE..=frame::MAX_FRAME_SIZE_LIMIT).contains(&size) {
                        return Err(Ending::error(
                            ErrorCode::PROTOCOL_ERROR,
                            "a frame size out of its range",
                        ));
                    }
                    self.peer_frame_size = size;
                }
                // The rest is advisory, or unknown and ignored.
                _ => {}
            }
        }
        Ok(())
    }

    /// A whole header block: decoded always, the table being shared,
    /// then given to its stream if it still has one.
    fn header_block(&mut self) -> Result<(), Ending> {
        let Some(block) = self.block.take() else {
            return Ok(());
        };
        let decoded = self
            .decoder
            .decode(&block.bytes, MAX_HEADER_LIST_SIZE)
            .map_err(|e| Ending::error(ErrorCode::COMPRESSION_ERROR, e.0))?;
        let id = block.stream;
        let Some(stream) = self.streams.get_mut(&id) else {
            // A stream this end reset or finished: nothing to do.
            return Ok(());
        };
        stream.heard = true;
        if decoded.too_large {
            self.reset(id, ErrorCode::CANCEL, StreamError::HeadersTooLarge);
            return Ok(());
        }
        if stream.head {
            // Trailers: they end the stream, and hold no pseudo-field.
            if !block.end_stream
                || decoded
                    .fields
                    .iter()
                    .any(|(name, _)| name.starts_with(b":"))
            {
                self.reset(
                    id,
                    ErrorCode::PROTOCOL_ERROR,
                    StreamError::Malformed("trailers that do not end the stream"),
                );
                return Ok(());
            }
            return self.end_stream(id);
        }
        match response_head(decoded.fields) {
            Err(why) => {
                self.reset(id, ErrorCode::PROTOCOL_ERROR, StreamError::Malformed(why));
            }
            Ok((status, _)) if (100..200).contains(&status) => {
                // An interim response: the final one is still to come.
                if block.end_stream || status == 101 {
                    self.reset(
                        id,
                        ErrorCode::PROTOCOL_ERROR,
                        StreamError::Malformed("an interim response ending its stream"),
                    );
                }
            }
            Ok((status, fields)) => {
                let content_length = match content_length(&fields) {
                    Ok(length) => length,
                    Err(why) => {
                        self.reset(id, ErrorCode::PROTOCOL_ERROR, StreamError::Malformed(why));
                        return Ok(());
                    }
                };
                stream.head = true;
                stream.bodiless = stream.is_head || status == 204 || status == 304;
                stream.content_length = content_length;
                let _ = stream.events.send(Event::Head { status, fields });
                if block.end_stream {
                    return self.end_stream(id);
                }
            }
        }
        Ok(())
    }

    /// A `DATA` frame: within the windows, after the head, and within
    /// the `content-length`.
    fn data(
        &mut self,
        id: u32,
        end_stream: bool,
        data: Vec<u8>,
        flow: usize,
    ) -> Result<(), Ending> {
        self.recv_window -= flow as i64;
        if self.recv_window < 0 {
            return Err(Ending::error(
                ErrorCode::FLOW_CONTROL_ERROR,
                "data past the connection's window",
            ));
        }
        if data.is_empty() && !end_stream {
            self.control()?;
        }
        if self.idle(id) {
            return Err(Ending::error(
                ErrorCode::PROTOCOL_ERROR,
                "data on a stream this end did not open",
            ));
        }
        let Some(stream) = self.streams.get_mut(&id) else {
            // A stream this end let go of: its bytes are the connection's
            // to give back at once.
            self.unacked += flow;
            return Ok(());
        };
        stream.heard = true;
        stream.recv_window -= flow as i64;
        // Padding is given back at once; the data as it is taken.
        self.unacked += flow - data.len();
        stream.unacked += flow - data.len();
        if stream.recv_window < 0 {
            self.reset(
                id,
                ErrorCode::FLOW_CONTROL_ERROR,
                StreamError::Reset(ErrorCode::FLOW_CONTROL_ERROR),
            );
            return Ok(());
        }
        if !stream.head {
            self.reset(
                id,
                ErrorCode::PROTOCOL_ERROR,
                StreamError::Malformed("data before the response's head"),
            );
            return Ok(());
        }
        stream.received += data.len() as u64;
        let past = stream
            .content_length
            .is_some_and(|length| stream.received > length);
        if (stream.bodiless && !data.is_empty()) || past {
            self.reset(
                id,
                ErrorCode::PROTOCOL_ERROR,
                StreamError::Malformed("more body than the response says"),
            );
            return Ok(());
        }
        if !data.is_empty() {
            let _ = stream.events.send(Event::Data(data));
        }
        if end_stream {
            self.end_stream(id)?;
        }
        Ok(())
    }

    /// The server ended stream `id`: its response is whole if its
    /// `content-length` says so.
    fn end_stream(&mut self, id: u32) -> Result<(), Ending> {
        let Some(stream) = self.streams.get(&id) else {
            return Ok(());
        };
        let short = !stream.bodiless
            && stream
                .content_length
                .is_some_and(|length| stream.received != length);
        if short {
            self.reset(
                id,
                ErrorCode::PROTOCOL_ERROR,
                StreamError::Malformed("less body than the response says"),
            );
            return Ok(());
        }
        let stream = self.streams.remove(&id).expect("the stream");
        self.tokens.remove(&stream.token);
        // A request still sending its body is not needed further (8.1).
        if !stream.sent_end {
            Frame::RstStream {
                stream: id,
                code: ErrorCode::NO_ERROR,
            }
            .write(&mut self.output);
        }
        let _ = stream.events.send(Event::End);
        Ok(())
    }

    /// The session ends: the server told why when it should be, and
    /// every stream told; a stream that heard nothing of its response,
    /// of a request that may be sent again, is retried, but on a
    /// timeout.
    pub(crate) fn end(&mut self, ending: &Ending) {
        if ending.tell {
            Frame::GoAway {
                last_stream: 0,
                code: ending.code,
                debug: Vec::new(),
            }
            .write(&mut self.output);
        }
        let timeout = ending.why == "timeout";
        let streams: Vec<Stream> = self.streams.drain().map(|(_, stream)| stream).collect();
        self.tokens.clear();
        for stream in streams {
            let event = if timeout {
                Event::Failed(StreamError::Timeout)
            } else if !stream.heard && stream.idempotent {
                Event::Retry
            } else {
                Event::Failed(StreamError::Connection(ending.code, ending.why.clone()))
            };
            let _ = stream.events.send(event);
        }
    }
}

/// A response's head checked as RFC 9113 8.3 and 8.2 say: its `:status`
/// and no other pseudo-field, before every field, and no field a
/// connection-specific one or of an uppercase name. Its status and its
/// fields without the pseudo-field.
fn response_head(fields: Vec<hpack::Field>) -> Result<(u16, Vec<hpack::Field>), &'static str> {
    let mut status = None;
    let mut regular = Vec::with_capacity(fields.len());
    for (name, value) in fields {
        if let Some(pseudo) = name.strip_prefix(b":") {
            if !regular.is_empty() {
                return Err("a pseudo-field after a field");
            }
            if pseudo != b"status" || status.is_some() {
                return Err("a pseudo-field a response may not have");
            }
            let digits = std::str::from_utf8(&value).ok().filter(|d| d.len() == 3);
            status = Some(
                digits
                    .and_then(|d| d.parse::<u16>().ok())
                    .filter(|status| (100..1000).contains(status))
                    .ok_or("a status that is not three digits")?,
            );
            continue;
        }
        if name.is_empty() || name.iter().any(|b| b.is_ascii_uppercase()) {
            return Err("a field name not in lowercase");
        }
        if !crate::headers::is_header_name(&name) || !crate::headers::is_header_value(&value) {
            return Err("a field that is not one");
        }
        if is_connection_specific(&name, &value) {
            return Err("a connection-specific field");
        }
        regular.push((name, value));
    }
    Ok((status.ok_or("no :status")?, regular))
}

/// Whether a field is one HTTP/2 forbids (8.2.2).
pub(crate) fn is_connection_specific(name: &[u8], value: &[u8]) -> bool {
    match name {
        b"connection" | b"keep-alive" | b"proxy-connection" | b"transfer-encoding" | b"upgrade" => {
            true
        }
        b"te" => !value.eq_ignore_ascii_case(b"trailers"),
        _ => false,
    }
}

/// The response's `content-length`, if it gives one; its values all
/// alike, or the response is malformed.
fn content_length(fields: &[hpack::Field]) -> Result<Option<u64>, &'static str> {
    let mut length = None;
    for (name, value) in fields {
        if name != b"content-length" {
            continue;
        }
        for part in value.split(|&b| b == b',') {
            let text = std::str::from_utf8(part).map_err(|_| "a content-length not a number")?;
            let parsed: u64 = text
                .trim()
                .parse()
                .map_err(|_| "a content-length not a number")?;
            match length {
                Some(other) if other != parsed => return Err("two content-lengths"),
                _ => length = Some(parsed),
            }
        }
    }
    Ok(length)
}
