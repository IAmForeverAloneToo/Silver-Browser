//! Local Network Access's prompt (M33.4, docs/design/hardening.md): a
//! secure public page's request to the local network waits for the
//! user, who lets its site or not; an insecure page's is refused at
//! once; a frame asks as its top-level site when its origin or its
//! element's `allow` lets it; workers ask as their creator; the answers
//! are kept in the profile. The test servers are on this machine, put
//! in the public or the local space by the client's overrides.

use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use net::test_server::{Reply, ServerRequest, TestAuthority, TestServer, VALID};
use net::transport::Config;
use net::{AddressSpace, Url};
use profile::{Memory, Profile};

use crate::PermissionPrompt;
use crate::testing::*;

const LOCAL: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

/// The servers: the public site (`a.localhost` and `b.localhost` over
/// TLS), a printer on the local network (`printer.test`, plain), a
/// router's page there (`router.localhost` over TLS), and an insecure
/// public site (`insecure.test`, plain).
struct Servers {
    authority: TestAuthority,
    site: TestServer,
    printer: TestServer,
    router: TestServer,
    insecure: TestServer,
}

/// The script that asks the printer, saying `tag` and what came of it
/// with `say`.
fn probe(printer: u16, tag: &str, say: &str) -> String {
    format!(
        "fetch('http://printer.test:{printer}/status', {{ targetAddressSpace: 'local' }})\
         .then(r => r.text()).then(t => {say}('{tag} got ' + t), e => {say}('{tag} failed'));"
    )
}

fn servers() -> Servers {
    let authority = TestAuthority::new();
    let printer = TestServer::plain(|_: &ServerRequest| {
        Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/plain"),
                ("Access-Control-Allow-Origin", "*"),
            ],
            b"local",
        )
    });
    let port = printer.port();
    let router = TestServer::tls(
        authority.server(&["router.localhost"], VALID),
        |_: &ServerRequest| Reply::ok("text/html", b"<title>router</title>"),
    );
    let router_url = format!("https://router.localhost:{}/", router.port());
    let site = TestServer::tls(
        authority.server(&["a.localhost", "b.localhost"], VALID),
        move |request: &ServerRequest| {
            let path = request.target.as_str();
            // Its own port, as the request's Host says it.
            let own = request
                .header("Host")
                .and_then(|host| host.rsplit(':').next());
            let host = |name: &str| format!("https://{name}.localhost:{}", own.unwrap_or("0"));
            let frame = |attributes: &str, src: &str| {
                format!(
                    "<script>addEventListener('message', e => console.log(e.data));</script>\
                     <iframe {attributes} src='{src}'></iframe>"
                )
            };
            let page = match path {
                "/page.html" => format!(
                    "<script>{} {}</script>",
                    probe(port, "one", "console.log"),
                    "function again() { ".to_owned() + &probe(port, "two", "console.log") + " }"
                ),
                "/frame.html" => format!(
                    "<script>function tell(m) {{ parent.postMessage(m, '*'); }} {}</script>",
                    probe(port, "frame", "tell")
                ),
                "/top-none.html" => frame("", &format!("{}/frame.html", host("b"))),
                "/top-allow.html" => frame(
                    "allow='local-network'",
                    &format!("{}/frame.html", host("b")),
                ),
                "/top-same.html" => frame("", &format!("{}/frame.html", host("a"))),
                "/top-router.html" => frame("", &router_url),
                "/top-router-allow.html" => frame("allow='local-network-access'", &router_url),
                "/worker.html" => {
                    "<script>new Worker('w.js').onmessage = e => console.log(e.data);</script>"
                        .to_owned()
                }
                "/w.js" => {
                    return Reply::ok(
                        "text/javascript",
                        probe(port, "worker", "postMessage").as_bytes(),
                    );
                }
                _ => "<p>other".to_owned(),
            };
            Reply::ok("text/html", page.as_bytes())
        },
    );
    let insecure = TestServer::plain(move |_: &ServerRequest| {
        Reply::ok(
            "text/html",
            format!(
                "<script>{}</script>",
                probe(port, "insecure", "console.log")
            )
            .as_bytes(),
        )
    });
    Servers {
        authority,
        site,
        printer,
        router,
        insecure,
    }
}

