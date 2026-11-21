//! A WebSocket's connection once its opening handshake is done (G74,
//! RFC 6455): a thread of its own owns the socket, reads the server's
//! frames as they come and writes the page's, answers pings, and runs
//! the closing handshake. The page's side holds a [`Socket`], whose
//! sends and close wake the thread's poll; what the thread learns goes
//! to the page's side as [`SocketEvent`]s, in order.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use mio::Waker;

use super::stream::{Socket as Raw, Stream, Waiter};
use crate::websocket::{Event, Failure, Opcode, Reader, close_payload, encode};

/// How long the server has to close the connection once both close
/// frames are sent (the client waits for it to, section 7.1.1), and
/// to answer the client's close frame: then the client ends it.
pub const CLOSING_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a quiet connection's thread waits before it looks again.
const IDLE_WAIT: Duration = Duration::from_secs(60);

/// A connection the opening handshake switched to the WebSocket
/// protocol, and the bytes already read from it past the 101's head.
pub(crate) struct Upgraded {
    pub(crate) stream: Stream,
    pub(crate) buffered: Vec<u8>,
}

/// What a connection's thread tells the page's side, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketEvent {
    /// A message.
    Text(String),
    Binary(Vec<u8>),
    /// The page's messages of so many payload bytes, the last sent, are
    /// all written to the socket: `bufferedAmount` goes down by them.
    Sent(usize),
    /// The closing handshake began: the server's close frame came, the
    /// page's state becomes CLOSING.
    Closing,
    /// The end. `clean`: both close frames went and the server closed
    /// the connection; `code` and `reason`, the server's close frame's
    /// (1005, no code, when it had none; 1006 when none came);
    /// `failed`: the connection was failed (a breach of the protocol,
    /// section 7.1.7), when the page gets an `error` event first.
    Closed {
        clean: bool,
        code: u16,
        reason: String,
        failed: bool,
    },
}

/// What the page's side asks of the connection's thread.
enum Command {
    Send(Opcode, Vec<u8>),
    /// Starts the closing handshake: `code` and `reason` in the close
    /// frame, or none.
    Close(Option<u16>, String),
}

struct Shared {
    commands: Mutex<VecDeque<Command>>,
    waker: Arc<Waker>,
}

impl Shared {
    fn command(&self, command: Command) {
        self.commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(command);
        let _ = self.waker.wake();
    }
}

/// The page's side of a WebSocket connection. Dropped (the page gone),
/// it closes the connection with 1001, going away.
pub struct Socket {
    shared: Arc<Shared>,
}

impl Socket {
    /// Starts the connection's thread on `upgraded`, the server having
    /// agreed to `deflate`; `events` is told what comes.
    pub(crate) fn start(
        upgraded: Upgraded,
        deflate: Option<crate::websocket::DeflateParams>,
        events: Box<dyn FnMut(SocketEvent) + Send>,
    ) -> io::Result<Socket> {
        let (raw, waiter) = upgraded.stream.into_parts();
        let shared = Arc::new(Shared {
            commands: Mutex::new(VecDeque::new()),
            waker: waiter.waker().clone(),
        });
        let mut worker = Worker {
            raw,
            waiter,
            reader: Reader::new(deflate),
            unread: upgraded.buffered,
            output: Vec::new(),
            queued: 0,
            written: 0,
            messages: VecDeque::new(),
            shared: shared.clone(),
            events,
            sent_close: false,
            got_close: None,
            deadline: None,
        };
        std::thread::Builder::new()
            .name("websocket".to_owned())
            .spawn(move || worker.run())?;
        Ok(Socket { shared })
    }

    /// Sends a text message.
    pub fn send_text(&self, text: &str) {
        self.shared
            .command(Command::Send(Opcode::Text, text.as_bytes().to_vec()));
    }

    /// Sends a binary message.
    pub fn send_binary(&self, bytes: &[u8]) {
        self.shared
            .command(Command::Send(Opcode::Binary, bytes.to_vec()));
    }

