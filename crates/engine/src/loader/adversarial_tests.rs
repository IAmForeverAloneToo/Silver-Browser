//! The adversarial site, its second version (M33.8,
//! docs/design/hardening.md): the pages of `tests/security/` beside
//! M27.7's, each attacking what M33 defends, served by one server under
//! several names: the attacker at `a.localhost`, the victim at
//! `b.localhost`, `plain.test` for a site not served securely,
//! `public.test` for a public one and `hsts.localhost` for a host known
//! as HTTPS only, and a printer on the local network beside it. Each
//! runs with the defences on, where no attack may succeed, and with the
//! defence each meets taken away, where every one must, so that none
//! passes by failing at everything. An attack succeeded when its page
//! says so or when a server had a request that should not have come.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use net::test_server::{Reply, ServerRequest, TestServer};
use net::transport::Config;
use net::{AddressSpace, Url};

use super::tests::load_all;
use crate::testing::*;

const FRAMES: &str = include_str!("../../tests/security/frames.html");
const CSP: &str = include_str!("../../tests/security/csp.html");
const COOKIES: &str = include_str!("../../tests/security/cookies.html");
const NETWORK: &str = include_str!("../../tests/security/network.html");

/// The frames' attacks: what the page reads, and what reached the
/// victim's server.
const FRAME_ATTACKS: [&str; 17] = [
    "frame cookie",
    "frame document",
    "frame location",
    "frame name",
    "frame storage",
    "frame window document",
    "frame-ancestors",
    "message for another origin",
    "parent document from a frame",
    "sandboxed cookie",
    "sandboxed handler",
    "sandboxed parent cookie",
    "sandboxed parent storage",
    "sandboxed script",
    "sandboxed storage",
    "sandboxed-form",
    "x-frame-options",
];

/// The frames' attacks that reach across frames: they fail with the
/// defences gone too, every frame being of an origin of its own to
/// every other until M34 (G90), and must succeed then once it closes.
const ACROSS_FRAMES: [&str; 9] = [
    "frame cookie",
    "frame document",
    "frame location",
    "frame name",
    "frame storage",
    "frame window document",
    "parent document from a frame",
    "sandboxed parent cookie",
    "sandboxed parent storage",
];

/// The injected markup's attacks.
const CSP_ATTACKS: [&str; 15] = [
    "base",
    "eval",
    "event handler",
    "fetch",
    "form",
    "frame",
    "function",
    "image",
    "inline script",
    "javascript url",
    "nonce selector",
    "string timer",
    "style attribute",
    "style element",
    "webassembly",
];

/// The cookie page's requests and its reader's reads.
const COOKIE_REQUESTS: [&str; 4] = ["fetch", "form", "frame", "image"];
const COOKIE_READS: [&str; 4] = [
    "third-party lax cookie",
    "third-party partitioned cookie",
    "third-party storage",
    "third-party strict cookie",
];

/// The victim's cookies, all but the `HttpOnly` one, by name.
const COOKIE_NAMES: [&str; 3] = ["lax", "partitioned", "strict"];

