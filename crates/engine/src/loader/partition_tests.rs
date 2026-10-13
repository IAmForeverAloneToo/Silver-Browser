//! What a frame keeps is kept apart by the top-level site it is under
//! (M33.2, `docs/design/hardening.md`, D25): its storage, its cookies'
//! same-site context and its partitioned cookies, as the reference
//! browser keeps them (p7).

use net::Url;
use net::test_server::TestServer;

use crate::testing::*;

/// A server of `pages` under two sites, `a.localhost` and `b.localhost`.
fn sites(pages: &'static [(&'static str, &'static str)]) -> TestServer {
    serve(move |path| {
        pages
            .iter()
            .find(|(at, _)| *at == path)
            .map(|(_, body)| html(body))
    })
}

fn at(server: &TestServer, host: &str, path: &str) -> Url {
    let port = server.url("/").port().expect("a port");
    Url::parse(&format!("http://{host}:{port}{path}")).expect("a URL")
}

/// Loads `url` in `tab` and runs it until the console has `count` more
/// lines: those lines.
fn lines(tab: &mut Tab, url: Url, count: usize) -> Vec<String> {
    tab.engine.navigate(url);
    tab.run_until(|lines| lines.len() >= count)
}

#[test]
fn a_third_partys_frame_keeps_apart_what_it_keeps_by_the_top_level_site() {
    static PAGES: &[(&str, &str)] = &[
        (
            "/first.html",
            "<script>\
             localStorage.x = 'first-party';\
             document.cookie = 'none1=1; SameSite=None; Secure';\
             document.cookie = 'lax1=1; SameSite=Lax';\
             document.cookie = 'part1=1; SameSite=None; Secure; Partitioned';\
             console.log('set', document.cookie);\
             </script>",
        ),
        (
            "/page.html",
            "<iframe src='http://b.localhost:PORT/frame.html'></iframe><script>\
             const f = document.querySelector('iframe');\
             f.src = f.src.replace('PORT', location.port);\
             </script>",
        ),
        (
            "/frame.html",
            "<script>\
             console.log(JSON.stringify(Object.assign({}, localStorage)), document.cookie);\
             localStorage.y = 'third-party';\
             document.cookie = 'framepart=1; SameSite=None; Secure; Partitioned';\
             document.cookie = 'framelax=1; SameSite=Lax';\
             console.log(document.cookie);\
             </script>",
        ),
        (
            "/check.html",
            "<script>\
             console.log(JSON.stringify(Object.assign({}, localStorage)), document.cookie);\
             </script>",
        ),
    ];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    let set = lines(&mut tab, at(&server, "b.localhost", "/first.html"), 1);
    assert_eq!(set, ["set none1=1; lax1=1; part1=1"]);
    let port = server.url("/").port().expect("a port");
    let framed = lines(&mut tab, at(&server, "a.localhost", "/page.html"), 2);
    let prefix = format!("[frame http://b.localhost:{port}/frame.html] ");
    assert_eq!(
        framed,
        [
            format!("{prefix}{{}} none1=1"),
            format!("{prefix}none1=1; framepart=1"),
        ]
    );
    // At the top again, b's own: its storage and cookies, none of what
    // its frame under a kept.
    let back = lines(&mut tab, at(&server, "b.localhost", "/check.html"), 1);
    assert_eq!(back, ["{\"x\":\"first-party\"} none1=1; lax1=1; part1=1"]);
}

#[test]
fn a_same_site_frame_shares_its_sites_storage_and_cookies() {
    static PAGES: &[(&str, &str)] = &[
        (
            "/first.html",
            "<script>localStorage.x = '1'; document.cookie = 'lax=1'; console.log('set')</script>",
        ),
        ("/page.html", "<iframe src=frame.html></iframe>"),
        (
            "/frame.html",
            "<script>console.log(localStorage.x, document.cookie)</script>",
        ),
    ];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    lines(&mut tab, at(&server, "a.localhost", "/first.html"), 1);
    let framed = lines(&mut tab, at(&server, "a.localhost", "/page.html"), 1);
    let port = server.url("/").port().expect("a port");
    assert_eq!(
        framed,
        [format!(
            "[frame http://a.localhost:{port}/frame.html] 1 lax=1"
        )]
    );
}

#[test]
fn a_first_party_frame_under_a_third_party_one_is_in_a_cross_site_context() {
    // a, b inside it, a inside b: the innermost is cross-site through
    // its ancestor, as the Storage Standard's ancestor bit has it.
    static PAGES: &[(&str, &str)] = &[
        (
            "/first.html",
            "<script>localStorage.x = '1'; document.cookie = 'lax=1'; \
             document.cookie = 'none=1; SameSite=None; Secure'; console.log('set')</script>",
        ),
        (
            "/page.html",
            "<iframe src='http://b.localhost:PORT/middle.html'></iframe><script>\
             const f = document.querySelector('iframe');\
             f.src = f.src.replace('PORT', location.port);\
             </script>",
        ),
        (
            "/middle.html",
            "<iframe src='http://a.localhost:PORT/inner.html'></iframe><script>\
             const f = document.querySelector('iframe');\
             f.src = f.src.replace('PORT', location.port);\
             </script>",
        ),
        (
            "/inner.html",
            "<script>console.log('inner', JSON.stringify(Object.assign({}, localStorage)), \
             document.cookie)</script>",
        ),
    ];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    lines(&mut tab, at(&server, "a.localhost", "/first.html"), 1);
    let framed = lines(&mut tab, at(&server, "a.localhost", "/page.html"), 1);
    assert_eq!(framed.len(), 1);
    assert!(framed[0].ends_with("inner {} none=1"), "{framed:?}");
}