    /// Starts the closing handshake, `code` and `reason` in the close
    /// frame (the page checked them: 1000 or 3000 to 4999, the reason at
    /// most 123 bytes).
    pub fn close(&self, code: Option<u16>, reason: &str) {
        self.shared.command(Command::Close(code, reason.to_owned()));
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.shared
            .command(Command::Close(Some(1001), String::new()));
    }
}

/// How the connection ended, as the thread found it.
enum Ending {
    /// The server closed it, or the closing timed out.
    Over,
    /// Failed: a breach of the protocol.
    Failed(Failure),
    /// The socket failed.
    Broken,
}

struct Worker {
    raw: Raw,
    waiter: Waiter,
    reader: Reader,
    /// Bytes read before the thread began: the handshake's reader's.
    unread: Vec<u8>,
    /// What waits to be written.
    output: Vec<u8>,
    /// How many bytes were ever queued and written: the page's messages
    /// are told sent by where their frames end.
    queued: u64,
    written: u64,
    /// The page's messages not yet written whole: where each one's frame
    /// ends, and its payload's length.
    messages: VecDeque<(u64, usize)>,
    shared: Arc<Shared>,
    events: Box<dyn FnMut(SocketEvent) + Send>,
    sent_close: bool,
    /// The server's close frame, once it came: its code and reason.
    got_close: Option<Option<(u16, String)>>,
    /// When the closing handshake gives up waiting.
    deadline: Option<Instant>,
}

impl Worker {
    fn run(&mut self) {
        let ending = loop {
            if let Err(ending) = self.turn() {
                break ending;
            }
        };
        let (code, reason) = match &self.got_close {
            Some(Some((code, reason))) => (*code, reason.clone()),
            Some(None) => (1005, String::new()),
            None => (1006, String::new()),
        };
        let event = match ending {
            Ending::Failed(failure) => {
                // The failure's close frame, if there is still room for
                // one; then the connection goes (section 7.1.7).
                if !self.sent_close {
                    self.queue(
                        Opcode::Close,
                        close_payload(Some(failure.code), failure.reason),
                        false,
                    );
                    let _ = self.write_out();
                }
                SocketEvent::Closed {
                    clean: false,
                    code: 1006,
                    reason: String::new(),
                    failed: true,
                }
            }
            Ending::Broken => SocketEvent::Closed {
                clean: false,
                code: 1006,
                reason: String::new(),
                failed: false,
            },
            Ending::Over => SocketEvent::Closed {
                clean: self.sent_close && self.got_close.is_some(),
                code,
                reason,
                failed: false,
            },
        };
        (self.events)(event);
        if let Raw::Tls(tls) = &mut self.raw {
            tls.conn.send_close_notify();
            let _ = tls.flush();
        }
        let _ = self.raw.tcp().shutdown(std::net::Shutdown::Both);
    }

