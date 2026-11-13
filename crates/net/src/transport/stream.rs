//! A connection's byte stream: TCP, or TLS over TCP, on a non-blocking
//! socket. A read or a write that cannot go on waits for the socket in
//! a poll of the connection's own, which the abort of the fetch using it
//! wakes. So an abort ends a waiting read or write at once on every OS:
//! shutting a socket down from another thread does not end a call
//! blocked on it on Windows. A wait that runs out of time ends the
//! connection's use; the socket's own timeouts are not used, as Windows
//! leaves a socket whose send or receive timed out in an indeterminate
//! state.

use std::fmt;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use mio::net::TcpStream;
use mio::{Events, Interest, Poll, Token, Waker};
use rustls::{ClientConnection, StreamOwned};

use crate::error::{AbortHandle, Watch};

/// The error a read or write returns when its fetch was aborted.
#[derive(Debug)]
pub(crate) struct Aborted;

impl fmt::Display for Aborted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("aborted")
    }
}

impl std::error::Error for Aborted {}

/// Whether `error` says a fetch was aborted.
pub(crate) fn is_abort(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|inner| inner.is::<Aborted>())
}

const SOCKET: Token = Token(0);
const WAKE: Token = Token(1);

/// A connection's poll: it waits for the socket to be ready, or for the
/// waker an abort uses.
pub(crate) struct Waiter {
    poll: Poll,
    events: Events,
    waker: Arc<Waker>,
    /// Whether the socket's room to write wakes the wait too.
    writable: bool,
    /// Whether the last wait ran out its time, nothing woken.
    timed_out: bool,
}

/// How many waits ran out their time though what came after found data
/// at once: data that came and woke nothing, a read's latency raised to
/// the wait's time. None is expected; the site sweep says the count.
static LATE_WAKES: AtomicU64 = AtomicU64::new(0);

/// [`LATE_WAKES`]' count.
pub fn late_wakes() -> u64 {
    LATE_WAKES.load(Ordering::Relaxed)
}

/// Counts a late wake: a wait that ran out its time, then data at once.
pub(crate) fn late_wake() {
    LATE_WAKES.fetch_add(1, Ordering::Relaxed);
}

impl Waiter {
    /// A poll watching `tcp` for what comes in, and its waker.
    pub(crate) fn new(tcp: &mut TcpStream) -> io::Result<Waiter> {
        let poll = Poll::new()?;
        poll.registry().register(tcp, SOCKET, Interest::READABLE)?;
        let waker = Arc::new(Waker::new(poll.registry(), WAKE)?);
        Ok(Waiter {
            poll,
            events: Events::with_capacity(4),
            waker,
            writable: false,
            timed_out: false,
        })
    }

    /// What an abort wakes this poll with.
    pub(crate) fn waker(&self) -> &Arc<Waker> {
        &self.waker
    }

    /// Waits at most `timeout` for the socket `tcp` (the one this poll
    /// watches) to change or the waker to be woken. The caller tries
    /// again and looks at the clock and the abort itself: readiness is a
    /// hint, and a wait may end early. `write`: whether something waits
    /// to be written, when alone the socket's room to write wakes it.
    /// Windows' poll says an idle socket can be written to at each wait
    /// after a read found nothing: a connection that watched for it
    /// always never waited, a core held while the page was read.
    pub(crate) fn wait(
        &mut self,
        tcp: &mut TcpStream,
        timeout: Duration,
        write: bool,
    ) -> io::Result<()> {
        if write != self.writable {
            let interest = match write {
                true => Interest::READABLE | Interest::WRITABLE,
                false => Interest::READABLE,
            };
            self.poll.registry().reregister(tcp, SOCKET, interest)?;
            self.writable = write;
        }
        let polled = match self.poll.poll(&mut self.events, Some(timeout)) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(()),
            other => other,
        };
        self.timed_out = polled.is_ok() && self.events.is_empty();
        polled
    }

    /// Whether the last wait ran out its time, nothing woken.
    pub(crate) fn timed_out(&self) -> bool {
        self.timed_out
    }
}

