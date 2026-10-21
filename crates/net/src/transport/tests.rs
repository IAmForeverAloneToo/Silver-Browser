//! The transport against the test server: reuse and its limits, the
//! retry, time limits and aborts, content codings, address spaces, and
//! TLS with certificates made for the test.

use std::io::Write;
use std::net::SocketAddr;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use flate2::Compression;
use flate2::write::GzEncoder;

use rustls::ServerConfig;

use super::*;
use crate::test_server::{Reply, ServerRequest, TestAuthority, TestServer};
use crate::{Origin, TlsFailure};

fn transport() -> Transport {
    Transport::new(Config::default())
}

fn send(
    transport: &Transport,
    method: &str,
    url: &Url,
    initiator: Option<AddressSpace>,
    abort: &AbortHandle,
) -> Result<Received, NetworkError> {
    send_in(transport, method, url, initiator, None, abort)
}

/// [`send`] in `partition` (M33.2).
fn send_in(
    transport: &Transport,
    method: &str,
    url: &Url,
    initiator: Option<AddressSpace>,
    partition: Option<&str>,
    abort: &AbortHandle,
) -> Result<Received, NetworkError> {
    let headers = HeaderList::new();
    transport.exchange(&Exchange {
        partition,
        method,
        url,
        headers: &headers,
        body: (method == "POST").then_some(&b"x"[..]),
        initiator,
        ask: None,
        target: None,
        abort,
        body_limit: None,
        priority: None,
    })
}

fn get(transport: &Transport, url: &Url) -> Result<Received, NetworkError> {
    send(transport, "GET", url, None, &AbortHandle::new())
}

fn hello() -> Reply {
    Reply::ok("text/plain", b"hello")
}

#[test]
fn a_plain_exchange() {
    let server = TestServer::plain(|_| hello());
    let received = get(&transport(), &server.url("/path?q=1#frag")).unwrap();
    assert_eq!(received.response.status, 200);
    assert_eq!(received.response.body, b"hello");
    assert_eq!(received.address_space, AddressSpace::Loopback);
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].target, "/path?q=1");
    let host = format!("127.0.0.1:{}", server.port());
    assert_eq!(requests[0].header("Host"), Some(host.as_str()));
}

#[test]
fn connections_are_reused() {
    let server = TestServer::plain(|_| hello());
    let transport = transport();
    for _ in 0..3 {
        assert_eq!(
            get(&transport, &server.url("/")).unwrap().response.body,
            b"hello"
        );
    }
    assert_eq!(server.connections(), 1);
    let requests = server.requests();
    assert_eq!(
        requests
            .iter()
            .map(|r| (r.connection, r.index))
            .collect::<Vec<_>>(),
        [(0, 0), (0, 1), (0, 2)]
    );
}

#[test]
fn what_is_not_reused() {
    // Connection: close.
    let server =
        TestServer::plain(
            |_| match Reply::with_fields(200, &[("Connection", "close")], b"x") {
                Reply::Send(bytes) => Reply::SendAndClose(bytes),
                other => other,
            },
        );
    let transport = transport();
    get(&transport, &server.url("/")).unwrap();
    get(&transport, &server.url("/")).unwrap();
    assert_eq!(server.connections(), 2);
    // A body read to the connection's end.
    let server =
        TestServer::plain(|_| Reply::SendAndClose(b"HTTP/1.1 200 OK\r\n\r\nto the end".to_vec()));
    assert_eq!(
        get(&transport, &server.url("/")).unwrap().response.body,
        b"to the end"
    );
    get(&transport, &server.url("/")).unwrap();
    assert_eq!(server.connections(), 2);
    // Bytes already waiting after the response.
    let server = TestServer::plain(|_| {
        Reply::Send(
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nxHTTP/1.1 200 OK\r\n\r\n".to_vec(),
        )
    });
    get(&transport, &server.url("/")).unwrap();
    get(&transport, &server.url("/")).unwrap();
    assert_eq!(server.connections(), 2);
}

#[test]
fn what_says_it_ends_is_not_reused_though_left_open() {
    for response in [
        &b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 1\r\n\r\nx"[..],
        b"HTTP/1.0 200 OK\r\nContent-Length: 1\r\n\r\nx",
    ] {
        let server = TestServer::plain(move |_| Reply::Send(response.to_vec()));
        let transport = transport();
        get(&transport, &server.url("/")).unwrap();
        get(&transport, &server.url("/")).unwrap();
        assert_eq!(
            server.connections(),
            2,
            "{}",
            String::from_utf8_lossy(response)
        );
    }
}