/// `config` with the server put in `space`, at both of this machine's
/// addresses: a `localhost` name gives the IPv6 one too.
fn put(config: Config, server: &TestServer, space: AddressSpace) -> Config {
    let v6 = IpAddr::V6(std::net::Ipv6Addr::LOCALHOST);
    config
        .treat_as(SocketAddr::new(LOCAL, server.port()), space)
        .treat_as(SocketAddr::new(v6, server.port()), space)
}

/// A tab whose client trusts the servers' root and puts each server in
/// its space.
fn tab(servers: &Servers) -> Tab {
    let mut tab = Tab::new();
    let config = Config::default()
        .trust(servers.authority.root.clone())
        .resolve("printer.test", LOCAL)
        .resolve("insecure.test", LOCAL);
    let config = put(config, &servers.site, AddressSpace::Public);
    let config = put(config, &servers.insecure, AddressSpace::Public);
    let config = put(config, &servers.printer, AddressSpace::Local);
    let config = put(config, &servers.router, AddressSpace::Local);
    tab.engine.set_network(net::Client::new(config));
    tab.engine.set_scripting(true);
    tab
}

fn site(servers: &Servers, host: &str, path: &str) -> Url {
    Url::parse(&format!("https://{host}:{}{path}", servers.site.port())).unwrap()
}

