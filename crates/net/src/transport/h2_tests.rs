//! HTTP/2 against the test server (M38): chosen by ALPN, many requests
//! on one connection at once, flow control both ways, the retries a
//! `GOAWAY` or a refused stream allow, a server that breaks the rules,
//! aborts, and the connection let go of when idle.

use std::io::Write;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use flate2::Compression;
use flate2::write::GzEncoder;

use super::*;
use crate::h2::frame::{self, ErrorCode, Frame};
use crate::h2::hpack::Encoder;
use crate::test_server::{Reply, ServerRequest, TestAuthority, TestServer, VALID};

/// An HTTP/2 server answering with `script`, and a transport trusting it.
fn h2_server(
    script: impl Fn(&ServerRequest) -> Reply + Send + Sync + 'static,
) -> (TestServer, Transport) {
    h2_server_with(Config::default(), script)
}

fn h2_server_with(
    config: Config,
    script: impl Fn(&ServerRequest) -> Reply + Send + Sync + 'static,
) -> (TestServer, Transport) {
    let authority = TestAuthority::new();
    let server = TestServer::tls(authority.server_h2(&["127.0.0.1"], VALID), script);
    let transport = Transport::new(config.trust(authority.root.clone()));
    (server, transport)
}

fn exchange(
    transport: &Transport,
    method: &str,
    url: &Url,
    body: Option<&[u8]>,
    abort: &AbortHandle,
) -> Result<Received, NetworkError> {
    let headers = HeaderList::new();
    transport.exchange(&Exchange {
        partition: None,
        method,
        url,
        headers: &headers,
        body,
        initiator: None,
        ask: None,
        target: None,
        abort,
        body_limit: None,
        priority: None,
    })
}

fn get(transport: &Transport, url: &Url) -> Result<Received, NetworkError> {
    exchange(transport, "GET", url, None, &AbortHandle::new())
}

#[test]
fn a_server_that_offers_http2_is_spoken_to_in_it() {
    let (server, transport) = h2_server(|request| {
        Reply::with_fields(
            200,
            &[("Content-Type", "text/plain"), ("X-Path", &request.target)],
            b"hello",
        )
    });
    let received = get(&transport, &server.url("/path?q=1")).unwrap();
    let response = received.response;
    assert_eq!(response.version, http::Version::Http2);
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"hello");
    assert_eq!(
        response.headers.get(b"x-path").as_deref(),
        Some(&b"/path?q=1"[..])
    );
    assert!(response.reason.is_empty(), "HTTP/2 has none");
    let requests = server.requests();
    assert_eq!(requests[0].stream, 1);
    assert_eq!(requests[0].method, "GET");
    let host = format!("127.0.0.1:{}", server.port());
    assert_eq!(requests[0].header("host"), Some(host.as_str()));
    // A second request goes on the same connection, the next stream.
    get(&transport, &server.url("/again")).unwrap();
    assert_eq!(server.connections(), 1);
    assert_eq!(server.requests()[1].stream, 3);
    assert!(transport.has_h2(&server.url("/")));
}

