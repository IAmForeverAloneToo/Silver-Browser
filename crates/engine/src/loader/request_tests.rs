//! What the engine's requests carry since M27 (docs/design/fetch.md):
//! the document's origin on its subresources and on the navigations it
//! starts, so that `Origin` and the `Sec-Fetch-*` fields say who asked;
//! a response's `Cross-Origin-Resource-Policy` honoured; and the CORS
//! settings attribute of scripts and stylesheets (M27.3).

use net::Url;
use net::test_server::{Reply, ServerRequest, TestServer};

use super::tests::load_all;
use crate::LoadState;
use crate::testing::*;

/// The request the server last had for `path`.
fn sent(server: &TestServer, path: &str) -> ServerRequest {
    server
        .requests()
        .into_iter()
        .rev()
        .find(|request| request.target == path)
        .unwrap_or_else(|| panic!("a request for {path}"))
}

/// The `Origin` and `Sec-Fetch-*` fields of a request.
fn fields(request: &ServerRequest) -> [Option<String>; 5] {
    [
        "Origin",
        "Sec-Fetch-Dest",
        "Sec-Fetch-Mode",
        "Sec-Fetch-Site",
        "Sec-Fetch-User",
    ]
    .map(|name| request.header(name).map(str::to_owned))
}

fn some(values: [Option<&str>; 5]) -> [Option<String>; 5] {
    values.map(|value| value.map(str::to_owned))
}

/// Waits until the engine shows `url`, loaded.
fn shown(tab: &mut Tab, url: &Url) {
    tab.wait_until(|engine| engine.url() == Some(url) && engine.load_state() == &LoadState::Loaded);
}

#[test]
fn navigations_say_whether_the_user_or_a_document_started_them() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<body style=margin:0><a href=/next.html style=display:block>next</a>\
             <form method=post action=/sent><input name=a value=1></form>",
        )),
        "/sent" | "/next.html" | "/moved.html" => Some(html("<p>there</p>")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    let origin = server.url("/").origin().serialization();
    // Typed by the user: no origin, none of a site.
    assert_eq!(
        fields(&sent(&server, "/page.html")),
        some([
            None,
            Some("document"),
            Some("navigate"),
            Some("none"),
            Some("?1")
        ])
    );
    // A form's POST from the page carries its origin; a script sent it,
    // which is no activation by the user.
    tab.engine.run_classic("document.forms[0].submit()");
    let posted = server.url("/sent");
    shown(&mut tab, &posted);
    assert_eq!(
        fields(&sent(&server, "/sent")),
        some([
            Some(&origin),
            Some("document"),
            Some("navigate"),
            Some("same-origin"),
            None
        ])
    );
    // A click on a link is the user's activation.
    load_all(&mut tab, server.url("/page.html"));
    tab.engine.frame(400, 100);
    let link = tab.engine.link_at(2.0, 2.0).expect("the link");
    tab.engine.follow_link(&link);
    let next = server.url("/next.html");
    shown(&mut tab, &next);
    assert_eq!(
        fields(&sent(&server, "/next.html")),
        some([
            None,
            Some("document"),
            Some("navigate"),
            Some("same-origin"),
            Some("?1")
        ])
    );
    // A script's location.href is no activation.
    load_all(&mut tab, server.url("/page.html"));
    tab.engine.run_classic("location.href = '/moved.html'");
    let moved = server.url("/moved.html");
    shown(&mut tab, &moved);
    assert_eq!(
        fields(&sent(&server, "/moved.html")),
        some([
            None,
            Some("document"),
            Some("navigate"),
            Some("same-origin"),
            None
        ])
    );
}

