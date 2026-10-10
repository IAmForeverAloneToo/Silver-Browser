//! Mixed content for what scripts fetch (M27.5, docs/design/fetch.md):
//! an `https:` document's `fetch()` and `XMLHttpRequest` of an `http:`
//! URL that is not potentially trustworthy are blocked, on every hop;
//! the page learns only that they failed, and the console says why.

use std::time::{Duration, Instant};

use net::test_server::{Reply, ServerRequest, TestAuthority, TestServer, VALID};
use net::transport::Config;

use super::tests::load_all;
use crate::testing::*;

/// Runs the engine's tasks as results come until `count` lines are
/// logged, or fails after ten seconds.
fn logged(tab: &mut Tab, count: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        tab.engine.poll_network();
        tab.engine.run_tasks();
        lines.extend(
            tab.engine
                .take_console()
                .into_iter()
                .map(|entry| entry.text),
        );
        if lines.len() >= count {
            return lines;
        }
        assert!(Instant::now() < deadline, "logged only {lines:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A page whose script is `script`, served over TLS at `/page.html`
/// with the server's other paths, loaded with scripting on by a tab
/// whose client trusts the server's root.
fn secure_page_with(
    script: &str,
    route: impl Fn(&ServerRequest) -> Option<Reply> + Send + Sync + 'static,
) -> (Tab, TestServer) {
    let authority = TestAuthority::new();
    let page = format!("<script>{script}</script>");
    let server = TestServer::tls(authority.server(&["127.0.0.1"], VALID), move |request| {
        if request.target == "/page.html" {
            return Reply::ok("text/html", page.as_bytes());
        }
        route(request)
            .unwrap_or_else(|| Reply::with_fields(404, &[("Content-Type", "text/plain")], b"no"))
    });
    let mut tab = Tab::new();
    tab.engine.set_network(net::Client::new(
        Config::default().trust(authority.root.clone()),
    ));
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    (tab, server)
}

#[test]
fn an_https_pages_scripts_may_not_fetch_over_http() {
    let plain = TestServer::plain(|_: &ServerRequest| {
        Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/plain"),
                ("Access-Control-Allow-Origin", "*"),
            ],
            b"plain",
        )
    });
    // Blocked before the name is looked up: example.invalid never is.
    let script = format!(
        "fetch('http://example.invalid/a').catch(e => console.log('fetch', e.name, e.message));\
         const x = new XMLHttpRequest(); x.open('GET', 'http://example.invalid/b');\
         x.onerror = () => console.log('xhr', x.status, x.readyState); x.send();\
         fetch('/away').catch(e => console.log('redirected', e.message));\
         fetch('{loopback}').then(r => r.text()).then(t => console.log('loopback', t));\
         fetch('http://localhost:{port}/h').then(r => r.text()).then(t => console.log('localhost', t));\
         const s = new XMLHttpRequest(); s.open('GET', 'http://example.invalid/c', false);\
         try {{ s.send() }} catch (e) {{ console.log('sync', e.name) }}",
        loopback = plain.url("/l"),
        port = plain.port(),
    );
    let (mut tab, server) = secure_page_with(&script, |request| match request.target.as_str() {
        "/away" => Some(Reply::with_fields(
            302,
            &[("Location", "http://example.invalid/d")],
            b"",
        )),
        _ => None,
    });
    assert!(tab.engine.url().is_some_and(|url| url.scheme() == "https"));
    let mut lines = logged(&mut tab, 10);
    lines.sort();
    // The redirect's hop is blocked too, and said of the request.
    let blocked = "an http: request from an https: page";
    assert_eq!(
        lines,
        [
            format!("Failed to fetch http://example.invalid/a: {blocked}"),
            format!("Failed to fetch http://example.invalid/b: {blocked}"),
            format!("Failed to fetch http://example.invalid/c: {blocked}"),
            format!("Failed to fetch {}: {blocked}", server.url("/away")),
            "fetch TypeError Failed to fetch".to_owned(),
            "localhost plain".to_owned(),
            "loopback plain".to_owned(),
            "redirected Failed to fetch".to_owned(),
            "sync NetworkError".to_owned(),
            "xhr 0 4".to_owned(),
        ]
    );
}

#[test]
fn an_http_pages_scripts_fetch_what_an_https_page_may_not() {
    // The same fetch from an http: page is not mixed content: it fails
    // only for want of the name, and the console says so.
    let plain = TestServer::plain(|request: &ServerRequest| {
        match request.target.as_str() {
        "/page.html" => Reply::ok(
            "text/html",
            b"<script>fetch('http://nowhere.invalid/a').catch(e => console.log(e.message))</script>",
        ),
        _ => Reply::with_fields(404, &[], b""),
    }
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, plain.url("/page.html"));
    let mut lines = logged(&mut tab, 2);
    lines.sort();
    assert_eq!(lines[0], "Failed to fetch");
    assert!(
        lines[1].starts_with("Failed to fetch http://nowhere.invalid/a: the name did not resolve"),
        "{lines:?}"
    );
}