#[test]
fn many_requests_share_one_connection_at_once() {
    let (server, transport) = h2_server(|_| {
        Reply::After(
            Duration::from_millis(400),
            Box::new(Reply::ok("text/plain", b"slow")),
        )
    });
    let transport = Arc::new(transport);
    let started = Instant::now();
    let fetches: Vec<_> = (0..20)
        .map(|n| {
            let transport = transport.clone();
            let url = server.url(&format!("/{n}"));
            thread::spawn(move || get(&transport, &url).map(|r| r.response.body))
        })
        .collect();
    for fetch in fetches {
        assert_eq!(fetch.join().unwrap().unwrap(), b"slow");
    }
    // One connection, its streams waiting together: here about one
    // reply's time; the bound is loose, for slow machines, but well
    // under the twenty replies' one after another.
    assert_eq!(server.connections(), 1);
    assert!(server.most_streams() >= 10, "{}", server.most_streams());
    assert!(
        started.elapsed() < Duration::from_millis(4000),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_server_without_http2_is_spoken_to_as_before() {
    let authority = TestAuthority::new();
    let server = TestServer::tls(authority.server(&["127.0.0.1"], VALID), |_| {
        Reply::After(
            Duration::from_millis(100),
            Box::new(Reply::ok("text/plain", b"one")),
        )
    });
    let transport = Arc::new(Transport::new(
        Config::default().trust(authority.root.clone()),
    ));
    // Fetches started together wait for the first connection's choice,
    // then use HTTP/1.1, several connections at once.
    let fetches: Vec<_> = (0..6)
        .map(|n| {
            let transport = transport.clone();
            let url = server.url(&format!("/{n}"));
            thread::spawn(move || get(&transport, &url).map(|r| r.response.version))
        })
        .collect();
    for fetch in fetches {
        assert_eq!(fetch.join().unwrap().unwrap(), http::Version::Http11);
    }
    assert!(server.most_open() > 1);
    assert!(!transport.has_h2(&server.url("/")));
}

#[test]
fn a_body_larger_than_a_window_comes_whole() {
    // Larger than a stream's window and than the connection's.
    let body: Vec<u8> = (0..20 << 20).map(|n: usize| (n % 251) as u8).collect();
    let served = body.clone();
    let (server, transport) = h2_server(move |_| Reply::ok("application/octet-stream", &served));
    let received = get(&transport, &server.url("/big")).unwrap();
    assert_eq!(received.response.body.len(), body.len());
    assert!(received.response.body == body);
}

#[test]
fn a_request_body_larger_than_the_servers_window_is_sent_whole() {
    let (server, transport) =
        h2_server(|request| Reply::ok("text/plain", request.body.len().to_string().as_bytes()));
    let body = vec![7u8; 300_000];
    let received = exchange(
        &transport,
        "POST",
        &server.url("/upload"),
        Some(&body),
        &AbortHandle::new(),
    )
    .unwrap();
    assert_eq!(received.response.body, b"300000");
    assert_eq!(server.requests()[0].body, body);
    assert_eq!(
        server.requests()[0].header("content-length"),
        Some("300000")
    );
}

#[test]
fn a_streamed_response_and_a_coded_one() {
    let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
    gzip.write_all(b"unzipped").unwrap();
    let coded = gzip.finish().unwrap();
    let (server, transport) = h2_server(move |request| match request.target.as_str() {
        "/gzip" => Reply::with_fields(200, &[("Content-Encoding", "gzip")], &coded),
        _ => Reply::Pieces(vec![
            (Duration::ZERO, b"HTTP/1.1 200 OK\r\n\r\n".to_vec()),
            (Duration::from_millis(50), b"first ".to_vec()),
            (Duration::from_millis(50), b"second".to_vec()),
        ]),
    });
    assert_eq!(
        get(&transport, &server.url("/gzip")).unwrap().response.body,
        b"unzipped"
    );
    struct Collect(Vec<u8>, Option<u16>);
    impl ResponseStream for Collect {
        fn head(&mut self, status: u16, _: &[u8], _: &HeaderList, _: AddressSpace) -> bool {
            self.1 = Some(status);
            true
        }
        fn body(&mut self, bytes: &[u8]) {
            self.0.extend_from_slice(bytes);
        }
    }
    let mut collect = Collect(Vec::new(), None);
    let headers = HeaderList::new();
    let url = server.url("/pieces");
    let received = transport
        .exchange_streaming(
            &Exchange {
                partition: None,
                method: "GET",
                url: &url,
                headers: &headers,
                body: None,
                initiator: None,
                ask: None,
                target: None,
                abort: &AbortHandle::new(),
                body_limit: None,
                priority: None,
            },
            &mut collect,
        )
        .unwrap();
    assert_eq!(collect.1, Some(200));
    assert_eq!(collect.0, b"first second");
    assert!(received.response.body.is_empty());
}

#[test]
fn a_goaway_or_a_refused_stream_sends_the_request_again() {
    let (server, transport) = h2_server(|request| match (request.connection, request.index) {
        // The first connection's first request is not processed.
        (0, 0) => Reply::GoAway,
        _ => Reply::ok("text/plain", b"again"),
    });
    assert_eq!(
        get(&transport, &server.url("/")).unwrap().response.body,
        b"again"
    );
    assert_eq!(server.connections(), 2, "on a new connection");
    let (server, transport) = h2_server(|request| match request.index {
        0 => Reply::Refuse,
        _ => Reply::ok("text/plain", b"refused once"),
    });
    assert_eq!(
        get(&transport, &server.url("/")).unwrap().response.body,
        b"refused once"
    );
    // Refused every time, it gives up.
    let (server, transport) = h2_server(|_| Reply::Refuse);
    assert!(matches!(
        get(&transport, &server.url("/")),
        Err(NetworkError::Http2(_))
    ));
    assert_eq!(server.requests().len(), 3, "the first and two again");
}

/// A response's frames on stream 1: its head's fields, then `data`.
fn frames(fields: &[(&str, &str)], data: &[u8], end: bool) -> Vec<u8> {
    let block = Encoder::default().encode(
        fields
            .iter()
            .map(|(name, value)| (name.as_bytes(), value.as_bytes())),
    );
    let mut out = Vec::new();
    frame::write_headers(
        &mut out,
        1,
        &block,
        data.is_empty() && end,
        frame::MAX_FRAME_SIZE,
    );
    if !data.is_empty() {
        Frame::Data {
            stream: 1,
            end_stream: end,
            data: data.to_vec(),
            flow: data.len(),
        }
        .write(&mut out);
    }
    out
}

#[test]
fn a_response_that_breaks_the_rules_fails_its_fetch() {
    for (what, bytes) in [
        (
            "an uppercase name",
            frames(&[(":status", "200"), ("X-Upper", "1")], b"", true),
        ),
        ("no status", frames(&[("x-a", "1")], b"", true)),
        (
            "a connection-specific field",
            frames(&[(":status", "200"), ("connection", "close")], b"", true),
        ),
        (
            "more body than its length",
            frames(
                &[(":status", "200"), ("content-length", "2")],
                b"three",
                true,
            ),
        ),
        (
            "less body than its length",
            frames(
                &[(":status", "200"), ("content-length", "9")],
                b"three",
                true,
            ),
        ),
        (
            "a status of two digits",
            frames(&[(":status", "20")], b"", true),
        ),
    ] {
        let reply = bytes.clone();
        let (server, transport) = h2_server(move |_| Reply::Frames(reply.clone()));
        let result = get(&transport, &server.url("/"));
        assert!(
            matches!(result, Err(NetworkError::Http2(_))),
            "{what}: {result:?}"
        );
    }
    // A push this end did not allow ends the connection.
    let mut push = Vec::new();
    Frame::PushPromise {
        stream: 1,
        promised: 2,
        end_headers: true,
        block: vec![0x82],
    }
    .write(&mut push);
    let (server, transport) = h2_server(move |_| Reply::Frames(push.clone()));
    assert!(matches!(
        get(&transport, &server.url("/")),
        Err(NetworkError::Http2(_))
    ));
}

#[test]
fn an_abort_ends_a_stream_and_the_connection_goes_on() {
    let (server, transport) = h2_server(|request| match request.target.as_str() {
        "/hang" => Reply::Hang,
        _ => Reply::ok("text/plain", b"fine"),
    });
    let transport = Arc::new(transport);
    let abort = AbortHandle::new();
    let waiting = {
        let (transport, abort, url) = (transport.clone(), abort.clone(), server.url("/hang"));
        thread::spawn(move || exchange(&transport, "GET", &url, None, &abort))
    };
    thread::sleep(Duration::from_millis(300));
    let started = Instant::now();
    abort.abort();
    assert!(matches!(
        waiting.join().unwrap(),
        Err(NetworkError::Aborted)
    ));
    assert!(started.elapsed() < Duration::from_millis(500));
    // The server heard the stream was cancelled.
    let waited = Instant::now();
    while server.resets() == 0 {
        assert!(waited.elapsed() < Duration::from_secs(5), "a reset");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        get(&transport, &server.url("/")).unwrap().response.body,
        b"fine"
    );
    assert_eq!(server.connections(), 1, "the same connection");
}

#[test]
fn an_idle_connection_is_let_go_of() {
    let config = Config {
        idle_timeout: Duration::from_millis(300),
        ..Config::default()
    };
    let (server, transport) = h2_server_with(config, |_| Reply::ok("text/plain", b"x"));
    get(&transport, &server.url("/")).unwrap();
    let started = Instant::now();
    while server.open() > 0 {
        assert!(started.elapsed() < Duration::from_secs(5), "closed in time");
        thread::sleep(Duration::from_millis(20));
    }
    // The next request opens another.
    get(&transport, &server.url("/")).unwrap();
    assert_eq!(server.connections(), 2);
}

#[test]
fn a_status_with_no_body_and_a_head_request() {
    let (server, transport) = h2_server(|request| match request.method.as_str() {
        // The length of the body a GET would have, which a HEAD's
        // response does not carry.
        "HEAD" => Reply::Send(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n".to_vec()),
        _ => Reply::Send(b"HTTP/1.1 204 No Content\r\n\r\n".to_vec()),
    });
    let received = exchange(
        &transport,
        "HEAD",
        &server.url("/"),
        None,
        &AbortHandle::new(),
    )
    .unwrap();
    assert_eq!(received.response.status, 200);
    assert!(received.response.body.is_empty());
    assert_eq!(
        get(&transport, &server.url("/")).unwrap().response.status,
        204
    );
}

#[test]
fn the_connections_error_codes_have_their_names() {
    assert_eq!(ErrorCode::REFUSED_STREAM.name(), "REFUSED_STREAM");
    assert_eq!(ErrorCode(0x99).name(), "0x99");
}

/// A URL on `server` under `host`, which the transport resolves to it.
fn at(server: &TestServer, host: &str, path: &str) -> Url {
    Url::parse(&format!("https://{host}:{}{path}", server.port())).unwrap()
}

/// A transport resolving `hosts` to this machine and trusting `root`.
fn resolving(root: &rustls::pki_types::CertificateDer<'static>, hosts: &[&str]) -> Transport {
    let here = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
    let config = hosts
        .iter()
        .fold(Config::default().trust(root.clone()), |config, host| {
            config.resolve(host, here)
        });
    Transport::new(config)
}

#[test]
fn a_connection_carries_another_hosts_requests_when_its_certificate_covers_it() {
    let authority = TestAuthority::new();
    let server = TestServer::tls(
        authority.server_h2(&["a.test", "b.test"], VALID),
        |request| {
            Reply::ok(
                "text/plain",
                request.header("host").unwrap_or_default().as_bytes(),
            )
        },
    );
    let transport = resolving(&authority.root, &["a.test", "b.test", "c.test"]);
    let body = |url: Url| get(&transport, &url).map(|r| r.response.body);
    let port = server.port();
    assert_eq!(
        body(at(&server, "a.test", "/")).unwrap(),
        format!("a.test:{port}").into_bytes()
    );
    assert_eq!(
        body(at(&server, "b.test", "/")).unwrap(),
        format!("b.test:{port}").into_bytes()
    );
    assert_eq!(server.connections(), 1, "b.test on a.test's connection");
    // A host the certificate does not cover is not carried: it opens its
    // own, whose TLS refuses the certificate.
    assert!(matches!(
        body(at(&server, "c.test", "/")),
        Err(NetworkError::Tls(_))
    ));
    assert_eq!(server.connections(), 2);
}

#[test]
fn a_misdirected_request_goes_again_on_its_hosts_own_connection() {
    let authority = TestAuthority::new();
    let server = TestServer::tls(
        authority.server_h2(&["a.test", "b.test"], VALID),
        |request| {
            let host = request.header("host").unwrap_or_default();
            match (request.connection, host.starts_with("b.test")) {
                (0, true) => {
                    Reply::with_fields(421, &[("Content-Type", "text/plain")], b"not here")
                }
                _ => Reply::ok("text/plain", b"here"),
            }
        },
    );
    let transport = resolving(&authority.root, &["a.test", "b.test"]);
    get(&transport, &at(&server, "a.test", "/")).unwrap();
    let received = get(&transport, &at(&server, "b.test", "/")).unwrap();
    assert_eq!(received.response.status, 200);
    assert_eq!(server.connections(), 2);
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!((requests[1].connection, requests[2].connection), (0, 1));
}

#[test]
fn clients_given_one_transport_share_its_connections() {
    let authority = TestAuthority::new();
    let server = TestServer::tls(authority.server_h2(&["127.0.0.1"], VALID), |_| {
        Reply::ok("text/plain", b"x")
    });
    let transport = Arc::new(Transport::new(
        Config::default().trust(authority.root.clone()),
    ));
    for _ in 0..2 {
        let client = crate::fetch::Client::default().with_transport(transport.clone());
        let request = crate::fetch::Request::get(server.url("/"), crate::fetch::Destination::Empty);
        client.fetch(&request).unwrap();
    }
    assert_eq!(server.connections(), 1);
}

#[test]
fn a_requests_priority_goes_with_it_over_http2_alone() {
    let authority = TestAuthority::new();
    let server = TestServer::tls(authority.server_h2(&["127.0.0.1"], VALID), |_| {
        Reply::ok("text/plain", b"x")
    });
    let client = crate::fetch::Client::new(Config::default().trust(authority.root.clone()));
    for destination in [
        crate::fetch::Destination::Image,
        crate::fetch::Destination::Style,
        crate::fetch::Destination::Empty,
    ] {
        let request = crate::fetch::Request::get(server.url("/"), destination);
        client.fetch(&request).unwrap();
    }
    let priorities: Vec<Option<String>> = server
        .requests()
        .iter()
        .map(|request| request.header("priority").map(str::to_owned))
        .collect();
    assert_eq!(
        priorities,
        [Some("u=5, i".to_owned()), Some("u=0".to_owned()), None]
    );
    // Over HTTP/1.1, none.
    let plain = TestServer::plain(|_| Reply::ok("text/plain", b"x"));
    let request = crate::fetch::Request::get(plain.url("/"), crate::fetch::Destination::Image);
    crate::fetch::Client::default().fetch(&request).unwrap();
    assert_eq!(plain.requests()[0].header("priority"), None);
}

#[test]
fn a_host_at_another_address_is_not_carried() {
    let authority = TestAuthority::new();
    let server = TestServer::tls(authority.server_h2(&["a.test", "d.test"], VALID), |_| {
        Reply::ok("text/plain", b"x")
    });
    // d.test is at another address of this machine, where the server is
    // not: its own connection fails, as it must not take a.test's.
    let here = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
    let elsewhere = std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 2));
    let transport = Transport::new(
        Config::default()
            .trust(authority.root.clone())
            .resolve("a.test", here)
            .resolve("d.test", elsewhere),
    );
    get(&transport, &at(&server, "a.test", "/")).unwrap();
    assert!(get(&transport, &at(&server, "d.test", "/")).is_err());
    assert_eq!(server.requests().len(), 1);
}
