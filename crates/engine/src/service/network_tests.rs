//! The network service through a renderer's link (M34.1, D31): fetches,
//! streamed and whole, cookies, cancellation, violations, and every check
//! a renderer's word meets.

use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use net::cookies::Context;
use net::test_server::{Reply, TestServer};
use net::{
    Client, Delivery, Destination, NetworkError, Origin, Partition, Referrer, Request,
    ResponseType, Url,
};

use super::network::{FrameGrant, Grants, Lock, NetworkService};
use crate::ipc::{self, Sender, ToBrowser, ToRenderer};
use crate::loader::{Cookies, Fetcher};
use crate::renderer::link::{Link, RemoteCookies, RemoteFetcher};

const WAIT: Duration = Duration::from_secs(20);

/// A body longer than the longest message.
const BIG: usize = ipc::MAX_MESSAGE + (1 << 20);

/// A renderer's link to a browser's network service over pipes in memory,
/// renderer 1 locked to `lock`, its frame 7 of `grant`; and where the
/// browser says why it would kill the renderer.
struct Wired {
    link: Arc<Link>,
    grants: Arc<Grants>,
    service: Arc<NetworkService>,
    bad: mpsc::Receiver<String>,
}

fn wired(lock: Lock, grant: FrameGrant) -> Wired {
    let grants = Grants::new();
    assert!(grants.set_lock(1, lock));
    grants.grant_frame(1, 7, grant);
    let service = NetworkService::new(Client::default(), grants.clone());
    let ((browser_in, browser_out), (renderer_in, renderer_out)) = ipc::duplex();
    let reply = Sender::<ToRenderer>::new(browser_out);
    let (said, bad) = mpsc::channel();
    let serving = service.clone();
    ipc::spawn_reader::<ToBrowser, _>(
        "test-browser",
        browser_in,
        move |message| match serving.serve(1, message, &reply) {
            Ok(None) => {}
            Ok(Some(other)) => panic!("not the network's: {other:?}"),
            Err(why) => {
                let _ = said.send(why);
            }
        },
        |_| {},
    )
    .expect("a thread");
    let link = Link::new(Sender::new(renderer_out));
    let routing = link.clone();
    ipc::spawn_reader::<ToRenderer, _>(
        "test-renderer",
        renderer_in,
        move |message| {
            let left = routing.route(message).expect("routed");
            assert!(left.is_none(), "{left:?}");
        },
        |_| {},
    )
    .expect("a thread");
    Wired {
        link,
        grants,
        service,
        bad,
    }
}

fn site_of(server: &TestServer) -> Lock {
    Lock::of(&server.url("/"))
}

fn top(server: &TestServer) -> FrameGrant {
    FrameGrant {
        top: server.url("/").origin(),
        cross_site: false,
    }
}

/// A request of the page at `server`'s root for `url`, as a document
/// asks: of its origin.
fn of_page(server: &TestServer, url: Url, destination: Destination) -> Request {
    let mut request = Request::get(url, destination);
    request.origin = Some(server.url("/").origin());
    request
}

fn serve(script: impl Fn(&str) -> Option<Reply> + Send + Sync + 'static) -> TestServer {
    TestServer::plain(move |request| {
        script(&request.target).unwrap_or_else(|| Reply::with_fields(404, &[], b""))
    })
}

#[test]
fn a_renderer_fetches_through_the_browser() {
    let server = serve(|path| match path {
        "/a.css" => Some(Reply::with_fields(
            200,
            &[("Content-Type", "text/css"), ("Set-Cookie", "s=1")],
            b"p { color: red }",
        )),
        "/big" => Some(Reply::ok("text/plain", &vec![b'x'; BIG])),
        _ => None,
    });
    let wired = wired(site_of(&server), top(&server));
    let fetcher = RemoteFetcher::new(wired.link.clone(), 7);
    let mut request = Request::get(server.url("/a.css"), Destination::Style);
    request.origin = Some(server.url("/").origin());
    let response = fetcher.fetch(&request).expect("fetched");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"p { color: red }");
    assert_eq!(response.headers.get(b"Set-Cookie"), None, "the jar's alone");
    // The jar kept it, in the browser.
    let kept = wired.service.jar().lock().expect("a jar").header(
        &server.url("/"),
        net::cookies::Api::Http,
        Context::SAME_SITE,
        std::time::SystemTime::now(),
    );
    assert_eq!(kept.as_deref(), Some(&b"s=1"[..]));

    // A body longer than a message comes in pieces, whole or streamed
    // (a navigation's).
    let big = of_page(&server, server.url("/big"), Destination::Empty);
    assert_eq!(fetcher.fetch(&big).expect("fetched").body.len(), BIG);
    let mut heads = 0;
    let mut pieces = Vec::new();
    let navigation = Request::get(server.url("/big"), Destination::Document);
    let streamed = fetcher
        .fetch_streaming(&navigation, &mut |delivery| match delivery {
            Delivery::Head(head) => {
                heads += 1;
                assert_eq!(head.status, 200);
            }
            Delivery::Body(bytes) => pieces.push(bytes.len()),
        })
        .expect("fetched");
    assert_eq!(heads, 1);
    assert_eq!(pieces.iter().sum::<usize>(), BIG);
    assert!(streamed.body.is_empty());
    assert!(wired.bad.try_recv().is_err());
}

