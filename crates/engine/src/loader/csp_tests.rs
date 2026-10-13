//! Content Security Policy's fetch directives (M33.1,
//! `docs/design/hardening.md`): every request a document makes held to
//! its destination's directive, report-only policies reporting, every
//! kind of document with its policies, a frame's navigations held to its
//! container's `frame-src`, a worker to `worker-src` and its own.

use net::Url;
use net::test_server::{Reply, TestServer};

use crate::testing::*;

/// Pages by path, each with its header fields and body.
type Pages = &'static [(
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
)];

/// A server of `pages` under two sites, `a.localhost` and `b.localhost`,
/// a query left aside; a body's `PORT` is the server's port.
fn sites(pages: Pages) -> TestServer {
    let port = std::sync::Arc::new(std::sync::OnceLock::<u16>::new());
    let seen = port.clone();
    let server = serve(move |target| {
        let path = target.split('?').next().unwrap_or_default();
        let (_, fields, body) = pages.iter().find(|(at, ..)| *at == path)?;
        let mut all = vec![("Content-Type", "text/html")];
        all.extend_from_slice(fields);
        let port = seen.get().map(u16::to_string).unwrap_or_default();
        let body = body.replace("PORT", &port);
        Some(Reply::with_fields(200, &all, body.as_bytes()))
    });
    let _ = port.set(server.url("/").port().expect("a port"));
    server
}

fn at(server: &TestServer, host: &str, path: &str) -> Url {
    let port = server.url("/").port().expect("a port");
    Url::parse(&format!("http://{host}:{port}{path}")).expect("a URL")
}

/// A tab with scripts on, `url` loaded, run until a line of the console
/// ends with `last`: the lines, the port written `PORT`, in order.
fn run_to(server: &TestServer, url: Url, last: &str) -> Vec<String> {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(url);
    let port = server.url("/").port().expect("a port").to_string();
    let mut lines: Vec<String> = tab
        .run_until(|lines| lines.iter().any(|line| line.ends_with(last)))
        .into_iter()
        .map(|line| line.replace(&port, "PORT"))
        .collect();
    lines.sort();
    lines
}

/// The paths of `server`'s requests.
fn asked(server: &TestServer) -> Vec<String> {
    server
        .requests()
        .into_iter()
        .map(|request| request.target)
        .collect()
}

#[test]
fn each_request_is_held_to_its_destinations_directive() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[(
                "Content-Security-Policy",
                "default-src 'self'; img-src 'none'; connect-src 'none'; \
                 script-src 'self' 'unsafe-inline'",
            )],
            "<link rel=stylesheet href='http://b.localhost:PORT/s.css'>\
             <link rel=stylesheet href=/own.css>\
             <img src=/i.png onerror=\"console.log('image error')\">\
             <iframe src='http://b.localhost:PORT/f.html' onload=\"console.log('frame load')\">\
             </iframe>\
             <script>\
             onload = () => fetch('/data').catch(e => console.log('fetch', e.name))\
               .then(() => console.log('loaded'));\
             </script>",
        ),
        (
            "/own.css",
            &[("Content-Type", "text/css")],
            "p { color: red }",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_to(&server, at(&server, "a.localhost", "/page.html"), "loaded");
    let refused = |what: &str, directive: &str| {
        format!("Content-Security-Policy refused {what}: {directive}")
    };
    assert_eq!(
        lines,
        [
            refused(
                "the connection to http://a.localhost:PORT/data",
                "connect-src 'none'"
            ),
            refused(
                "the frame http://b.localhost:PORT/f.html",
                "default-src 'self'"
            ),
            refused(
                "the image http://a.localhost:PORT/favicon.ico",
                "img-src 'none'"
            ),
            refused("the image http://a.localhost:PORT/i.png", "img-src 'none'"),
            refused(
                "the stylesheet http://b.localhost:PORT/s.css",
                "default-src 'self'"
            ),
            "Failed to fetch http://a.localhost:PORT/data: blocked by \
             Content-Security-Policy: connect-src 'none'"
                .to_owned(),
            "fetch TypeError".to_owned(),
            "frame load".to_owned(),
            "image error".to_owned(),
            "loaded".to_owned(),
        ]
    );
    // None of them was asked for.
    let mut paths = asked(&server);
    paths.sort();
    assert_eq!(paths, ["/own.css", "/page.html"]);
}

