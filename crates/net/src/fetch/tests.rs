//! Fetching against the test server and the file system: each scheme,
//! redirects and their rules, the checks made on every hop, the fields a
//! request sends, and the response's type.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use super::*;
use crate::http::Limits;
use crate::test_server::{Reply, TestServer};

fn client() -> Client {
    Client::default()
}

fn url(input: &str) -> Url {
    Url::parse(input).expect(input)
}

fn document(url: Url) -> Request {
    Request::get(url, Destination::Document)
}

fn page(input: &str, address_space: AddressSpace) -> Initiator {
    Initiator {
        url: url(input),
        address_space,
        origin: url(input).origin(),
    }
}

fn hello() -> Reply {
    Reply::ok("text/html;charset=utf-8", b"hello")
}

fn redirect(status: u16, location: &str) -> Reply {
    Reply::with_fields(status, &[("Location", location)], b"")
}

/// What a streamed fetch handed over, and when.
#[derive(Debug, Default)]
struct Handed {
    head: Option<(u16, Vec<Url>)>,
    pieces: Vec<(Instant, Vec<u8>)>,
}

fn streamed(request: &Request) -> (Result<Response, NetworkError>, Handed) {
    let mut handed = Handed::default();
    let result = client().fetch_streaming(request, &mut |delivery| match delivery {
        Delivery::Head(response) => {
            assert!(handed.head.is_none(), "one head");
            assert!(response.body.is_empty());
            handed.head = Some((response.status, response.url_list.clone()));
        }
        Delivery::Body(bytes) => handed.pieces.push((Instant::now(), bytes.to_vec())),
    });
    (result, handed)
}

#[test]
fn a_streamed_body_comes_in_pieces_as_it_arrives() {
    let gap = Duration::from_millis(400);
    let server = TestServer::plain(move |request| match request.target.as_str() {
        "/slow" => Reply::Pieces(vec![
            (
                Duration::ZERO,
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 10\r\n\r\nfirst"
                    .to_vec(),
            ),
            (gap, b"after".to_vec()),
        ]),
        "/moved" => redirect(302, "/slow"),
        _ => Reply::with_fields(404, &[], b""),
    });
    let start = Instant::now();
    let (result, handed) = streamed(&document(server.url("/moved")));
    let response = result.unwrap();
    assert!(response.body.is_empty());
    assert_eq!(response.status, 200);
    // The redirect is followed, not handed over.
    let (status, urls) = handed.head.unwrap();
    assert_eq!(status, 200);
    assert_eq!(urls, [server.url("/moved"), server.url("/slow")]);
    let body: Vec<u8> = handed.pieces.iter().flat_map(|(_, p)| p.clone()).collect();
    assert_eq!(body, b"firstafter");
    // The first piece came before the second was sent.
    let (first_at, first) = &handed.pieces[0];
    assert_eq!(first, b"first");
    assert!(
        first_at.duration_since(start) < gap,
        "{:?}",
        first_at.duration_since(start)
    );
}

