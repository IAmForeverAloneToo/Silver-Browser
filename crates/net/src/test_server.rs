//! A server for tests: a thread on 127.0.0.1 that answers each request
//! with what a script says, plain or over TLS, and counts connections.
//! It is built for `net`'s own tests and, behind the `test-server`
//! feature, for the engine's.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    date_time_ymd,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use crate::Url;

/// A request as the server read it.
#[derive(Debug, Clone)]
pub struct ServerRequest {
    pub method: String,
    /// The request target as sent: the path and query.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Which connection, counting from 0 in the order they came.
    pub connection: usize,
    /// Which request on that connection, counting from 0.
    pub index: usize,
    /// The name the client sent by SNI, over TLS.
    pub sni: Option<String>,
}

impl ServerRequest {
    /// The value of the first header named `name`, in any case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// What the server does with a request.
#[derive(Debug, Clone)]
pub enum Reply {
    /// Writes these bytes, and keeps the connection for another
    /// request.
    Send(Vec<u8>),
    /// Writes these bytes and closes the connection.
    SendAndClose(Vec<u8>),
    /// Waits, then does the rest.
    After(Duration, Box<Reply>),
    /// Closes the connection without a word.
    Close,
    /// Never answers, holding the connection open until the server stops.
    Hang,
    /// Over TLS, writes these bytes and the close_notify that ends the
    /// session in one flight, then holds the TCP connection open and
    /// silent until the server stops: only TLS says it is over.
    SendWithCloseNotify(Vec<u8>),
}

impl Reply {
    /// A `200 OK` with a `Content-Type`, a `Content-Length` and the body.
    pub fn ok(content_type: &str, body: &[u8]) -> Reply {
        Reply::with_fields(200, &[("Content-Type", content_type)], body)
    }

    /// A response with `status`, the fields, a `Content-Length` and the
    /// body.
    pub fn with_fields(status: u16, fields: &[(&str, &str)], body: &[u8]) -> Reply {
        let mut bytes = format!("HTTP/1.1 {status} Status\r\n").into_bytes();
        for (name, value) in fields {
            bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        bytes.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        bytes.extend_from_slice(body);
        Reply::Send(bytes)
    }
}

type Script = dyn Fn(&ServerRequest) -> Reply + Send + Sync;

struct Shared {
    script: Box<Script>,
    tls: Option<Arc<ServerConfig>>,
    connections: AtomicUsize,
    open: AtomicUsize,
    most_open: AtomicUsize,
    requests: Mutex<Vec<ServerRequest>>,
    /// The connections being served, by number, so that stopping can
    /// shut them down and end the reads waiting on them.
    sockets: Mutex<HashMap<usize, TcpStream>>,
    stop: AtomicBool,
}

/// A running server, stopped when dropped.
pub struct TestServer {
    address: SocketAddr,
    shared: Arc<Shared>,
    accepting: Option<JoinHandle<()>>,
}

impl TestServer {
    /// A plain HTTP server answering with `script`.
    pub fn plain(script: impl Fn(&ServerRequest) -> Reply + Send + Sync + 'static) -> TestServer {
        TestServer::start(None, Box::new(script))
    }

    /// An HTTPS server presenting `config`'s certificate.
    pub fn tls(
        config: Arc<ServerConfig>,
        script: impl Fn(&ServerRequest) -> Reply + Send + Sync + 'static,
    ) -> TestServer {
        TestServer::start(Some(config), Box::new(script))
    }