#[test]
fn a_report_only_policy_reports_and_lets_everything_through() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[(
                "Content-Security-Policy-Report-Only",
                "img-src 'none'; script-src 'none'",
            )],
            "<img src=/i.png onerror=\"console.log('image done')\">\
             <script>console.log('ran'); onload = () => console.log('loaded')</script>",
        ),
        ("/i.png", &[("Content-Type", "image/png")], "not a png"),
    ];
    let server = sites(PAGES);
    let lines = run_to(&server, at(&server, "a.localhost", "/page.html"), "loaded");
    assert_eq!(
        lines,
        [
            "Content-Security-Policy (report-only) would refuse an inline script: \
             script-src 'none'",
            "Content-Security-Policy (report-only) would refuse the image \
             http://a.localhost:PORT/favicon.ico: img-src 'none'",
            "Content-Security-Policy (report-only) would refuse the image \
             http://a.localhost:PORT/i.png: img-src 'none'",
            "image done",
            "loaded",
            "ran",
        ]
    );
    assert!(asked(&server).contains(&"/i.png".to_owned()));
}

#[test]
fn nonces_and_strict_dynamic_decide_external_scripts_and_stylesheets() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[(
                "Content-Security-Policy",
                "script-src 'nonce-abc' 'strict-dynamic'; style-src 'nonce-xyz'",
            )],
            "<link rel=stylesheet nonce=xyz href=/a.css>\
             <link rel=stylesheet href=/b.css>\
             <script nonce=abc src=/a.js></script>\
             <script src=/b.js></script>",
        ),
        ("/a.css", &[("Content-Type", "text/css")], ""),
        ("/b.css", &[("Content-Type", "text/css")], ""),
        (
            "/a.js",
            &[("Content-Type", "text/javascript")],
            "console.log('a');\
             const s = document.createElement('script');\
             s.src = '/c.js'; document.head.append(s);",
        ),
        (
            "/b.js",
            &[("Content-Type", "text/javascript")],
            "console.log('b')",
        ),
        (
            "/c.js",
            &[("Content-Type", "text/javascript")],
            "console.log('c')",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_to(&server, at(&server, "a.localhost", "/page.html"), "c");
    assert_eq!(
        lines,
        [
            "Content-Security-Policy refused the script http://a.localhost:PORT/b.js: \
             script-src 'nonce-abc' 'strict-dynamic'",
            "Content-Security-Policy refused the stylesheet http://a.localhost:PORT/b.css: \
             style-src 'nonce-xyz'",
            "a",
            "c",
        ]
    );
}

#[test]
fn every_kind_of_document_and_a_javascript_urls_has_its_policies() {
    static PAGES: Pages = &[
        (
            "/image.svg",
            &[
                ("Content-Type", "image/svg+xml"),
                ("Content-Security-Policy", "script-src 'none'"),
            ],
            "<svg xmlns='http://www.w3.org/2000/svg'><script>console.log('ran')</script>\
             <image href='/after.png'/></svg>",
        ),
        (
            "/page.html",
            &[(
                "Content-Security-Policy",
                "script-src 'unsafe-inline'; img-src 'none'",
            )],
            "<a href=\"javascript:'<img src=/i.png><script>console.log(1)</script>'\">go</a>\
             <script>onload = () => document.querySelector('a').click()</script>",
        ),
    ];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.load(at(&server, "a.localhost", "/image.svg"));
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(
        lines,
        ["Content-Security-Policy refused an inline script: script-src 'none'"]
    );
    let lines = run_to(
        &server,
        at(&server, "a.localhost", "/page.html"),
        "i.png: img-src 'none'",
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some(
            "Content-Security-Policy refused the image http://a.localhost:PORT/i.png: \
             img-src 'none'"
        )
    );
    assert!(lines.contains(&"1".to_owned()));
    assert!(!asked(&server).contains(&"/i.png".to_owned()));
}

