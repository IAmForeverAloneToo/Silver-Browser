//! Connections for `http:` and `https:` (docs/design/net.md,
//! "Connections"): names resolved, addresses tried, address spaces
//! checked, TLS, the pool, the one retry, content codings, limits,
//! timeouts and aborts. [`Transport::exchange`] sends one request and
//! returns its response; redirects and the rest of Fetch are above it.

mod connect;
mod pool;
mod stream;
mod tls;

use std::io::{self, BufReader};
use std::sync::Arc;
use std::time::Duration;

use rustls::ClientConfig;
use rustls::pki_types::CertificateDer;

use crate::Url;
use crate::address_space::AddressSpace;
use crate::content_coding;
use crate::error::{AbortHandle, NetworkError};
use crate::headers::HeaderList;
use crate::http::{self, BodySink, Limits, ReadError, RequestMessage, Response};

use pool::{Connection, OriginKey, Pool};
use stream::{Socket, Stream, Waiter};

/// How long the attempts to connect to a host may take together.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a read may wait for a byte, or a write for room, before the
/// fetch times out.
pub const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// How long an idle connection is kept for another request.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// How many connections may be open to one origin (M14.3).
pub const MAX_CONNECTIONS_PER_ORIGIN: usize = 6;

/// How often the steps no abort can interrupt (resolving a name,
/// connecting, waiting for a free connection) look at the abort flag.
pub(crate) const ABORT_POLL: Duration = Duration::from_millis(250);

/// The transport's settings: the constants above, and the limits on a
/// response, unless a test wants them shorter.
#[derive(Debug, Clone)]
pub struct Config {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_per_origin: usize,
    pub limits: Limits,
    /// Roots trusted beside Mozilla's: only tests add one.
    pub(crate) extra_roots: Vec<CertificateDer<'static>>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            connect_timeout: CONNECT_TIMEOUT,
            read_timeout: READ_TIMEOUT,
            idle_timeout: IDLE_TIMEOUT,
            max_per_origin: MAX_CONNECTIONS_PER_ORIGIN,
            limits: Limits::default(),
            extra_roots: Vec::new(),
        }
    }
}

impl Config {
    /// Trusts `root` beside Mozilla's roots: a test server's certificate
    /// authority. There is no such option outside tests.
    #[cfg(any(test, feature = "test-server"))]
    pub fn trust(mut self, root: CertificateDer<'static>) -> Config {
        self.extra_roots.push(root);
        self
    }

    /// Trusts the certificates of the PEM file `pem` beside Mozilla's
    /// roots: the root of a network that intercepts TLS, for the live
    /// tests run by hand on one. A certificate no root store would take
    /// is left out; a file with none it would is refused. There is no
    /// such option outside tests.
    #[cfg(any(test, feature = "test-server"))]
    pub fn trust_pem(mut self, pem: &[u8]) -> Result<Config, String> {
        use rustls::pki_types::pem::PemObject;
        let mut store = rustls::RootCertStore::empty();
        for root in CertificateDer::pem_slice_iter(pem) {
            let root = root.map_err(|error| format!("not a PEM file: {error:?}"))?;
            if store.add(root.clone()).is_ok() {
                self.extra_roots.push(root);
            }
        }
        if store.is_empty() {
            return Err("no certificate in it can be a root".to_owned());
        }
        Ok(self)
    }
}

/// One request to send.
#[derive(Debug, Clone, Copy)]
pub struct Exchange<'a> {
    pub method: &'a str,
    /// An `http:` or `https:` URL.
    pub url: &'a Url,
    pub headers: &'a HeaderList,
    pub body: Option<&'a [u8]>,
    /// The address space of the page asking, or `None` for a load the
    /// user asked for, which may go anywhere.
    pub initiator: Option<AddressSpace>,
    pub abort: &'a AbortHandle,
    /// The most the body may hold, when less than the configured limit.
    pub body_limit: Option<usize>,
}

/// Where a streamed exchange's response goes as it is read (M24.8).
pub trait ResponseStream {
    /// The response's head has been read from a server in `space`:
    /// whether its body is to be handed here as it arrives, rather than
    /// read whole into the response.
    fn head(
        &mut self,
        status: u16,
        reason: &[u8],
        headers: &HeaderList,
        space: AddressSpace,
    ) -> bool;
    /// A piece of the body, its content codings removed.
    fn body(&mut self, bytes: &[u8]);
}

/// The reader's sink for a streamed exchange: the stream told of the
/// head, and the body's content codings removed on the way to it.
struct Decoding<'a> {
    stream: &'a mut dyn ResponseStream,
    space: AddressSpace,
    limit: usize,
    decoder: Option<content_coding::Decoder>,
    /// Why the decoding stopped the read.
    error: Option<NetworkError>,
}

