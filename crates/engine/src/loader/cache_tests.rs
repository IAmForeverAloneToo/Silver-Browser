//! The HTTP cache in the engine (M28.4): a page's subresources used
//! again while fresh, its document fetched each time, and a script's
//! fetch by its request's cache mode.

use std::time::{Duration, Instant};

use net::test_server::{Reply, TestServer};

use super::tests::load_all;
use crate::testing::*;

/// How many requests for `target` reached `server`.
fn reached(server: &TestServer, target: &str) -> usize {
    server
        .requests()
        .iter()
        .filter(|request| request.target == target)
        .count()
}

/// The console's lines, taken.
fn console(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

fn lasting(content_type: &str, body: &[u8]) -> Reply {
    Reply::with_fields(
        200,
        &[
            ("Content-Type", content_type),
            ("Cache-Control", "max-age=600"),
        ],
        body,
    )
}

#[test]
fn a_pages_fresh_subresources_come_from_the_cache_and_its_document_does_not() {
    let server = serve(|path| match path {
        "/page.html" => Some(Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Cache-Control", "max-age=600"),
            ],
            b"<link rel=stylesheet href=kept.css><link rel=stylesheet href=fetched.css>\
              <p class=a>text</p>",
        )),
        "/kept.css" => Some(lasting("text/css", b".a { color: red }")),
        "/fetched.css" => Some(css(".b { color: blue }")),
        _ => None,
    });
    let mut tab = Tab::new();
    load_all(&mut tab, server.url("/page.html"));
    tab.engine.reload();
    tab.settle();
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(reached(&server, "/page.html"), 3);
    assert_eq!(reached(&server, "/fetched.css"), 3);
    assert_eq!(reached(&server, "/kept.css"), 1);
    assert_eq!(tab.names(), ["a", "b"]);
    // The cache outlives the client: a new one keeps it.
    tab.engine.set_network(net::Client::default());
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(reached(&server, "/kept.css"), 1);
    // Another engine has a cache of its own.
    let mut other = Tab::new();
    load_all(&mut other, server.url("/page.html"));
    assert_eq!(reached(&server, "/kept.css"), 2);
}

#[test]
fn a_scripts_fetch_uses_the_cache_as_its_mode_says() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script>(async () => {\n\
             const text = (r) => r.text();\n\
             const log = (name, p) => p.then((v) => name + ' ' + v, (e) => name + ' ' + e.name);\n\
             const lines = [];\n\
             lines.push(await log('miss', fetch('/r', { cache: 'only-if-cached', mode: 'same-origin' }).then(text)));\n\
             lines.push(await log('first', fetch('/r').then(text)));\n\
             lines.push(await log('again', fetch('/r').then(text)));\n\
             lines.push(await log('hit', fetch('/r', { cache: 'only-if-cached', mode: 'same-origin' }).then(text)));\n\
             lines.push(await log('nostore', fetch('/r', { cache: 'no-store' }).then(text)));\n\
             lines.push(await log('reload', fetch('/r', { cache: 'reload' }).then(text)));\n\
             lines.push(await log('cors', Promise.resolve().then(() => new Request('/r', { cache: 'only-if-cached' }))));\n\
             document.title = lines.join(', ');\n\
             })()</script>",
        )),
        "/r" => Some(lasting("text/plain", b"r")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    // The answers are tasks: run as they come.
    let deadline = Instant::now() + WAIT;
    while tab.engine.title().is_empty() {
        assert!(Instant::now() < deadline, "the script did not finish");
        tab.engine.poll_network();
        tab.engine.run_tasks();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        tab.engine.title(),
        "miss TypeError, first r, again r, hit r, nostore r, reload r, cors TypeError"
    );
    // The console says why the first failed.
    let url = server.url("/r");
    assert_eq!(
        console(&mut tab),
        [format!(
            "Failed to fetch {url}: nothing cached, and only the cache asked"
        )]
    );
    // The first, then no-store and reload.
    assert_eq!(reached(&server, "/r"), 3);
}

#[test]
fn the_engines_of_a_context_share_its_connections() {
    // M38: the context holds the transport, so a second tab of the
    // profile takes the first's idle connection.
    let server = serve(|_| Some(html("<title>t</title>")));
    let context = crate::Context::new();
    for _ in 0..2 {
        let mut tab = Tab::new();
        tab.engine.join(&context);
        tab.load(server.url("/"));
    }
    assert_eq!(server.connections(), 1);
    // Engines of two contexts do not.
    let other = crate::Context::new();
    let mut tab = Tab::new();
    tab.engine.join(&other);
    tab.load(server.url("/"));
    assert_eq!(server.connections(), 2);
}
