//! The adversarial page (M27.7, docs/design/fetch.md):
//! `tests/security/cross-origin.html`, served from a.localhost, tries
//! every way of reading b.localhost, which holds a secret in its
//! answers and its cookies. From a victim that allows nothing it must
//! learn nothing; from one that allows its origin by CORS, with
//! credentials, it must learn by every CORS read and by nothing else.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use net::Url;
use net::test_server::{Reply, ServerRequest, TestServer};

use super::tests::load_all;
use crate::testing::*;

const PAGE: &str = include_str!("../../tests/security/cross-origin.html");

/// Every read the page makes.
const READS: [&str; 26] = [
    "cookies",
    "cors script error",
    "data as script",
    "fetch body",
    "fetch credentials",
    "fetch headers",
    "fetch status",
    "fetch without credentials",
    "followed redirect",
    "manual redirect",
    "no-cors blob",
    "no-cors body",
    "no-cors buffer",
    "no-cors clone",
    "no-cors credentials",
    "no-cors fields",
    "no-cors redirect",
    "resource policy",
    "script error",
    "stylesheet",
    "synchronous xhr",
    "unexposed field",
    "write",
    "xhr",
    "xhr credentials",
    "xhr redirect",
];

/// The reads that learn the secret once the victim allows the page.
const CORS_READS: [&str; 11] = [
    "cors script error",
    "fetch body",
    "fetch credentials",
    "fetch headers",
    "fetch status",
    "followed redirect",
    "synchronous xhr",
    "write",
    "xhr",
    "xhr credentials",
    "xhr redirect",
];

