//! HTTP Strict Transport Security and the `Upgrade-Insecure-Requests`
//! field (M33.3, docs/design/hardening.md): a host that sent the field
//! over `https:` is fetched over it from then on, its subdomains too when
//! it said so, in the profile as well; and a form of a secure page is
//! not sent to `http:` until its warning page's "Send anyway".

use net::Url;
use net::test_server::{Reply, ServerRequest, TestAuthority, TestServer, VALID};
use net::transport::Config;
use profile::{Memory, Profile};

use super::tests::load_all;
use crate::testing::*;
use crate::{Key, LoadState};

/// A TLS server for `a.localhost` and its subdomains: `/set` sends the
/// field with `includeSubDomains`, `/forget` sends `max-age=0`, every
/// other path a page that says which it is.
fn secure_server(authority: &TestAuthority) -> TestServer {
    let config = authority.server(&["a.localhost", "sub.a.localhost"], VALID);
    TestServer::tls(config, |request: &ServerRequest| {
        let field = match request.target.as_str() {
            "/set" => "max-age=100; includeSubDomains",
            "/forget" => "max-age=0",
            "/image" => return Reply::ok("text/html", b"<img src=/i.png>"),
            _ => "",
        };
        let mut fields = vec![("Content-Type", "text/html")];
        if !field.is_empty() {
            fields.push(("Strict-Transport-Security", field));
        }
        Reply::with_fields(200, &fields, format!("<p>{}", request.target).as_bytes())
    })
}

/// A tab whose client trusts `authority`'s root.
fn trusting(authority: &TestAuthority) -> Tab {
    let mut tab = Tab::new();
    tab.engine.set_network(net::Client::new(
        Config::default().trust(authority.root.clone()),
    ));
    tab
}

/// Whether the tab's engine knows `host` as HTTPS only. (An `http:`
/// request to the TLS server's port would wait for its answer.)
fn known(tab: &Tab, host: &str) -> bool {
    let hsts = tab.engine.network.hsts();
    let hsts = hsts.lock().unwrap();
    hsts.known(host, std::time::SystemTime::now())
}

fn at(scheme: &str, host: &str, server: &TestServer, path: &str) -> Url {
    Url::parse(&format!("{scheme}://{host}:{}{path}", server.port())).unwrap()
}

/// Navigates to `url` and waits for whatever comes of it to be shown:
/// the URL shown then.
fn arrive(tab: &mut Tab, url: Url) -> Option<Url> {
    tab.engine.navigate(url);
    tab.wait_until(|engine| {
        incoming(engine).is_none() && engine.load_state() == &LoadState::Loaded
    });
    tab.engine.url().cloned()
}

#[test]
fn a_host_that_sent_the_field_is_fetched_over_https() {
    let authority = TestAuthority::new();
    let server = secure_server(&authority);
    let mut tab = trusting(&authority);
    let page = at("http", "a.localhost", &server, "/page");
    assert!(!known(&tab, "a.localhost"), "not yet");
    load_all(&mut tab, at("https", "a.localhost", &server, "/set"));
    assert_eq!(
        arrive(&mut tab, page.clone()),
        Some(at("https", "a.localhost", &server, "/page"))
    );
    assert!(tab.engine.shown.failure.is_none());
    // Its subdomains with it.
    assert_eq!(
        arrive(&mut tab, at("http", "sub.a.localhost", &server, "/sub")),
        Some(at("https", "sub.a.localhost", &server, "/sub"))
    );
    // Every navigation said it would take https:.
    let sent = server
        .requests()
        .into_iter()
        .find(|request| request.target == "/page")
        .expect("the page asked for");
    assert_eq!(sent.header("Upgrade-Insecure-Requests"), Some("1"));
    // Not a subresource.
    load_all(&mut tab, at("https", "a.localhost", &server, "/image"));
    tab.wait_until(|_| {
        server
            .requests()
            .iter()
            .any(|request| request.target == "/i.png")
    });
    let image = server
        .requests()
        .into_iter()
        .find(|request| request.target == "/i.png")
        .expect("the image asked for");
    assert_eq!(image.header("Upgrade-Insecure-Requests"), None);
    // max-age=0 forgets it.
    load_all(&mut tab, at("https", "a.localhost", &server, "/forget"));
    assert!(!known(&tab, "a.localhost"), "forgotten");
}

#[test]
fn the_field_over_http_is_not_taken() {
    let server = TestServer::plain(|_: &ServerRequest| {
        Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Strict-Transport-Security", "max-age=100"),
            ],
            b"<p>plain",
        )
    });
    let mut tab = Tab::new();
    load_all(&mut tab, at("http", "c.localhost", &server, "/set"));
    let page = at("http", "c.localhost", &server, "/page");
    assert_eq!(arrive(&mut tab, page.clone()), Some(page));
    assert!(tab.engine.shown.failure.is_none());
}

#[test]
fn the_hosts_outlive_the_engine_in_its_profile() {
    let authority = TestAuthority::new();
    let server = secure_server(&authority);
    let memory = Memory::new();
    let on = |memory: &Memory| {
        let mut tab = trusting(&authority);
        tab.engine
            .set_profile(Profile::in_memory(memory).unwrap())
            .unwrap();
        tab
    };
    let mut tab = on(&memory);
    load_all(&mut tab, at("https", "a.localhost", &server, "/set"));
    assert!(tab.engine.next_task_at().is_some(), "a write waits");
    tab.engine.flush_profile().unwrap();
    assert_eq!(tab.engine.next_task_at(), None, "none after it");
    drop(tab);
    let written = Profile::in_memory(&memory.snapshot())
        .unwrap()
        .hsts()
        .unwrap();
    assert_eq!(written.len(), 1);
    let mut tab = on(&memory);
    assert_eq!(
        arrive(&mut tab, at("http", "sub.a.localhost", &server, "/again")),
        Some(at("https", "sub.a.localhost", &server, "/again"))
    );
}