#[test]
fn at_most_six_connections_to_an_origin() {
    let server = TestServer::plain(|_| Reply::After(Duration::from_millis(300), Box::new(hello())));
    let transport = Arc::new(transport());
    let url = server.url("/");
    let start = Arc::new(Barrier::new(8));
    let fetches: Vec<_> = (0..8)
        .map(|_| {
            let (transport, url, start) = (transport.clone(), url.clone(), start.clone());
            thread::spawn(move || {
                start.wait();
                get(&transport, &url).map(|r| r.response.body)
            })
        })
        .collect();
    for fetch in fetches {
        assert_eq!(fetch.join().unwrap().unwrap(), b"hello");
    }
    assert_eq!(server.most_open(), 6);
    assert_eq!(server.connections(), 6);
    assert_eq!(server.requests().len(), 8);
}

/// Waits until the server has closed every connection, and a little
/// longer, for the close to reach the client's socket.
fn wait_until_closed(server: &TestServer) {
    let started = Instant::now();
    while server.open() > 0 {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the server kept a connection"
        );
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(100));
}

#[test]
fn a_connection_the_server_closed_while_idle_is_not_taken() {
    // Each connection answers once, without saying it will close, and
    // closes.
    let server = TestServer::plain(|_| match hello() {
        Reply::Send(bytes) => Reply::SendAndClose(bytes),
        other => other,
    });
    let transport = transport();
    // A POST, which would not be sent again, as well.
    for method in ["GET", "POST", "POST"] {
        let sent = send(
            &transport,
            method,
            &server.url("/"),
            None,
            &AbortHandle::new(),
        );
        assert_eq!(sent.unwrap().response.body, b"hello");
        wait_until_closed(&server);
    }
    let requests = server.requests();
    assert_eq!(
        requests
            .iter()
            .map(|r| (r.connection, r.index))
            .collect::<Vec<_>>(),
        [(0, 0), (1, 0), (2, 0)]
    );
}

#[test]
fn a_get_the_server_dropped_unanswered_is_sent_again() {
    // Each connection answers one request, then closes on the next
    // without a word, as a server ending an idle connection just as a
    // request comes would.
    let server = TestServer::plain(|request| match request.index {
        0 => hello(),
        _ => Reply::Close,
    });
    let transport = transport();
    for _ in 0..3 {
        assert_eq!(
            get(&transport, &server.url("/")).unwrap().response.body,
            b"hello"
        );
    }
    let sent = |requests: Vec<ServerRequest>| {
        requests
            .into_iter()
            .map(|r| (r.method, r.connection, r.index))
            .collect::<Vec<_>>()
    };
    let get = |connection, index| ("GET".to_owned(), connection, index);
    assert_eq!(
        sent(server.requests()),
        [get(0, 0), get(0, 1), get(1, 0), get(1, 1), get(2, 0)]
    );
    // A POST is not sent again: it fails, and the connection is not
    // kept.
    let posted = send(
        &transport,
        "POST",
        &server.url("/"),
        None,
        &AbortHandle::new(),
    );
    assert!(matches!(posted, Err(NetworkError::Io(_))), "{posted:?}");
    let posted = send(
        &transport,
        "POST",
        &server.url("/"),
        None,
        &AbortHandle::new(),
    );
    assert_eq!(posted.unwrap().response.body, b"hello");
    let post = |connection, index| ("POST".to_owned(), connection, index);
    assert_eq!(sent(server.requests())[5..], [post(2, 1), post(3, 0)]);
}

#[test]
fn the_retry_is_on_a_new_connection() {
    // Two connections answer once each, slowly enough to be open
    // together, and then close on their next request.
    let server = TestServer::plain(|request| match request.index {
        0 => Reply::After(Duration::from_millis(200), Box::new(hello())),
        _ => Reply::Close,
    });
    let transport = Arc::new(transport());
    let url = server.url("/");
    let start = Arc::new(Barrier::new(2));
    let fetches: Vec<_> = (0..2)
        .map(|_| {
            let (transport, url, start) = (transport.clone(), url.clone(), start.clone());
            thread::spawn(move || {
                start.wait();
                get(&transport, &url).map(|r| r.response.body)
            })
        })
        .collect();
    for fetch in fetches {
        assert_eq!(fetch.join().unwrap().unwrap(), b"hello");
    }
    assert_eq!(server.connections(), 2);
    // One idle connection fails; the retry opens a third rather than
    // take the other, which would fail as well.
    assert_eq!(get(&transport, &url).unwrap().response.body, b"hello");
    assert_eq!(server.connections(), 3);
}