#[test]
fn a_streamed_fetch_hands_over_codings_removed_and_other_schemes_whole() {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
    gzip.write_all(&b"zipped ".repeat(1000)).unwrap();
    let gzip = gzip.finish().unwrap();
    let server = TestServer::plain(move |request| match request.target.as_str() {
        "/zipped" => Reply::with_fields(
            200,
            &[("Content-Type", "text/plain"), ("Content-Encoding", "gzip")],
            &gzip,
        ),
        _ => Reply::with_fields(404, &[], b""),
    });
    let (result, handed) = streamed(&document(server.url("/zipped")));
    assert!(result.unwrap().body.is_empty());
    let body: Vec<u8> = handed.pieces.iter().flat_map(|(_, p)| p.clone()).collect();
    assert_eq!(body, b"zipped ".repeat(1000));
    let (result, handed) = streamed(&document(url("data:text/html,hi")));
    assert!(result.unwrap().body.is_empty());
    assert_eq!(handed.head.unwrap().0, 200);
    assert_eq!(handed.pieces.len(), 1);
    assert_eq!(handed.pieces[0].1, b"hi");
    // A failure after the head is the fetch's error.
    let server = TestServer::plain(|_| {
        Reply::SendAndClose(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort".to_vec())
    });
    let (result, handed) = streamed(&document(server.url("/cut")));
    assert_eq!(result.unwrap_err(), NetworkError::Truncated);
    assert!(handed.head.is_some());
}

#[test]
fn a_client_is_shared_between_threads() {
    fn shared<T: Send + Sync>() {}
    shared::<Client>();
}

#[test]
fn about_blank_is_an_empty_document() {
    for input in ["about:blank", "about:blank#top", "about:blank?q"] {
        let response = client().fetch(&document(url(input))).unwrap();
        assert_eq!(response.status, 200);
        assert!(response.body.is_empty());
        let mime_type = response.mime_type.clone().unwrap();
        assert_eq!(mime_type.essence(), "text/html");
        assert_eq!(mime_type.parameter("charset"), Some("utf-8"));
        assert_eq!(response.address_space, None);
        assert_eq!(response.url().as_str(), input);
    }
    for input in ["about:srcdoc", "about:silver", "about:"] {
        let error = client().fetch(&document(url(input))).unwrap_err();
        assert_eq!(error, NetworkError::BadScheme("about".into()), "{input}");
    }
}

#[test]
fn data_urls_are_their_own_body() {
    let response = client()
        .fetch(&document(url("data:text/plain;base64,aGk=")))
        .unwrap();
    assert_eq!(response.body, b"hi");
    assert_eq!(response.mime_type.unwrap().essence(), "text/plain");
    assert_eq!(
        response.headers.get(b"Content-Type").unwrap(),
        b"text/plain"
    );
    let response = client().fetch(&document(url("data:,a%20b"))).unwrap();
    assert_eq!(response.body, b"a b");
    assert_eq!(
        response.mime_type.unwrap().to_string(),
        "text/plain;charset=US-ASCII"
    );
    let broken = client().fetch(&document(url("data:;base64,a")));
    assert_eq!(broken.unwrap_err(), NetworkError::DataUrl);
}

#[test]
fn a_request_sends_its_fields() {
    let server = TestServer::plain(|_| hello());
    let response = client().fetch(&document(server.url("/a?b=c"))).unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.status_message, b"Status");
    assert_eq!(response.body, b"hello");
    assert_eq!(response.address_space, Some(AddressSpace::Loopback));
    assert!(response.ok());
    let mime_type = response.mime_type.unwrap();
    assert_eq!(
        (mime_type.essence(), mime_type.parameter("charset")),
        ("text/html", Some("utf-8"))
    );
    client()
        .fetch(&Request::get(server.url("/style.css"), Destination::Style))
        .unwrap();
    let icon = Request::get(server.url("/favicon.ico"), Destination::Image);
    assert_eq!(icon.mode, Mode::NoCors);
    client().fetch(&icon).unwrap();
    let script = Request::get(server.url("/a.js"), Destination::Script);
    assert_eq!(script.mode, Mode::NoCors);
    client().fetch(&script).unwrap();
    let requests = server.requests();
    let sent = &requests[0];
    assert_eq!(
        (sent.method.as_str(), sent.target.as_str()),
        ("GET", "/a?b=c")
    );
    assert_eq!(sent.header("User-Agent"), Some(USER_AGENT));
    assert!(USER_AGENT.starts_with("Mozilla/5.0 (compatible; SilverBrowser/"));
    // D12's escape hatch sends the reference browser's string, this
    // browser named after it, when a client is made to.
    let compat = client().with_user_agent(super::COMPAT_USER_AGENT);
    compat
        .fetch(&Request::get(server.url("/style.css"), Destination::Style))
        .unwrap();
    let agent = server
        .requests()
        .last()
        .unwrap()
        .header("User-Agent")
        .map(str::to_owned);
    let agent = agent.unwrap();
    assert!(
        agent.starts_with("Mozilla/5.0 (") && agent.contains(" Chrome/"),
        "{agent}"
    );
    assert!(agent.ends_with(concat!(" SilverBrowser/", env!("CARGO_PKG_VERSION"))));
    assert_eq!(
        sent.header("Accept"),
        Some("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
    );
    assert_eq!(sent.header("Accept-Language"), Some("en-US,en;q=0.9"));
    assert_eq!(sent.header("Accept-Encoding"), Some("gzip, deflate"));
    for unsent in ["Referer", "Cookie", "Origin"] {
        assert_eq!(sent.header(unsent), None, "{unsent}");
    }
    // A load the user asked for, to a loopback address, which is
    // potentially trustworthy (M27).
    assert_eq!(sent.header("Sec-Fetch-Site"), Some("none"));
    assert_eq!(sent.header("Sec-Fetch-Mode"), Some("navigate"));
    assert_eq!(sent.header("Sec-Fetch-Dest"), Some("document"));
    assert_eq!(sent.header("Sec-Fetch-User"), Some("?1"));
    assert_eq!(requests[1].header("Accept"), Some("text/css,*/*;q=0.1"));
    assert_eq!(
        requests[2].header("Accept"),
        Some("image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5")
    );
    assert_eq!(requests[3].header("Accept"), Some("*/*"));
}

#[test]
fn a_post_sends_its_body() {
    let server = TestServer::plain(|_| hello());
    let mut request = document(server.url("/form"));
    request.method = Method::POST;
    request.body = Some(b"a=1".to_vec());
    client().fetch(&request).unwrap();
    let sent = &server.requests()[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.body, b"a=1");
    assert_eq!(sent.header("Content-Length"), Some("3"));
}

#[test]
fn an_error_status_is_still_a_response() {
    let server = TestServer::plain(|_| Reply::with_fields(404, &[], b"not here"));
    let response = client().fetch(&document(server.url("/"))).unwrap();
    assert_eq!(response.status, 404);
    assert!(!response.ok());
    assert_eq!(response.body, b"not here");
    assert_eq!(response.mime_type, None);
}

#[test]
fn redirects_are_followed() {
    let server = TestServer::plain(|request| match request.target.as_str() {
        "/a" => redirect(302, "/b"),
        "/b" => redirect(301, "c?x"),
        "/c?x" => redirect(307, "/d"),
        "/d" => redirect(308, "/e"),
        _ => hello(),
    });
    let response = client().fetch(&document(server.url("/a"))).unwrap();
    assert_eq!(response.body, b"hello");
    let list: Vec<String> = response.url_list.iter().map(|u| u.path_str()).collect();
    assert_eq!(list, ["/a", "/b", "/c", "/d", "/e"]);
    assert_eq!(response.url().as_str(), server.url("/e").as_str());
    // A redirect with no location is the answer itself, and so is a 300
    // or a 304.
    for status in [302, 300, 304] {
        let server = TestServer::plain(move |_| Reply::with_fields(status, &[], b""));
        let response = client().fetch(&document(server.url("/"))).unwrap();
        assert_eq!(response.status, status);
    }
    let server = TestServer::plain(|_| Reply::with_fields(300, &[("Location", "/x")], b""));
    assert_eq!(
        client().fetch(&document(server.url("/"))).unwrap().status,
        300
    );
}

#[test]
fn methods_after_a_redirect() {
    let cases = [
        (301, Method::POST, "GET", false),
        (302, Method::POST, "GET", false),
        (303, Method::POST, "GET", false),
        (307, Method::POST, "POST", true),
        (308, Method::POST, "POST", true),
        (301, Method::GET, "GET", false),
        (302, Method::HEAD, "HEAD", false),
        (303, Method::HEAD, "HEAD", false),
        (303, Method::GET, "GET", false),
    ];
    for (status, method, after, with_body) in cases {
        let server = TestServer::plain(move |request| match request.target.as_str() {
            "/from" => redirect(status, "/to"),
            _ => hello(),
        });
        let mut request = document(server.url("/from"));
        request.method = method.clone();
        request.body = (method == Method::POST).then(|| b"x=1".to_vec());
        if method == Method::POST {
            request.headers.append(b"Content-Type", b"text/plain");
        }
        client().fetch(&request).unwrap();
        let sent = server.requests();
        let last = sent.last().unwrap();
        assert_eq!(last.method, after, "{status} {method:?}");
        assert_eq!(last.body.is_empty(), !with_body, "{status} {method:?}");
        assert_eq!(
            last.header("Content-Length").is_some(),
            with_body,
            "{status} {method:?}"
        );
        // A body's type goes with it, and goes when it does (M26.9).
        assert_eq!(
            last.header("Content-Type").is_some(),
            with_body,
            "{status} {method:?}"
        );
    }
}

#[test]
fn a_redirect_without_a_fragment_keeps_the_request_s() {
    let server = TestServer::plain(|request| match request.target.as_str() {
        "/a" => redirect(302, "/b"),
        "/c" => redirect(302, "/d#theirs"),
        _ => hello(),
    });
    let response = client().fetch(&document(server.url("/a#mine"))).unwrap();
    assert_eq!(response.url().fragment(), Some("mine"));
    let response = client().fetch(&document(server.url("/c#mine"))).unwrap();
    assert_eq!(response.url().fragment(), Some("theirs"));
}

#[test]
fn twenty_redirects_and_no_more() {
    // /n/k?m redirects to /n/k+1?m until k reaches m.
    let server = TestServer::plain(|request| {
        let (path, stop) = request.target.split_once('?').unwrap();
        let k: usize = path.trim_start_matches("/n/").parse().unwrap();
        if k < stop.parse().unwrap() {
            redirect(302, &format!("/n/{}?{stop}", k + 1))
        } else {
            hello()
        }
    });
    let response = client()
        .fetch(&document(server.url(&format!("/n/0?{MAX_REDIRECTS}"))))
        .unwrap();
    assert_eq!(response.url_list.len(), MAX_REDIRECTS + 1);
    let too_many = client().fetch(&document(
        server.url(&format!("/n/0?{}", MAX_REDIRECTS + 1)),
    ));
    assert_eq!(too_many.unwrap_err(), NetworkError::RedirectLimit);
}

#[test]
fn redirects_that_go_nowhere() {
    let two = Reply::with_fields(302, &[("Location", "/a"), ("Location", "/b")], b"");
    let cases = [
        (two, NetworkError::BadRedirect),
        (redirect(302, "http://[::1"), NetworkError::BadRedirect),
        (redirect(302, "data:,x"), NetworkError::BadRedirect),
        (
            redirect(301, "file:///etc/passwd"),
            NetworkError::BadRedirect,
        ),
        (
            redirect(307, "javascript:alert(1)"),
            NetworkError::BadRedirect,
        ),
        (
            redirect(302, "http://127.0.0.1:25/"),
            NetworkError::BadPort(25),
        ),
    ];
    for (reply, expected) in cases {
        let server = TestServer::plain(move |_| reply.clone());
        let result = client().fetch(&document(server.url("/")));
        assert_eq!(result.unwrap_err(), expected);
    }
}

#[test]
fn bad_ports_are_refused_before_connecting() {
    assert_eq!(BAD_PORTS.len(), 83);
    for port in [0, 1, 25, 6667, 10080] {
        for scheme in ["http", "https"] {
            let result = client().fetch(&document(url(&format!("{scheme}://127.0.0.1:{port}/"))));
            assert_eq!(result.unwrap_err(), NetworkError::BadPort(port));
        }
    }
    // The default ports are not bad.
    assert!(!is_bad_port(80) && !is_bad_port(443));
}

#[test]
fn schemes_a_fetch_may_use() {
    for input in [
        "javascript:void(0)",
        "blob:https://a/b",
        "ftp://a/b",
        "ws://a/",
        "mailto:a@b",
    ] {
        let error = client().fetch(&document(url(input))).unwrap_err();
        assert!(
            matches!(error, NetworkError::BadScheme(_)),
            "{input}: {error:?}"
        );
    }
    // A page's subresource may be http:, https: or data:...
    let from_http = page("http://127.0.0.1/", AddressSpace::Loopback);
    let style =
        |input: &str| Request::get(url(input), Destination::Style).initiated_by(from_http.clone());
    assert!(client().fetch(&style("data:text/css,a{}")).is_ok());
    for input in ["about:blank", "file:///srv/a.css"] {
        let error = client().fetch(&style(input)).unwrap_err();
        assert!(
            matches!(error, NetworkError::BadScheme(_)),
            "{input}: {error:?}"
        );
    }
    // ...and file: from a file: page.
    let dir = scratch("schemes");
    let sheet = dir.join("a.css");
    std::fs::write(&sheet, "a {}").unwrap();
    let from_file = page(
        file_url(&dir.join("page.html")).as_str(),
        AddressSpace::Loopback,
    );
    let request = Request::get(file_url(&sheet), Destination::Style).initiated_by(from_file);
    assert_eq!(client().fetch(&request).unwrap().body, b"a {}");
}

#[test]
fn a_response_to_head_has_no_body_whatever_its_scheme() {
    let mut request = Request::get(url("data:text/plain,hello"), Destination::Empty);
    request.method = Method::HEAD;
    let response = client().fetch(&request).unwrap();
    assert!(response.body.is_empty());
    assert_eq!(
        response.headers.get(b"Content-Type").as_deref(),
        Some(&b"text/plain"[..])
    );
    request.method = Method::GET;
    assert_eq!(client().fetch(&request).unwrap().body, b"hello");
}

#[test]
fn mixed_content_is_blocked() {
    let from_https = page("https://example.org/", AddressSpace::Loopback);
    let style = |url: Url| Request::get(url, Destination::Style).initiated_by(from_https.clone());
    // Refused before the name is even looked up.
    let error = client().fetch(&style(url("http://example.invalid/a.css")));
    assert_eq!(error.unwrap_err(), NetworkError::MixedContent);
    // Loopback and localhost are trustworthy all the same.
    let server = TestServer::plain(|request| match request.target.as_str() {
        "/away" => redirect(302, "http://example.invalid/a.css"),
        _ => hello(),
    });
    assert!(client().fetch(&style(server.url("/a.css"))).is_ok());
    let localhost = url(&format!("http://localhost:{}/a.css", server.port()));
    assert!(client().fetch(&style(localhost)).is_ok());
    // And each hop is checked again.
    let error = client().fetch(&style(server.url("/away")));
    assert_eq!(error.unwrap_err(), NetworkError::MixedContent);
    // An http: page has nothing to mix.
    let from_http = page("http://127.0.0.1/", AddressSpace::Loopback);
    let request = Request::get(server.url("/a.css"), Destination::Style).initiated_by(from_http);
    assert!(client().fetch(&request).is_ok());
}

#[test]
fn an_http_image_of_an_https_page_is_upgraded() {
    let from_https = page("https://example.org/", AddressSpace::Loopback);
    let from_http = page("http://example.org/", AddressSpace::Loopback);
    let image = |input: &str, from: &Initiator| {
        Request::get(url(input), Destination::Image).initiated_by(from.clone())
    };
    let upgraded = |request: &Request| {
        upgrade_mixed_content(request, &request.url).map(|url| url.as_str().to_owned())
    };
    assert_eq!(
        upgraded(&image("http://example.com/a.png", &from_https)).as_deref(),
        Some("https://example.com/a.png")
    );
    // The default port goes with the scheme; another stays.
    assert_eq!(
        upgraded(&image("http://example.com:80/a.png", &from_https)).as_deref(),
        Some("https://example.com/a.png")
    );
    assert_eq!(
        upgraded(&image("http://example.com:8080/a.png", &from_https)).as_deref(),
        Some("https://example.com:8080/a.png")
    );
    // Not an IP address, nor what is trustworthy already, nor from an
    // http: page, nor anything but an image.
    assert_eq!(
        upgraded(&image("http://192.0.2.1/a.png", &from_https)),
        None
    );
    assert_eq!(
        upgraded(&image("http://[2001:db8::1]/a.png", &from_https)),
        None
    );
    assert_eq!(
        upgraded(&image("http://localhost/a.png", &from_https)),
        None
    );
    assert_eq!(
        upgraded(&image("http://example.com/a.png", &from_http)),
        None
    );
    let style = Request::get(url("http://example.com/a.css"), Destination::Style)
        .initiated_by(from_https.clone());
    assert_eq!(upgraded(&style), None);
    // Nor a crossorigin image, which is blocked.
    let cors = image("http://example.com/a.png", &from_https).with_cors(Credentials::SameOrigin);
    assert_eq!(upgraded(&cors), None);
    assert_eq!(
        client().fetch(&cors).unwrap_err(),
        NetworkError::MixedContent
    );
    // An image from an IP address stays mixed, and blocked.
    let error = client().fetch(&image("http://192.0.2.1/a.png", &from_https));
    assert_eq!(error.unwrap_err(), NetworkError::MixedContent);
}

#[test]
fn a_request_may_hold_its_body_to_less() {
    let server = TestServer::plain(|_| Reply::ok("image/png", &[7; 100]));
    let request = |limit: usize| {
        Request::get(server.url("/a.png"), Destination::Image).with_body_limit(limit)
    };
    assert_eq!(client().fetch(&request(100)).unwrap().body.len(), 100);
    assert_eq!(
        client().fetch(&request(99)).unwrap_err(),
        NetworkError::BodyTooLarge
    );
    let data = |limit: usize| {
        Request::get(url("data:,0123456789"), Destination::Image).with_body_limit(limit)
    };
    assert_eq!(client().fetch(&data(10)).unwrap().body, b"0123456789");
    assert_eq!(
        client().fetch(&data(9)).unwrap_err(),
        NetworkError::BodyTooLarge
    );
}

#[test]
fn address_spaces_are_checked() {
    let server = TestServer::plain(|_| hello());
    let from_public = page("https://example.org/", AddressSpace::Public);
    let request = Request::get(server.url("/"), Destination::Style).initiated_by(from_public);
    assert_eq!(
        client().fetch(&request).unwrap_err(),
        NetworkError::AddressSpace {
            initiator: AddressSpace::Public,
            target: AddressSpace::Loopback,
        }
    );
    assert_eq!(server.connections(), 0);
}

#[test]
fn an_abort_ends_a_fetch() {
    let server = TestServer::plain(|_| Reply::Hang);
    let request = document(server.url("/"));
    let aborter = request.abort.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        aborter.abort();
    });
    let started = Instant::now();
    assert_eq!(client().fetch(&request).unwrap_err(), NetworkError::Aborted);
    assert!(started.elapsed() < Duration::from_millis(1500));
    // One aborted before it starts goes nowhere.
    let server = TestServer::plain(|_| hello());
    let request = document(server.url("/"));
    request.abort.abort();
    assert_eq!(client().fetch(&request).unwrap_err(), NetworkError::Aborted);
    assert_eq!(server.connections(), 0);
}

