//! The client's HTTP/2 connection (M38, docs/design/http.md): a thread
//! of its own owns the TLS socket and drives the session (`session`)
//! with it; a fetch opens a stream by handing the thread its request,
//! and reads its stream's events from a channel of its own.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use mio::Waker;
use rustls::pki_types::{CertificateDer, ServerName};

use super::frame::ErrorCode;
use super::hpack;
use super::session::{Ending, Session};
use crate::address_space::AddressSpace;
use crate::error::AbortHandle;
use crate::transport::stream::{TlsSocket, Waiter};

/// The window this end gives each stream: the reference browser's.
pub const H2_STREAM_WINDOW: u32 = 6 << 20;
/// The window this end gives the connection.
pub const H2_CONNECTION_WINDOW: u32 = 15 << 20;
/// The most streams this end opens at once on a connection, whatever
/// more the server allows.
pub const MAX_H2_STREAMS: usize = 100;
/// The most control frames (`PING`, `SETTINGS`, empty `DATA`) a server
/// may send in a second before the connection ends with
/// `ENHANCE_YOUR_CALM`.
pub const MAX_H2_CONTROL_FRAMES: u32 = 1000;
/// The most a response's header fields may take, decoded, which this end
/// announces (`SETTINGS_MAX_HEADER_LIST_SIZE`): HTTP/1.1's bound.
pub const MAX_HEADER_LIST_SIZE: usize = crate::http::HEADER_LIMIT;

/// How long the thread waits for the socket at most before it looks at
/// its clocks again.
const TICK: Duration = Duration::from_millis(250);

/// What a stream's fetch hears.
#[derive(Debug)]
pub(crate) enum Event {
    /// The response's head: its status and its fields, lowercased, the
    /// pseudo-fields taken out.
    Head {
        status: u16,
        fields: Vec<hpack::Field>,
    },
    /// A piece of the body as it came.
    Data(Vec<u8>),
    /// The response is whole.
    End,
    /// The server did not process the request: it may be sent again on
    /// another connection.
    Retry,
    /// The stream failed.
    Failed(StreamError),
}

/// Why a stream failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamError {
    /// The server reset the stream with this code.
    Reset(ErrorCode),
    /// The connection ended with this code, ours or the server's.
    Connection(ErrorCode, String),
    /// The response broke RFC 9113's rules for a message.
    Malformed(&'static str),
    HeadersTooLarge,
    Io(String),
    Timeout,
}

/// A request for a new stream: its header fields, pseudo-fields first,
/// names lowercase, and its body.
pub(crate) struct Request {
    pub(crate) fields: Vec<hpack::Field>,
    pub(crate) body: Option<Vec<u8>>,
    /// Whether it may be sent again when the connection ends before any
    /// of its response came: a `GET` or a `HEAD`.
    pub(crate) idempotent: bool,
}

enum Command {
    Open {
        token: u64,
        request: Request,
        events: Sender<Event>,
    },
    Cancel(u64),
    Consumed(u64, usize),
}

/// What the fetches and the thread share.
struct State {
    commands: VecDeque<Command>,
    /// The streams the fetches hold.
    active: usize,
    /// The most streams the server lets this end open.
    limit: usize,
    /// Whether the connection takes no new stream: a `GOAWAY` came, or
    /// stream ids ran out.
    going_away: bool,
    closed: bool,
    /// Whether the pool let the connection go: it ends once idle.
    dropped: bool,
    next_token: u64,
}

struct Shared {
    state: Mutex<State>,
    /// Wakes the fetches waiting for a stream.
    changed: Condvar,
    /// Wakes the connection's thread.
    waker: Arc<Waker>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn command(&self, command: Command) {
        self.lock().commands.push_back(command);
        let _ = self.waker.wake();
    }
}

/// An HTTP/2 connection the pool keeps.
pub(crate) struct Connection {
    shared: Arc<Shared>,
    /// The address space of the server it reached.
    pub(crate) space: AddressSpace,
    /// The address it reached, and the server's certificate, for a
    /// request to another host that may share it (9.1.1).
    pub(crate) peer: SocketAddr,
    certificate: Option<CertificateDer<'static>>,
}

/// Why a stream could not be opened.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unopened {
    /// The connection takes no new stream: another is to be used.
    Unusable,
    Aborted,
}

/// The settings a connection's thread keeps to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Settings {
    /// How long the server may send nothing while a stream waits.
    pub(crate) read_timeout: Duration,
    /// How long the connection is kept with no stream.
    pub(crate) idle_timeout: Duration,
}