impl BodySink for Decoding<'_> {
    fn head(&mut self, status: u16, reason: &[u8], headers: &HeaderList) -> bool {
        let streamed = self.stream.head(status, reason, headers, self.space);
        if streamed {
            self.decoder = Some(content_coding::Decoder::new(headers, self.limit));
        }
        streamed
    }

    fn body(&mut self, bytes: &[u8]) -> Result<(), ReadError> {
        let (Some(decoder), stream) = (self.decoder.as_mut(), &mut *self.stream) else {
            return Ok(());
        };
        decoder
            .push(bytes, &mut |decoded| stream.body(decoded))
            .map_err(|error| {
                self.error = Some(error);
                ReadError::Stopped
            })
    }
}

/// A response, and the address space of the server it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    pub response: Response,
    pub address_space: AddressSpace,
}

/// The pool of connections and the TLS configuration every fetch
/// shares.
pub struct Transport {
    config: Config,
    tls: Arc<ClientConfig>,
    pool: Pool,
}

/// How an exchange on one connection failed.
enum Failure {
    /// Writing the request failed.
    Send(io::Error),
    /// Reading the response failed.
    Read(ReadError),
}

impl Failure {
    /// Whether the server cannot have answered: the request could not be
    /// written, or the connection ended before a byte of response, as an
    /// idle connection the server has just closed does.
    fn nothing_received(&self) -> bool {
        let dropped = |e: &io::Error| {
            matches!(
                e.kind(),
                io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::UnexpectedEof
            )
        };
        match self {
            Failure::Send(e) | Failure::Read(ReadError::NoResponse(Some(e))) => dropped(e),
            Failure::Read(ReadError::NoResponse(None)) => true,
            Failure::Read(_) => false,
        }
    }

    fn into_error(self) -> NetworkError {
        match self {
            Failure::Send(e) => io_error(e),
            Failure::Read(read) => match read {
                ReadError::NoResponse(None) => {
                    NetworkError::Io("the connection closed with no response".to_owned())
                }
                ReadError::NoResponse(Some(e)) | ReadError::Io(e) => io_error(e),
                ReadError::Truncated => NetworkError::Truncated,
                ReadError::Malformed(what) => NetworkError::Malformed(what),
                ReadError::HeadersTooLarge => NetworkError::HeadersTooLarge,
                ReadError::BodyTooLarge => NetworkError::BodyTooLarge,
                ReadError::SwitchingProtocols => NetworkError::UnaskedUpgrade,
                // Its cause is the sink's; see `exchange_to`.
                ReadError::Stopped => NetworkError::ContentCoding,
            },
        }
    }
}

/// What an I/O error on a connection means for the fetch.
fn io_error(error: io::Error) -> NetworkError {
    if stream::is_abort(&error) {
        return NetworkError::Aborted;
    }
    if error.kind() == io::ErrorKind::TimedOut {
        return NetworkError::Timeout;
    }
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        Some(tls) => NetworkError::Tls(tls::failure(tls)),
        None => NetworkError::Io(error.to_string()),
    }
}

impl Transport {
    pub fn new(config: Config) -> Transport {
        Transport {
            tls: tls::client_config(&config.extra_roots),
            pool: Pool::new(config.max_per_origin, config.idle_timeout),
            config,
        }
    }

    /// Sends `exchange`'s request and reads the response, its body whole
    /// and its content codings removed (none for a `HEAD`).
    ///
    /// The connection is a pooled one to the origin, or a new one. A
    /// `GET` or `HEAD` sent on a pooled connection that ends before any
    /// byte of a response arrives is sent once more, on a new
    /// connection: the server may have closed it just as it was taken
    /// (RFC 9110 §9.2.2).
    pub fn exchange(&self, exchange: &Exchange) -> Result<Received, NetworkError> {
        self.exchange_to(exchange, None)
    }

    /// [`Transport::exchange`], the response's head given to `stream`
    /// as soon as it is read and, if the stream asks, its body a piece
    /// at a time as it arrives, its content codings removed; the
    /// response returned then has an empty body (M24.8).
    pub fn exchange_streaming(
        &self,
        exchange: &Exchange,
        stream: &mut dyn ResponseStream,
    ) -> Result<Received, NetworkError> {
        self.exchange_to(exchange, Some(stream))
    }