    fn start(tls: Option<Arc<ServerConfig>>, script: Box<Script>) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port on 127.0.0.1");
        let address = listener.local_addr().expect("its address");
        let shared = Arc::new(Shared {
            script,
            tls,
            connections: AtomicUsize::new(0),
            open: AtomicUsize::new(0),
            most_open: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            sockets: Mutex::new(HashMap::new()),
            stop: AtomicBool::new(false),
        });
        let accepting = {
            let shared = shared.clone();
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if shared.stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(stream) = stream else { continue };
                    let Ok(clone) = stream.try_clone() else {
                        continue;
                    };
                    let index = shared.connections.fetch_add(1, Ordering::SeqCst);
                    let open = shared.open.fetch_add(1, Ordering::SeqCst) + 1;
                    shared.most_open.fetch_max(open, Ordering::SeqCst);
                    lock(&shared.sockets).insert(index, clone);
                    let shared = shared.clone();
                    thread::spawn(move || {
                        let _ = serve(&shared, stream, index);
                        // The last handle on the socket goes, and closes it.
                        lock(&shared.sockets).remove(&index);
                        shared.open.fetch_sub(1, Ordering::SeqCst);
                    });
                }
            })
        };
        TestServer {
            address,
            shared,
            accepting: Some(accepting),
        }
    }

    /// The server's port.
    pub fn port(&self) -> u16 {
        self.address.port()
    }

    /// A URL on the server: `https:` over TLS, `http:` otherwise, with
    /// 127.0.0.1 as its host.
    pub fn url(&self, path: &str) -> Url {
        let scheme = if self.shared.tls.is_some() {
            "https"
        } else {
            "http"
        };
        Url::parse(&format!("{scheme}://127.0.0.1:{}{path}", self.port())).expect("a URL")
    }

    /// How many connections were accepted.
    pub fn connections(&self) -> usize {
        self.shared.connections.load(Ordering::SeqCst)
    }

    /// How many connections are open now: accepted, and not yet closed
    /// by the server.
    pub fn open(&self) -> usize {
        self.shared.open.load(Ordering::SeqCst)
    }

    /// The most connections open at one time.
    pub fn most_open(&self) -> usize {
        self.shared.most_open.load(Ordering::SeqCst)
    }

    /// Every request read so far, in the order they came.
    pub fn requests(&self) -> Vec<ServerRequest> {
        lock(&self.shared.requests).clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        for socket in lock(&self.shared.sockets).values() {
            let _ = socket.shutdown(Shutdown::Both);
        }
        // Wakes the accepting thread so that it sees the flag.
        let _ = TcpStream::connect(self.address);
        if let Some(accepting) = self.accepting.take() {
            let _ = accepting.join();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A connection's stream on the server's side.
enum ServerStream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ServerConnection, TcpStream>>),
}

impl Read for ServerStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            ServerStream::Plain(tcp) => tcp.read(buf),
            ServerStream::Tls(tls) => tls.read(buf),
        }
    }
}

impl Write for ServerStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            ServerStream::Plain(tcp) => tcp.write(buf),
            ServerStream::Tls(tls) => tls.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            ServerStream::Plain(tcp) => tcp.flush(),
            ServerStream::Tls(tls) => tls.flush(),
        }
    }
}

