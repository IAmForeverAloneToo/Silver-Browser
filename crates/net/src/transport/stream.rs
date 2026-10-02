//! A connection's byte stream: TCP, or TLS over TCP. A read or a write
//! waits at most the socket's own timeout, and one that runs out ends
//! the connection's use. An abort shuts the socket down, which ends a
//! read or write waiting on it at once.
//!
//! Nothing here retries an operation whose timeout ran out: Windows
//! leaves such a socket in an indeterminate state, which may have lost
//! data (setsockopt, SO_RCVTIMEO and SO_SNDTIMEO).

use std::fmt;
use std::io::{self, Read, Write};
use std::net::TcpStream;

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

/// Whether `error` is a socket timeout running out: Unix says the call
/// would block, Windows that it timed out.
pub(crate) fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

pub(crate) enum Socket {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Socket {
    /// The TCP stream underneath.
    pub(crate) fn tcp(&self) -> &TcpStream {
        match self {
            Socket::Plain(tcp) => tcp,
            Socket::Tls(tls) => &tls.sock,
        }
    }
}

/// A connection's stream, and the fetch using it, if one is.
pub(crate) struct Stream {
    socket: Socket,
    /// The abort handle of the fetch the stream is lent to, and the
    /// watch through which that abort shuts the socket down.
    user: Option<(AbortHandle, Watch)>,
}

impl Stream {
    /// `socket`, whose TCP stream already has its read and write
    /// timeouts.
    pub(crate) fn new(socket: Socket) -> Stream {
        Stream { socket, user: None }
    }

    /// Lends the stream to a fetch: its abort shuts the socket down
    /// from now on, and ends the reads and writes.
    pub(crate) fn lend(&mut self, abort: &AbortHandle) -> io::Result<()> {
        self.user = None;
        let watch = abort.watch(self.socket.tcp())?;
        self.user = Some((abort.clone(), watch));
        Ok(())
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

    /// Runs `op` on the socket. An abort shuts the socket down, so
    /// whatever the call then returned, the fetch was aborted; a
    /// timeout that ran out is told apart from other errors.
    fn io<T>(&mut self, op: impl FnOnce(&mut Socket) -> io::Result<T>) -> io::Result<T> {
        if self.aborted() {
            return Err(io::Error::other(Aborted));
        }
        let result = op(&mut self.socket);
        if self.aborted() {
            return Err(io::Error::other(Aborted));
        }
        match result {
            Err(e) if is_timeout(&e) => Err(io::ErrorKind::TimedOut.into()),
            other => other,
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
        let tcp = self.socket.tcp();
        if tcp.set_nonblocking(true).is_err() {
            return false;
        }
        let idle = matches!(tcp.peek(&mut [0]), Err(e) if e.kind() == io::ErrorKind::WouldBlock);
        tcp.set_nonblocking(false).is_ok() && idle
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.io(|socket| match socket {
            Socket::Plain(tcp) => tcp.read(buf),
            Socket::Tls(tls) => tls.read(buf),
        })
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.io(|socket| match socket {
            Socket::Plain(tcp) => tcp.write(buf),
            Socket::Tls(tls) => tls.write(buf),
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.io(|socket| match socket {
            Socket::Plain(tcp) => tcp.flush(),
            Socket::Tls(tls) => tls.flush(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, Instant};

    /// A connected pair: the client side as a `Stream` whose reads and
    /// writes wait `timeout`, and the server's end.
    fn pair(timeout: Duration) -> (Stream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client.set_read_timeout(Some(timeout)).unwrap();
        client.set_write_timeout(Some(timeout)).unwrap();
        let (server, _) = listener.accept().unwrap();
        (Stream::new(Socket::Plain(client)), server)
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
    fn a_silent_server_times_out() {
        let (mut stream, _server) = pair(Duration::from_millis(600));
        let started = Instant::now();
        let error = stream.read(&mut [0; 8]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() >= Duration::from_millis(500));
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
        let abort = aborted_after(Duration::from_millis(300));
        stream.lend(&abort).unwrap();
        let started = Instant::now();
        let error = stream.read(&mut [0; 8]).unwrap_err();
        assert!(is_abort(&error), "{error:?}");
        assert!(started.elapsed() < PROMPTLY, "{:?}", started.elapsed());
        assert!(stream.write(b"x").is_err_and(|e| is_abort(&e)));
    }

    #[test]
    fn an_abort_ends_a_waiting_write_at_once() {
        let (mut stream, _server) = pair(Duration::from_secs(60));
        let abort = AbortHandle::new();
        stream.lend(&abort).unwrap();
        // Fill the buffers first, then abort while a write waits.
        let aborter = abort.clone();
        let started = Instant::now();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(1500));
            aborter.abort();
        });
        let error = write_until_stuck(&mut stream);
        assert!(is_abort(&error), "{error:?}");
        assert!(started.elapsed() < Duration::from_millis(1500) + PROMPTLY);
    }

    #[test]
    fn an_abort_before_the_loan_ends_the_first_read() {
        let (mut stream, _server) = pair(Duration::from_secs(60));
        let abort = AbortHandle::new();
        abort.abort();
        stream.lend(&abort).unwrap();
        let started = Instant::now();
        assert!(stream.read(&mut [0; 8]).is_err_and(|e| is_abort(&e)));
        assert!(started.elapsed() < PROMPTLY);
    }

    #[test]
    fn a_released_stream_is_not_aborted_with_its_fetch() {
        let (mut stream, mut server) = pair(Duration::from_secs(5));
        let abort = AbortHandle::new();
        stream.lend(&abort).unwrap();
        stream.release();
        abort.abort();
        server.write_all(b"still here").unwrap();
        let mut buf = [0; 10];
        stream.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"still here");
        // A later loan to another fetch works as the first.
        stream.lend(&AbortHandle::new()).unwrap();
        stream.write_all(b"x").unwrap();
    }

    #[test]
    fn bytes_arrive_while_a_read_waits() {
        let (mut stream, mut server) = pair(Duration::from_secs(60));
        stream.lend(&AbortHandle::new()).unwrap();
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
        // Looking leaves the socket blocking: a read waits its timeout,
        // where a non-blocking one would fail at once.
        let started = Instant::now();
        assert_eq!(
            stream.read(&mut [0; 8]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(started.elapsed() >= Duration::from_millis(500));
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