    /// One turn: the page's commands, the socket written and read, the
    /// clock; then a wait for the socket or a command.
    fn turn(&mut self) -> Result<(), Ending> {
        self.commands();
        self.write_out()?;
        self.read_in()?;
        self.write_out()?;
        let timeout = match self.deadline {
            Some(deadline) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(Ending::Over);
                }
                deadline - now
            }
            // An open connection may be quiet for as long as it likes:
            // the wait ends for a byte or a command.
            None => IDLE_WAIT,
        };
        let write = !self.output.is_empty() || self.raw.tls_wants_write();
        self.waiter
            .wait(self.raw.tcp_mut(), timeout, write)
            .map_err(|_| Ending::Broken)
    }

    fn queue(&mut self, opcode: Opcode, payload: Vec<u8>, message: bool) {
        let frame = encode(opcode, &payload);
        self.queued += frame.len() as u64;
        self.output.extend_from_slice(&frame);
        if message {
            self.messages.push_back((self.queued, payload.len()));
        }
    }

    fn commands(&mut self) {
        let commands: Vec<Command> = self
            .shared
            .commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect();
        for command in commands {
            match command {
                // Once the close frame is sent no data frame may follow
                // (section 5.5.1): the page's later sends are dropped.
                Command::Send(..) if self.sent_close => {}
                Command::Send(opcode, payload) => self.queue(opcode, payload, true),
                Command::Close(..) if self.sent_close => {}
                Command::Close(code, reason) => {
                    self.queue(Opcode::Close, close_payload(code, &reason), false);
                    self.sent_close = true;
                    self.deadline = Some(Instant::now() + CLOSING_TIMEOUT);
                }
            }
        }
    }

    /// Writes what is waiting, as far as the socket takes it.
    fn write_out(&mut self) -> Result<(), Ending> {
        while !self.output.is_empty() {
            let wrote = match &mut self.raw {
                Raw::Plain(tcp) => tcp.write(&self.output),
                Raw::Tls(tls) => tls.write(&self.output),
            };
            match wrote {
                Ok(0) => return Err(Ending::Broken),
                Ok(n) => {
                    self.output.drain(..n);
                    self.written += n as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Err(Ending::Broken),
            }
        }
        if let Raw::Tls(tls) = &mut self.raw {
            match tls.flush() {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(_) => return Err(Ending::Broken),
            }
        }
        let mut sent = 0;
        while let Some(&(end, len)) = self.messages.front() {
            if end > self.written {
                break;
            }
            sent += len;
            self.messages.pop_front();
        }
        if sent > 0 {
            (self.events)(SocketEvent::Sent(sent));
        }
        Ok(())
    }

    /// Reads what the server sent, as far as there is any, and acts on
    /// it.
    fn read_in(&mut self) -> Result<(), Ending> {
        if !self.unread.is_empty() {
            let bytes = std::mem::take(&mut self.unread);
            self.take(&bytes)?;
        }
        let mut buf = [0u8; 16 << 10];
        loop {
            let read = match &mut self.raw {
                Raw::Plain(tcp) => tcp.read(&mut buf),
                Raw::Tls(tls) => tls.read(&mut buf),
            };
            match read {
                // The server closed the connection: after its close frame
                // and the client's, the clean end (section 7.1.1).
                Ok(0) => return Err(Ending::Over),
                Ok(n) => self.take(&buf[..n])?,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) if self.got_close.is_some() => return Err(Ending::Over),
                Err(_) => return Err(Ending::Broken),
            }
        }
    }

    fn take(&mut self, bytes: &[u8]) -> Result<(), Ending> {
        // Bytes after the server's close frame are not read (5.5.1).
        if self.got_close.is_some() {
            return Ok(());
        }
        let events = self.reader.read(bytes).map_err(Ending::Failed)?;
        for event in events {
            match event {
                Event::Text(text) => (self.events)(SocketEvent::Text(text)),
                Event::Binary(bytes) => (self.events)(SocketEvent::Binary(bytes)),
                // A pong answers each ping, its payload the ping's
                // (section 5.5.3); none once the close frame went.
                Event::Ping(payload) if !self.sent_close => {
                    self.queue(Opcode::Pong, payload, false)
                }
                Event::Ping(_) | Event::Pong(_) => {}
                Event::Close(close) => {
                    if !self.sent_close {
                        // The close frame echoed, its code and reason
                        // the server's (section 5.5.1), as the reference
                        // browser echoes them.
                        let payload = match &close {
                            Some((code, reason)) => close_payload(Some(*code), reason),
                            None => Vec::new(),
                        };
                        self.queue(Opcode::Close, payload, false);
                        self.sent_close = true;
                        (self.events)(SocketEvent::Closing);
                    }
                    self.got_close = Some(close);
                    self.deadline = Some(Instant::now() + CLOSING_TIMEOUT);
                    break;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "websocket_tests.rs"]
mod tests;