/// A directory of its own under the system's temporary directory.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("silver-net-fetch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The `file:` URL of an absolute path, on Unix or Windows.
fn file_url(path: &Path) -> Url {
    Url::from_file_path(path).expect("an absolute UTF-8 path")
}

#[test]
fn files_are_read_and_typed() {
    let dir = scratch("files");
    let files: [(&str, &[u8], &str); 12] = [
        ("a.html", b"<p>hi", "text/html"),
        ("b.HTM", b"<p>hi", "text/html"),
        // Which sniffing would call text/plain.
        ("upper.CSS", b"a {}", "text/css"),
        ("c.css", b"a {}", "text/css"),
        ("d.txt", b"plain", "text/plain"),
        ("e.png", b"\x89PNG", "image/png"),
        ("f.jpeg", b"x", "image/jpeg"),
        ("g.svg", b"<svg/>", "image/svg+xml"),
        // No extension the engine knows: sniffed.
        ("page", b"<!DOCTYPE html><p>hi", "text/html"),
        ("blob.bin", b"\x00\x01\x02", "application/octet-stream"),
        ("with space.txt", b"spaced", "text/plain"),
        // What a URL would read otherwise, named as from_file_path names
        // it, and read back as the same file.
        ("odd #1 100%.txt", b"odd", "text/plain"),
    ];
    for (name, body, essence) in files {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        let response = client().fetch(&document(file_url(&path))).unwrap();
        assert_eq!(response.body, body, "{name}");
        assert_eq!(response.mime_type.unwrap().essence(), essence, "{name}");
        assert_eq!(response.address_space, Some(AddressSpace::Loopback));
    }
}