#[test]
fn a_frames_navigations_are_held_to_its_containers_frame_src() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[("Content-Security-Policy", "frame-src 'self'")],
            "<iframe src=/own.html></iframe>",
        ),
        (
            "/own.html",
            &[],
            "<script>console.log('own');\
             onload = () => location.href = 'http://b.localhost:PORT/away.html'</script>",
        ),
        ("/away.html", &[], "<script>console.log('away')</script>"),
    ];
    let server = sites(PAGES);
    let lines = run_to(&server, at(&server, "a.localhost", "/page.html"), "'self'");
    assert_eq!(
        lines,
        [
            "Content-Security-Policy refused the frame http://b.localhost:PORT/away.html: \
             frame-src 'self'",
            "[frame http://a.localhost:PORT/own.html] own",
        ]
    );
    assert!(!asked(&server).contains(&"/away.html".to_owned()));
}

#[test]
fn a_worker_is_held_to_worker_src_and_its_own_policies() {
    static PAGES: Pages = &[
        (
            "/refused.html",
            &[("Content-Security-Policy", "worker-src 'none'")],
            "<script>new Worker('/w.js').onerror = () => console.log('worker error')</script>",
        ),
        (
            "/page.html",
            &[],
            "<script>new Worker('/w.js').onmessage = e => console.log(e.data)</script>",
        ),
        (
            "/w.js",
            &[
                ("Content-Type", "text/javascript"),
                ("Content-Security-Policy", "connect-src 'none'"),
            ],
            "fetch('/data').then(() => postMessage('fetched'), e => postMessage(e.name))",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_to(
        &server,
        at(&server, "a.localhost", "/refused.html"),
        "worker error",
    );
    assert_eq!(
        lines,
        [
            "Content-Security-Policy refused the worker http://a.localhost:PORT/w.js: \
             worker-src 'none'",
            "worker error",
        ]
    );
    let lines = run_to(
        &server,
        at(&server, "a.localhost", "/page.html"),
        "TypeError",
    );
    assert!(
        lines.contains(
            &"Content-Security-Policy refused the connection to \
              http://a.localhost:PORT/data: connect-src 'none'"
                .to_owned()
        ),
        "{lines:?}"
    );
    assert!(!asked(&server).contains(&"/data".to_owned()));
}

#[test]
fn a_meta_policy_holds_for_a_scripts_requests_before_the_document_is_shown() {
    static PAGES: Pages = &[(
        "/page.html",
        &[],
        "<head><meta http-equiv=Content-Security-Policy content=\"connect-src 'none'\">\
         <script>\
         const x = new XMLHttpRequest(); x.open('GET', '/sync', false);\
         try { x.send() } catch (e) { console.log('xhr', e.name) }\
         fetch('/data').catch(e => console.log('fetch', e.name));\
         </script>",
    )];
    let server = sites(PAGES);
    let lines = run_to(
        &server,
        at(&server, "a.localhost", "/page.html"),
        "fetch TypeError",
    );
    let refused = |path: &str| {
        format!(
            "Content-Security-Policy refused the connection to http://a.localhost:PORT/{path}: \
             connect-src 'none'"
        )
    };
    assert!(lines.contains(&refused("sync")), "{lines:?}");
    assert!(lines.contains(&refused("data")), "{lines:?}");
    assert!(lines.contains(&"xhr NetworkError".to_owned()), "{lines:?}");
    let paths = asked(&server);
    assert!(!paths.contains(&"/sync".to_owned()) && !paths.contains(&"/data".to_owned()));
}
