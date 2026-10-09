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
    let requests = server.requests();
    let sent = &requests[0];
    assert_eq!(
        (sent.method.as_str(), sent.target.as_str()),
        ("GET", "/a?b=c")
    );
    assert_eq!(sent.header("User-Agent"), Some(USER_AGENT));
    assert!(USER_AGENT.starts_with("Mozilla/5.0 (compatible; SilverBrowser/"));
    assert_eq!(
        sent.header("Accept"),
        Some("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
    );
    assert_eq!(sent.header("Accept-Language"), Some("en-US,en;q=0.9"));
    assert_eq!(sent.header("Accept-Encoding"), Some("gzip, deflate"));
    for unsent in ["Referer", "Cookie", "Origin", "Sec-Fetch-Site"] {
        assert_eq!(sent.header(unsent), None, "{unsent}");
    }
    assert_eq!(requests[1].header("Accept"), Some("text/css,*/*;q=0.1"));
    assert_eq!(
        requests[2].header("Accept"),
        Some("image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5")
    );
}

#[test]
fn a_post_sends_its_body() {
    let server = TestServer::plain(|_| hello());
    let mut request = document(server.url("/form"));
    request.method = Method::Post;
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
        (301, Method::Post, "GET", false),
        (302, Method::Post, "GET", false),
        (303, Method::Post, "GET", false),
        (307, Method::Post, "POST", true),
        (308, Method::Post, "POST", true),
        (301, Method::Get, "GET", false),
        (302, Method::Head, "HEAD", false),
        (303, Method::Head, "HEAD", false),
        (303, Method::Get, "GET", false),
    ];
    for (status, method, after, with_body) in cases {
        let server = TestServer::plain(move |request| match request.target.as_str() {
            "/from" => redirect(status, "/to"),
            _ => hello(),
        });
        let mut request = document(server.url("/from"));
        request.method = method;
        request.body = (method == Method::Post).then(|| b"x=1".to_vec());
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