#[test]
fn a_get_answered_in_part_is_not_sent_again() {
    let server = TestServer::plain(|request| match request.index {
        0 => hello(),
        _ => Reply::SendAndClose(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort".to_vec()),
    });
    let transport = transport();
    get(&transport, &server.url("/")).unwrap();
    assert_eq!(
        get(&transport, &server.url("/")).unwrap_err(),
        NetworkError::Truncated
    );
    assert_eq!(server.connections(), 1);
}

#[test]
fn idle_connections_are_closed_in_time() {
    let server = TestServer::plain(|_| hello());
    let transport = Transport::new(Config {
        idle_timeout: Duration::from_millis(200),
        ..Config::default()
    });
    let url = server.url("/");
    get(&transport, &url).unwrap();
    let key = OriginKey {
        scheme: "http".into(),
        host: "127.0.0.1".into(),
        port: server.port(),
        partition: None,
    };
    assert_eq!(transport.pool.counts(&key), (1, 1));
    thread::sleep(Duration::from_millis(600));
    assert_eq!(transport.pool.counts(&key), (0, 0));
    get(&transport, &url).unwrap();
    assert_eq!(server.connections(), 2);
}

#[test]
fn a_silent_server_times_out() {
    let server = TestServer::plain(|_| Reply::Hang);
    let transport = Transport::new(Config {
        read_timeout: Duration::from_millis(500),
        ..Config::default()
    });
    let started = Instant::now();
    assert_eq!(
        get(&transport, &server.url("/")).unwrap_err(),
        NetworkError::Timeout
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn an_abort_ends_the_exchange() {
    let server = TestServer::plain(|_| Reply::Hang);
    let transport = transport();
    let abort = AbortHandle::new();
    let aborter = abort.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        aborter.abort();
    });
    let started = Instant::now();
    let result = send(&transport, "GET", &server.url("/"), None, &abort);
    assert_eq!(result.unwrap_err(), NetworkError::Aborted);
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn an_abort_after_the_response_spares_the_connection() {
    let server = TestServer::plain(|_| hello());
    let transport = transport();
    let abort = AbortHandle::new();
    send(&transport, "GET", &server.url("/"), None, &abort).unwrap();
    // The page that asked goes away, its fetch long done.
    abort.abort();
    get(&transport, &server.url("/")).unwrap();
    assert_eq!(server.connections(), 1);
}

#[test]
fn a_server_that_takes_nothing_times_the_request_out() {
    // A listener that the kernel connects to, and nobody reads from.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
    let transport = Transport::new(Config {
        read_timeout: Duration::from_millis(500),
        ..Config::default()
    });
    // More than the socket buffers on both sides hold.
    let body = vec![0; 32 << 20];
    let headers = HeaderList::new();
    let started = Instant::now();
    let result = transport.exchange(&Exchange {
        partition: None,
        method: "POST",
        url: &url,
        headers: &headers,
        body: Some(&body),
        initiator: None,
        ask: None,
        target: None,
        abort: &AbortHandle::new(),
        body_limit: None,
        priority: None,
    });
    assert_eq!(result.unwrap_err(), NetworkError::Timeout);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn content_codings_are_removed() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(b"hello, decoded").unwrap();
    let gzipped = encoder.finish().unwrap();
    let server = TestServer::plain(move |_| {
        Reply::with_fields(200, &[("Content-Encoding", "gzip")], &gzipped)
    });
    let transport = transport();
    let received = get(&transport, &server.url("/")).unwrap();
    assert_eq!(received.response.body, b"hello, decoded");
    assert_eq!(
        received.response.headers.get(b"content-encoding").unwrap(),
        b"gzip"
    );
    // A HEAD's empty body is not decoded.
    let head = send(
        &transport,
        "HEAD",
        &server.url("/"),
        None,
        &AbortHandle::new(),
    )
    .unwrap();
    assert_eq!(head.response.body, b"");
    let broken = TestServer::plain(|_| {
        Reply::with_fields(200, &[("Content-Encoding", "gzip")], b"not gzip")
    });
    assert_eq!(
        get(&transport, &broken.url("/")).unwrap_err(),
        NetworkError::ContentCoding
    );
}

#[test]
fn responses_keep_to_their_limits() {
    let server = TestServer::plain(|_| hello());
    let transport = Transport::new(Config {
        limits: Limits {
            header_bytes: 1024,
            body_bytes: 4,
        },
        ..Config::default()
    });
    assert_eq!(
        get(&transport, &server.url("/")).unwrap_err(),
        NetworkError::BodyTooLarge
    );
}

#[test]
fn address_spaces() {
    let server = TestServer::plain(|_| hello());
    let transport = transport();
    let url = server.url("/");
    let send_from = |initiator| send(&transport, "GET", &url, initiator, &AbortHandle::new());
    for initiator in [AddressSpace::Public, AddressSpace::Local] {
        assert_eq!(
            send_from(Some(initiator)).unwrap_err(),
            NetworkError::AddressSpace {
                initiator,
                target: AddressSpace::Loopback,
            }
        );
    }
    assert_eq!(server.connections(), 0);
    assert!(send_from(Some(AddressSpace::Loopback)).is_ok());
    assert!(send_from(None).is_ok());
    // A connection a load the user asked for opened is not one a public
    // page may use; it stays for the next request that may.
    assert!(matches!(
        send_from(Some(AddressSpace::Public)),
        Err(NetworkError::AddressSpace { .. })
    ));
    assert!(send_from(None).is_ok());
    assert_eq!(server.connections(), 1);
}

/// A gate that answers `allow` after `wait`, counting what it was asked.
#[derive(Debug)]
struct Answering {
    allow: bool,
    wait: Duration,
    asked: Mutex<Vec<(String, AddressSpace)>>,
}

impl LocalNetworkGate for Answering {
    fn allow(&self, origin: &Origin, target: AddressSpace, _: &AbortHandle) -> bool {
        std::thread::sleep(self.wait);
        let asked = (origin.serialization(), target);
        self.asked.lock().unwrap().push(asked);
        self.allow
    }
}

#[test]
fn a_page_asks_the_user_for_a_more_private_space() {
    let server = TestServer::plain(|_| hello());
    let url = server.url("/");
    let address = SocketAddr::from(([127, 0, 0, 1], server.port()));
    let access = LocalNetworkAccess {
        origin: Url::parse("https://page.test/").unwrap().origin(),
        local: true,
        loopback: true,
    };
    let send_asking = |transport: &Transport, gate: &Answering, access, target| {
        let headers = HeaderList::new();
        transport.exchange(&Exchange {
            partition: None,
            method: "GET",
            url: &url,
            headers: &headers,
            body: None,
            initiator: Some(AddressSpace::Public),
            ask: Some(Ask { gate, access }),
            target,
            abort: &AbortHandle::new(),
            body_limit: None,
            priority: None,
        })
    };
    let gate = |allow, wait| Answering {
        allow,
        wait,
        asked: Mutex::new(Vec::new()),
    };
    // Let: through, after a wait longer than the connection may take.
    let transport = Transport::new(Config {
        connect_timeout: Duration::from_millis(200),
        ..Config::default()
    });
    let yes = gate(true, Duration::from_millis(400));
    assert!(send_asking(&transport, &yes, &access, None).is_ok());
    assert_eq!(
        *yes.asked.lock().unwrap(),
        [("https://page.test".to_owned(), AddressSpace::Loopback)]
    );
    // Refused: D19's error.
    let no = gate(false, Duration::ZERO);
    assert_eq!(
        send_asking(&transport, &no, &access, None).unwrap_err(),
        NetworkError::AddressSpace {
            initiator: AddressSpace::Public,
            target: AddressSpace::Loopback,
        }
    );
    // A policy that does not let it ask is refused without asking.
    let unasked = gate(true, Duration::ZERO);
    let local_only = LocalNetworkAccess {
        loopback: false,
        ..access.clone()
    };
    assert!(send_asking(&transport, &unasked, &local_only, None).is_err());
    assert!(unasked.asked.lock().unwrap().is_empty());
    // A server in another space than the one named.
    assert_eq!(
        send_asking(&transport, &yes, &access, Some(AddressSpace::Local)).unwrap_err(),
        NetworkError::TargetAddressSpace {
            named: AddressSpace::Local,
            reached: AddressSpace::Loopback,
        }
    );
    // A test's server put in the local network: asked for that.
    let transport = Transport::new(Config::default().treat_as(address, AddressSpace::Local));
    let yes = gate(true, Duration::ZERO);
    assert!(send_asking(&transport, &yes, &access, Some(AddressSpace::Local)).is_ok());
    assert_eq!(
        *yes.asked.lock().unwrap(),
        [("https://page.test".to_owned(), AddressSpace::Local)]
    );
    let from_local = send(
        &transport,
        "GET",
        &url,
        Some(AddressSpace::Local),
        &AbortHandle::new(),
    );
    assert!(from_local.is_ok(), "a local page reaches it");
}

#[test]
fn names_and_ports_that_fail() {
    let transport = transport();
    let unresolved = get(&transport, &Url::parse("http://nowhere.invalid/").unwrap());
    assert!(
        matches!(unresolved, Err(NetworkError::Resolve(_))),
        "{unresolved:?}"
    );
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let refused = get(
        &transport,
        &Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
    );
    assert!(
        matches!(refused, Err(NetworkError::Connect(_))),
        "{refused:?}"
    );
}

#[test]
fn the_tls_configuration() {
    let config = tls::client_config(&[]);
    assert_eq!(
        config.alpn_protocols,
        [b"h2".to_vec(), b"http/1.1".to_vec()]
    );
    assert!(!config.enable_early_data);
}

use crate::test_server::VALID;

#[test]
fn both_versions_of_tls_are_spoken() {
    let authority = TestAuthority::new();
    let names = ["127.0.0.1"];
    for version in [&rustls::version::TLS13, &rustls::version::TLS12] {
        let config = authority.server_speaking(&names, VALID, &[version]);
        let server = TestServer::tls(config, |_| hello());
        let transport = Transport::new(Config::default().trust(authority.root.clone()));
        let received = get(&transport, &server.url("/"));
        assert_eq!(received.unwrap().response.body, b"hello", "{version:?}");
    }
}

/// `der` as a PEM file's certificate section.
fn pem(der: &[u8]) -> String {
    const DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in der.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            text.push(if i <= chunk.len() {
                DIGITS[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    let lines: Vec<&str> = text
        .as_bytes()
        .chunks(64)
        .map(|line| std::str::from_utf8(line).unwrap())
        .collect();
    format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
        lines.join("\n")
    )
}

#[test]
fn roots_from_a_pem_file_are_trusted_in_tests() {
    let authority = TestAuthority::new();
    let server = TestServer::tls(authority.server(&["127.0.0.1"], VALID), |_| hello());
    let file = format!("a comment\n{}", pem(&authority.root));
    let config = Config::default().trust_pem(file.as_bytes()).unwrap();
    let transport = Transport::new(config);
    assert_eq!(
        get(&transport, &server.url("/")).unwrap().response.body,
        b"hello"
    );
    for refused in [
        &b""[..],
        b"no certificate here",
        b"-----BEGIN CERTIFICATE-----\n!!\n",
    ] {
        assert!(Config::default().trust_pem(refused).is_err(), "{refused:?}");
    }
}

#[test]
fn an_https_exchange() {
    let authority = TestAuthority::new();
    let server = TestServer::tls(authority.server(&["localhost", "127.0.0.1"], VALID), |_| {
        hello()
    });
    let transport = Transport::new(Config::default().trust(authority.root.clone()));
    let received = get(&transport, &server.url("/")).unwrap();
    assert_eq!(received.response.body, b"hello");
    // By name too, which SNI sends.
    let by_name = Url::parse(&format!("https://localhost:{}/", server.port())).unwrap();
    assert_eq!(get(&transport, &by_name).unwrap().response.body, b"hello");
    // The first connection was reused for its own origin.
    get(&transport, &server.url("/")).unwrap();
    assert_eq!(server.connections(), 2);
    // SNI named the host by name, and not by address.
    let sni: Vec<_> = server.requests().into_iter().map(|r| r.sni).collect();
    assert_eq!(sni, [None, Some("localhost".to_owned()), None]);
}

#[test]
fn a_handshake_that_never_ends() {
    // A listener that the kernel connects to, and nobody answers.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = Url::parse(&format!("https://127.0.0.1:{port}/")).unwrap();
    let impatient = Transport::new(Config {
        read_timeout: Duration::from_millis(500),
        ..Config::default()
    });
    let started = Instant::now();
    assert_eq!(get(&impatient, &url).unwrap_err(), NetworkError::Timeout);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let abort = AbortHandle::new();
    let aborter = abort.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        aborter.abort();
    });
    let started = Instant::now();
    let result = send(&transport(), "GET", &url, None, &abort);
    assert_eq!(result.unwrap_err(), NetworkError::Aborted);
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_close_notify_that_came_with_the_response_is_seen() {
    let authority = TestAuthority::new();
    let config = authority.server(&["127.0.0.1"], VALID);
    // The TCP connection stays open and quiet: only rustls has read
    // that the session ended.
    let server = TestServer::tls(config, |_| match hello() {
        Reply::Send(bytes) => Reply::SendWithCloseNotify(bytes),
        other => other,
    });
    let transport = Transport::new(Config {
        read_timeout: Duration::from_secs(2),
        ..Config::default().trust(authority.root.clone())
    });
    get(&transport, &server.url("/")).unwrap();
    // A POST, which nothing would send again had it gone on the ended
    // session.
    let posted = send(
        &transport,
        "POST",
        &server.url("/"),
        None,
        &AbortHandle::new(),
    );
    assert_eq!(posted.unwrap().response.body, b"hello");
    assert_eq!(server.connections(), 2);
}

#[test]
fn certificate_faults_are_named() {
    let authority = TestAuthority::new();
    let fault = |config: Arc<ServerConfig>, trusted: bool| {
        let server = TestServer::tls(config, |_| hello());
        let mut transport_config = Config::default();
        if trusted {
            transport_config = transport_config.trust(authority.root.clone());
        }
        match get(&Transport::new(transport_config), &server.url("/")) {
            Err(NetworkError::Tls(failure)) => failure,
            other => panic!("{other:?}"),
        }
    };
    let names = ["localhost", "127.0.0.1"];
    assert_eq!(
        fault(authority.server(&names, (2000, 2001)), true),
        TlsFailure::Expired
    );
    assert_eq!(
        fault(authority.server(&names, (2900, 3000)), true),
        TlsFailure::NotYetValid
    );
    assert_eq!(
        fault(authority.server(&["example.org"], VALID), true),
        TlsFailure::WrongName
    );
    assert_eq!(
        fault(authority.server(&names, VALID), false),
        TlsFailure::UnknownIssuer
    );
}

#[test]
fn a_partitions_connections_are_its_own() {
    // M33.2: the same origin's connection is not reused across top-level
    // sites.
    let server = TestServer::plain(|_| hello());
    let transport = transport();
    let url = server.url("/");
    let abort = AbortHandle::new();
    send_in(&transport, "GET", &url, None, Some("http://a.test"), &abort).unwrap();
    send_in(&transport, "GET", &url, None, Some("http://b.test"), &abort).unwrap();
    send_in(&transport, "GET", &url, None, Some("http://a.test"), &abort).unwrap();
    assert_eq!(server.connections(), 2);
    let key = |partition: Option<&str>| OriginKey {
        scheme: "http".into(),
        host: "127.0.0.1".into(),
        port: server.port(),
        partition: partition.map(str::to_owned),
    };
    assert_eq!(transport.pool.counts(&key(Some("http://a.test"))), (1, 1));
    assert_eq!(transport.pool.counts(&key(None)), (0, 0));
}

#[test]
fn a_partitions_tls_sessions_are_its_own() {
    // M33.2: each partition's configuration has a session cache of its
    // own; a top-level document's requests share the transport's.
    let transport = transport();
    let a = transport.tls_for(Some("https://a.test"));
    let b = transport.tls_for(Some("https://b.test"));
    assert!(!Arc::ptr_eq(&a, &b));
    assert!(Arc::ptr_eq(&a, &transport.tls_for(Some("https://a.test"))));
    assert!(Arc::ptr_eq(&transport.tls_for(None), &transport.tls));
    assert!(!Arc::ptr_eq(&a, &transport.tls));
}