#[test]
fn files_that_are_not_read() {
    let dir = scratch("refused");
    let fetch = |url: Url| client().fetch(&document(url));
    // A directory, and a file that is not there.
    assert!(matches!(fetch(file_url(&dir)), Err(NetworkError::File(_))));
    let missing = fetch(file_url(&dir.join("missing.html")));
    assert!(matches!(missing, Err(NetworkError::File(_))), "{missing:?}");
    // A file on another machine, even one whose path is here too.
    let here = dir.join("here.txt");
    std::fs::write(&here, "here").unwrap();
    let path = file_url(&here).path_str();
    let remote = fetch(url(&format!("file://server{path}")));
    assert!(matches!(remote, Err(NetworkError::File(_))), "{remote:?}");
    // A separator hidden in a segment.
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::write(dir.join("a").join("b.txt"), "b").unwrap();
    let hidden = url(&format!("{}/a%2Fb.txt", file_url(&dir).as_str()));
    assert!(matches!(fetch(hidden), Err(NetworkError::File(_))));
    // A file over the body limit.
    let big = dir.join("big.txt");
    std::fs::write(&big, "12345").unwrap();
    let small = Client::new(transport::Config {
        limits: Limits {
            header_bytes: 1024,
            body_bytes: 4,
        },
        ..transport::Config::default()
    });
    assert_eq!(
        small.fetch(&document(file_url(&big))).unwrap_err(),
        NetworkError::BodyTooLarge
    );
}

