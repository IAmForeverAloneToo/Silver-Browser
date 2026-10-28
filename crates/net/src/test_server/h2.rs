//! The test server's HTTP/2 (M38): a connection whose TLS chose `h2`
//! is served here. Each request's reply runs on a thread of its own, so
//! that slow replies are seen to overlap; one loop reads the client's
//! frames and writes the replies', within the client's windows. A
//! reply's bytes are an HTTP/1.1 response, read into a head and a body;
//! `Reply::Frames` writes frames as they are, for tests of a server
//! that breaks the rules.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use rustls::{ServerConnection, StreamOwned};

use super::{Reply, ServerRequest, Shared, lock, sleep_unless_stopped};
use crate::h2::frame::{self, ErrorCode, Frame, setting};
use crate::h2::hpack::{Decoder, Encoder, Field};

/// What a reply's thread asks the connection to write.
enum Out {
    Head {
        stream: u32,
        fields: Vec<(Vec<u8>, Vec<u8>)>,
        end: bool,
    },
    Data {
        stream: u32,
        data: Vec<u8>,
        end: bool,
    },
    Raw(Vec<u8>),
    Reset(u32, ErrorCode),
    GoAway(u32),
    Close,
    /// What the reader's thread read from the socket, and its end.
    Tls(Vec<u8>),
    Eof,
}

/// Writes `bytes` to the client: through the TLS connection, as much
/// as it buffers at a time, and its records to the socket, never
/// reading from it, which the reader's thread alone does.
fn send(tls: &mut StreamOwned<ServerConnection, TcpStream>, mut bytes: &[u8]) -> io::Result<()> {
    loop {
        let taken = tls.conn.writer().write(bytes)?;
        bytes = &bytes[taken..];
        while tls.conn.wants_write() {
            tls.conn.write_tls(&mut tls.sock)?;
        }
        if bytes.is_empty() {
            return tls.sock.flush();
        }
    }
}

/// Shuts the socket down when the connection's loop ends, which ends
/// the reader's thread too.
struct Closer(TcpStream);