    fn exchange_to(
        &self,
        exchange: &Exchange,
        mut stream: Option<&mut dyn ResponseStream>,
    ) -> Result<Received, NetworkError> {
        let url = exchange.url;
        let mut limits = self.config.limits;
        if let Some(own) = exchange.body_limit {
            limits.body_bytes = limits.body_bytes.min(own);
        }
        let (Some(host), Some(port), true) = (
            url.host(),
            url.port_or_default(),
            matches!(url.scheme(), "http" | "https"),
        ) else {
            return Err(NetworkError::BadScheme(url.scheme().to_owned()));
        };
        let key = OriginKey {
            scheme: url.scheme().to_owned(),
            host: host.to_string(),
            port,
        };
        let message = RequestMessage {
            method: exchange.method,
            url,
            headers: exchange.headers,
            body: exchange.body,
        };
        let to_head = exchange.method == "HEAD";
        let mut fresh = false;
        loop {
            let mut lease = self.pool.checkout(&key, fresh, exchange.abort)?;
            let reused = lease.connection.is_some();
            if !reused {
                lease.connection = Some(self.open(url, exchange.initiator, exchange.abort)?);
            }
            let connection = lease.connection.as_mut().expect("a connection");
            if let Some(initiator) = exchange.initiator
                && !initiator.may_request(connection.space)
            {
                // The connection itself is fine for another request.
                lease.keep = true;
                return Err(NetworkError::AddressSpace {
                    initiator,
                    target: connection.space,
                });
            }
            connection.reader.get_mut().lend(exchange.abort);
            let mut decoding = stream.as_deref_mut().map(|stream| Decoding {
                stream,
                space: connection.space,
                limit: limits.body_bytes,
                decoder: None,
                error: None,
            });
            let result = match http::write_request(connection.reader.get_mut(), &message) {
                Err(e) => Err(Failure::Send(e)),
                Ok(()) => http::read_response_to(
                    &mut connection.reader,
                    to_head,
                    limits,
                    decoding.as_mut().map(|d| d as &mut dyn BodySink),
                )
                .map_err(Failure::Read),
            };
            connection.reader.get_mut().release();
            // The body was streamed: its codings' last bytes, or why the
            // decoding stopped it. The response's own body is then empty.
            let result = match (result, decoding) {
                (result, Some(mut decoding)) if decoding.decoder.is_some() => {
                    let stream = &mut *decoding.stream;
                    match (result, decoding.error.take()) {
                        (_, Some(error)) => return Err(error),
                        (Ok(response), None) => {
                            let finished = decoding
                                .decoder
                                .as_mut()
                                .expect("a decoder")
                                .finish(&mut |bytes| stream.body(bytes));
                            match finished {
                                Ok(()) => Ok(response),
                                Err(error) => return Err(error),
                            }
                        }
                        (Err(failure), None) => Err(failure),
                    }
                }
                (result, _) => result,
            };
            match result {
                Ok(mut response) => {
                    // Bytes already waiting would be read as the next
                    // response: such a connection is not reused.
                    lease.keep = response.persistent && connection.reader.buffer().is_empty();
                    let address_space = connection.space;
                    drop(lease);
                    if !to_head {
                        let body = std::mem::take(&mut response.body);
                        response.body =
                            content_coding::decode(&response.headers, body, limits.body_bytes)?;
                    }
                    return Ok(Received {
                        response,
                        address_space,
                    });
                }
                Err(failure)
                    if reused
                        && !fresh
                        && matches!(exchange.method, "GET" | "HEAD")
                        && failure.nothing_received()
                        && !exchange.abort.is_aborted() =>
                {
                    fresh = true;
                }
                Err(failure) => return Err(failure.into_error()),
            }
        }
    }

    /// A new connection to `url`'s host: resolved, connected to an
    /// address the initiator may reach, and over TLS for `https:`.
    fn open(
        &self,
        url: &Url,
        initiator: Option<AddressSpace>,
        abort: &AbortHandle,
    ) -> Result<Connection, NetworkError> {
        let host = url.host().expect("an http: or https: URL has a host");
        let port = url.port_or_default().expect("and a port");
        let addresses = connect::resolve_abortable(host, port, abort)?;
        let allowed = |address| {
            let target = AddressSpace::of(address);
            match initiator {
                Some(initiator) if !initiator.may_request(target) => {
                    Err(NetworkError::AddressSpace { initiator, target })
                }
                _ => Ok(()),
            }
        };
        let (tcp, address) =
            connect::connect(&addresses, allowed, self.config.connect_timeout, abort)?;
        let setup =
            |result: io::Result<()>| result.map_err(|e| NetworkError::Connect(e.to_string()));
        setup(tcp.set_nodelay(true))?;
        let mut tcp =
            stream::non_blocking(tcp).map_err(|e| NetworkError::Connect(e.to_string()))?;
        let mut waiter = Waiter::new(&mut tcp).map_err(|e| NetworkError::Connect(e.to_string()))?;
        let timeout = self.config.read_timeout;
        let socket = if url.scheme() == "https" {
            Socket::Tls(Box::new(tls::handshake(
                &self.tls,
                host,
                tcp,
                &mut waiter,
                timeout,
                abort,
            )?))
        } else {
            Socket::Plain(tcp)
        };
        Ok(Connection {
            reader: BufReader::new(Stream::new(socket, waiter, timeout)),
            space: AddressSpace::of(address.ip()),
        })
    }
}

#[cfg(test)]
mod tests;