/// A TLS connection's socket, as the HTTP/2 connection's thread owns it
/// (M38).
pub(crate) type TlsSocket = StreamOwned<ClientConnection, TcpStream>;

pub(crate) enum Socket {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Socket {
    /// The TCP stream underneath.
    fn tcp(&self) -> &TcpStream {
        match self {
            Socket::Plain(tcp) => tcp,
            Socket::Tls(tls) => &tls.sock,
        }
    }

    fn tcp_mut(&mut self) -> &mut TcpStream {
        match self {
            Socket::Plain(tcp) => tcp,
            Socket::Tls(tls) => &mut tls.sock,
        }
    }

    /// Whether TLS has records of its own to send (an alert, a key's
    /// update) besides what a write gives it.
    fn tls_wants_write(&self) -> bool {
        matches!(self, Socket::Tls(tls) if tls.conn.wants_write())
    }
}

/// A connection's stream, its poll, and the fetch using it, if one is.
pub(crate) struct Stream {
    socket: Socket,
    waiter: Waiter,
    /// How long a read or write may wait for the socket.
    timeout: Duration,
    /// The abort handle of the fetch the stream is lent to, and the
    /// watch through which that abort wakes the poll.
    user: Option<(AbortHandle, Watch)>,
}

impl Stream {
    /// `socket`, non-blocking and watched by `waiter`.
    pub(crate) fn new(socket: Socket, waiter: Waiter, timeout: Duration) -> Stream {
        Stream {
            socket,
            waiter,
            timeout,
            user: None,
        }
    }

    /// Lends the stream to a fetch: its abort ends the reads and writes
    /// from now on.
    pub(crate) fn lend(&mut self, abort: &AbortHandle) {
        self.user = None;
        let watch = abort.watch(self.waiter.waker());
        self.user = Some((abort.clone(), watch));
    }

    /// Takes the stream back from its fetch, before it waits in the pool:
    /// an abort of that fetch no longer touches it.
    pub(crate) fn release(&mut self) {
        self.user = None;
    }

    fn aborted(&self) -> bool {
        self.user
            .as_ref()
            .is_some_and(|(abort, _)| abort.is_aborted())
    }

    /// Runs `op` on the socket until it does not need to wait, waiting
    /// for the socket between tries: at most the stream's timeout in all,
    /// and no longer than the fetch's abort. `write`: whether `op` writes,
    /// when the socket's room to write is what it waits for.
    fn io<T>(
        &mut self,
        write: bool,
        mut op: impl FnMut(&mut Socket) -> io::Result<T>,
    ) -> io::Result<T> {
        let deadline = Instant::now() + self.timeout;
        let mut waited_out = false;
        loop {
            if self.aborted() {
                return Err(io::Error::other(Aborted));
            }
            match op(&mut self.socket) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Ok(done) => {
                    if waited_out {
                        late_wake();
                    }
                    return Ok(done);
                }
                other => return other,
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
            let write = write || self.socket.tls_wants_write();
            self.waiter
                .wait(self.socket.tcp_mut(), deadline - now, write)?;
            waited_out = self.waiter.timed_out();
        }
    }