/// A response whose status text is `reason`.
fn reply(status: u16, reason: &str, fields: &[(&str, String)], body: &str) -> Reply {
    let mut bytes = format!("HTTP/1.1 {status} {reason}\r\n").into_bytes();
    for (name, value) in fields {
        bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    bytes.extend_from_slice(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes());
    Reply::Send(bytes)
}

/// The victim, holding `secret` in a body, two fields (`X-Secret`, which
/// it exposes when it allows CORS, and `X-Hidden`), a status text, a
/// redirect's location, a script's error, a stylesheet and the cookies
/// `/login` sets; allowing `allow`, if any, by CORS with credentials.
fn victim(secret: String, allow: Option<String>) -> TestServer {
    TestServer::plain(move |request: &ServerRequest| {
        let s = secret.as_str();
        let mut fields: Vec<(&str, String)> = Vec::new();
        if let Some(origin) = &allow {
            fields.push(("Access-Control-Allow-Origin", origin.clone()));
            fields.push(("Access-Control-Allow-Credentials", "true".to_owned()));
            fields.push(("Access-Control-Expose-Headers", "X-Secret".to_owned()));
        }
        let mut with = |more: &[(&'static str, String)]| {
            fields.extend_from_slice(more);
            fields.clone()
        };
        let text = |value: &str| ("Content-Type", value.to_owned());
        match (request.method.as_str(), request.target.as_str()) {
            (_, "/login") => reply(
                200,
                "OK",
                &with(&[
                    text("text/html"),
                    (
                        "Set-Cookie",
                        format!("ho={s}-ho; HttpOnly; SameSite=None; Secure"),
                    ),
                    ("Set-Cookie", format!("lax={s}-lax; SameSite=Lax")),
                ]),
                "<p>signed in</p>",
            ),
            ("OPTIONS", "/write") => {
                let allowed = match allow {
                    Some(_) => vec![
                        ("Access-Control-Allow-Methods", "PUT".to_owned()),
                        ("Access-Control-Allow-Headers", "X-Write".to_owned()),
                    ],
                    None => Vec::new(),
                };
                reply(204, "No Content", &with(&allowed), "")
            }
            (_, "/write") => reply(
                200,
                "OK",
                &with(&[text("text/plain")]),
                &format!("written {s}"),
            ),
            (_, "/secret") => reply(
                200,
                s,
                &with(&[
                    text("text/plain"),
                    ("X-Secret", s.to_owned()),
                    ("X-Hidden", s.to_owned()),
                ]),
                s,
            ),
            (_, "/secret.js") => reply(
                200,
                "OK",
                &with(&[text("text/javascript")]),
                &format!("throw new Error('{s}');"),
            ),
            (_, "/secret.css") => reply(
                200,
                "OK",
                &with(&[text("text/plain")]),
                &format!("#probe {{ font-family: \"{s}\" }}"),
            ),
            (_, "/cookies") => reply(
                200,
                "OK",
                &with(&[text("text/plain")]),
                request.header("Cookie").unwrap_or(""),
            ),
            (_, "/redirect") => reply(
                302,
                "Found",
                &with(&[("Location", format!("/landing?{s}"))]),
                "",
            ),
            (_, target) if target.starts_with("/landing") => {
                reply(200, "OK", &with(&[text("text/plain")]), "landed")
            }
            (_, "/policy") => reply(
                200,
                "OK",
                &with(&[
                    text("text/plain"),
                    ("Cross-Origin-Resource-Policy", "same-origin".to_owned()),
                ]),
                s,
            ),
            _ => reply(404, "Not Found", &with(&[]), ""),
        }
    })
}

/// What the page learned and what it ran, and the requests the victim
/// had from it, the victim allowing it by CORS or not. The user signed
/// in to the victim first, so it holds the victim's cookies.
fn attack(allow: bool) -> (String, String, Vec<ServerRequest>) {
    let secret = format!(
        "s{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos()
    );
    let page = Arc::new(OnceLock::<String>::new());
    let attacker = {
        let page = page.clone();
        TestServer::plain(
            move |request: &ServerRequest| match request.target.as_str() {
                "/page.html" => Reply::ok("text/html", page.get().expect("the page").as_bytes()),
                _ => Reply::with_fields(404, &[], b""),
            },
        )
    };
    let a = format!("http://a.localhost:{}", attacker.port());
    let victim = victim(secret.clone(), allow.then(|| a.clone()));
    let b = format!("http://b.localhost:{}", victim.port());
    page.set(
        PAGE.replace("{{victim}}", &b)
            .replace("{{secret}}", &secret),
    )
    .expect("set once");
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, Url::parse(&format!("{b}/login")).unwrap());
    // Its icon too, which is same-site and carries the Lax cookie: what
    // comes after is the page's.
    tab.settle();
    let before = victim.requests().len();
    load_all(&mut tab, Url::parse(&format!("{a}/page.html")).unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    let line = loop {
        tab.engine.poll_network();
        tab.engine.run_tasks();
        let lines = tab.engine.take_console();
        if let Some(entry) = lines
            .iter()
            .find(|entry| entry.text.starts_with("{\"learned\""))
        {
            break entry.text.clone();
        }
        assert!(Instant::now() < deadline, "the page did not finish");
        std::thread::sleep(Duration::from_millis(2));
    };
    let (learned, ran) = line
        .strip_prefix("{\"learned\":")
        .and_then(|rest| rest.strip_suffix('}'))
        .and_then(|rest| rest.split_once(",\"ran\":"))
        .expect("the page's line");
    let requests = victim.requests()[before..].to_vec();
    (learned.to_owned(), ran.to_owned(), requests)
}

/// Checks the cookies the page's requests carried, as the reference
/// browser's did: the Lax cookie never, across sites; the HttpOnly one,
/// `SameSite=None`, with each of the three reads of `/cookies` that
/// include credentials, whatever the victim allows.
fn check_cookies(requests: &[ServerRequest]) {
    let cookie = |request: &ServerRequest| request.header("Cookie").unwrap_or("").to_owned();
    assert!(
        requests
            .iter()
            .all(|request| !cookie(request).contains("lax="))
    );
    let credentialed = requests
        .iter()
        .filter(|request| request.target == "/cookies" && cookie(request).contains("ho="))
        .count();
    assert_eq!(credentialed, 3);
}

/// `reads` as the page writes a list: sorted, in JSON.
fn json(reads: &[&str]) -> String {
    let mut reads = reads.to_vec();
    reads.sort_unstable();
    let quoted: Vec<String> = reads.iter().map(|read| format!("\"{read}\"")).collect();
    format!("[{}]", quoted.join(","))
}

#[test]
fn a_page_learns_nothing_of_an_origin_that_allows_nothing() {
    let (learned, ran, requests) = attack(false);
    assert_eq!(learned, "[]");
    assert_eq!(ran, json(&READS));
    // The write was refused at its preflight.
    assert!(requests.iter().all(|request| request.method != "PUT"));
    assert!(requests.iter().any(|request| request.method == "OPTIONS"));
    check_cookies(&requests);
}

#[test]
fn a_page_learns_by_cors_what_the_origin_allows_and_nothing_more() {
    let (learned, ran, requests) = attack(true);
    assert_eq!(learned, json(&CORS_READS));
    assert_eq!(ran, json(&READS));
    let puts = requests
        .iter()
        .filter(|request| request.method == "PUT")
        .count();
    assert_eq!(puts, 1);
    check_cookies(&requests);
}