/// The one fetch that goes to the real network, against the real roots.
/// Run by hand: `cargo test -p net -- --ignored example_com`.
#[test]
#[ignore = "needs the network"]
fn example_com() {
    let response = client()
        .fetch(&document(url("https://example.com/")))
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.mime_type.unwrap().essence(), "text/html");
    assert!(String::from_utf8_lossy(&response.body).contains("Example Domain"));
    assert_eq!(response.address_space, Some(AddressSpace::Public));
}

#[test]
fn a_scripts_policy_is_checked_before_each_hop() {
    let other = TestServer::plain(|_| hello());
    let elsewhere = other.url("/a.js");
    let away = elsewhere.to_string();
    let server = TestServer::plain(move |request| match request.target.as_str() {
        "/redirect" => Reply::with_fields(302, &[("Location", away.as_str())], b""),
        "/js/away" => Reply::with_fields(302, &[("Location", "/other.js")], b""),
        _ => hello(),
    });
    let page = server.url("/page.html");
    let request = |url: Url, destination: Destination, policy: &str| {
        let mut headers = HeaderList::new();
        headers.append(b"Content-Security-Policy", policy.as_bytes());
        Request::get(url, destination)
            .initiated_by(Initiator {
                url: page.clone(),
                address_space: AddressSpace::Loopback,
                origin: page.origin(),
            })
            .with_csp(Csp::from_headers(&headers, &page))
    };
    let blocked = Err(NetworkError::Csp("script-src 'self'".to_owned()));
    let fetched = |request: Request| client().fetch(&request).map(|_| ());
    assert_eq!(
        fetched(request(
            server.url("/a.js"),
            Destination::Script,
            "script-src 'self'"
        )),
        Ok(())
    );
    // Not at the first URL, nor at a redirect: never asked.
    assert_eq!(
        fetched(request(
            elsewhere.clone(),
            Destination::Script,
            "script-src 'self'"
        )),
        blocked
    );
    assert_eq!(
        fetched(request(
            server.url("/redirect"),
            Destination::Script,
            "script-src 'self'"
        )),
        blocked
    );
    assert!(other.requests().is_empty());
    // Only a script's fetch is held to it.
    assert_eq!(
        fetched(request(elsewhere, Destination::Style, "script-src 'self'")),
        Ok(())
    );
    assert_eq!(other.requests().len(), 1);
    // A path holds the first URL alone: after a redirect, the host does.
    let local = |path: &str| url(&format!("http://localhost:{}{path}", server.port()));
    let policy = format!("script-src http://localhost:{}/js/", server.port());
    let directive = Err(NetworkError::Csp(policy.clone()));
    assert_eq!(
        fetched(request(local("/js/a.js"), Destination::Script, &policy)),
        Ok(())
    );
    assert_eq!(
        fetched(request(local("/other.js"), Destination::Script, &policy)),
        directive
    );
    assert_eq!(
        fetched(request(local("/js/away"), Destination::Script, &policy)),
        Ok(())
    );
}