    /// Whether an idle connection can still carry a request: the server
    /// has neither closed it nor sent anything since the last response.
    /// A server that ends an idle connection, with or without a `408`
    /// first, makes it useless; looking before it is taken spares the
    /// request a failure that only a `GET` or `HEAD` would get past.
    pub(crate) fn is_idle_and_open(&mut self) -> bool {
        if let Socket::Tls(tls) = &mut self.socket {
            // Plaintext rustls has already taken off the socket, or the
            // server's close_notify, would be found here.
            match tls.conn.reader().read(&mut [0]) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                _ => return false,
            }
        }
        matches!(self.socket.tcp().peek(&mut [0]), Err(e) if e.kind() == io::ErrorKind::WouldBlock)
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.io(false, |socket| match socket {
            Socket::Plain(tcp) => tcp.read(buf),
            Socket::Tls(tls) => tls.read(buf),
        })
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.io(true, |socket| match socket {
            Socket::Plain(tcp) => tcp.write(buf),
            Socket::Tls(tls) => tls.write(buf),
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.io(true, |socket| match socket {
            Socket::Plain(tcp) => tcp.flush(),
            Socket::Tls(tls) => tls.flush(),
        })
    }
}

/// `tcp`, made non-blocking for a [`Waiter`].
pub(crate) fn non_blocking(tcp: std::net::TcpStream) -> io::Result<TcpStream> {
    tcp.set_nonblocking(true)?;
    Ok(TcpStream::from_std(tcp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    /// A connected pair: the client side as a `Stream` whose reads and
    /// writes wait `timeout`, and the server's end.
    fn pair(timeout: Duration) -> (Stream, std::net::TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        let mut client = non_blocking(client).unwrap();
        let waiter = Waiter::new(&mut client).unwrap();
        (Stream::new(Socket::Plain(client), waiter, timeout), server)
    }

    /// An abort handle that aborts itself after `after`.
    fn aborted_after(after: Duration) -> AbortHandle {
        let abort = AbortHandle::new();
        let aborter = abort.clone();
        thread::spawn(move || {
            thread::sleep(after);
            aborter.abort();
        });
        abort
    }

    /// Writes to a server that reads nothing until a write fails, and
    /// returns that error.
    fn write_until_stuck(stream: &mut Stream) -> io::Error {
        let chunk = vec![0; 1 << 20];
        let started = Instant::now();
        loop {
            if let Err(e) = stream.write(&chunk) {
                return e;
            }
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "the writes never filled up"
            );
        }
    }

    const PROMPTLY: Duration = Duration::from_millis(1500);

    #[test]
    fn a_wait_to_write_wakes_when_the_socket_takes_more() {
        // The send buffer full, a wait for room to write is woken by the
        // room the peer's reads make, not by its time running out: what a
        // TLS handshake, a request's body and an HTTP/2 connection's frames
        // rest on once the socket is watched for writing only while they
        // have something to write.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        let mut client = non_blocking(client).unwrap();
        let mut waiter = Waiter::new(&mut client).unwrap();
        let chunk = vec![0u8; 1 << 16];
        let mut written = 0usize;
        loop {
            match io::Write::write(&mut client, &chunk) {
                Ok(n) => written += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("{e}"),
            }
            assert!(written < 1 << 30, "the send buffer never filled");
        }
        // The server reads, after a while, until the client closes: it
        // sends nothing and stays open, so that only room to write can
        // wake the client.
        let _ = written;
        let reader = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            let mut sink = vec![0u8; 1 << 16];
            while io::Read::read(&mut server, &mut sink).is_ok_and(|n| n > 0) {}
        });
        let began = Instant::now();
        loop {
            waiter
                .wait(&mut client, Duration::from_secs(5), true)
                .unwrap();
            match io::Write::write(&mut client, &chunk) {
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => panic!("{e}"),
            }
            assert!(began.elapsed() < Duration::from_secs(5), "never woken");
        }
        assert!(
            began.elapsed() < Duration::from_secs(1),
            "woken after {:?}",
            began.elapsed()
        );
        drop(waiter);
        drop(client);
        reader.join().unwrap();
    }

    #[test]
    fn an_idle_connections_wait_waits() {
        // Nothing to read and nothing to write: a wait lasts its time, as
        // an HTTP/2 connection's thread waits between turns (the browser's
        // idle CPU: a connection that never waited held a core).
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_server, _) = listener.accept().unwrap();
        let mut client = non_blocking(client).unwrap();
        let mut waiter = Waiter::new(&mut client).unwrap();
        // Something written, as a connection's request is: the socket can
        // take more, which is no reason to wake when there is no more.
        io::Write::write_all(&mut client, b"hello").unwrap();
        // The first wait may see the connection made.
        let _ = waiter.wait(&mut client, Duration::from_millis(50), false);
        // Each turn reads until there is nothing, as a connection's does,
        // then waits.
        let began = Instant::now();
        for _ in 0..4 {
            let read = io::Read::read(&mut client, &mut [0; 64]);
            assert!(matches!(read, Err(ref e) if e.kind() == io::ErrorKind::WouldBlock));
            waiter
                .wait(&mut client, Duration::from_millis(50), false)
                .unwrap();
        }
        assert!(
            began.elapsed() >= Duration::from_millis(150),
            "four waits of 50 ms took {:?}",
            began.elapsed()
        );
    }

