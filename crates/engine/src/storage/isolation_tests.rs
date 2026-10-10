//! Storage isolation (M28.5): what one origin keeps another cannot
//! read, from both sides, in the session and after the profile is
//! closed and opened again, in memory and on disk; and the HTTP cache
//! kept apart by site.

use std::time::{Duration, Instant};

use net::Url;
use net::test_server::{Reply, TestServer};
use profile::{Memory, Profile};

use crate::LoadState;
use crate::testing::*;

/// Loads `url` in `tab` until it is shown and loaded.
fn load(tab: &mut Tab, url: &Url) {
    tab.engine.navigate(url.clone());
    tab.wait_until(|engine| {
        incoming(engine).is_none()
            && engine.url() == Some(url)
            && engine.load_state() == &LoadState::Loaded
    });
}

/// The console's lines, taken.
fn console(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// A server whose `/write.html` keeps its origin's name in each store,
/// and whose `/read.html` says what each store holds.
fn stores() -> TestServer {
    serve(|path| match path {
        "/write.html" => Some(html(
            "<script>localStorage.setItem('mine', location.host); \
             sessionStorage.setItem('mine', location.host); \
             document.cookie = 'mine=' + location.host + '; Max-Age=3600';</script>",
        )),
        "/read.html" => Some(html(
            "<script>const all = (s) => Array.from({ length: s.length }, (_, i) => \
             s.key(i) + '=' + s.getItem(s.key(i))).join(' '); \
             console.log('local ' + all(localStorage) + ' | session ' + \
             all(sessionStorage) + ' | cookie ' + document.cookie);</script>",
        )),
        _ => None,
    })
}

/// The origins the tests keep apart: two hosts of one server, the
/// server by its address, and another server, another port of that
/// address.
fn origins(server: &TestServer, other: &TestServer) -> [String; 4] {
    let port = server.url("/").port().unwrap();
    [
        format!("http://a.localhost:{port}"),
        format!("http://b.localhost:{port}"),
        format!("http://127.0.0.1:{port}"),
        format!("http://127.0.0.1:{}", other.url("/").port().unwrap()),
    ]
}

fn page(origin: &str, path: &str) -> Url {
    Url::parse(&format!("{origin}{path}")).unwrap()
}

/// What `tab` reads at each origin of `origins`.
fn read_each(tab: &mut Tab, origins: &[String]) -> Vec<String> {
    origins
        .iter()
        .flat_map(|origin| {
            load(tab, &page(origin, "/read.html"));
            console(tab)
        })
        .collect()
}

/// Each origin writes, then each reads, on a tab of `open`'s profile;
/// then, the profile written and closed, on a new tab of the profile
/// opened again.
fn each_origin_reads_its_own(open: impl Fn() -> Profile) {
    let (server, other) = (stores(), stores());
    let origins = origins(&server, &other);
    let host = |origin: &str| origin.trim_start_matches("http://").to_owned();
    let (a, b, ip, ip2) = (
        host(&origins[0]),
        host(&origins[1]),
        host(&origins[2]),
        host(&origins[3]),
    );
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_profile(open()).unwrap();
    for origin in &origins {
        load(&mut tab, &page(origin, "/write.html"));
    }
    // Cookies are a host's, whatever the port: the two ports of the
    // address share theirs, the last written; storage is the origin's.
    let in_session = [
        format!("local mine={a} | session mine={a} | cookie mine={a}"),
        format!("local mine={b} | session mine={b} | cookie mine={b}"),
        format!("local mine={ip} | session mine={ip} | cookie mine={ip2}"),
        format!("local mine={ip2} | session mine={ip2} | cookie mine={ip2}"),
    ];
    assert_eq!(read_each(&mut tab, &origins), in_session);
    tab.engine.flush_profile().unwrap();
    drop(tab);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_profile(open()).unwrap();
    let reopened: Vec<String> = in_session
        .iter()
        .map(|line| {
            let (local, rest) = line.split_once(" | session ").unwrap();
            let (_, cookie) = rest.split_once(" | ").unwrap();
            format!("{local} | session  | {cookie}")
        })
        .collect();
    assert_eq!(read_each(&mut tab, &origins), reopened);
    drop(tab);
    // The profile holds each origin's apart.
    let profile = open();
    for (origin, name) in origins.iter().zip([&a, &b, &ip, &ip2]) {
        let items = profile.local_storage(origin).unwrap();
        assert_eq!(items.len(), 1, "{origin}");
        assert_eq!((items[0].key.as_str(), &items[0].value), ("mine", name));
    }
}

#[test]
fn each_origin_reads_its_own_storage_alone_in_memory() {
    let memory = Memory::new();
    each_origin_reads_its_own(|| Profile::in_memory(&memory).unwrap());
}

#[test]
fn each_origin_reads_its_own_storage_alone_on_disk() {
    let dir = std::env::temp_dir().join(format!("silver-isolation-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    each_origin_reads_its_own(|| Profile::open(&dir).unwrap());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_tabs_session_storage_is_its_own() {
    let server = stores();
    let url = |path: &str| server.url(path);
    let mut first = Tab::new();
    first.engine.set_scripting(true);
    load(&mut first, &url("/write.html"));
    let mut second = Tab::new();
    second.engine.set_scripting(true);
    load(&mut second, &url("/read.html"));
    assert_eq!(console(&mut second), ["local  | session  | cookie "]);
    load(&mut first, &url("/read.html"));
    let origin = url("/").origin().serialization();
    let mine = origin.trim_start_matches("http://");
    assert_eq!(
        console(&mut first),
        [format!(
            "local mine={mine} | session mine={mine} | cookie mine={mine}"
        )]
    );
}

#[test]
fn the_http_cache_is_kept_apart_by_site() {
    // A resource every page fetches, which may be kept for ten minutes.
    let resources = serve(|path| match path {
        "/shared.txt" => Some(Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/plain"),
                ("Cache-Control", "max-age=600"),
                ("Access-Control-Allow-Origin", "*"),
            ],
            b"shared",
        )),
        _ => None,
    });
    let shared = resources.url("/shared.txt");
    let pages = serve(move |path| match path {
        "/page.html" => Some(html(&format!(
            "<script>fetch('{shared}').then((r) => r.text()).then((t) => \
             document.title = t)</script>"
        ))),
        _ => None,
    });
    let port = pages.url("/").port().unwrap();
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    let fetched = |tab: &mut Tab, host: &str| {
        let url = Url::parse(&format!("http://{host}:{port}/page.html")).unwrap();
        load(tab, &url);
        let deadline = Instant::now() + WAIT;
        while tab.engine.title() != "shared" {
            assert!(Instant::now() < deadline, "{host} fetched nothing");
            tab.engine.poll_network();
            tab.engine.run_tasks();
            std::thread::sleep(Duration::from_millis(2));
        }
        resources.requests().len()
    };
    // a.localhost and its subdomain are one site; b.localhost another.
    assert_eq!(fetched(&mut tab, "a.localhost"), 1);
    assert_eq!(fetched(&mut tab, "a.localhost"), 1);
    assert_eq!(fetched(&mut tab, "www.a.localhost"), 1);
    assert_eq!(fetched(&mut tab, "b.localhost"), 2);
    assert_eq!(fetched(&mut tab, "b.localhost"), 2);
    assert_eq!(fetched(&mut tab, "a.localhost"), 2);
}