#[test]
fn upgrade_insecure_requests_upgrades_but_localhost_and_another_origins_navigation() {
    let page = url("http://example.test/page.html");
    let mut headers = HeaderList::new();
    headers.append(b"Content-Security-Policy", b"upgrade-insecure-requests");
    let csp = Csp::from_headers(&headers, &page);
    let request = |destination: Destination| {
        let mut request = Request::get(page.clone(), destination).with_csp(csp.clone());
        request.origin = Some(page.origin());
        request
    };
    let upgraded = |destination, target: &str| {
        upgrade_insecure(&request(destination), &url(target)).map(|url| url.to_string())
    };
    assert_eq!(
        upgraded(Destination::Image, "http://cdn.test/a.png").as_deref(),
        Some("https://cdn.test/a.png")
    );
    assert_eq!(
        upgraded(Destination::Image, "http://cdn.test:80/a.png").as_deref(),
        Some("https://cdn.test/a.png")
    );
    assert_eq!(
        upgraded(Destination::Image, "http://cdn.test:8080/a.png").as_deref(),
        Some("https://cdn.test:8080/a.png")
    );
    assert_eq!(upgraded(Destination::Image, "http://localhost/a.png"), None);
    assert_eq!(
        upgraded(Destination::Image, "http://a.localhost/a.png"),
        None
    );
    assert_eq!(upgraded(Destination::Image, "http://127.0.0.1/a.png"), None);
    assert_eq!(upgraded(Destination::Image, "https://cdn.test/a.png"), None);
    // A navigation of the top: to the document's own origin alone.
    assert_eq!(
        upgraded(Destination::Document, "http://example.test/next").as_deref(),
        Some("https://example.test/next")
    );
    assert_eq!(upgraded(Destination::Document, "http://other.test/"), None);
    assert!(upgraded(Destination::Iframe, "http://other.test/").is_some());
    // Without the directive, or with it only reported, nothing.
    let mut report = HeaderList::new();
    report.append(
        b"Content-Security-Policy-Report-Only",
        b"upgrade-insecure-requests",
    );
    let quiet =
        Request::get(page.clone(), Destination::Image).with_csp(Csp::from_headers(&report, &page));
    assert_eq!(
        upgrade_insecure(&quiet, &url("http://cdn.test/a.png")),
        None
    );
}

