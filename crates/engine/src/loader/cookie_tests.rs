//! Cookies in the engine (M27.4, docs/design/fetch.md): a document's
//! own cookies visible to its first script, `document.cookie` read and
//! written, `HttpOnly` kept from scripts, the cookies of navigations and
//! subresources by `SameSite`, and a cookie-averse document's none.

use net::Url;
use net::test_server::{Reply, ServerRequest, TestServer};

use super::tests::load_all;
use crate::LoadState;
use crate::testing::*;

/// The console's lines, taken.
fn console(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// The `Cookie` the server was last sent for `path`.
fn cookie(server: &TestServer, path: &str) -> Option<String> {
    server
        .requests()
        .into_iter()
        .rev()
        .find(|request: &ServerRequest| request.target == path)
        .and_then(|request| request.header("Cookie").map(str::to_owned))
}

#[test]
fn a_documents_cookies_are_its_scripts_and_http_only_ones_its_servers() {
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
        "/next.html" => Some(html("<p>next</p>")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    assert_eq!(console(&mut tab), ["a=1 true", "b=2"]);
    load_all(&mut tab, server.url("/next.html"));
    assert_eq!(
        cookie(&server, "/next.html").as_deref(),
        Some("h=secret; b=2")
    );
}

#[test]
fn a_documents_whose_url_is_not_http_has_no_cookies() {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    let page = "data:text/html,<script>document.cookie = 'a=1'; \
                console.log(JSON.stringify(document.cookie))</script>";
    load_all(&mut tab, Url::parse(page).unwrap());
    assert_eq!(console(&mut tab), ["\"\""]);
}

#[test]
fn same_site_cookies_go_with_the_navigations_and_subresources_they_may() {
    // b.localhost's cookies, set when the user loads it.
    let other = serve(|path| match path {
        "/set.html" => Some(Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Set-Cookie", "dflt=1"),
                ("Set-Cookie", "strict=1; SameSite=Strict"),
                ("Set-Cookie", "none=1; SameSite=None; Secure"),
            ],
            b"<p>set</p>",
        )),
        "/style.css" => Some(css(".b {}")),
        "/landing.html" => Some(html("<p>landing</p>")),
        _ => None,
    });
    let port = other.url("/").port().expect("a port");
    let b = |path: &str| Url::parse(&format!("http://b.localhost:{port}{path}")).unwrap();
    let page = format!(
        "<body style=margin:0><a href={} style=display:block>go</a>\
         <link rel=stylesheet href={}>",
        b("/landing.html"),
        b("/style.css")
    );
    let server = serve(move |path| (path == "/page.html").then(|| html(&page)));
    let mut tab = Tab::new();
    load_all(&mut tab, b("/set.html"));
    // A page of a.localhost: its subresource from b is cross-site.
    let a = Url::parse(&format!(
        "http://a.localhost:{}/page.html",
        server.url("/").port().expect("a port")
    ))
    .unwrap();
    tab.load(a.clone());
    assert_eq!(cookie(&other, "/style.css").as_deref(), Some("none=1"));
    // Its link to b is a cross-site top-level navigation: Lax goes,
    // Strict does not.
    tab.engine.frame(400, 100);
    let link = tab.engine.link_at(2.0, 2.0).expect("the link");
    tab.engine.follow_link(&link);
    let landing = b("/landing.html");
    tab.wait_until(|engine| {
        engine.url() == Some(&landing) && engine.load_state() == &LoadState::Loaded
    });
    assert_eq!(
        cookie(&other, "/landing.html").as_deref(),
        Some("dflt=1; none=1")
    );
    // Typed by the user, it is same-site: all go.
    load_all(&mut tab, b("/landing.html?typed"));
    assert_eq!(
        cookie(&other, "/landing.html?typed").as_deref(),
        Some("dflt=1; strict=1; none=1")
    );
}

#[test]
fn the_jar_outlives_the_client_and_a_file_page_has_none() {
    let server = serve(|path| match path {
        "/set.html" => Some(Reply::with_fields(
            200,
            &[("Content-Type", "text/html"), ("Set-Cookie", "a=1")],
            b"<p>set</p>",
        )),
        "/next.html" => Some(html("<p>next</p>")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/set.html"));
    // D12's switch makes a new client: the cookies stay the engine's.
    tab.engine.set_compat_user_agent(true);
    load_all(&mut tab, server.url("/next.html"));
    assert_eq!(cookie(&server, "/next.html").as_deref(), Some("a=1"));
    let dir = std::env::temp_dir().join(format!("silver-cookies-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("page.html"),
        "<script>document.cookie = 'f=1'; console.log(JSON.stringify(document.cookie))</script>",
    )
    .unwrap();
    load_all(
        &mut tab,
        Url::from_file_path(&dir.join("page.html")).unwrap(),
    );
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(console(&mut tab), ["\"\""]);
    assert!(
        tab.engine
            .network
            .cookies()
            .lock()
            .unwrap()
            .cookies()
            .iter()
            .all(|c| c.name != b"f")
    );
}
