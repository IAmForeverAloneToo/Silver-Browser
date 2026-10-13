//! Referrer Policy (M33.3, `docs/design/hardening.md`): the `Referer`
//! a document's requests carry by its policy (its response's, a
//! `<meta>`'s, an element's `referrerpolicy`, a `fetch()`'s), at each
//! hop, and `document.referrer`, as the reference browser has them (p5).

use net::Url;
use net::test_server::{Reply, TestServer};

use super::tests::load_all;
use crate::testing::*;

/// Pages by path, each with its header fields and body.
type Pages = &'static [(
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
)];

/// A server of `pages` under two sites, `a.localhost` and `b.localhost`,
/// a query left aside; `PORT` in a body or a field is the server's
/// port, and a page with a `Location` field redirects there.
fn sites(pages: Pages) -> TestServer {
    let port = std::sync::Arc::new(std::sync::OnceLock::<u16>::new());
    let seen = port.clone();
    let server = serve(move |target| {
        let path = target.split('?').next().unwrap_or_default();
        let (_, fields, body) = pages.iter().find(|(at, ..)| *at == path)?;
        let port = seen.get().map(u16::to_string).unwrap_or_default();
        let mut all: Vec<(&str, String)> = vec![("Content-Type", "text/html".to_owned())];
        all.extend(
            fields
                .iter()
                .map(|(name, value)| (*name, value.replace("PORT", &port))),
        );
        let all: Vec<(&str, &str)> = all
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        let status = match fields.iter().any(|(name, _)| *name == "Location") {
            true => 302,
            false => 200,
        };
        Some(Reply::with_fields(
            status,
            &all,
            body.replace("PORT", &port).as_bytes(),
        ))
    });
    let _ = port.set(server.url("/").port().expect("a port"));
    server
}

fn at(server: &TestServer, host: &str, path: &str) -> Url {
    let port = server.url("/").port().expect("a port");
    Url::parse(&format!("http://{host}:{port}{path}")).expect("a URL")
}

/// A tab with scripts on, `url` loaded, run until a line of the console
/// ends with `last`: the lines, the port written `PORT`.
fn run_to(server: &TestServer, url: Url, last: &str) -> Vec<String> {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(url);
    let port = server.url("/").port().expect("a port").to_string();
    tab.run_until(|lines| lines.iter().any(|line| line.ends_with(last)))
        .into_iter()
        .map(|line| line.replace(&port, "PORT"))
        .collect()
}

/// The `Referer` each request for `path` of `server` carried, in order,
/// `(none)` for none, the port written `PORT`.
fn referers(server: &TestServer, path: &str) -> Vec<String> {
    let port = server.url("/").port().expect("a port").to_string();
    server
        .requests()
        .into_iter()
        .filter(|request| request.target.split('?').next() == Some(path))
        .map(|request| {
            request
                .header("Referer")
                .map_or_else(|| "(none)".to_owned(), |value| value.replace(&port, "PORT"))
        })
        .collect()
}

#[test]
fn a_fetch_carries_its_documents_referrer_as_each_policy_says() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[],
            "<script>\
             const policies = ['', 'no-referrer', 'no-referrer-when-downgrade', 'origin',\
               'origin-when-cross-origin', 'same-origin', 'strict-origin',\
               'strict-origin-when-cross-origin', 'unsafe-url'];\
             (async () => {\
               for (const policy of policies) {\
                 await fetch('/same?' + policy, { referrerPolicy: policy });\
                 await fetch('http://b.localhost:' + location.port + '/cross?' + policy,\
                   { referrerPolicy: policy, mode: 'no-cors' });\
               }\
               await fetch('/option', { referrer: '/other/path?q' });\
               await fetch('/client', { referrer: '' });\
               console.log('done');\
             })();\
             </script>",
        ),
        ("/same", &[], ""),
        ("/cross", &[], ""),
        ("/option", &[], ""),
        ("/client", &[], ""),
    ];
    let server = sites(PAGES);
    run_to(&server, at(&server, "a.localhost", "/page.html"), "done");
    let page = "http://a.localhost:PORT/page.html";
    let origin = "http://a.localhost:PORT/";
    let none = "(none)";
    // Each policy in turn, the default first, as p5 found them.
    assert_eq!(
        referers(&server, "/same"),
        [page, none, page, origin, page, page, origin, page, page]
    );
    assert_eq!(
        referers(&server, "/cross"),
        [
            origin, none, page, origin, origin, none, origin, origin, page
        ]
    );
    assert_eq!(
        referers(&server, "/option"),
        ["http://a.localhost:PORT/other/path?q"]
    );
    assert_eq!(referers(&server, "/client"), [none]);
}