#[test]
fn a_forms_submission_is_held_to_form_action_at_each_hop() {
    let other = TestServer::plain(|_| hello());
    let away = other.url("/landing").to_string();
    let server = TestServer::plain(move |request| match request.target.as_str() {
        "/redirect" => Reply::with_fields(302, &[("Location", away.as_str())], b""),
        _ => hello(),
    });
    let page = server.url("/page.html");
    let mut headers = HeaderList::new();
    headers.append(
        b"Content-Security-Policy",
        b"form-action 'self'; default-src 'none'",
    );
    let csp = Csp::from_headers(&headers, &page).with_sink();
    let submit = |path: &str| {
        let mut request = Request::get(server.url(path), Destination::Document)
            .from_document(page.origin(), true)
            .with_csp(csp.clone());
        request.form_submission = true;
        client().fetch(&request).map(|_| ())
    };
    assert_eq!(submit("/self"), Ok(()));
    assert_eq!(
        submit("/redirect"),
        Err(NetworkError::Csp("form-action 'self'".to_owned()))
    );
    assert!(other.requests().is_empty());
    let recorded = csp.take_violations();
    assert_eq!(recorded.len(), 1);
    // A report names the first URL after a redirect to another origin.
    assert_eq!(recorded[0].blocked, server.url("/redirect").as_str());
}

/// A gate whose answer the test sets, counting how often it was asked.
#[derive(Debug, Default)]
struct Switch {
    allow: std::sync::atomic::AtomicBool,
    asked: std::sync::atomic::AtomicUsize,
}

impl LocalNetworkGate for Switch {
    fn allow(&self, _: &Origin, _: AddressSpace, _: &AbortHandle) -> bool {
        use std::sync::atomic::Ordering::SeqCst;
        self.asked.fetch_add(1, SeqCst);
        self.allow.load(SeqCst)
    }
}

