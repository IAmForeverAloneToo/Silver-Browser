//! The WebSocket interface's state (G74, WebSockets section 3), behind
//! the bindings' calls and tasks so that any engine's bindings, a shell
//! of a class, share it: the URL's and the subprotocols' checks, the
//! ready state, the buffered amount, the close code's and reason's
//! checks, the binary type, and which events a connection's news fires,
//! in the reference browser's order and with its messages. A document's
//! host and a worker's each keep one, and start the connections it asks
//! for.

use std::collections::{HashMap, VecDeque};

use js::{Answer, DomError, SocketCall};
use net::url::Part;
use net::websocket::SocketEvent;
use net::{Initiator, Url};

use crate::loader::{SocketHandle, SocketUpdate};

/// The most WebSockets a document or worker opens; past it the
/// constructor throws, as the reference browser's does past its own.
pub(crate) const MAX_SOCKETS: usize = 255;

const CONNECTING: u8 = 0;
const OPEN: u8 = 1;
const CLOSING: u8 = 2;
const CLOSED: u8 = 3;

/// A socket's state.
struct Socket {
    /// Its URL, `ws:` or `wss:`, serialized, and its origin's
    /// serialization, its messages' `origin`.
    url: String,
    origin: String,
    state: u8,
    buffered: u64,
    protocol: String,
    extensions: String,
    /// Whether binary messages are `ArrayBuffer`s, not `Blob`s.
    array_buffer: bool,
    /// The connection, once open; dropped, it closes.
    handle: Option<Box<dyn SocketHandle>>,
}

/// What resolves a constructor's URL, and what says whether policies
/// refuse a socket's URL, reporting it.
type Resolve<'a> = &'a dyn Fn(&str) -> Option<Url>;
type Blocked<'a> = &'a mut dyn FnMut(&Url) -> bool;

/// What a socket asks of its owner, the host's engine: a connection to
/// open.
pub(crate) struct Wanted {
    pub(crate) id: u32,
    pub(crate) request: net::Request,
    pub(crate) protocols: Vec<String>,
}

/// A document's or worker's WebSockets.
#[derive(Default)]
pub(crate) struct Sockets {
    sockets: HashMap<u32, Socket>,
    next: u32,
    open: usize,
    /// The connections asked for, for the owner to start.
    pub(crate) wanted: Vec<Wanted>,
    /// The tasks the connections' news makes: the JSON and the bytes of
    /// `js::Task::Socket`.
    pub(crate) tasks: VecDeque<(String, Vec<u8>)>,
    /// What the console is to say, as errors.
    pub(crate) said: Vec<String>,
}

fn syntax(message: String) -> DomError {
    DomError::new("SyntaxError", message)
}

/// Whether `protocol` may be offered: a token (RFC 6455 4.1, RFC 2616's
/// token), not empty.
fn is_token(protocol: &str) -> bool {
    !protocol.is_empty()
        && protocol
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"()<>@,;:\\\"/[]?={}".contains(&b))
}

/// Web IDL's `[Clamp] unsigned short`: rounded half to even, within 0
/// to 65535; NaN is 0.
fn clamp_u16(value: f64) -> u16 {
    if value.is_nan() {
        return 0;
    }
    let clamped = value.clamp(0.0, 65535.0);
    let floor = clamped.floor();
    let rounded = match clamped - floor {
        d if d < 0.5 => floor,
        d if d > 0.5 => floor + 1.0,
        _ if floor % 2.0 == 0.0 => floor,
        _ => floor + 1.0,
    };
    rounded as u16
}

impl Sockets {
    /// Whether any socket is not yet closed: the document is then not
    /// kept for back and forward.
    pub(crate) fn any(&self) -> bool {
        self.open > 0
    }

    /// Whether a socket's handshake is under way: the network has yet to
    /// answer it.
    pub(crate) fn connecting(&self) -> bool {
        self.sockets
            .values()
            .any(|socket| socket.state == CONNECTING)
    }