#[test]
fn subresources_say_their_destination_and_site() {
    let other = serve(|path| match path {
        "/b.css" => Some(css(".b {}")),
        _ => None,
    });
    let port = other.url("/").port().expect("a port");
    let page = format!(
        "<link rel=stylesheet href=/a.css>\
         <link rel=stylesheet href=http://b.localhost:{port}/b.css>"
    );
    let server = serve(move |path| match path {
        "/page.html" => Some(html(&page)),
        "/a.css" => Some(css(".a {}")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    assert_eq!(tab.names(), ["a", "b"]);
    assert_eq!(
        fields(&sent(&server, "/a.css")),
        some([
            None,
            Some("style"),
            Some("no-cors"),
            Some("same-origin"),
            None
        ])
    );
    assert_eq!(
        fields(&sent(&other, "/b.css")),
        some([
            None,
            Some("style"),
            Some("no-cors"),
            Some("cross-site"),
            None
        ])
    );
}

#[test]
fn a_sheet_whose_resource_policy_refuses_the_page_is_not_applied() {
    let corp = |policy: &'static str| {
        Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/css"),
                ("Cross-Origin-Resource-Policy", policy),
            ],
            match policy {
                "same-origin" => b".so {}",
                "same-site" => b".ss {}",
                _ => b".co {}",
            },
        )
    };
    let other = serve(move |path| match path {
        "/so.css" => Some(corp("same-origin")),
        "/ss.css" => Some(corp("same-site")),
        "/co.css" => Some(corp("cross-origin")),
        _ => None,
    });
    let port = other.url("/").port().expect("a port");
    // Another port of 127.0.0.1 is another origin of the same site.
    let page = format!(
        "<link rel=stylesheet href=http://127.0.0.1:{port}/so.css>\
         <link rel=stylesheet href=http://127.0.0.1:{port}/ss.css>\
         <link rel=stylesheet href=http://127.0.0.1:{port}/co.css>"
    );
    let server = serve(move |path| (path == "/page.html").then(|| html(&page)));
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    assert_eq!(tab.names(), ["ss", "co"]);
}

/// A reply of `content_type` and `body` that CORS allows to any origin.
fn shared(content_type: &str, body: &[u8]) -> Reply {
    Reply::with_fields(
        200,
        &[
            ("Content-Type", content_type),
            ("Access-Control-Allow-Origin", "*"),
        ],
        body,
    )
}

#[test]
fn a_crossorigin_script_runs_by_cors_and_its_errors_are_unmuted() {
    let other = serve(|path| match path {
        "/shared.js" => Some(shared("text/javascript", b"throw new Error('open')")),
        "/bare.js" => Some(Reply::ok("text/javascript", b"throw new Error('closed')")),
        _ => None,
    });
    let (shared_js, bare_js) = (other.url("/shared.js"), other.url("/bare.js"));
    let page = format!(
        "<script src={shared_js} crossorigin></script>\
         <script src={bare_js} crossorigin onerror=\"console.log('refused')\"></script>\
         <script src={bare_js}></script>\
         <script src={shared_js} crossorigin=use-credentials \
         onerror=\"console.log('no credentials')\"></script>"
    );
    let server = serve(move |path| (path == "/page.html").then(|| html(&page)));
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .filter(|text| !text.starts_with("Failed to load"))
        .collect();
    assert_eq!(
        lines,
        [
            format!("Uncaught Error: open at {shared_js}:1:11"),
            "refused".to_owned(),
            "Uncaught Script error.".to_owned(),
            "no credentials".to_owned(),
        ]
    );
    assert_eq!(
        sent(&other, "/shared.js")
            .header("Origin")
            .map(str::to_owned),
        Some(server.url("/").origin().serialization())
    );
    assert_eq!(
        sent(&other, "/shared.js").header("Sec-Fetch-Mode"),
        Some("cors")
    );
}

#[test]
fn a_crossorigin_sheet_applies_only_when_cors_allows_it() {
    let other = serve(|path| match path {
        "/a.css" => Some(shared("text/css", b".a {}")),
        "/b.css" => Some(css(".b {}")),
        _ => None,
    });
    let (a, b) = (other.url("/a.css"), other.url("/b.css"));
    let page = format!(
        "<link rel=stylesheet href={a} crossorigin=anonymous>\
         <link rel=stylesheet href={b} crossorigin>\
         <link rel=stylesheet href={b}>"
    );
    let server = serve(move |path| (path == "/page.html").then(|| html(&page)));
    let mut tab = Tab::new();
    tab.load(server.url("/page.html"));
    assert_eq!(tab.names(), ["a", "b"]);
    let modes: Vec<Option<String>> = other
        .requests()
        .iter()
        .map(|request| request.header("Sec-Fetch-Mode").map(str::to_owned))
        .collect();
    assert_eq!(
        modes
            .iter()
            .filter(|mode| mode.as_deref() == Some("cors"))
            .count(),
        2
    );
}

#[test]
fn a_cors_settings_attribute_says_credentials_for_use_credentials_alone() {
    use super::cors_settings;
    use net::Credentials;
    assert_eq!(cors_settings(None), None);
    for value in ["", "anonymous", "Anonymous", "nonsense"] {
        assert_eq!(
            cors_settings(Some(value)),
            Some(Credentials::SameOrigin),
            "{value}"
        );
    }
    for value in ["use-credentials", "USE-Credentials"] {
        assert_eq!(
            cors_settings(Some(value)),
            Some(Credentials::Include),
            "{value}"
        );
    }
}