/// Answers the requests on one connection until it closes, the script
/// closes it, or the server stops.
fn serve(shared: &Shared, tcp: TcpStream, connection: usize) -> io::Result<()> {
    let mut stream = match &shared.tls {
        Some(config) => {
            let tls = ServerConnection::new(config.clone()).map_err(io::Error::other)?;
            ServerStream::Tls(Box::new(StreamOwned::new(tls, tcp)))
        }
        None => ServerStream::Plain(tcp),
    };
    let mut buffered = Vec::new();
    for index in 0.. {
        let Some(mut request) = read_request(shared, &mut stream, &mut buffered)? else {
            return Ok(());
        };
        request.connection = connection;
        request.index = index;
        if let ServerStream::Tls(tls) = &stream {
            request.sni = tls.conn.server_name().map(str::to_owned);
        }
        lock(&shared.requests).push(request.clone());
        let mut reply = (shared.script)(&request);
        loop {
            match reply {
                Reply::Send(bytes) => {
                    stream.write_all(&bytes)?;
                    stream.flush()?;
                    break;
                }
                Reply::SendAndClose(bytes) => {
                    stream.write_all(&bytes)?;
                    stream.flush()?;
                    if let ServerStream::Tls(tls) = &mut stream {
                        tls.conn.send_close_notify();
                        let _ = tls.flush();
                    }
                    return Ok(());
                }
                Reply::After(wait, then) => {
                    sleep_unless_stopped(shared, wait);
                    reply = *then;
                }
                Reply::Close => return Ok(()),
                Reply::Hang => {
                    hang(shared);
                    return Ok(());
                }
                Reply::SendWithCloseNotify(bytes) => {
                    match &mut stream {
                        ServerStream::Tls(tls) => {
                            tls.conn.writer().write_all(&bytes)?;
                            tls.conn.send_close_notify();
                            while tls.conn.wants_write() {
                                tls.conn.write_tls(&mut tls.sock)?;
                            }
                        }
                        ServerStream::Plain(tcp) => tcp.write_all(&bytes)?,
                    }
                    hang(shared);
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

/// Waits until the server stops.
fn hang(shared: &Shared) {
    while !shared.stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(20));
    }
}

fn sleep_unless_stopped(shared: &Shared, wait: Duration) {
    let step = Duration::from_millis(10);
    let mut slept = Duration::ZERO;
    while slept < wait && !shared.stop.load(Ordering::SeqCst) {
        thread::sleep(step);
        slept += step;
    }
}

/// The next request on the connection: a request line, fields up to an
/// empty line, and a body of its `Content-Length`. `None` when the
/// connection closes first or the server stops.
fn read_request(
    shared: &Shared,
    stream: &mut ServerStream,
    buffered: &mut Vec<u8>,
) -> io::Result<Option<ServerRequest>> {
    let head_end = loop {
        if let Some(end) = buffered.windows(4).position(|w| w == b"\r\n\r\n") {
            break end + 4;
        }
        if !fill(shared, stream, buffered)? {
            return Ok(None);
        }
    };
    let head = String::from_utf8_lossy(&buffered[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split(' ');
    let method = request_line.next().unwrap_or_default().to_owned();
    let target = request_line.next().unwrap_or_default().to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_owned(), value.trim().to_owned()))
        .collect();
    let length = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("Content-Length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    while buffered.len() < head_end + length {
        if !fill(shared, stream, buffered)? {
            return Ok(None);
        }
    }
    let body = buffered[head_end..head_end + length].to_vec();
    buffered.drain(..head_end + length);
    Ok(Some(ServerRequest {
        method,
        target,
        headers,
        body,
        connection: 0,
        index: 0,
        sni: None,
    }))
}

/// Reads more of the connection into `buffered`; false at its end or
/// when the server stops, which shuts the connection down.
fn fill(shared: &Shared, stream: &mut ServerStream, buffered: &mut Vec<u8>) -> io::Result<bool> {
    let mut chunk = [0; 4096];
    if shared.stop.load(Ordering::SeqCst) {
        return Ok(false);
    }
    match stream.read(&mut chunk)? {
        0 => Ok(false),
        n => {
            buffered.extend_from_slice(&chunk[..n]);
            Ok(true)
        }
    }
}

/// Years a certificate is valid over that cover any day a test runs.
pub const VALID: (i32, i32) = (2000, 3000);

/// A certificate authority made for one test, and certificates it
/// issues.
pub struct TestAuthority {
    params: CertificateParams,
    key: KeyPair,
    /// The root's certificate, for a client to trust.
    pub root: CertificateDer<'static>,
}

impl TestAuthority {
    pub fn new() -> TestAuthority {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(DnType::CommonName, "Silver Browser test root");
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let key = KeyPair::generate().unwrap();
        let root = params.self_signed(&key).unwrap().der().clone();
        TestAuthority { params, key, root }
    }

    /// A server configuration presenting a certificate for `names`,
    /// valid over the given years.
    pub fn server(&self, names: &[&str], years: (i32, i32)) -> Arc<ServerConfig> {
        self.server_speaking(names, years, rustls::DEFAULT_VERSIONS)
    }

    /// As [`TestAuthority::server`], speaking only `versions` of TLS.
    pub fn server_speaking(
        &self,
        names: &[&str],
        years: (i32, i32),
        versions: &[&'static rustls::SupportedProtocolVersion],
    ) -> Arc<ServerConfig> {
        let mut params =
            CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>())
                .unwrap();
        params.not_before = date_time_ymd(years.0, 1, 1);
        params.not_after = date_time_ymd(years.1, 1, 1);
        let key = KeyPair::generate().unwrap();
        let issuer = Issuer::from_params(&self.params, &self.key);
        let leaf = params.signed_by(&key, &issuer).unwrap().der().clone();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(versions)
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![leaf], key)
            .unwrap();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Arc::new(config)
    }
}

impl Default for TestAuthority {
    fn default() -> TestAuthority {
        TestAuthority::new()
    }
}