/// Follows the link of the page shown whose `href` is `href`.
fn follow(tab: &mut Tab, href: &str) {
    let document = tab.engine.document();
    let node = document
        .subtree(document.root())
        .find(|&node| document.attr(node, "href") == Some(href))
        .expect("the link");
    let link = crate::Link {
        page: tab.engine.shown.page.id(),
        node,
        url: Url::parse(href).unwrap(),
    };
    tab.engine.follow_link(&link);
}

#[test]
fn a_secure_pages_form_to_http_waits_for_send_anyway() {
    // plain.test and insecure.test: this machine, but not potentially
    // trustworthy names.
    let plain = TestServer::plain(|request: &ServerRequest| match request.target.as_str() {
        "/form" => Reply::ok(
            "text/html",
            b"<form id=f method=post action=/got><input name=x value=2></form>",
        ),
        _ => Reply::ok("text/html", b"<title>got</title>"),
    });
    let got = at("http", "plain.test", &plain, "/got");
    let authority = TestAuthority::new();
    let config = authority.server(&["a.localhost"], VALID);
    let page = format!(
        "<form id=f method=post action='{got}'>\
         <input id=x name=x value=1><button id=go>go</button></form>\
         <form id=safe action='http://localhost:9/'></form>"
    );
    let server = TestServer::tls(config, move |request: &ServerRequest| {
        match request.target.as_str() {
            "/form" => Reply::ok("text/html", page.as_bytes()),
            _ => Reply::ok("text/html", b"<title>back</title>"),
        }
    });
    let local = std::net::IpAddr::from([127, 0, 0, 1]);
    let open = |scripts: bool| {
        let mut tab = Tab::new();
        tab.engine.set_network(net::Client::new(
            Config::default()
                .trust(authority.root.clone())
                .resolve("plain.test", local)
                .resolve("insecure.test", local),
        ));
        tab.engine.set_scripting(scripts);
        tab
    };
    let posts = || {
        plain
            .requests()
            .iter()
            .filter(|request| request.target == "/got" && request.method == "POST")
            .count()
    };
    let form = at("https", "a.localhost", &server, "/form");
    // The user's own submission, with scripts off.
    let mut tab = open(false);
    load_all(&mut tab, form.clone());
    let document = tab.engine.document();
    let element = document
        .subtree(document.root())
        .find(|&node| document.attr(node, "id") == Some("f"))
        .expect("the form");
    tab.engine.submit_form(element, None, false);
    // Not sent: the browser's page in its place, under its URL.
    assert!(incoming(&tab.engine).is_none(), "nothing fetched");
    assert_eq!(tab.engine.title(), "Form Not Secure");
    assert_eq!(tab.engine.url(), Some(&got));
    // A script's, by Enter in its field.
    let mut tab = open(true);
    load_all(&mut tab, at("https", "a.localhost", &server, "/first"));
    load_all(&mut tab, form.clone());
    tab.engine
        .run_classic("document.getElementById('x').focus();");
    tab.engine
        .key_down(&Key::new("Enter", "Enter").with_text("\r"));
    assert!(incoming(&tab.engine).is_none(), "nothing fetched");
    assert_eq!(tab.engine.title(), "Form Not Secure");
    // The address typed sends nothing: only the page's link does.
    tab.engine
        .navigate(Url::parse(crate::pages::SEND_FORM).unwrap());
    assert!(
        incoming(&tab.engine).is_none(),
        "the user's own address does nothing"
    );
    // "Go back" goes back to the form; its entry, when gone forward to
    // again, is never fetched.
    load_all(&mut tab, form.clone());
    tab.engine
        .run_classic("document.getElementById('go').click();");
    follow(&mut tab, crate::pages::GO_BACK);
    tab.wait_until(|engine| {
        engine.url() == Some(&form) && engine.load_state() == &LoadState::Loaded
    });
    tab.engine.go_forward();
    assert!(incoming(&tab.engine).is_none(), "not fetched again");
    tab.engine.go_back();
    tab.wait_until(|engine| {
        engine.url() == Some(&form) && engine.load_state() == &LoadState::Loaded
    });
    tab.engine
        .run_classic("document.getElementById('go').click();");
    assert_eq!(posts(), 0);
    // "Send anyway" sends it, a POST, in the page's entry.
    let entries = tab.engine.history.len();
    follow(&mut tab, crate::pages::SEND_FORM);
    tab.wait_until(|engine| engine.title() == "got");
    assert_eq!(posts(), 1);
    assert_eq!(tab.engine.history.len(), entries);
    // A form to a potentially trustworthy http: URL is sent at once.
    load_all(&mut tab, form.clone());
    tab.engine
        .run_classic("document.getElementById('safe').submit();");
    let navigation = tab.engine.navigation.as_ref().expect("sent at once");
    assert_eq!(navigation.url.as_str(), "http://localhost:9/?");
    tab.engine.stop();
    // And an insecure page's to http:.
    load_all(&mut tab, at("http", "insecure.test", &plain, "/form"));
    tab.engine
        .run_classic("document.getElementById('f').submit();");
    tab.wait_until(|engine| engine.title() == "got");
    assert_eq!(posts(), 2);
}