    #[test]
    fn a_silent_server_times_out() {
        let (mut stream, _server) = pair(Duration::from_millis(600));
        let started = Instant::now();
        let error = stream.read(&mut [0; 8]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() >= Duration::from_millis(600));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_server_that_reads_nothing_times_a_write_out() {
        let (mut stream, _server) = pair(Duration::from_millis(600));
        let error = write_until_stuck(&mut stream);
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn an_abort_ends_a_waiting_read_at_once() {
        let (mut stream, _server) = pair(Duration::from_secs(60));
        stream.lend(&aborted_after(Duration::from_millis(300)));
        let started = Instant::now();
        let error = stream.read(&mut [0; 8]).unwrap_err();
        assert!(is_abort(&error), "{error:?}");
        assert!(started.elapsed() < PROMPTLY, "{:?}", started.elapsed());
        assert!(stream.write(b"x").is_err_and(|e| is_abort(&e)));
    }

    #[test]
    fn an_abort_ends_a_waiting_write_at_once() {
        let (mut stream, _server) = pair(Duration::from_secs(60));
        // The buffers fill at once; the abort comes while a write waits.
        stream.lend(&aborted_after(Duration::from_millis(1500)));
        let started = Instant::now();
        let error = write_until_stuck(&mut stream);
        assert!(is_abort(&error), "{error:?}");
        assert!(started.elapsed() < Duration::from_millis(1500) + PROMPTLY);
    }

    #[test]
    fn an_abort_before_the_loan_ends_the_first_read() {
        let (mut stream, _server) = pair(Duration::from_secs(60));
        let abort = AbortHandle::new();
        abort.abort();
        stream.lend(&abort);
        let started = Instant::now();
        assert!(stream.read(&mut [0; 8]).is_err_and(|e| is_abort(&e)));
        assert!(started.elapsed() < PROMPTLY);
    }

    #[test]
    fn a_released_stream_is_not_aborted_with_its_fetch() {
        let (mut stream, mut server) = pair(Duration::from_secs(5));
        let abort = AbortHandle::new();
        stream.lend(&abort);
        stream.release();
        abort.abort();
        server.write_all(b"still here").unwrap();
        let mut buf = [0; 10];
        stream.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"still here");
        // A later loan to another fetch works as the first.
        stream.lend(&AbortHandle::new());
        stream.write_all(b"x").unwrap();
    }

    #[test]
    fn bytes_arrive_while_a_read_waits() {
        let (mut stream, mut server) = pair(Duration::from_secs(60));
        stream.lend(&AbortHandle::new());
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(400));
            server.write_all(b"late").unwrap();
        });
        let mut buf = [0; 8];
        let n = stream.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"late");
    }

    #[test]
    fn an_idle_connection_is_looked_at_before_reuse() {
        let (mut stream, _server) = pair(Duration::from_millis(600));
        assert!(stream.is_idle_and_open());
        // Looking takes nothing: a read still waits its time.
        let started = Instant::now();
        assert_eq!(
            stream.read(&mut [0; 8]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(started.elapsed() >= Duration::from_millis(600));
        // Anything the server sends unasked makes it unusable...
        let (mut stream, mut server) = pair(Duration::from_secs(5));
        assert!(stream.is_idle_and_open());
        server
            .write_all(b"HTTP/1.1 408 Request Timeout\r\n\r\n")
            .unwrap();
        let waited = Instant::now();
        while stream.is_idle_and_open() {
            assert!(waited.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(10));
        }
        // ...without taking it from the stream.
        let mut buf = [0; 8];
        stream.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"HTTP/1.1");
        // And so does its end.
        let (mut stream, server) = pair(Duration::from_secs(5));
        drop(server);
        let waited = Instant::now();
        while stream.is_idle_and_open() {
            assert!(waited.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(10));
        }
    }
}