    /// Answers `call`: `resolve` resolves a constructor's URL against
    /// the API base URL, as UTF-8; `blocked` says whether the policies
    /// of the document or worker refuse a `ws:` or `wss:` URL, and
    /// reports it if so (`connect-src`); `initiator`
    /// is the document or worker it is made by (none, a realm gone).
    pub(crate) fn call(
        &mut self,
        call: SocketCall,
        resolve: &dyn Fn(&str) -> Option<Url>,
        blocked: &mut dyn FnMut(&Url) -> bool,
        initiator: Option<Initiator>,
    ) -> Result<Answer, DomError> {
        match call {
            SocketCall::Open { url, protocols } => {
                self.open(&url, protocols, (resolve, blocked), initiator)
            }
            SocketCall::Send { id, binary, data } => {
                let Some(socket) = self.sockets.get_mut(&id) else {
                    return Ok(Answer::Nothing);
                };
                match socket.state {
                    CONNECTING => Err(DomError::new(
                        "InvalidStateError",
                        "Failed to execute 'send' on 'WebSocket': Still in CONNECTING state.",
                    )),
                    OPEN => {
                        socket.buffered += data.len() as u64;
                        if let Some(handle) = &socket.handle {
                            send(handle.as_ref(), binary, data);
                        }
                        Ok(Answer::Nothing)
                    }
                    // Sent nowhere, but counted (section 3.1, `send()`).
                    _ => {
                        socket.buffered += data.len() as u64;
                        self.said
                            .push("WebSocket is already in CLOSING or CLOSED state.".to_owned());
                        Ok(Answer::Nothing)
                    }
                }
            }
            SocketCall::Close { id, code, reason } => self.close(id, code, reason),
            SocketCall::Get { id, name } => {
                let Some(socket) = self.sockets.get(&id) else {
                    return Ok(Answer::Nothing);
                };
                Ok(match name.as_str() {
                    "url" => Answer::String(socket.url.clone()),
                    "readyState" => Answer::Number(f64::from(socket.state)),
                    "bufferedAmount" => Answer::Number(socket.buffered as f64),
                    "protocol" => Answer::String(socket.protocol.clone()),
                    "extensions" => Answer::String(socket.extensions.clone()),
                    "binaryType" => Answer::String(
                        if socket.array_buffer {
                            "arraybuffer"
                        } else {
                            "blob"
                        }
                        .to_owned(),
                    ),
                    _ => Answer::Nothing,
                })
            }
            SocketCall::SetBinaryType { id, value } => {
                if let Some(socket) = self.sockets.get_mut(&id) {
                    // Another value is no `BinaryType`: ignored.
                    match value.as_str() {
                        "blob" => socket.array_buffer = false,
                        "arraybuffer" => socket.array_buffer = true,
                        _ => {}
                    }
                }
                Ok(Answer::Nothing)
            }
        }
    }