impl Drop for Closer {
    fn drop(&mut self) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

/// A request arriving.
struct Incoming {
    fields: Vec<(Vec<u8>, Vec<u8>)>,
    body: Vec<u8>,
}

/// The default window, before the client's settings.
const DEFAULT_WINDOW: i64 = 65_535;

/// Serves one HTTP/2 connection until it ends or the server stops.
pub(super) fn serve(
    shared: &Arc<Shared>,
    mut tls: StreamOwned<ServerConnection, TcpStream>,
    connection: usize,
) -> io::Result<()> {
    let sni = tls.conn.server_name().map(str::to_owned);
    let mut input: Vec<u8> = Vec::new();
    // The client's preface, then frames.
    let mut prefaced = false;
    let mut encoder = Encoder::default();
    let mut decoder = Decoder::default();
    let mut incoming: HashMap<u32, Incoming> = HashMap::new();
    let mut block: Option<(u32, bool, Vec<u8>)> = None;
    // The client's windows for what this end sends.
    let mut client_initial = DEFAULT_WINDOW;
    let mut connection_window = DEFAULT_WINDOW;
    let mut windows: HashMap<u32, i64> = HashMap::new();
    let mut pending: HashMap<u32, VecDeque<(Vec<u8>, bool)>> = HashMap::new();
    let mut open_streams = 0usize;
    // What the client may still send this end, which it holds the client
    // to, as a server must: past it, the connection ends.
    let mut receive_window = DEFAULT_WINDOW;
    let mut receive_windows: HashMap<u32, i64> = HashMap::new();
    let mut index = 0usize;
    let (sender, outgoing): (Sender<Out>, Receiver<Out>) = mpsc::channel();
    // The socket is read on a thread of its own, which hands what it
    // reads to this loop as the replies' threads hand their frames: the
    // loop wakes at once for either, where a read with a timeout made
    // each reply wait for the timer (15.6 ms on Windows).
    tls.sock.set_read_timeout(None)?;
    let _closer = Closer(tls.sock.try_clone()?);
    {
        let mut socket = tls.sock.try_clone()?;
        let sender = sender.clone();
        thread::spawn(move || {
            let mut buffer = vec![0u8; 16_384];
            loop {
                match socket.read(&mut buffer) {
                    Ok(0) | Err(_) => {
                        let _ = sender.send(Out::Eof);
                        return;
                    }
                    Ok(n) => {
                        if sender.send(Out::Tls(buffer[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                }
            }
        });
    }
    let mut out = Vec::new();
    Frame::Settings {
        ack: false,
        values: vec![(setting::MAX_CONCURRENT_STREAMS, 100)],
    }
    .write(&mut out);
    send(&mut tls, &out)?;
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        // What the replies' threads and the reader's sent; no wait while
        // a body has room in the client's windows to go on.
        let room = connection_window > 0
            && pending
                .keys()
                .any(|stream| windows.get(stream).copied().unwrap_or(client_initial) > 0);
        let wait = if room {
            Duration::ZERO
        } else {
            Duration::from_millis(50)
        };
        let first = match outgoing.recv_timeout(wait) {
            Ok(item) => Some(item),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        let mut out = Vec::new();
        let mut chunk = [0u8; 16_384];
        let items: Vec<Out> = first
            .into_iter()
            .chain(std::iter::from_fn(|| outgoing.try_recv().ok()))
            .collect();
        for item in items {
            match item {
                Out::Tls(bytes) => {
                    // rustls takes about 4 KiB a call and refuses more
                    // once 16 KiB of plaintext is unread: the plaintext is
                    // taken after each call, or a read holding a whole
                    // record and more of the next ended the connection
                    // (a large upload on CI's loaded runners).
                    let mut rest = &bytes[..];
                    while !rest.is_empty() {
                        tls.conn.read_tls(&mut rest)?;
                        tls.conn.process_new_packets().map_err(io::Error::other)?;
                        loop {
                            match tls.conn.reader().read(&mut chunk) {
                                Ok(0) => return Ok(()),
                                Ok(n) => input.extend_from_slice(&chunk[..n]),
                                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                                Err(e) => return Err(e),
                            }
                        }
                    }
                }
                Out::Eof => return Ok(()),
                Out::Head {
                    stream,
                    fields,
                    end,
                } => {
                    let block = encoder.encode(
                        fields
                            .iter()
                            .map(|(name, value)| (name.as_slice(), value.as_slice())),
                    );
                    frame::write_headers(&mut out, stream, &block, end, frame::MAX_FRAME_SIZE);
                    if end {
                        open_streams = open_streams.saturating_sub(1);
                    }
                }
                Out::Data { stream, data, end } => {
                    pending.entry(stream).or_default().push_back((data, end));
                }
                Out::Raw(bytes) => out.extend_from_slice(&bytes),
                Out::Reset(stream, code) => {
                    Frame::RstStream { stream, code }.write(&mut out);
                    pending.remove(&stream);
                    open_streams = open_streams.saturating_sub(1);
                }
                Out::GoAway(last) => Frame::GoAway {
                    last_stream: last,
                    code: ErrorCode::NO_ERROR,
                    debug: Vec::new(),
                }
                .write(&mut out),
                Out::Close => {
                    send(&mut tls, &out)?;
                    return Ok(());
                }
            }
        }
        // The bodies, within the client's windows.
        let streams: Vec<u32> = pending.keys().copied().collect();
        for stream in streams {
            let queue = pending.get_mut(&stream).expect("the stream's queue");
            let window = windows.entry(stream).or_insert(client_initial);
            while let Some((data, end)) = queue.front_mut() {
                let room = (*window)
                    .min(connection_window)
                    .min(frame::MAX_FRAME_SIZE as i64)
                    .max(0) as usize;
                if room == 0 && !data.is_empty() {
                    break;
                }
                let size = room.min(data.len());
                let piece: Vec<u8> = data.drain(..size).collect();
                let last = data.is_empty() && *end;
                if data.is_empty() || size > 0 {
                    *window -= size as i64;
                    connection_window -= size as i64;
                    Frame::Data {
                        stream,
                        end_stream: last,
                        data: piece,
                        flow: size,
                    }
                    .write(&mut out);
                }
                if data.is_empty() {
                    queue.pop_front();
                    if last {
                        open_streams = open_streams.saturating_sub(1);
                    }
                }
            }
            if queue.is_empty() {
                pending.remove(&stream);
            }
        }
        if !out.is_empty() {
            send(&mut tls, &out)?;
        }
        if !prefaced {
            if input.len() < frame::PREFACE.len() {
                continue;
            }
            if &input[..frame::PREFACE.len()] != frame::PREFACE {
                return Ok(());
            }
            input.drain(..frame::PREFACE.len());
            prefaced = true;
        }
        let mut reply = Vec::new();
        let mut at = 0;
        while let Ok(Some((frame, length))) = frame::read(&input[at..], frame::MAX_FRAME_SIZE) {
            at += length;
            match frame {
                Frame::Settings { ack: false, values } => {
                    for (id, value) in values {
                        if id == setting::INITIAL_WINDOW_SIZE {
                            let delta = i64::from(value) - client_initial;
                            client_initial = i64::from(value);
                            for window in windows.values_mut() {
                                *window += delta;
                            }
                        }
                    }
                    Frame::Settings {
                        ack: true,
                        values: Vec::new(),
                    }
                    .write(&mut reply);
                }
                Frame::Ping { ack: false, data } => {
                    Frame::Ping { ack: true, data }.write(&mut reply);
                }
                Frame::WindowUpdate {
                    stream: 0,
                    increment,
                } => {
                    connection_window += i64::from(increment);
                }
                Frame::WindowUpdate { stream, increment } => {
                    *windows.entry(stream).or_insert(client_initial) += i64::from(increment);
                }
                Frame::Headers {
                    stream,
                    end_stream,
                    end_headers,
                    block: bytes,
                } => {
                    block = Some((stream, end_stream, bytes));
                    if end_headers {
                        let (stream, end, bytes) = block.take().expect("the block");
                        let fields = decoder
                            .decode(&bytes, usize::MAX)
                            .map(|decoded| decoded.fields)
                            .unwrap_or_default();
                        incoming.insert(
                            stream,
                            Incoming {
                                fields,
                                body: Vec::new(),
                            },
                        );
                        if end {
                            open_streams += 1;
                            shared
                                .most_streams
                                .fetch_max(open_streams, Ordering::SeqCst);
                            start(
                                shared,
                                &mut incoming,
                                stream,
                                connection,
                                &mut index,
                                &sni,
                                &sender,
                            );
                        }
                    }
                }
                Frame::Continuation {
                    end_headers,
                    block: bytes,
                    ..
                } => {
                    if let Some((_, _, open)) = &mut block {
                        open.extend_from_slice(&bytes);
                    }
                    if end_headers && let Some((stream, end, bytes)) = block.take() {
                        let fields = decoder
                            .decode(&bytes, usize::MAX)
                            .map(|decoded| decoded.fields)
                            .unwrap_or_default();
                        incoming.insert(
                            stream,
                            Incoming {
                                fields,
                                body: Vec::new(),
                            },
                        );
                        if end {
                            open_streams += 1;
                            shared
                                .most_streams
                                .fetch_max(open_streams, Ordering::SeqCst);
                            start(
                                shared,
                                &mut incoming,
                                stream,
                                connection,
                                &mut index,
                                &sni,
                                &sender,
                            );
                        }
                    }
                }
                Frame::Data {
                    stream,
                    end_stream,
                    data,
                    flow,
                } => {
                    receive_window -= flow as i64;
                    let window = receive_windows.entry(stream).or_insert(DEFAULT_WINDOW);
                    *window -= flow as i64;
                    if receive_window < 0 || *window < 0 {
                        Frame::GoAway {
                            last_stream: stream,
                            code: ErrorCode::FLOW_CONTROL_ERROR,
                            debug: Vec::new(),
                        }
                        .write(&mut reply);
                        send(&mut tls, &reply)?;
                        return Ok(());
                    }
                    if let Some(request) = incoming.get_mut(&stream) {
                        request.body.extend_from_slice(&data);
                    }
                    // The windows given back as the bytes are taken.
                    if flow > 0 {
                        receive_window += flow as i64;
                        Frame::WindowUpdate {
                            stream: 0,
                            increment: flow as u32,
                        }
                        .write(&mut reply);
                        if !end_stream {
                            *receive_windows.entry(stream).or_insert(DEFAULT_WINDOW) += flow as i64;
                            Frame::WindowUpdate {
                                stream,
                                increment: flow as u32,
                            }
                            .write(&mut reply);
                        }
                    }
                    if end_stream {
                        open_streams += 1;
                        shared
                            .most_streams
                            .fetch_max(open_streams, Ordering::SeqCst);
                        start(
                            shared,
                            &mut incoming,
                            stream,
                            connection,
                            &mut index,
                            &sni,
                            &sender,
                        );
                    }
                }
                Frame::RstStream { stream, .. } => {
                    shared.resets.fetch_add(1, Ordering::SeqCst);
                    pending.remove(&stream);
                    incoming.remove(&stream);
                }
                Frame::GoAway { .. } => return Ok(()),
                _ => {}
            }
        }
        input.drain(..at);
        if !reply.is_empty() {
            send(&mut tls, &reply)?;
        }
    }
}

/// Records the whole request on `stream` and starts its reply.
fn start(
    shared: &Arc<Shared>,
    incoming: &mut HashMap<u32, Incoming>,
    stream: u32,
    connection: usize,
    index: &mut usize,
    sni: &Option<String>,
    sender: &Sender<Out>,
) {
    let Some(request) = incoming.remove(&stream) else {
        return;
    };
    let field = |name: &[u8]| {
        request
            .fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, value)| String::from_utf8_lossy(value).into_owned())
            .unwrap_or_default()
    };
    let mut headers: Vec<(String, String)> = request
        .fields
        .iter()
        .filter(|(name, _)| !name.starts_with(b":"))
        .map(|(name, value)| {
            (
                String::from_utf8_lossy(name).into_owned(),
                String::from_utf8_lossy(value).into_owned(),
            )
        })
        .collect();
    headers.insert(0, ("host".to_owned(), field(b":authority")));
    let request = ServerRequest {
        method: field(b":method"),
        target: field(b":path"),
        headers,
        body: request.body,
        connection,
        index: *index,
        sni: sni.clone(),
        stream,
    };
    *index += 1;
    lock(&shared.requests).push(request.clone());
    let reply = (shared.script)(&request);
    let (sender, shared) = (sender.clone(), shared.clone());
    thread::spawn(move || answer(&shared, reply, stream, &sender));
}

/// Answers `stream` as `reply` says.
fn answer(shared: &Shared, reply: Reply, stream: u32, sender: &Sender<Out>) {
    let send = |item| {
        let _ = sender.send(item);
    };
    match reply {
        Reply::Send(bytes) | Reply::SendWithCloseNotify(bytes) => {
            let (head, body) = split(&bytes);
            send(Out::Head {
                stream,
                fields: head,
                end: body.is_empty(),
            });
            if !body.is_empty() {
                send(Out::Data {
                    stream,
                    data: body,
                    end: true,
                });
            }
        }
        Reply::SendAndClose(bytes) => {
            answer(shared, Reply::Send(bytes), stream, sender);
            send(Out::Close);
        }
        Reply::After(wait, then) => {
            sleep_unless_stopped(shared, wait);
            answer(shared, *then, stream, sender);
        }
        Reply::Pieces(pieces) => {
            let mut bytes = Vec::new();
            let mut headed = false;
            for (wait, piece) in pieces {
                sleep_unless_stopped(shared, wait);
                bytes.extend_from_slice(&piece);
                if !headed && let Some(end) = head_end(&bytes) {
                    let (head, _) = split(&bytes[..end]);
                    send(Out::Head {
                        stream,
                        fields: head,
                        end: false,
                    });
                    bytes.drain(..end);
                    headed = true;
                }
                if headed && !bytes.is_empty() {
                    send(Out::Data {
                        stream,
                        data: std::mem::take(&mut bytes),
                        end: false,
                    });
                }
            }
            send(Out::Data {
                stream,
                data: Vec::new(),
                end: true,
            });
        }
        Reply::Close => send(Out::Close),
        Reply::Hang => {}
        Reply::Refuse => send(Out::Reset(stream, ErrorCode::REFUSED_STREAM)),
        Reply::GoAway => send(Out::GoAway(stream.saturating_sub(2))),
        Reply::Frames(bytes) => send(Out::Raw(bytes)),
    }
}

/// Where an HTTP/1.1 response's head ends, if it has.
fn head_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|at| at + 4)
}

/// An HTTP/1.1 response's bytes as HTTP/2's head and body: `:status`,
/// the fields lowercased, the connection-specific ones left out.
fn split(bytes: &[u8]) -> (Vec<Field>, Vec<u8>) {
    let end = head_end(bytes).unwrap_or(bytes.len());
    let head = String::from_utf8_lossy(&bytes[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or("200")
        .to_owned();
    let mut fields = vec![(b":status".to_vec(), status.into_bytes())];
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            if crate::h2::session::is_connection_specific(name.as_bytes(), value.trim().as_bytes())
            {
                continue;
            }
            fields.push((name.into_bytes(), value.trim().as_bytes().to_vec()));
        }
    }
    (fields, bytes[end..].to_vec())
}
