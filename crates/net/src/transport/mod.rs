//! Connections for `http:` and `https:` (docs/design/net.md,
//! "Connections"): names resolved, addresses tried, address spaces
//! checked, TLS, the pool, the one retry, content codings, limits,
//! timeouts and aborts. [`Transport::exchange`] sends one request and
//! returns its response; redirects and the rest of Fetch are above it.

mod connect;
mod pool;
pub(crate) mod stream;
mod tls;

use std::collections::HashMap;
use std::io::{self, BufReader};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::ClientConfig;
use rustls::pki_types::CertificateDer;

use crate::address_space::{AddressSpace, LocalNetworkAccess, LocalNetworkGate};
use crate::content_coding;
use crate::error::{AbortHandle, NetworkError};
use crate::h2::conn::{self as h2, Connection as H2Connection, Event, StreamError, Unopened};
use crate::headers::HeaderList;
use crate::http::{self, BodySink, Limits, ReadError, RequestMessage, Response};
use crate::{Host, Url};

use pool::{Connection, OriginKey, Pool, Speaks};
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
    /// Names resolved without the resolver: only tests give one.
    pub(crate) hosts: Vec<(String, IpAddr)>,
    /// Address spaces given to addresses in place of their own: only
    /// tests give one.
    pub(crate) spaces: Vec<(SocketAddr, AddressSpace)>,
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
            hosts: Vec::new(),
            spaces: Vec::new(),
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

    /// Resolves the domain `host` to `address` without asking the
    /// resolver (M33.3): a test's names for its servers on this machine,
    /// as the reference browser's resolver rules give them. There is no
    /// such option outside tests.
    #[cfg(any(test, feature = "test-server"))]
    pub fn resolve(mut self, host: &str, address: IpAddr) -> Config {
        self.hosts.push((host.to_ascii_lowercase(), address));
        self
    }

    /// Puts the server at `address` in `space` (M33.4): a test's server on
    /// this machine as a public or a local one, as the reference
    /// browser's address space overrides do. There is no such option
    /// outside tests.
    #[cfg(any(test, feature = "test-server"))]
    pub fn treat_as(mut self, address: SocketAddr, space: AddressSpace) -> Config {
        self.spaces.push((address, space));
        self
    }

    /// The space of the server at `address`.
    fn space_of(&self, address: SocketAddr) -> AddressSpace {
        self.spaces
            .iter()
            .find(|(given, _)| *given == address)
            .map_or_else(|| AddressSpace::of(address.ip()), |&(_, space)| space)
    }
}

/// One request to send.
#[derive(Debug, Clone, Copy)]
pub struct Exchange<'a> {
    /// The partition its request is made in (M33.2), whose connections
    /// and TLS sessions are its own; none for a top-level document's.
    pub partition: Option<&'a str>,
    pub method: &'a str,
    /// An `http:` or `https:` URL.
    pub url: &'a Url,
    pub headers: &'a HeaderList,
    pub body: Option<&'a [u8]>,
    /// The address space of the page asking, or `None` for a load the
    /// user asked for, which may go anywhere.
    pub initiator: Option<AddressSpace>,
    /// What it asks the user with, when its page may ask its way into a
    /// more private space (M33.4); none refuses at once.
    pub ask: Option<Ask<'a>>,
    /// The space its server is to be in, when the request named one
    /// (M33.4, `targetAddressSpace`).
    pub target: Option<AddressSpace>,
    pub abort: &'a AbortHandle,
    /// The most the body may hold, when less than the configured limit.
    pub body_limit: Option<usize>,
}

/// What a request asks the user with for its way into a more private
/// address space (M33.4): the gate, and what its page may ask for.
#[derive(Debug, Clone, Copy)]
pub struct Ask<'a> {
    pub gate: &'a dyn LocalNetworkGate,
    pub access: &'a LocalNetworkAccess,
}