    fn open(
        &mut self,
        url: &str,
        protocols: Vec<String>,
        (resolve, blocked): (Resolve<'_>, Blocked<'_>),
        initiator: Option<Initiator>,
    ) -> Result<Answer, DomError> {
        let fail = |what: String| Err(syntax(format!("Failed to construct 'WebSocket': {what}")));
        let Some(parsed) = resolve(url) else {
            return fail(format!("The URL '{url}' is invalid."));
        };
        // `http:` and `https:` are `ws:` and `wss:` (section 3.1, step 4).
        let scheme = match parsed.scheme() {
            "http" | "ws" => "ws",
            "https" | "wss" => "wss",
            other => {
                return fail(format!(
                    "The URL's scheme must be either 'http', 'https', 'ws', or 'wss'. '{other}' is not allowed."
                ));
            }
        };
        if let Some(fragment) = parsed.fragment() {
            return fail(format!(
                "The URL contains a fragment identifier ('{fragment}'). Fragment identifiers are not allowed in WebSocket URLs."
            ));
        }
        for (i, protocol) in protocols.iter().enumerate() {
            if !is_token(protocol) {
                return fail(format!("The subprotocol '{protocol}' is invalid."));
            }
            if protocols[..i].contains(protocol) {
                return fail(format!("The subprotocol '{protocol}' is duplicated."));
            }
        }
        let initiator = initiator
            .ok_or_else(|| syntax("Failed to construct 'WebSocket': no document".into()))?;
        // Mixed content (Mixed Content 4.3): a `ws:` URL of an `https:`
        // document throws, but one the reference counts as trustworthy
        // already, a loopback host's.
        let trustworthy = parsed
            .with_part(Part::Protocol, "http")
            .is_potentially_trustworthy();
        if scheme == "ws" && initiator.url.scheme() == "https" && !trustworthy {
            return Err(DomError::new(
                "SecurityError",
                "Failed to construct 'WebSocket': An insecure WebSocket connection may not be initiated from a page loaded over HTTPS.",
            ));
        }
        if self.open >= MAX_SOCKETS {
            return Err(DomError::new(
                "SecurityError",
                "Failed to construct 'WebSocket': too many WebSockets.",
            ));
        }
        let socket_url = parsed.with_part(Part::Protocol, scheme);
        let origin = socket_url.origin().serialization();
        let shown = socket_url.as_str().to_owned();
        // The handshake's request is the `http:` or `https:` one.
        let parsed = parsed.with_part(
            Part::Protocol,
            if scheme == "ws" { "http" } else { "https" },
        );
        let mut request =
            net::Request::get(parsed, net::Destination::Empty).initiated_by(initiator);
        request.credentials = net::Credentials::Include;
        request.redirect = net::Redirect::Error;
        request.cache = net::CacheMode::NoStore;
        self.next += 1;
        let id = self.next;
        // Refused by `connect-src` (WebSockets 3.1, step 11, as the
        // reference has it): no exception, the socket closed at once,
        // and an `error` alone, in a task.
        if blocked(&socket_url) {
            self.sockets.insert(
                id,
                Socket {
                    url: shown,
                    origin,
                    state: CLOSED,
                    buffered: 0,
                    protocol: String::new(),
                    extensions: String::new(),
                    array_buffer: false,
                    handle: None,
                },
            );
            self.task(id, r#""type":"error""#, Vec::new());
            return Ok(Answer::Number(f64::from(id)));
        }
        self.sockets.insert(
            id,
            Socket {
                url: shown,
                origin,
                state: CONNECTING,
                buffered: 0,
                protocol: String::new(),
                extensions: String::new(),
                array_buffer: false,
                handle: None,
            },
        );
        self.open += 1;
        self.wanted.push(Wanted {
            id,
            request,
            protocols,
        });
        Ok(Answer::Number(f64::from(id)))
    }

    fn close(
        &mut self,
        id: u32,
        code: Option<f64>,
        reason: Option<String>,
    ) -> Result<Answer, DomError> {
        let fail = |name: &'static str, what: String| {
            Err(DomError::new(
                name,
                format!("Failed to execute 'close' on 'WebSocket': {what}"),
            ))
        };
        // A reason with no code is a code of 0, as the reference has it.
        let code = match (code, &reason) {
            (Some(code), _) => Some(clamp_u16(code)),
            (None, Some(_)) => Some(0),
            (None, None) => None,
        };
        if let Some(code) = code
            && code != 1000
            && !(3000..=4999).contains(&code)
        {
            return fail(
                "InvalidAccessError",
                format!(
                    "The close code must be either 1000, or between 3000 and 4999. {code} is neither."
                ),
            );
        }
        let reason = reason.unwrap_or_default();
        if reason.len() > 123 {
            return fail(
                "SyntaxError",
                "The close reason must not be greater than 123 UTF-8 bytes.".to_owned(),
            );
        }
        let Some(socket) = self.sockets.get_mut(&id) else {
            return Ok(Answer::Nothing);
        };
        match socket.state {
            // The connection failed (section 3.1, `close()` step 3): its
            // handshake goes on, its end thrown away.
            CONNECTING => {
                socket.state = CLOSING;
                self.said
                    .push("WebSocket is closed before the connection is established.".to_owned());
                self.fail(id);
            }
            OPEN => {
                socket.state = CLOSING;
                if let Some(handle) = &socket.handle {
                    handle.close(code, &reason);
                }
            }
            _ => {}
        }
        Ok(Answer::Nothing)
    }