impl Connection {
    /// Starts the connection's thread on `socket`, whose TLS chose `h2`.
    pub(crate) fn start(
        socket: TlsSocket,
        waiter: Waiter,
        space: AddressSpace,
        peer: SocketAddr,
        settings: Settings,
    ) -> io::Result<Arc<Connection>> {
        let certificate = socket
            .conn
            .peer_certificates()
            .and_then(|chain| chain.first())
            .map(|certificate| certificate.clone().into_owned());
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                commands: VecDeque::new(),
                active: 0,
                limit: MAX_H2_STREAMS,
                going_away: false,
                closed: false,
                dropped: false,
                next_token: 0,
            }),
            changed: Condvar::new(),
            waker: waiter.waker().clone(),
        });
        let mut worker = Worker::new(shared.clone(), socket, waiter, settings);
        std::thread::Builder::new()
            .name("http/2".to_owned())
            .spawn(move || worker.run())?;
        Ok(Arc::new(Connection {
            shared,
            space,
            peer,
            certificate,
        }))
    }

    /// Whether the server's certificate is valid for `host`: the
    /// connection may then carry its requests too.
    pub(crate) fn covers(&self, host: &crate::Host) -> bool {
        let Some(certificate) = &self.certificate else {
            return false;
        };
        let Ok(parsed) = rustls::server::ParsedCertificate::try_from(certificate) else {
            return false;
        };
        let name = match host {
            crate::Host::Domain(name) => ServerName::try_from(name.clone()).ok(),
            _ => None,
        };
        name.is_some_and(|name| rustls::client::verify_server_name(&parsed, &name).is_ok())
    }

    /// Whether the connection takes new streams.
    pub(crate) fn usable(&self) -> bool {
        let state = self.shared.lock();
        !state.going_away && !state.closed
    }

    /// Opens a stream for `request`, waiting while the server's limit of
    /// streams is reached; its events come on the handle's channel.
    pub(crate) fn open(
        self: &Arc<Self>,
        request: Request,
        abort: &AbortHandle,
    ) -> Result<StreamHandle, Unopened> {
        let mut state = self.shared.lock();
        loop {
            if state.going_away || state.closed {
                return Err(Unopened::Unusable);
            }
            if abort.is_aborted() {
                return Err(Unopened::Aborted);
            }
            if state.active < state.limit.min(MAX_H2_STREAMS) {
                break;
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, crate::transport::ABORT_POLL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        state.active += 1;
        let token = state.next_token;
        state.next_token += 1;
        let (sender, events) = mpsc::channel();
        state.commands.push_back(Command::Open {
            token,
            request,
            events: sender,
        });
        drop(state);
        let _ = self.shared.waker.wake();
        Ok(StreamHandle {
            token,
            connection: self.clone(),
            events,
            ended: false,
        })
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.shared.lock().dropped = true;
        let _ = self.shared.waker.wake();
    }
}

/// A fetch's stream.
pub(crate) struct StreamHandle {
    token: u64,
    connection: Arc<Connection>,
    events: Receiver<Event>,
    /// Whether the stream ended: its end, a failure, or a retry.
    ended: bool,
}

impl StreamHandle {
    /// The next event, waiting at most `timeout`; `None` if none came.
    pub(crate) fn next(&mut self, timeout: Duration) -> Option<Event> {
        let event = match self.events.recv_timeout(timeout) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => return None,
            Err(RecvTimeoutError::Disconnected) => {
                Event::Failed(StreamError::Io("the connection ended".to_owned()))
            }
        };
        if matches!(event, Event::End | Event::Retry | Event::Failed(_)) {
            self.ended = true;
        }
        Some(event)
    }

    /// The fetch took `bytes` of the body: the window may be given back.
    pub(crate) fn consumed(&self, bytes: usize) {
        if bytes > 0 {
            self.connection
                .shared
                .command(Command::Consumed(self.token, bytes));
        }
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        let shared = &self.connection.shared;
        if !self.ended {
            shared.command(Command::Cancel(self.token));
        }
        shared.lock().active -= 1;
        shared.changed.notify_all();
    }
}

/// The connection's thread: the socket read and written for the
/// session, the fetches' commands given to it, the clocks kept.
struct Worker {
    shared: Arc<Shared>,
    socket: TlsSocket,
    waiter: Waiter,
    settings: Settings,
    session: Session,
    /// The session's bytes not yet taken by the socket.
    output: Vec<u8>,
    last_read: Instant,
    idle_since: Option<Instant>,
}

impl Worker {
    fn new(shared: Arc<Shared>, socket: TlsSocket, waiter: Waiter, settings: Settings) -> Worker {
        Worker {
            shared,
            socket,
            waiter,
            settings,
            session: Session::new(),
            output: Vec::new(),
            last_read: Instant::now(),
            idle_since: Some(Instant::now()),
        }
    }