#[test]
fn a_frames_navigation_is_cross_site_through_its_ancestors() {
    // b inside b inside a: the inner frame's request, of b's site from
    // a document of b's, is still made in a cross-site context.
    static PAGES: &[(&str, &str)] = &[
        (
            "/first.html",
            "<script>document.cookie = 'lax=1'; document.cookie = \
             'none=1; SameSite=None; Secure'; console.log('set')</script>",
        ),
        (
            "/page.html",
            "<iframe src='http://b.localhost:PORT/middle.html'></iframe><script>\
             const f = document.querySelector('iframe');\
             f.src = f.src.replace('PORT', location.port);\
             </script>",
        ),
        ("/middle.html", "<iframe src=leaf.html></iframe>"),
        ("/leaf.html", "<script>console.log('leaf')</script>"),
    ];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    lines(&mut tab, at(&server, "b.localhost", "/first.html"), 1);
    lines(&mut tab, at(&server, "a.localhost", "/page.html"), 1);
    let leaf = server
        .requests()
        .into_iter()
        .find(|request| request.target == "/leaf.html")
        .expect("the leaf fetched");
    assert_eq!(leaf.header("Cookie"), Some("none=1"));
}

#[test]
fn a_cross_site_frames_requests_carry_no_lax_cookie() {
    // Its navigation and its subresources, in a cross-site context.
    static PAGES: &[(&str, &str)] = &[
        (
            "/first.html",
            "<script>document.cookie = 'lax=1'; document.cookie = \
             'none=1; SameSite=None; Secure'; console.log('set')</script>",
        ),
        (
            "/page.html",
            "<iframe src='http://b.localhost:PORT/frame.html'></iframe><script>\
             const f = document.querySelector('iframe');\
             f.src = f.src.replace('PORT', location.port);\
             </script>",
        ),
        (
            "/frame.html",
            "<script>fetch('/sub').then(() => location.href = '/next.html', \
             () => console.log('failed'))</script>",
        ),
        ("/sub", "sub"),
        ("/next.html", "<script>console.log('next')</script>"),
    ];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    lines(&mut tab, at(&server, "b.localhost", "/first.html"), 1);
    lines(&mut tab, at(&server, "a.localhost", "/page.html"), 1);
    let cookie_of = |path: &str| {
        server
            .requests()
            .into_iter()
            .find(|request| request.target == path)
            .and_then(|request| request.header("Cookie").map(str::to_owned))
    };
    assert_eq!(cookie_of("/frame.html").as_deref(), Some("none=1"));
    assert_eq!(cookie_of("/sub").as_deref(), Some("none=1"));
    // Its own navigation, of its site to its site.
    assert_eq!(cookie_of("/next.html").as_deref(), Some("none=1"));
}

#[test]
fn a_cross_site_frames_workers_and_synchronous_requests_are_in_its_partition() {
    let server = serve(|path| match path {
        "/first.html" => Some(html(
            "<script>document.cookie = 'lax=1'; console.log('set')</script>",
        )),
        "/page.html" => Some(html(
            "<iframe src='http://b.localhost:PORT/frame.html'></iframe><script>\
             const f = document.querySelector('iframe');\
             f.src = f.src.replace('PORT', location.port);\
             </script>",
        )),
        "/frame.html" => Some(html(
            "<script>\
             const x = new XMLHttpRequest(); x.open('GET', '/sync', false); x.send();\
             new Worker('/worker.js').onmessage = e => console.log(e.data);\
             </script>",
        )),
        "/worker.js" => Some(net::test_server::Reply::ok(
            "text/javascript",
            b"fetch('/from-worker').then(() => postMessage('done'))",
        )),
        "/sync" | "/from-worker" => Some(html("ok")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    lines(&mut tab, at(&server, "b.localhost", "/first.html"), 1);
    lines(&mut tab, at(&server, "a.localhost", "/page.html"), 1);
    let cookie_of = |path: &str| {
        server
            .requests()
            .into_iter()
            .find(|request| request.target == path)
            .map(|request| request.header("Cookie").map(str::to_owned))
    };
    assert_eq!(
        cookie_of("/sync"),
        Some(None),
        "no Lax cookie, synchronously"
    );
    assert_eq!(
        cookie_of("/worker.js"),
        Some(None),
        "nor for its worker's script"
    );
    assert_eq!(cookie_of("/from-worker"), Some(None), "nor from its worker");
}