    /// Fails socket `id`: `error`, then `close` with 1006, not clean.
    fn fail(&mut self, id: u32) {
        self.task(id, r#""type":"error""#, Vec::new());
        self.closed(id, false, 1006, "");
    }

    fn closed(&mut self, id: u32, clean: bool, code: u16, reason: &str) {
        let Some(socket) = self.sockets.get_mut(&id) else {
            return;
        };
        if socket.state == CLOSED {
            return;
        }
        socket.state = CLOSED;
        socket.handle = None;
        self.open -= 1;
        let fields = format!(
            r#""type":"close","code":{code},"reason":{},"wasClean":{clean}"#,
            super::json_string(reason)
        );
        self.task(id, &fields, Vec::new());
    }

    fn task(&mut self, id: u32, fields: &str, bytes: Vec<u8>) {
        self.tasks
            .push_back((format!(r#"{{"id":{id},{fields}}}"#), bytes));
    }

    /// Takes what connection `id` did: its handshake's end, or an event.
    pub(crate) fn arrived(&mut self, id: u32, update: SocketUpdate) {
        let Some(socket) = self.sockets.get_mut(&id) else {
            return;
        };
        match update {
            SocketUpdate::Opened(Ok((agreed, handle))) => {
                // Closed while it opened: the connection goes at once.
                if socket.state != CONNECTING {
                    return;
                }
                socket.state = OPEN;
                socket.protocol = agreed.protocol;
                socket.extensions = agreed.extensions;
                socket.handle = Some(handle);
                self.task(id, r#""type":"open""#, Vec::new());
            }
            SocketUpdate::Opened(Err(why)) => {
                if socket.state == CLOSED {
                    return;
                }
                let url = socket.url.clone();
                self.said
                    .push(format!("WebSocket connection to '{url}' failed: {why}"));
                self.fail(id);
            }
            SocketUpdate::Event(event) => self.event(id, event),
        }
    }

    fn event(&mut self, id: u32, event: SocketEvent) {
        let Some(socket) = self.sockets.get_mut(&id) else {
            return;
        };
        match event {
            // A message comes to an open socket alone (section 3.3).
            SocketEvent::Text(text) if socket.state == OPEN => {
                let fields = format!(
                    r#""type":"message","text":{},"origin":{}"#,
                    super::json_string(&text),
                    super::json_string(&socket.origin)
                );
                self.task(id, &fields, Vec::new());
            }
            // Made a `Blob` or an `ArrayBuffer` as the binary type is when
            // its task runs (section 3.3, the task's step 3).
            SocketEvent::Binary(bytes) if socket.state == OPEN => {
                let fields = format!(
                    r#""type":"message","binary":true,"origin":{}"#,
                    super::json_string(&socket.origin)
                );
                self.task(id, &fields, bytes);
            }
            SocketEvent::Text(_) | SocketEvent::Binary(_) => {}
            SocketEvent::Sent(bytes) => {
                socket.buffered = socket.buffered.saturating_sub(bytes as u64);
            }
            SocketEvent::Closing => {
                if socket.state == OPEN {
                    socket.state = CLOSING;
                }
            }
            SocketEvent::Closed {
                clean,
                code,
                reason,
                failed,
            } => {
                if failed {
                    self.task(id, r#""type":"error""#, Vec::new());
                }
                self.closed(id, clean, code, &reason);
            }
        }
    }
}

/// Sends a message on `handle`: text the bindings made well formed.
fn send(handle: &dyn SocketHandle, binary: bool, data: Vec<u8>) {
    match binary {
        true => handle.send_binary(&data),
        false => match String::from_utf8(data) {
            Ok(text) => handle.send_text(&text),
            Err(e) => handle.send_binary(e.as_bytes()),
        },
    }
}