#[test]
fn document_cookie_is_read_and_written_in_the_browser() {
    let server = serve(|_| None);
    let wired = wired(site_of(&server), top(&server));
    let cookies = RemoteCookies::new(wired.link.clone(), 7);
    let url = server.url("/page.html");
    cookies.set_script_cookie(b"a=1", &url, Context::SAME_SITE);
    cookies.set_script_cookie(b"b=2; HttpOnly", &url, Context::SAME_SITE);
    assert_eq!(
        cookies.script_cookies(&url, Context::SAME_SITE).as_deref(),
        Some(&b"a=1"[..])
    );
    assert!(wired.bad.try_recv().is_err());
}

#[test]
fn a_fetch_is_cancelled_when_it_is_aborted() {
    let (release, released) = mpsc::channel::<()>();
    let released = Arc::new(Mutex::new(released));
    let server = TestServer::plain(move |_| {
        let _ = released
            .lock()
            .expect("held")
            .recv_timeout(Duration::from_secs(30));
        Reply::ok("text/plain", b"late")
    });
    let wired = wired(site_of(&server), top(&server));
    let fetcher = RemoteFetcher::new(wired.link.clone(), 7);
    let request = of_page(&server, server.url("/slow"), Destination::Empty);
    let abort = request.abort.clone();
    let fetching = thread::spawn(move || fetcher.fetch(&request).map(|r| r.status));
    thread::sleep(Duration::from_millis(100));
    abort.abort();
    let result = fetching.join().expect("joined");
    assert!(matches!(result, Err(NetworkError::Aborted)), "{result:?}");
    // The browser was told, and let go of it.
    let began = std::time::Instant::now();
    while wired.service.in_flight(1) > 0 && began.elapsed() < WAIT {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(wired.service.in_flight(1), 0);
    drop(release);
}

#[test]
fn a_fetch_numbered_again_while_in_flight_kills_the_renderer() {
    let (release, released) = mpsc::channel::<()>();
    let released = Arc::new(Mutex::new(released));
    let server = TestServer::plain(move |_| {
        let _ = released
            .lock()
            .expect("held")
            .recv_timeout(Duration::from_secs(30));
        Reply::ok("text/plain", b"late")
    });
    let wired = wired(site_of(&server), top(&server));
    for _ in 0..2 {
        wired
            .link
            .send(&ToBrowser::Fetch {
                id: 5,
                frame: 7,
                streamed: false,
                request: Box::new(of_page(&server, server.url("/slow"), Destination::Empty)),
            })
            .expect("sent");
    }
    assert_eq!(killed_for(&wired), "fetch 5 again");
    drop(release);
}

#[test]
fn a_policys_violations_come_back_to_the_renderer() {
    let server = serve(|_| Some(Reply::ok("text/plain", b"x")));
    let wired = wired(site_of(&server), top(&server));
    let fetcher = RemoteFetcher::new(wired.link.clone(), 7);
    let page = server.url("/");
    let mut headers = net::HeaderList::new();
    headers.append(b"Content-Security-Policy", b"connect-src 'none'");
    let csp = net::csp::Csp::from_headers(&headers, &page).with_sink();
    let request = of_page(&server, server.url("/data"), Destination::Empty).with_csp(csp.clone());
    let result = fetcher.fetch(&request);
    assert!(matches!(result, Err(NetworkError::Csp(_))), "{result:?}");
    let violations = csp.take_violations();
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert_eq!(violations[0].directive, "connect-src");
}

#[test]
fn an_opaque_response_keeps_its_body_only_if_blocking_lets_it() {
    let server = serve(|path| match path {
        "/page.html" => Some(Reply::ok("text/html", b"<!doctype html><p>secret")),
        "/data.json" => Some(Reply::ok("application/json", b"{\"secret\": 1}")),
        "/script.js" => Some(Reply::ok("text/javascript", b"var x = 1;")),
        "/pixel" => Some(Reply::ok("image/png", b"\x89PNG\r\n\x1a\nrest")),
        _ => None,
    });
    // The renderer is of another site than the server's: its requests are
    // no-cors and cross-origin.
    let other = Url::parse("http://other.test/").expect("a URL");
    let wired = wired(
        Lock::of(&other),
        FrameGrant {
            top: other.origin(),
            cross_site: false,
        },
    );
    let fetcher = RemoteFetcher::new(wired.link.clone(), 7);
    let get = |path: &str, destination| {
        let mut request = Request::get(server.url(path), destination);
        request.origin = Some(other.origin());
        fetcher.fetch(&request).expect("fetched")
    };
    let page = get("/page.html", Destination::Image);
    assert_eq!(page.response_type, ResponseType::Opaque);
    assert!(page.body.is_empty(), "HTML blocked");
    assert_eq!(page.headers.len(), 1, "its type alone");
    assert!(get("/data.json", Destination::Script).body.is_empty());
    assert_eq!(get("/script.js", Destination::Script).body, b"var x = 1;");
    assert_eq!(get("/pixel", Destination::Image).body.len(), 12);
    assert!(wired.bad.try_recv().is_err());
}

/// The reason the browser gave for killing the renderer.
fn killed_for(wired: &Wired) -> String {
    wired.bad.recv_timeout(WAIT).expect("a bad message")
}

#[test]
fn what_a_renderer_may_not_ask_for_kills_it() {
    let server = serve(|_| Some(Reply::ok("text/plain", b"x")));
    let other = Url::parse("http://victim.test/").expect("a URL");
    let wired = wired(site_of(&server), top(&server));
    let send = |message: ToBrowser| wired.link.send(&message).expect("sent");
    let request = |url: Url| of_page(&server, url, Destination::Empty);

    send(ToBrowser::Fetch {
        id: 1,
        frame: 8,
        streamed: false,
        request: Box::new(request(server.url("/"))),
    });
    assert_eq!(killed_for(&wired), "frame 8 is not renderer 1's");

    let mut claimed = request(other.clone());
    claimed.origin = Some(other.origin());
    send(ToBrowser::Fetch {
        id: 2,
        frame: 7,
        streamed: false,
        request: Box::new(claimed),
    });
    assert_eq!(killed_for(&wired), "a request of http://victim.test");

    let mut partitioned = request(server.url("/"));
    partitioned.partition = Some(Partition {
        top: other.origin(),
        cross_site: true,
    });
    send(ToBrowser::Fetch {
        id: 3,
        frame: 7,
        streamed: false,
        request: Box::new(partitioned),
    });
    assert_eq!(
        killed_for(&wired),
        "a request in the partition of http://victim.test"
    );

    let mut initiated = request(server.url("/"));
    initiated.initiator = Some(net::Initiator {
        url: other.clone(),
        address_space: net::AddressSpace::Public,
        origin: other.origin(),
    });
    send(ToBrowser::Fetch {
        id: 4,
        frame: 7,
        streamed: false,
        request: Box::new(initiated),
    });
    assert_eq!(
        killed_for(&wired),
        "a request initiated by http://victim.test"
    );

    let mut asking = request(server.url("/"));
    asking.local_network = Some(net::LocalNetworkAccess {
        origin: other.origin(),
        local: true,
        loopback: true,
    });
    send(ToBrowser::Fetch {
        id: 5,
        frame: 7,
        streamed: false,
        request: Box::new(asking),
    });
    assert_eq!(
        killed_for(&wired),
        "local network access for http://victim.test"
    );

    // No origin, or a navigation's mode, is the user's own load, which
    // the fetch would not filter; and only a navigation is streamed.
    send(ToBrowser::Fetch {
        id: 6,
        frame: 7,
        streamed: false,
        request: Box::new(Request::get(other.clone(), Destination::Empty)),
    });
    assert_eq!(killed_for(&wired), "a request of no origin");
    let mut navigating = request(other.clone());
    navigating.mode = net::Mode::Navigate;
    send(ToBrowser::Fetch {
        id: 7,
        frame: 7,
        streamed: true,
        request: Box::new(navigating),
    });
    assert_eq!(killed_for(&wired), "a fetch in a navigation's mode");
    send(ToBrowser::Fetch {
        id: 8,
        frame: 7,
        streamed: true,
        request: Box::new(request(server.url("/"))),
    });
    assert_eq!(
        killed_for(&wired),
        "a stream that is not a navigation's, or the reverse"
    );
    send(ToBrowser::Fetch {
        id: 9,
        frame: 7,
        streamed: false,
        request: Box::new(Request::get(server.url("/"), Destination::Document)),
    });
    assert_eq!(
        killed_for(&wired),
        "a stream that is not a navigation's, or the reverse"
    );

    send(ToBrowser::Cookies {
        call: 1,
        frame: 7,
        url: other.clone(),
        context: Context::SAME_SITE,
    });
    assert_eq!(
        killed_for(&wired),
        "the cookies of http://victim.test/, not of its site"
    );
    send(ToBrowser::SetCookie {
        frame: 7,
        url: other,
        value: ipc::Bytes(b"a=1".to_vec()),
        context: Context::SAME_SITE,
    });
    assert_eq!(
        killed_for(&wired),
        "the cookies of http://victim.test/, not of its site"
    );
    assert!(wired.grants.set_lock(1, site_of(&server)), "the lock stays");
    assert!(!wired.grants.set_lock(1, Lock::File), "and is not changed");
}

#[test]
fn the_browser_decides_the_partition_and_drops_a_referrer_of_another_site() {
    let server = serve(|_| Some(Reply::ok("text/plain", b"x")));
    let top_url = Url::parse("http://top.test/").expect("a URL");
    // Frame 7 is of the server's site, framed under another.
    let wired = wired(
        site_of(&server),
        FrameGrant {
            top: top_url.origin(),
            cross_site: true,
        },
    );
    let fetcher = RemoteFetcher::new(wired.link.clone(), 7);
    let mut request = of_page(&server, server.url("/x"), Destination::Empty);
    request.partition = Some(Partition {
        top: top_url.origin(),
        cross_site: false,
    });
    request.referrer = Referrer::Url(Box::new(
        Url::parse("http://victim.test/secret").expect("a URL"),
    ));
    let response = fetcher.fetch(&request).expect("fetched");
    assert_eq!(response.status, 200);
    let sent = server.requests();
    assert_eq!(sent[0].header("Referer"), None, "the referrer dropped");
    assert!(wired.bad.try_recv().is_err());
    let file = of_page(
        &server,
        Url::parse("file:///C:/Windows/win.ini").expect("a URL"),
        Destination::Empty,
    );
    assert_eq!(
        fetcher.fetch(&file).map(|r| r.status),
        Err(NetworkError::Refused("file".into()))
    );
    let _ = Origin::new_opaque();
}

#[test]
fn an_engine_fetches_and_reaches_its_cookies_through_the_browser() {
    let server = serve(|path| match path {
        "/page.html" => Some(Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Set-Cookie", "a=1"),
                ("Set-Cookie", "h=secret; HttpOnly"),
            ],
            b"<script>console.log(document.cookie, navigator.cookieEnabled); \
              document.cookie = 'b=2; Path=/'; document.cookie = 'h=evil'; \
              document.cookie = 'x=1; HttpOnly'; document.cookie = 'a=; Max-Age=0'; \
              console.log(document.cookie)</script>",
        )),
        "/next.html" => Some(Reply::ok("text/html", b"<p>next</p>")),
        _ => None,
    });
    let wired = wired(site_of(&server), top(&server));
    let mut tab = crate::testing::Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_remote_network(
        Arc::new(RemoteFetcher::new(wired.link.clone(), 7)),
        Arc::new(RemoteCookies::new(wired.link.clone(), 7)),
    );
    tab.engine.navigate(server.url("/page.html"));
    let lines = tab.run_until(|lines| lines.len() >= 2);
    assert_eq!(lines, ["a=1 true", "b=2"], "as in one process");
    let next = server.url("/next.html");
    tab.engine.navigate(next.clone());
    tab.wait_until(|engine| {
        engine.url() == Some(&next) && engine.load_state() == &crate::LoadState::Loaded
    });
    let sent = server
        .requests()
        .into_iter()
        .rev()
        .find(|request| request.target == "/next.html")
        .and_then(|request| request.header("Cookie").map(str::to_owned));
    assert_eq!(sent.as_deref(), Some("h=secret; b=2"));
    assert!(wired.bad.try_recv().is_err());
}

#[test]
fn a_framed_documents_cookies_are_cross_site_whatever_the_renderer_says() {
    let server = serve(|_| None);
    let top_url = Url::parse("http://top.test/").expect("a URL");
    let wired = wired(
        site_of(&server),
        FrameGrant {
            top: top_url.origin(),
            cross_site: true,
        },
    );
    let url = server.url("/page.html");
    {
        let mut jar = wired.service.jar().lock().expect("a jar");
        let now = std::time::SystemTime::now();
        for cookie in ["lax=1; SameSite=Lax", "none=1; SameSite=None; Secure"] {
            jar.store(
                cookie.as_bytes(),
                &url,
                net::cookies::Api::Http,
                Context::SAME_SITE,
                now,
            );
        }
    }
    let cookies = RemoteCookies::new(wired.link.clone(), 7);
    // The renderer claims a same-site context: the browser knows better.
    let seen = cookies.script_cookies(&url, Context::SAME_SITE);
    assert!(
        !seen.as_deref().unwrap_or_default().starts_with(b"lax"),
        "{seen:?}"
    );
    assert!(wired.bad.try_recv().is_err());
}