/// Whether a request from a page in `initiator`'s space may reach an
/// address in `target`: by D19's rule, or by the user's answer when its
/// page may ask for that space (M33.4); a load the user asked for goes
/// anywhere.
pub(crate) fn reachable(
    initiator: Option<AddressSpace>,
    target: AddressSpace,
    ask: Option<Ask<'_>>,
    abort: &AbortHandle,
) -> Result<(), NetworkError> {
    let Some(initiator) = initiator else {
        return Ok(());
    };
    if initiator.may_request(target) {
        return Ok(());
    }
    match ask {
        Some(ask)
            if ask.access.may_ask(target) && ask.gate.allow(&ask.access.origin, target, abort) =>
        {
            Ok(())
        }
        _ => Err(NetworkError::AddressSpace { initiator, target }),
    }
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

/// A new connection: one speaking HTTP/1.1, or HTTP/2 (M38).
enum Opened {
    H1(Connection),
    H2(Arc<H2Connection>),
}

/// How an HTTP/2 exchange failed.
enum H2Failure {
    /// The server did not process it: it may go again.
    Retry,
    Error(NetworkError),
}

/// How long a fetch on an HTTP/2 stream waits for its next event
/// before it looks at its abort.
const H2_POLL: Duration = Duration::from_millis(50);

/// The pool of connections and the TLS configuration every fetch
/// shares.
pub struct Transport {
    config: Config,
    tls: Arc<ClientConfig>,
    /// Each partition's TLS configuration (M33.2), with a session cache
    /// of its own, so that a resumed session tells no top-level site
    /// what another visited; at most [`MAX_TLS_PARTITIONS`].
    partitioned: Mutex<HashMap<String, Arc<ClientConfig>>>,
    pool: Pool,
}

/// The most partitions whose TLS sessions are kept apart at once: past
/// them, all are forgotten.
pub const MAX_TLS_PARTITIONS: usize = 64;

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
            partitioned: Mutex::new(HashMap::new()),
            pool: Pool::new(config.max_per_origin, config.idle_timeout),
            config,
        }
    }

    /// The TLS configuration of `partition`'s connections: the shared
    /// one for none, else one like it with a session cache of its own.
    fn tls_for(&self, partition: Option<&str>) -> Arc<ClientConfig> {
        let Some(partition) = partition else {
            return self.tls.clone();
        };
        let mut configs = self
            .partitioned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !configs.contains_key(partition) && configs.len() >= MAX_TLS_PARTITIONS {
            configs.clear();
        }
        configs
            .entry(partition.to_owned())
            .or_insert_with(|| {
                let mut config = (*self.tls).clone();
                config.resumption = rustls::client::Resumption::in_memory_sessions(256);
                Arc::new(config)
            })
            .clone()
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
            partition: exchange.partition.map(str::to_owned),
        };
        let message = RequestMessage {
            method: exchange.method,
            url,
            headers: exchange.headers,
            body: exchange.body,
        };
        let to_head = exchange.method == "HEAD";
        let mut fresh = false;
        // How many times the request went again on HTTP/2 (M38).
        let mut retries = 0;
        loop {
            // An `https:` origin's HTTP/2 connection, or the first
            // connection to find out whether it speaks it.
            let mut probe = None;
            if url.scheme() == "https" {
                match self.pool.speaks(&key, exchange.abort)? {
                    Speaks::H2(connection) => {
                        match self.exchange_h2(&connection, exchange, stream.as_deref_mut(), limits)
                        {
                            Err(H2Failure::Retry) if retries < 2 => {
                                retries += 1;
                                continue;
                            }
                            Err(H2Failure::Retry) => {
                                return Err(NetworkError::Http2(
                                    "the server did not process the request".to_owned(),
                                ));
                            }
                            Err(H2Failure::Error(error)) => return Err(error),
                            Ok(received) => return Ok(received),
                        }
                    }
                    Speaks::Probe(first) => probe = Some(first),
                    Speaks::H1 => {}
                }
            }
            let mut lease = self.pool.checkout(&key, fresh, exchange.abort)?;
            let reused = lease.connection.is_some();
            // An idle HTTP/1.1 connection of an origin being probed: the
            // origin chose HTTP/1.1 before.
            if reused && probe.is_some() {
                self.pool.speaks_h1(&key);
            }
            if !reused {
                match self.open(exchange)? {
                    Opened::H1(connection) => {
                        if url.scheme() == "https" {
                            self.pool.speaks_h1(&key);
                        }
                        lease.connection = Some(connection);
                    }
                    Opened::H2(connection) => {
                        // Its place among HTTP/1.1's is given up.
                        drop(lease);
                        self.pool.keep_h2(&key, connection.clone());
                        drop(probe);
                        match self.exchange_h2(&connection, exchange, stream.as_deref_mut(), limits)
                        {
                            Err(H2Failure::Retry) if retries < 2 => {
                                retries += 1;
                                continue;
                            }
                            Err(H2Failure::Retry) => {
                                return Err(NetworkError::Http2(
                                    "the server did not process the request".to_owned(),
                                ));
                            }
                            Err(H2Failure::Error(error)) => return Err(error),
                            Ok(received) => return Ok(received),
                        }
                    }
                }
            }
            drop(probe);
            let connection = lease.connection.as_mut().expect("a connection");
            let space = connection.space;
            let named = exchange.target.filter(|&named| named != space);
            let refused = match named {
                Some(named) => Err(NetworkError::TargetAddressSpace {
                    named,
                    reached: space,
                }),
                // A new connection's address was checked as it was made.
                None if reused => {
                    reachable(exchange.initiator, space, exchange.ask, exchange.abort)
                }
                None => Ok(()),
            };
            if let Err(error) = refused {
                // The connection itself is fine for another request.
                lease.keep = true;
                return Err(error);
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

    /// Sends `exchange`'s request on a stream of the HTTP/2 connection
    /// `connection` and reads its response, as an HTTP/1.1 exchange
    /// does (M38).
    fn exchange_h2<'s>(
        &self,
        connection: &Arc<H2Connection>,
        exchange: &Exchange,
        mut stream: Option<&mut (dyn ResponseStream + 's)>,
        limits: Limits,
    ) -> Result<Received, H2Failure> {
        let fail = |error| Err(H2Failure::Error(error));
        let space = connection.space;
        // The connection's address, checked as a reused one's is.
        if let Some(named) = exchange.target.filter(|&named| named != space) {
            return fail(NetworkError::TargetAddressSpace {
                named,
                reached: space,
            });
        }
        if let Err(error) = reachable(exchange.initiator, space, exchange.ask, exchange.abort) {
            return fail(error);
        }
        let request = match h2_request(exchange) {
            Ok(request) => request,
            Err(error) => return fail(error),
        };
        let mut handle = match connection.open(request, exchange.abort) {
            Ok(handle) => handle,
            Err(Unopened::Unusable) => return Err(H2Failure::Retry),
            Err(Unopened::Aborted) => return fail(NetworkError::Aborted),
        };
        let to_head = exchange.method == "HEAD";
        let mut head: Option<(u16, HeaderList)> = None;
        let mut decoder: Option<content_coding::Decoder> = None;
        let mut body = Vec::new();
        let mut received = 0usize;
        loop {
            let Some(event) = handle.next(H2_POLL) else {
                if exchange.abort.is_aborted() {
                    return fail(NetworkError::Aborted);
                }
                continue;
            };
            match event {
                Event::Head { status, fields } => {
                    let mut headers = HeaderList::new();
                    for (name, value) in &fields {
                        headers.append(name, value);
                    }
                    if let Some(stream) = stream.as_deref_mut()
                        && stream.head(status, b"", &headers, space)
                    {
                        decoder = Some(content_coding::Decoder::new(&headers, limits.body_bytes));
                    }
                    head = Some((status, headers));
                }
                Event::Data(bytes) => {
                    received += bytes.len();
                    if received > limits.body_bytes {
                        return fail(NetworkError::BodyTooLarge);
                    }
                    match (&mut decoder, stream.as_deref_mut()) {
                        (Some(decoder), Some(stream)) => {
                            if let Err(error) = decoder.push(&bytes, &mut |d| stream.body(d)) {
                                return fail(error);
                            }
                        }
                        _ => body.extend_from_slice(&bytes),
                    }
                    handle.consumed(bytes.len());
                }
                Event::End => break,
                Event::Retry => return Err(H2Failure::Retry),
                Event::Failed(error) => {
                    return fail(match error {
                        StreamError::Reset(code) => {
                            NetworkError::Http2(format!("the stream was reset: {}", code.name()))
                        }
                        StreamError::Connection(code, why) => NetworkError::Http2(format!(
                            "the connection ended: {} ({why})",
                            code.name()
                        )),
                        StreamError::Malformed(why) => NetworkError::Http2(why.to_owned()),
                        StreamError::HeadersTooLarge => NetworkError::HeadersTooLarge,
                        StreamError::Io(why) => NetworkError::Io(why),
                        StreamError::Timeout => NetworkError::Timeout,
                    });
                }
            }
        }
        let Some((status, headers)) = head else {
            return fail(NetworkError::Http2("no response".to_owned()));
        };
        if let (Some(decoder), Some(stream)) = (&mut decoder, stream)
            && let Err(error) = decoder.finish(&mut |bytes| stream.body(bytes))
        {
            return fail(error);
        }
        if !to_head && decoder.is_none() {
            body = match content_coding::decode(&headers, body, limits.body_bytes) {
                Ok(body) => body,
                Err(error) => return fail(error),
            };
        }
        Ok(Received {
            response: Response {
                version: http::Version::Http2,
                status,
                reason: Vec::new(),
                headers,
                body,
                persistent: true,
            },
            address_space: space,
        })
    }

    /// A new connection to the exchange's URL's host: resolved,
    /// connected to an address its page may reach, or that the user lets
    /// it (M33.4), and over TLS for `https:`; speaking HTTP/2 when its
    /// TLS chose it (M38).
    fn open(&self, exchange: &Exchange) -> Result<Opened, NetworkError> {
        let (url, partition, abort) = (exchange.url, exchange.partition, exchange.abort);
        let host = url.host().expect("an http: or https: URL has a host");
        let port = url.port_or_default().expect("and a port");
        let given = match host {
            Host::Domain(name) => self
                .config
                .hosts
                .iter()
                .find(|(known, _)| known == name)
                .map(|&(_, address)| vec![SocketAddr::new(address, port)]),
            _ => None,
        };
        let addresses = match given {
            Some(addresses) => addresses,
            None => connect::resolve_abortable(host, port, abort)?,
        };
        let allowed = |address| {
            let target = self.config.space_of(address);
            reachable(exchange.initiator, target, exchange.ask, abort)
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
        let space = self.config.space_of(address);
        let socket = if url.scheme() == "https" {
            let tls = tls::handshake(
                &self.tls_for(partition),
                host,
                tcp,
                &mut waiter,
                timeout,
                abort,
            )?;
            if tls.conn.alpn_protocol() == Some(b"h2") {
                let settings = h2::Settings {
                    read_timeout: timeout,
                    idle_timeout: self.config.idle_timeout,
                };
                return H2Connection::start(tls, waiter, space, settings)
                    .map(Opened::H2)
                    .map_err(|e| NetworkError::Connect(e.to_string()));
            }
            Socket::Tls(Box::new(tls))
        } else {
            Socket::Plain(tcp)
        };
        Ok(Opened::H1(Connection {
            reader: BufReader::new(Stream::new(socket, waiter, timeout)),
            space,
        }))
    }
}

/// An exchange's request as an HTTP/2 stream's (8.3.1): its
/// pseudo-fields, then its fields lowercased, those HTTP/2 forbids
/// left out, and its body's length; refused as an HTTP/1.1 request is
/// for a method or field that could split the message.
fn h2_request(exchange: &Exchange) -> Result<h2::Request, NetworkError> {
    let invalid = |what: &str| Err(NetworkError::Io(format!("an invalid request: {what}")));
    let url = exchange.url;
    let Some(host) = url.host() else {
        return invalid("no host");
    };
    if !crate::headers::is_header_name(exchange.method.as_bytes()) {
        return invalid("the method is not a token");
    }
    let mut authority = host.to_string();
    if let Some(port) = url.port() {
        authority.push_str(&format!(":{port}"));
    }
    let mut path = url.path_str().to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    let mut fields: Vec<(Vec<u8>, Vec<u8>)> = vec![
        (b":method".to_vec(), exchange.method.as_bytes().to_vec()),
        (b":scheme".to_vec(), url.scheme().as_bytes().to_vec()),
        (b":authority".to_vec(), authority.into_bytes()),
        (b":path".to_vec(), path.into_bytes()),
    ];
    for (name, value) in exchange.headers.iter() {
        if !crate::headers::is_header_name(name) {
            return invalid("a field name is not a token");
        }
        if value.iter().any(|&b| matches!(b, b'\0' | b'\r' | b'\n')) {
            return invalid("a field value holds NUL, CR or LF");
        }
        let name = name.to_ascii_lowercase();
        if name == b"host"
            || name == b"content-length"
            || crate::h2::session::is_connection_specific(&name, value)
        {
            continue;
        }
        fields.push((name, value.to_vec()));
    }
    if let Some(body) = exchange.body {
        fields.push((
            b"content-length".to_vec(),
            body.len().to_string().into_bytes(),
        ));
    }
    Ok(h2::Request {
        fields,
        body: exchange.body.map(<[u8]>::to_vec),
        idempotent: matches!(exchange.method, "GET" | "HEAD"),
    })
}

impl Transport {
    /// Whether the pool keeps an HTTP/2 connection to `url`'s origin,
    /// for the tests.
    #[cfg(test)]
    pub(crate) fn has_h2(&self, url: &Url) -> bool {
        let key = OriginKey {
            scheme: url.scheme().to_owned(),
            host: url.host().map(|host| host.to_string()).unwrap_or_default(),
            port: url.port_or_default().unwrap_or(0),
            partition: None,
        };
        matches!(
            self.pool.speaks(&key, &AbortHandle::new()),
            Ok(Speaks::H2(_))
        )
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod h2_tests;