    fn run(&mut self) {
        let ending = loop {
            if let Err(ending) = self.turn() {
                break ending;
            }
        };
        {
            let mut state = self.shared.lock();
            state.closed = true;
            state.commands.clear();
        }
        self.shared.changed.notify_all();
        self.session.end(&ending);
        let _ = self.write_out();
        self.socket.conn.send_close_notify();
        let _ = self.flush_tls();
    }

    /// One turn: the fetches' commands, the socket written and read, the
    /// clocks; then a wait for the socket.
    fn turn(&mut self) -> Result<(), Ending> {
        self.commands();
        self.write_out()?;
        self.read_in()?;
        self.share();
        self.write_out()?;
        let now = Instant::now();
        if self.session.is_idle() {
            let since = *self.idle_since.get_or_insert(now);
            let state = self.shared.lock();
            let done = state.dropped
                || state.going_away
                || now.duration_since(since) >= self.settings.idle_timeout;
            if done && state.commands.is_empty() {
                return Err(Ending {
                    code: ErrorCode::NO_ERROR,
                    why: "idle".to_owned(),
                    tell: true,
                });
            }
        } else {
            self.idle_since = None;
            if now.duration_since(self.last_read) >= self.settings.read_timeout {
                return Err(Ending {
                    code: ErrorCode::CANCEL,
                    why: "timeout".to_owned(),
                    tell: true,
                });
            }
        }
        self.waiter
            .wait(TICK)
            .map_err(|e| Ending::error(ErrorCode::INTERNAL_ERROR, e.to_string()))
    }

    /// Gives the fetches' commands to the session.
    fn commands(&mut self) {
        let commands: Vec<Command> = self.shared.lock().commands.drain(..).collect();
        for command in commands {
            match command {
                Command::Open {
                    token,
                    request,
                    events,
                } => self.session.open(token, request, events),
                Command::Cancel(token) => self.session.cancel(token),
                Command::Consumed(token, bytes) => self.session.consumed(token, bytes),
            }
        }
        self.share();
    }

    /// Tells the fetches what the session learned: the server's limit
    /// of streams, and whether it goes away.
    fn share(&mut self) {
        let limit = self.session.take_limit();
        let going_away = self.session.going_away();
        if limit.is_none() && !going_away {
            return;
        }
        let mut state = self.shared.lock();
        if let Some(limit) = limit {
            state.limit = limit;
        }
        state.going_away |= going_away;
        drop(state);
        self.shared.changed.notify_all();
    }

    /// Writes what is waiting, as far as the socket takes it.
    fn write_out(&mut self) -> Result<(), Ending> {
        let io = |e: io::Error| Ending {
            code: ErrorCode::INTERNAL_ERROR,
            why: e.to_string(),
            tell: false,
        };
        if self.session.has_output() {
            let more = self.session.take_output();
            self.output.extend_from_slice(&more);
        }
        while !self.output.is_empty() {
            match self.socket.conn.writer().write(&self.output) {
                Ok(0) => break,
                Ok(n) => {
                    self.output.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(io(e)),
            }
            if !self.flush_tls().map_err(io)? {
                break;
            }
        }
        self.flush_tls().map_err(io)?;
        Ok(())
    }

    /// Writes rustls's records to the socket: whether all went.
    fn flush_tls(&mut self) -> io::Result<bool> {
        while self.socket.conn.wants_write() {
            match self.socket.conn.write_tls(&mut self.socket.sock) {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    /// Reads what the socket has, for the session.
    fn read_in(&mut self) -> Result<(), Ending> {
        let io = |e: io::Error| Ending {
            code: ErrorCode::INTERNAL_ERROR,
            why: e.to_string(),
            tell: false,
        };
        let closed = || Ending {
            code: ErrorCode::NO_ERROR,
            why: "the server closed the connection".to_owned(),
            tell: false,
        };
        loop {
            match self.socket.conn.read_tls(&mut self.socket.sock) {
                Ok(0) => return Err(closed()),
                Ok(_) => {
                    self.socket
                        .conn
                        .process_new_packets()
                        .map_err(|e| io(io::Error::other(e)))?;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(io(e)),
            }
            let mut chunk = [0u8; 16_384];
            loop {
                match self.socket.conn.reader().read(&mut chunk) {
                    Ok(0) => return Err(closed()),
                    Ok(n) => {
                        self.last_read = Instant::now();
                        self.session.receive(&chunk[..n])?;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(io(e)),
                }
            }
        }
    }
}
