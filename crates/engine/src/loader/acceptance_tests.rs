//! M38's acceptance (docs/design/http.md, "The acceptance"): a page of
//! twenty resources, each fresh for ten minutes, asked for once by a
//! profile's tabs, and once more by none after the profile is closed
//! and opened again; and the same page over HTTP/2, each reply held
//! back, on one connection with its streams open at once.

use std::time::{Duration, Instant};

use net::test_server::{Reply, ServerRequest, TestAuthority, TestServer, VALID};
use net::transport::Config;

use super::tests::load_all;
use crate::testing::*;

/// The page: six sheets, six scripts that block the parser, and eight
/// images.
fn page() -> String {
    let mut page = String::from("<!DOCTYPE html><title>twenty</title>");
    for n in 0..6 {
        page += &format!("<link rel=stylesheet href=/s{n}.css>");
    }
    for n in 0..6 {
        page += &format!("<script src=/j{n}.js></script>");
    }
    page += "<p class=s5>text</p>";
    for n in 0..8 {
        page += &format!("<img src=/i{n}.png>");
    }
    page
}

/// The twenty resources' paths.
fn resources() -> Vec<String> {
    let mut paths = Vec::new();
    paths.extend((0..6).map(|n| format!("/s{n}.css")));
    paths.extend((0..6).map(|n| format!("/j{n}.js")));
    paths.extend((0..8).map(|n| format!("/i{n}.png")));
    paths
}

/// The answer to `path`, fresh for ten minutes.
fn answer(path: &str, page: &str) -> Option<Reply> {
    let (kind, body) = match path.rsplit('.').next()? {
        "html" => ("text/html", page.to_owned()),
        "css" => {
            let n = &path[2..3];
            ("text/css", format!(".s{n} {{ color: red }}"))
        }
        "js" => ("text/javascript", format!("console.log('{path}')")),
        "png" => ("image/png", String::new()),
        _ => return None,
    };
    Some(Reply::with_fields(
        200,
        &[("Content-Type", kind), ("Cache-Control", "max-age=600")],
        body.as_bytes(),
    ))
}

fn reached(server: &TestServer, target: &str) -> usize {
    server
        .requests()
        .iter()
        .filter(|request| request.target == target)
        .count()
}

/// Each of the page and its twenty resources asked for once.
fn each_once(server: &TestServer) {
    for path in std::iter::once("/page.html".to_owned()).chain(resources()) {
        assert_eq!(reached(server, &path), 1, "{path}");
    }
}

fn scripts_ran(tab: &mut Tab) {
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    let wanted: Vec<String> = (0..6).map(|n| format!("/j{n}.js")).collect();
    assert_eq!(lines, wanted);
}

#[test]
fn twenty_resources_are_asked_for_once_by_a_profiles_tabs_and_after_it_is_opened_again() {
    let page = page();
    let server = serve(move |path| answer(path, &page));
    let memory = profile::Memory::new();
    let tab_of = |context: &crate::Context| {
        let mut tab = Tab::new();
        tab.engine.set_scripting(true);
        tab.engine.join(context);
        tab
    };
    {
        let context = crate::Context::new();
        context
            .set_profile(profile::Profile::in_memory(&memory).unwrap())
            .unwrap();
        let mut first = tab_of(&context);
        load_all(&mut first, server.url("/page.html"));
        scripts_ran(&mut first);
        each_once(&server);
        // Another tab of the profile: no request at all.
        let mut second = tab_of(&context);
        load_all(&mut second, server.url("/page.html"));
        scripts_ran(&mut second);
        assert_eq!(second.names(), ["s0", "s1", "s2", "s3", "s4", "s5"]);
        each_once(&server);
        context.flush_profile(Duration::from_secs(10)).unwrap();
    }
    // The profile opened again: from what it kept.
    let context = crate::Context::new();
    context
        .set_profile(profile::Profile::in_memory(&memory).unwrap())
        .unwrap();
    let mut again = tab_of(&context);
    load_all(&mut again, server.url("/page.html"));
    scripts_ran(&mut again);
    assert_eq!(again.names(), ["s0", "s1", "s2", "s3", "s4", "s5"]);
    each_once(&server);
}

#[test]
fn over_http2_the_page_takes_one_connection_and_many_streams_at_once() {
    const HELD: Duration = Duration::from_millis(300);
    let authority = TestAuthority::new();
    let page = page();
    let server = TestServer::tls(
        authority.server_h2(&["a.localhost"], VALID),
        move |request: &ServerRequest| {
            let reply =
                answer(&request.target, &page).unwrap_or_else(|| Reply::with_fields(404, &[], b""));
            Reply::After(HELD, Box::new(reply))
        },
    );
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    // The name resolved here: the system's resolver may take a while
    // for a `.localhost` name (G180), which the time is not about.
    tab.engine.set_network(net::Client::new(
        Config::default()
            .trust(authority.root.clone())
            .resolve("a.localhost", std::net::IpAddr::from([127, 0, 0, 1])),
    ));
    let url = net::Url::parse(&format!("https://a.localhost:{}/page.html", server.port())).unwrap();
    let started = Instant::now();
    load_all(&mut tab, url);
    let took = started.elapsed();
    scripts_ran(&mut tab);
    each_once(&server);
    assert!(
        server.requests().iter().all(|request| request.stream > 0),
        "every request over HTTP/2"
    );
    assert_eq!(server.connections(), 1);
    // The scanner asked for the twenty at once, the page first: here
    // about two replies' time, where one connection of HTTP/1.1 would
    // take twenty-one; the bound is loose, for slow machines (macOS's
    // runners took ten and a half).
    assert!(server.most_streams() >= 15, "{}", server.most_streams());
    assert!(took < HELD * 15, "{took:?}");
}