const LOCAL: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// What a run found: the attacks that succeeded, by its page's word or
/// a server's record, and the attacks its page made.
#[derive(Debug)]
struct Outcome {
    pwned: BTreeSet<String>,
    ran: BTreeSet<String>,
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// A secret no page could guess.
fn secret() -> String {
    format!(
        "s{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos()
    )
}

/// The server of every name but the printer's, the defences on or off;
/// the printer is at `printer`.
fn site(defended: bool, secret: String, printer: u16) -> TestServer {
    TestServer::plain(move |request: &ServerRequest| {
        let s = secret.as_str();
        let host = request.header("Host").unwrap_or_default();
        let (name, port) = host.rsplit_once(':').unwrap_or((host, ""));
        let origin = |name: &str| format!("http://{name}:{port}");
        let attacker = origin("a.localhost");
        let victim = origin("b.localhost");
        let fill = |page: &str| {
            page.replace("{{secret}}", s)
                .replace("{{attacker}}", &attacker)
                .replace("{{defended}}", if defended { "true" } else { "false" })
        };
        let html = |fields: &[(&str, String)], body: String| {
            let mut all = vec![("Content-Type", "text/html".to_owned())];
            all.extend_from_slice(fields);
            let all: Vec<(&str, &str)> = all.iter().map(|(n, v)| (*n, v.as_str())).collect();
            Reply::with_fields(200, &all, body.as_bytes())
        };
        let when = |on: bool, field: (&'static str, String)| {
            if on { vec![field] } else { Vec::new() }
        };
        let path = request.target.as_str();
        match path.split('?').next().unwrap_or_default() {
            // The frames: the victim is the attacker's own origin with the
            // defences off.
            "/frames.html" => {
                let page = fill(FRAMES)
                    .replace("{{victim}}", if defended { &victim } else { &attacker })
                    .replace(
                        "{{sandbox-scripts}}",
                        if defended {
                            "sandbox=\"allow-scripts\""
                        } else {
                            ""
                        },
                    )
                    .replace("{{sandbox}}", if defended { "sandbox=\"\"" } else { "" });
                html(&[], page)
            }
            "/framed-xfo" => html(
                &when(defended, ("X-Frame-Options", "DENY".to_owned())),
                "<img src=/pwned?x-frame-options>".to_owned(),
            ),
            "/framed-ancestors" => html(
                &when(
                    defended,
                    (
                        "Content-Security-Policy",
                        "frame-ancestors 'none'".to_owned(),
                    ),
                ),
                "<img src=/pwned?frame-ancestors>".to_owned(),
            ),
            "/sandboxed.html" => html(&[], fill(SANDBOXED)),
            "/victim.html" => html(&[], fill(VICTIM)),
            "/spy.html" => html(&[], fill(SPY)),
            // The injected markup, under the victim's policy or none.
            "/csp.html" => {
                let nonce = format!("n{s}");
                let policy = format!(
                    "default-src 'self'; script-src 'nonce-{nonce}'; style-src 'nonce-{nonce}'; \
                     img-src 'self'; form-action 'self'; base-uri 'none'; frame-src 'none'"
                );
                html(
                    &when(defended, ("Content-Security-Policy", policy)),
                    fill(CSP).replace("{{nonce}}", &nonce),
                )
            }
            // The script the attacker's `<base>` would take the page's
            // relative script to.
            "/lib.js" => {
                let body: &[u8] = if name == "a.localhost" {
                    b"pwn('base')"
                } else {
                    b"// the page's own"
                };
                Reply::ok("text/javascript", body)
            }
            // The cookies: the user signs in at the victim first; with the
            // defences off the attacker's page is the victim's site's and
            // the cookies have no `HttpOnly` or `Secure`.
            "/login" => html(
                &[
                    ("Set-Cookie", format!("strict={s}; SameSite=Strict")),
                    ("Set-Cookie", format!("lax={s}; SameSite=Lax")),
                    (
                        "Set-Cookie",
                        format!("partitioned={s}; SameSite=None; Secure; Partitioned"),
                    ),
                    (
                        "Set-Cookie",
                        format!("ho={s}-ho{}", if defended { "; HttpOnly" } else { "" }),
                    ),
                ],
                fill("<script>localStorage.setItem('secret', '{{secret}}')</script>"),
            ),
            "/cookies.html" => html(&[], fill(COOKIES).replace("{{victim}}", &victim)),
            "/cookies" => Reply::ok("text/plain", b"cookies"),
            "/reader.html" => html(&[], fill(READER)),
            "/xss.html" => html(&[], fill(XSS)),
            "/insecure.html" => html(
                &[(
                    "Set-Cookie",
                    format!("sec1={s}-sec1{}", if defended { "; Secure" } else { "" }),
                )],
                fill(INSECURE),
            ),
            // A page whose path holds the secret, and whose image is the
            // attacker's.
            _ if path.starts_with("/account/") => html(
                &when(!defended, ("Referrer-Policy", "unsafe-url".to_owned())),
                fill(ACCOUNT),
            ),
            "/network.html" => html(
                &[],
                NETWORK
                    .replace("{{hsts}}", &origin("hsts.localhost"))
                    .replace("{{printer}}", &format!("http://printer.test:{printer}")),
            ),
            "/pwned" => Reply::ok("text/plain", b"pwned"),
            _ => Reply::with_fields(404, &[("Content-Type", "text/plain")], b"not here"),
        }
    })
}

/// The attacker's page in a frame of its own, sandboxed with scripts on:
/// what it reads of its parent and of its origin's cookie and storage.
const SANDBOXED: &str = "<script>\
     function tell(what, read) {\
       parent.postMessage({ ran: what }, '*');\
       try {\
         if (String(read()).indexOf('{{secret}}') >= 0) parent.postMessage({ pwned: what }, '*');\
       } catch (e) {}\
     }\
     tell('sandboxed parent cookie', () => parent.document.cookie);\
     tell('sandboxed parent storage', () => parent.localStorage.getItem('secret'));\
     tell('sandboxed cookie', () => document.cookie);\
     tell('sandboxed storage', () => localStorage.getItem('secret'));\
     </script>";

/// The victim's page in a frame: the secret in its text, its name, its
/// storage and its cookie; a frame of the attacker's origin, to which it
/// posts the secret meant for its own origin.
const VICTIM: &str = "<p>{{secret}}</p><script>\
     name = '{{secret}}';\
     try { localStorage.setItem('secret', '{{secret}}'); } catch (e) {}\
     document.cookie = 'victim={{secret}}';\
     </script><iframe src='{{attacker}}/spy.html'></iframe><script>\
     document.querySelector('iframe').onload = () => frames[0].postMessage('{{secret}}', location.origin);\
     </script>";

/// The attacker's frame in the victim's page: it reads its parent and
/// listens, and tells the top, the attacker's page.
const SPY: &str = "<script>\
     const SECRET = '{{secret}}';\
     top.postMessage({ ran: 'parent document from a frame' }, '*');\
     top.postMessage({ ran: 'message for another origin' }, '*');\
     try {\
       if (parent.document.body.textContent.indexOf(SECRET) >= 0)\
         top.postMessage({ pwned: 'parent document from a frame' }, '*');\
     } catch (e) {}\
     addEventListener('message', (e) => {\
       if (String(e.data).indexOf(SECRET) >= 0)\
         top.postMessage({ pwned: 'message for another origin' }, '*');\
     });\
     </script>";

/// The victim's page framed by the attacker's: what it finds of the
/// victim's storage and cookies, told to its parent.
const READER: &str = "<script>\
     function tell(what, value) {\
       parent.postMessage({ ran: what }, '*');\
       if (String(value).indexOf('{{secret}}') >= 0) parent.postMessage({ pwned: what }, '*');\
     }\
     let storage = null;\
     try { storage = localStorage.getItem('secret'); } catch (e) {}\
     tell('third-party storage', storage);\
     const cookies = document.cookie.split('; ');\
     for (const name of ['strict', 'lax', 'partitioned']) {\
       tell('third-party ' + name + ' cookie', cookies.filter((c) => c.startsWith(name + '=')).join());\
     }\
     </script>";

/// A script injected into the victim's page, reading its cookies.
const XSS: &str = "<script>console.log(JSON.stringify({\
     pwned: document.cookie.indexOf('{{secret}}-ho') >= 0 ? ['httponly'] : [],\
     ran: ['httponly'],\
     }))</script>";

/// A page not served securely that keeps a `Secure` cookie, from its
/// response and from its script.
const INSECURE: &str = "<script>\
     document.cookie = 'sec2={{secret}}-sec2' + ({{defended}} ? '; Secure' : '');\
     const pwned = [];\
     if (document.cookie.indexOf('{{secret}}-sec1') >= 0) pwned.push('secure field');\
     if (document.cookie.indexOf('{{secret}}-sec2') >= 0) pwned.push('secure script');\
     console.log(JSON.stringify({ pwned, ran: ['secure field', 'secure script'] }));\
     </script>";

/// A victim's page whose path holds the secret, with an image of the
/// attacker's.
const ACCOUNT: &str = "<img src='{{attacker}}/pwned-referer.png'><script>\
     onload = () => console.log(JSON.stringify({ pwned: [], ran: ['referer'] }));\
     </script>";

/// What reached a server that should not have: `/pwned?attack`, the
/// victim's cookies on `/cookies?request`, and the secret in the
/// `Referer` of the attacker's image.
fn records(requests: &[ServerRequest], secret: &str) -> Vec<String> {
    let mut found = Vec::new();
    for request in requests {
        let target = request.target.as_str();
        if let Some(attack) = target.strip_prefix("/pwned?") {
            found.push(attack.to_owned());
        } else if let Some(kind) = target.strip_prefix("/cookies?") {
            let cookie = request.header("Cookie").unwrap_or_default();
            for name in COOKIE_NAMES {
                if cookie.contains(&format!("{name}={secret}")) {
                    found.push(format!("{kind} {name}"));
                }
            }
        } else if target == "/pwned-referer.png"
            && request
                .header("Referer")
                .is_some_and(|referer| referer.contains(secret))
        {
            found.push("referer".to_owned());
        }
    }
    found
}

/// What a page's line, `{"pwned":[...],"ran":[...]}`, says.
fn outcome_of(line: &str) -> Outcome {
    let (pwned, ran) = line
        .strip_prefix("{\"pwned\":")
        .and_then(|rest| rest.strip_suffix('}'))
        .and_then(|rest| rest.split_once(",\"ran\":"))
        .expect("the page's line");
    let list = |json: &str| -> BTreeSet<String> {
        json.trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .filter(|name| !name.is_empty())
            .map(|name| name.trim_matches('"').to_owned())
            .collect()
    };
    Outcome {
        pwned: list(pwned),
        ran: list(ran),
    }
}

/// `config` with `server` put in `space`, at both of this machine's
/// addresses.
fn put(config: Config, server: &TestServer, space: AddressSpace) -> Config {
    let v6 = IpAddr::V6(Ipv6Addr::LOCALHOST);
    config
        .treat_as(SocketAddr::new(LOCAL, server.port()), space)
        .treat_as(SocketAddr::new(v6, server.port()), space)
}

/// Runs the page at `path` on `host`, the defences on or off, after the
/// user signed in at the victim when `signed_in`: what the page said
/// and what reached the servers. It runs on after the page's line for
/// the requests on their way: until every attack in `awaited` is
/// recorded, and half a second at least.
fn attack(defended: bool, signed_in: bool, host: &str, path: &str, awaited: &[&str]) -> Outcome {
    let secret = secret();
    let printer = TestServer::plain(|_: &ServerRequest| Reply::ok("text/plain", b"local"));
    let server = site(defended, secret.clone(), printer.port());
    let url = |host: &str, path: &str| {
        Url::parse(&format!("http://{host}:{}{path}", server.port())).expect("a URL")
    };
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    // The site is public with the defences on, so that the printer is
    // asked for across spaces; with them off, it is on the local network
    // too.
    let space = if defended {
        AddressSpace::Public
    } else {
        AddressSpace::Local
    };
    let config = Config::default()
        .resolve("plain.test", LOCAL)
        .resolve("public.test", LOCAL)
        .resolve("printer.test", LOCAL);
    let config = put(put(config, &server, space), &printer, AddressSpace::Local);
    tab.engine.set_network(net::Client::new(config));
    if defended {
        let hsts = tab.engine.network.hsts();
        let known = Url::parse("https://hsts.localhost/").expect("a URL");
        hsts.lock()
            .unwrap()
            .store(&known, "max-age=1000", SystemTime::now());
    }
    if signed_in {
        load_all(&mut tab, url("b.localhost", "/login"));
    }
    let path = path.replace("{{secret}}", &secret);
    tab.engine.navigate(url(host, &path));
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut line = None;
    let mut least = None;
    let mut said = Vec::new();
    loop {
        tab.engine.poll_network();
        tab.engine.run_tasks();
        said.extend(
            tab.engine
                .take_console()
                .into_iter()
                .map(|entry| entry.text),
        );
        if line.is_none() {
            line = said
                .iter()
                .find(|text| text.starts_with("{\"pwned\""))
                .cloned();
            least = line
                .as_ref()
                .map(|_| Instant::now() + Duration::from_millis(500));
        }
        let recorded = [&server, &printer]
            .into_iter()
            .flat_map(|server| records(&server.requests(), &secret));
        if let Some((mut outcome, least)) = line.as_deref().map(outcome_of).zip(least) {
            outcome.pwned.extend(recorded);
            if Instant::now() >= least
                && awaited.iter().all(|attack| outcome.pwned.contains(*attack))
            {
                return outcome;
            }
        }
        assert!(
            Instant::now() < deadline,
            "{path}, defended {defended}: no end, {said:?} and {:?}",
            server.requests()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_framing_page_reaches_nothing_its_frames_keep_and_all_with_their_defences_gone() {
    let defended = attack(true, false, "a.localhost", "/frames.html", &[]);
    assert_eq!(defended.pwned, set(&[]));
    assert_eq!(defended.ran, set(&FRAME_ATTACKS));
    let learned: Vec<&str> = FRAME_ATTACKS
        .into_iter()
        .filter(|attack| !ACROSS_FRAMES.contains(attack))
        .collect();
    let open = attack(false, false, "a.localhost", "/frames.html", &learned);
    assert_eq!(open.pwned, set(&learned));
    assert_eq!(open.ran, set(&FRAME_ATTACKS));
    // A sandboxed frame's navigation of the top and its popup, each a
    // run of its own, since either leaves the page.
    for late in ["top", "popup"] {
        let attack_name = match late {
            "top" => "sandboxed-top-navigation",
            _ => "sandboxed-popup",
        };
        let path = format!("/frames.html?{late}");
        let defended = attack(true, false, "a.localhost", &path, &[]);
        assert_eq!(defended.pwned, set(&[]), "{late}");
        let open = attack(false, false, "a.localhost", &path, &[attack_name]);
        assert!(open.pwned.contains(attack_name), "{late}: {open:?}");
    }
}

#[test]
fn injected_markup_runs_nothing_and_reaches_nothing_under_the_policy_and_all_without() {
    let defended = attack(true, false, "b.localhost", "/csp.html?form", &[]);
    assert_eq!(defended.pwned, set(&[]));
    assert_eq!(defended.ran, set(&CSP_ATTACKS));
    let open = attack(false, false, "b.localhost", "/csp.html?form", &CSP_ATTACKS);
    assert_eq!(open.pwned, set(&CSP_ATTACKS));
}

#[test]
fn another_site_carries_and_reads_none_of_the_victims_cookies_and_storage() {
    // Sent from the frame, the image, the form and the fetch: each
    // request with each cookie.
    let carried: Vec<String> = COOKIE_REQUESTS
        .iter()
        .flat_map(|request| COOKIE_NAMES.map(|name| format!("{request} {name}")))
        .collect();
    let carried: Vec<&str> = carried.iter().map(String::as_str).collect();
    let mut all = carried.clone();
    all.extend(COOKIE_READS);
    let defended = attack(true, true, "a.localhost", "/cookies.html", &[]);
    assert_eq!(defended.pwned, set(&[]));
    let mut ran = COOKIE_REQUESTS.to_vec();
    ran.extend(COOKIE_READS);
    assert_eq!(defended.ran, set(&ran));
    // From the victim's own site, all of it.
    let open = attack(false, true, "b.localhost", "/cookies.html", &all);
    assert_eq!(open.pwned, set(&all));
    // An `HttpOnly` cookie is not a script's, a `Secure` one is not kept
    // by a page not served securely, and a cross-site `Referer` does not
    // carry the path.
    for (signed_in, host, path, attacks) in [
        (true, "b.localhost", "/xss.html", &["httponly"][..]),
        (
            false,
            "plain.test",
            "/insecure.html",
            &["secure field", "secure script"][..],
        ),
        (
            false,
            "b.localhost",
            "/account/{{secret}}/page.html",
            &["referer"][..],
        ),
    ] {
        let defended = attack(true, signed_in, host, path, &[]);
        assert_eq!(defended.pwned, set(&[]), "{path}");
        assert_eq!(defended.ran, set(attacks), "{path}");
        let open = attack(false, signed_in, host, path, attacks);
        assert_eq!(open.pwned, set(attacks), "{path}");
    }
}

#[test]
fn a_public_page_sends_nothing_in_clear_to_a_known_host_or_to_the_local_network() {
    let attacks = ["hsts", "local-network"];
    let defended = attack(true, false, "public.test", "/network.html", &[]);
    assert_eq!(defended.pwned, set(&[]));
    assert_eq!(defended.ran, set(&attacks));
    let open = attack(false, false, "public.test", "/network.html", &attacks);
    assert_eq!(open.pwned, set(&attacks));
}