#[test]
fn a_documents_policy_comes_from_its_field_its_meta_and_its_elements() {
    static PAGES: Pages = &[
        (
            "/field.html",
            &[("Referrer-Policy", "no-referrer, origin")],
            "<img src=/i.png>",
        ),
        (
            "/meta.html",
            &[("Referrer-Policy", "unsafe-url")],
            "<meta name=referrer content=never><img src=/m.png>",
        ),
        (
            "/elements.html",
            &[],
            "<img src=/e.png referrerpolicy=no-referrer>\
             <script src=/s.js referrerpolicy=origin></script>\
             <link rel=stylesheet href=/l.css referrerpolicy=origin>\
             <link rel='stylesheet noreferrer' href=/n.css>",
        ),
        ("/i.png", &[("Content-Type", "image/png")], ""),
        ("/m.png", &[("Content-Type", "image/png")], ""),
        ("/e.png", &[("Content-Type", "image/png")], ""),
        ("/s.js", &[("Content-Type", "text/javascript")], ""),
        ("/l.css", &[("Content-Type", "text/css")], ""),
        ("/n.css", &[("Content-Type", "text/css")], ""),
    ];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    for page in ["/field.html", "/meta.html", "/elements.html"] {
        load_all(&mut tab, at(&server, "a.localhost", page));
    }
    assert_eq!(referers(&server, "/i.png"), ["http://a.localhost:PORT/"]);
    assert_eq!(referers(&server, "/m.png"), ["(none)"]);
    assert_eq!(referers(&server, "/e.png"), ["(none)"]);
    assert_eq!(referers(&server, "/s.js"), ["http://a.localhost:PORT/"]);
    assert_eq!(referers(&server, "/l.css"), ["http://a.localhost:PORT/"]);
    assert_eq!(referers(&server, "/n.css"), ["(none)"]);
}

#[test]
fn a_navigation_and_a_frame_carry_their_documents_referrer() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[],
            "<a id=plain href=/next.html>next</a>\
             <a id=none href=/next.html rel=noreferrer>none</a>\
             <a id=origin href=/next.html referrerpolicy=origin>origin</a>\
             <a id=redirect href=/redirect.html>redirect</a>\
             <iframe src=/frame.html referrerpolicy=origin></iframe>",
        ),
        (
            "/next.html",
            &[],
            "<script>console.log('next ' + document.referrer)</script>",
        ),
        (
            "/frame.html",
            &[],
            "<script>console.log('frame ' + document.referrer)</script>",
        ),
        (
            "/redirect.html",
            &[
                ("Location", "http://a.localhost:PORT/next.html"),
                ("Referrer-Policy", "no-referrer"),
            ],
            "",
        ),
    ];
    let server = sites(PAGES);
    let page = at(&server, "a.localhost", "/page.html");
    let follow = |id: &str| {
        let mut tab = Tab::new();
        tab.engine.set_scripting(true);
        load_all(&mut tab, page.clone());
        tab.engine
            .run_classic(&format!("document.getElementById('{id}').click()"));
        let port = server.url("/").port().expect("a port").to_string();
        tab.run_until(|lines| lines.iter().any(|line| line.starts_with("next")))
            .into_iter()
            .filter(|line| line.starts_with("next"))
            .map(|line| line.replace(&port, "PORT"))
            .collect::<Vec<_>>()
    };
    assert_eq!(follow("plain"), ["next http://a.localhost:PORT/page.html"]);
    assert_eq!(follow("none"), ["next "]);
    assert_eq!(follow("origin"), ["next http://a.localhost:PORT/"]);
    // Its request was made by the link's policy too.
    assert_eq!(
        referers(&server, "/next.html").last().map(String::as_str),
        Some("http://a.localhost:PORT/")
    );
    assert_eq!(
        referers(&server, "/frame.html"),
        ["http://a.localhost:PORT/"; 3]
    );
    // A redirect's own policy holds for the hops after it: the next one
    // sends none, while document.referrer is the first hop's.
    assert_eq!(
        follow("redirect"),
        ["next http://a.localhost:PORT/page.html"]
    );
    let next = referers(&server, "/next.html");
    assert_eq!(next.last().map(String::as_str), Some("(none)"));
    // A frame's document's referrer is its element's request's.
    let lines = run_to(
        &server,
        page,
        "frame http://a.localhost:PORT/"
            .replace("PORT", &server.url("/").port().expect("a port").to_string())
            .as_str(),
    );
    assert!(
        lines.contains(
            &"[frame http://a.localhost:PORT/frame.html] frame http://a.localhost:PORT/".to_owned()
        ),
        "{lines:?}"
    );
}

#[test]
fn a_local_document_takes_its_creators_policy() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[("Referrer-Policy", "no-referrer")],
            "<iframe srcdoc=\"<img src=/i.png>\"></iframe>",
        ),
        ("/i.png", &[("Content-Type", "image/png")], ""),
    ];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    load_all(&mut tab, at(&server, "a.localhost", "/page.html"));
    wait_for(|| {
        tab.engine.poll_network();
        !referers(&server, "/i.png").is_empty()
    });
    assert_eq!(referers(&server, "/i.png"), ["(none)"]);
}