#[test]
fn a_request_into_a_more_private_space_asks_and_the_cache_does_too() {
    use std::sync::atomic::Ordering::SeqCst;
    let server = TestServer::plain(|_| {
        Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/plain"),
                ("Cache-Control", "max-age=600"),
            ],
            b"local",
        )
    });
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], server.port()));
    let config = transport::Config::default()
        .resolve("printer.test", std::net::IpAddr::from([127, 0, 0, 1]))
        .treat_as(address, AddressSpace::Local);
    let gate = Arc::new(Switch::default());
    let client = Client::new(config).with_local_network_gate(gate.clone());
    let target = url(&format!("http://printer.test:{}/status", server.port()));
    let from_public = |target_space| {
        let mut request = Request::get(target.clone(), Destination::Empty)
            .initiated_by(page("https://page.test/", AddressSpace::Public));
        request.local_network = Some(LocalNetworkAccess {
            origin: url("https://page.test/").origin(),
            local: true,
            loopback: false,
        });
        request.target_address_space = target_space;
        request.mode = Mode::NoCors;
        request
    };
    // An https: page's request to http: is mixed content, unless it names
    // the local space it is for.
    assert_eq!(
        client.fetch(&from_public(None)).unwrap_err(),
        NetworkError::MixedContent
    );
    assert_eq!(gate.asked.load(SeqCst), 0);
    // Let: fetched, and kept by the cache.
    gate.allow.store(true, SeqCst);
    let response = client
        .fetch(&from_public(Some(AddressSpace::Local)))
        .unwrap();
    assert_eq!(response.body, b"local");
    assert_eq!(gate.asked.load(SeqCst), 1);
    // The cache's answer is asked for as the server's would be.
    gate.allow.store(false, SeqCst);
    assert_eq!(
        client
            .fetch(&from_public(Some(AddressSpace::Local)))
            .unwrap_err(),
        NetworkError::AddressSpace {
            initiator: AddressSpace::Public,
            target: AddressSpace::Local,
        }
    );
    assert_eq!(gate.asked.load(SeqCst), 2);
    assert_eq!(server.requests().len(), 1, "the cache answered");
    // Naming another space than the server's fails, the cache's too.
    gate.allow.store(true, SeqCst);
    assert_eq!(
        client
            .fetch(&from_public(Some(AddressSpace::Loopback)))
            .unwrap_err(),
        NetworkError::TargetAddressSpace {
            named: AddressSpace::Loopback,
            reached: AddressSpace::Local,
        }
    );
}

/// A media element's stream (M40), not a navigation's: its head is typed
/// and checked as the whole response is before it is handed over; one
/// CORS refuses is never handed over.
#[test]
fn a_streamed_head_is_typed_and_checked_as_the_whole_response() {
    let server = TestServer::plain(|request| match request.target.as_str() {
        "/allowed" => Reply::with_fields(
            200,
            &[("Access-Control-Allow-Origin", "http://a.test")],
            b"media",
        ),
        _ => Reply::ok("video/webm", b"media"),
    });
    let of = |path: &str| {
        let mut request = Request::get(server.url(path), Destination::Video);
        request.origin = Some(url("http://a.test/").origin());
        request
    };
    let types = |request: &Request| {
        let mut heads = Vec::new();
        let result = client().fetch_streaming(request, &mut |delivery| {
            if let Delivery::Head(head) = delivery {
                heads.push(head.response_type);
            }
        });
        (result.map(|r| r.response_type), heads)
    };
    assert_eq!(
        types(&of("/plain")),
        (Ok(ResponseType::Opaque), vec![ResponseType::Opaque])
    );
    let cors = of("/allowed").with_cors(Credentials::SameOrigin);
    assert_eq!(
        types(&cors),
        (Ok(ResponseType::Cors), vec![ResponseType::Cors])
    );
    let refused = of("/plain").with_cors(Credentials::SameOrigin);
    let (result, heads) = types(&refused);
    assert!(matches!(result, Err(NetworkError::Cors(_))), "{result:?}");
    assert!(heads.is_empty(), "no head handed over");
}

/// A range is of the bytes as they are (Fetch 4.6, M40): asked for with
/// no content coding.
#[test]
fn a_ranged_request_asks_for_no_coding() {
    let server = TestServer::plain(|_| Reply::ok("video/webm", b"media"));
    let mut ranged = Request::get(server.url("/a"), Destination::Video);
    ranged.headers.append(b"Range", b"bytes=0-");
    client().fetch(&ranged).expect("fetched");
    client()
        .fetch(&Request::get(server.url("/b"), Destination::Video))
        .expect("fetched");
    let codings: Vec<String> = server
        .requests()
        .iter()
        .map(|r| r.header("Accept-Encoding").unwrap_or("").to_owned())
        .collect();
    assert_eq!(codings, ["identity", "gzip, deflate"]);
}