/// Runs the tab's work, keeping what its console says, until `done`
/// holds of it; fails after ten seconds.
fn run_until(tab: &mut Tab, lines: &mut Vec<String>, done: impl Fn(&Tab, &[String]) -> bool) {
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
        if done(tab, lines) {
            return;
        }
        assert!(Instant::now() < deadline, "only {lines:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Waits for a prompt.
fn prompted(tab: &mut Tab, lines: &mut Vec<String>) -> PermissionPrompt {
    run_until(tab, lines, |tab, _| {
        tab.engine.permission_prompt().is_some()
    });
    tab.engine.permission_prompt().unwrap()
}

/// Waits for a line that ends with `line`, with no prompt asked; the
/// lines so far are taken.
fn said(tab: &mut Tab, lines: &mut Vec<String>, line: &str) {
    run_until(tab, lines, |_, lines| {
        lines.iter().any(|said| said.ends_with(line))
    });
    assert_eq!(tab.engine.permission_prompt(), None, "{lines:?}");
    lines.clear();
}

#[test]
fn a_public_pages_request_to_the_local_network_waits_for_the_user() {
    let servers = servers();
    let mut tab = tab(&servers);
    let mut lines = Vec::new();
    tab.engine
        .navigate(site(&servers, "a.localhost", "/page.html"));
    let prompt = prompted(&mut tab, &mut lines);
    let origin = format!("https://a.localhost:{}", servers.site.port());
    assert_eq!(prompt.origin, origin);
    assert_eq!(prompt.permission, crate::Permission::LocalNetwork);
    assert!(servers.printer.requests().is_empty(), "nothing sent yet");
    tab.engine.answer_prompt(prompt.id, Some(true));
    said(&mut tab, &mut lines, "one got local");
    // Kept: not asked again.
    tab.engine.run_classic("again();");
    said(&mut tab, &mut lines, "two got local");
    // A worker asks as its creator, and its site was let.
    tab.engine
        .navigate(site(&servers, "a.localhost", "/worker.html"));
    said(&mut tab, &mut lines, "worker got local");
}

#[test]
fn a_blocked_or_dismissed_prompt_and_an_insecure_page_fail_the_request() {
    let servers = servers();
    let mut tab = tab(&servers);
    let mut lines = Vec::new();
    tab.engine
        .navigate(site(&servers, "a.localhost", "/page.html"));
    let prompt = prompted(&mut tab, &mut lines);
    tab.engine.answer_prompt(prompt.id, Some(false));
    said(&mut tab, &mut lines, "one failed");
    // Kept: refused without asking.
    tab.engine.run_classic("again();");
    said(&mut tab, &mut lines, "two failed");
    // Dismissed: refused, and asked again next time.
    tab.engine
        .navigate(site(&servers, "b.localhost", "/page.html"));
    let prompt = prompted(&mut tab, &mut lines);
    tab.engine.answer_prompt(prompt.id, None);
    said(&mut tab, &mut lines, "one failed");
    tab.engine.run_classic("again();");
    let again = prompted(&mut tab, &mut lines);
    assert_ne!(again.id, prompt.id);
    tab.engine.answer_prompt(again.id, None);
    // An insecure page is refused at once.
    let insecure = Url::parse(&format!(
        "http://insecure.test:{}/page.html",
        servers.insecure.port()
    ))
    .unwrap();
    tab.engine.navigate(insecure);
    said(&mut tab, &mut lines, "insecure failed");
    assert!(servers.printer.requests().is_empty());
}

#[test]
fn a_frame_asks_as_its_top_level_site_when_its_origin_or_allow_lets_it() {
    let servers = servers();
    let mut tab = tab(&servers);
    let mut lines = Vec::new();
    // Of another origin, no allow: refused at once.
    tab.engine
        .navigate(site(&servers, "a.localhost", "/top-none.html"));
    said(&mut tab, &mut lines, "frame failed");
    // Allowed by its element: its top-level site is asked about.
    tab.engine
        .navigate(site(&servers, "a.localhost", "/top-allow.html"));
    let prompt = prompted(&mut tab, &mut lines);
    let origin = format!("https://a.localhost:{}", servers.site.port());
    assert_eq!(prompt.origin, origin);
    tab.engine.answer_prompt(prompt.id, Some(true));
    said(&mut tab, &mut lines, "frame got local");
    // Of its parent's origin: by the default allowlist.
    tab.engine
        .navigate(site(&servers, "a.localhost", "/top-same.html"));
    said(&mut tab, &mut lines, "frame got local");
    // A frame's navigation into the local network asks as its
    // container's document: refused without `allow`, let with it.
    let loaded = |tab: &Tab| {
        tab.engine.load_state() == &crate::LoadState::Loaded
            && !tab.engine.shown.frames.is_empty()
            && tab.engine.shown.frames.values().all(|frame| {
                frame.engine.load_state() != &crate::LoadState::Loading
                    && frame.engine.navigation.is_none()
            })
    };
    tab.engine
        .navigate(site(&servers, "a.localhost", "/top-router.html"));
    run_until(&mut tab, &mut lines, |tab, _| loaded(tab));
    assert!(servers.router.requests().is_empty());
    tab.engine
        .navigate(site(&servers, "a.localhost", "/top-router-allow.html"));
    run_until(&mut tab, &mut lines, |_, _| {
        !servers.router.requests().is_empty()
    });
}

#[test]
fn the_answers_outlive_the_engine_in_its_profile() {
    let servers = servers();
    let memory = Memory::new();
    let on = |memory: &Memory| {
        let mut tab = tab(&servers);
        tab.engine
            .set_profile(Profile::in_memory(memory).unwrap())
            .unwrap();
        tab
    };
    let mut tab = on(&memory);
    let mut lines = Vec::new();
    tab.engine
        .navigate(site(&servers, "a.localhost", "/page.html"));
    let prompt = prompted(&mut tab, &mut lines);
    tab.engine.answer_prompt(prompt.id, Some(true));
    said(&mut tab, &mut lines, "one got local");
    // Written as the engine's tasks ran, with no flush asked for.
    let kept = Profile::in_memory(&memory.snapshot()).unwrap();
    assert_eq!(kept.permissions().unwrap().len(), 1);
    drop(tab);
    let mut tab = on(&memory);
    let mut lines = Vec::new();
    tab.engine
        .navigate(site(&servers, "a.localhost", "/page.html"));
    said(&mut tab, &mut lines, "one got local");
}
