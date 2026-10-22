//! What a site keeps, as the DevTools' storage panel reads it (M36.8):
//! its storage, cookies and databases listed, deleted and cleared, every
//! site the browser keeps anything for, and navigator.storage.

use devtools::{Json, Request};
use net::test_server::{Reply, TestServer};

use crate::testing::*;
use crate::view::View;

const KEEPER: &str = "<script>
localStorage.setItem('theme', 'dark');
sessionStorage.setItem('tab', '1');
document.cookie = 'seen=yes; max-age=3600';
const open = indexedDB.open('notes', 1);
open.onupgradeneeded = () => open.result.createObjectStore('n').put('hello', 1);
open.onsuccess = () => navigator.storage.estimate().then((e) => {
  console.log('estimate ' + (e.usage > 0) + ' ' + (e.quota > e.usage));
  return navigator.storage.persisted();
}).then((p) => console.log('persisted ' + p));
</script>";

fn ask(tab: &mut Tab, id: u64, method: &str, params: Json) -> Json {
    let request = Request {
        id,
        method: method.to_owned(),
        params,
    };
    let answer = tab.engine.storage_inspect(&request.write());
    let answer = Json::parse(&answer).expect("JSON");
    answer
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("{method}: {answer:?}"))
}

#[test]
fn a_sites_storage_is_listed_deleted_and_cleared() {
    // The page alone sets the cookie: a later request (its icon) setting
    // it again after the site is cleared would race the test.
    let server = TestServer::plain(|request| {
        if request.target != "/" {
            return Reply::with_fields(404, &[], b"");
        }
        Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Set-Cookie", "session=1; HttpOnly"),
            ],
            KEEPER.as_bytes(),
        )
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/"));
    let lines = tab.run_until(|lines| lines.iter().any(|l| l.starts_with("persisted")));
    assert_eq!(lines, ["estimate true true", "persisted false"]);
    // M38: what the HTTP cache keeps is the site's too.
    let cache = tab.engine.network.cache();
    let site = net::cache::Partition::from(server.url("/").origin().site());
    let fresh = || {
        let hit = cache.lookup(
            &site,
            &server.url("/r"),
            &net::HeaderList::new(),
            std::time::SystemTime::now(),
        );
        hit.is_some_and(|hit| hit.fresh)
    };
    cache.store(
        &site,
        &server.url("/r"),
        &net::HeaderList::new(),
        net::cache::Stored {
            status: 200,
            status_message: b"OK".to_vec(),
            headers: {
                let mut headers = net::HeaderList::new();
                headers.append(b"Cache-Control", b"max-age=600");
                headers
            },
            body: b"r".to_vec(),
            address_space: None,
        },
        std::time::SystemTime::now(),
        std::time::SystemTime::now(),
    );
    assert!(fresh());
    let local = ask(&mut tab, 1, "storage.local", Json::object([]));
    assert_eq!(local, Json::parse(r#"[["theme","dark"]]"#).unwrap());
    let session = ask(&mut tab, 2, "storage.session", Json::object([]));
    assert_eq!(session, Json::parse(r#"[["tab","1"]]"#).unwrap());
    // The cookies that go to the page, HttpOnly ones too.
    let cookies = ask(&mut tab, 3, "storage.cookies", Json::object([]));
    let mut names: Vec<&str> = cookies
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c.get("name").and_then(Json::as_str))
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["seen", "session"]);
    let idb = ask(&mut tab, 4, "storage.idb", Json::object([]));
    let db = &idb.as_array().unwrap()[0];
    assert_eq!(db.get("name").and_then(Json::as_str), Some("notes"));
    let store = &db.get("stores").and_then(Json::as_array).unwrap()[0];
    assert_eq!(store.get("count").and_then(Json::as_u64), Some(1));
    // The site is listed with what it uses.
    let origin = server.url("/").origin().serialization();
    let sites = ask(&mut tab, 5, "storage.sites", Json::object([]));
    let site = sites
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s.get("origin").and_then(Json::as_str) == Some(origin.as_str()))
        .expect("listed")
        .clone();
    assert!(site.get("usage").and_then(Json::as_u64).unwrap() > 0);
    // An item deleted, then the whole site cleared.
    ask(
        &mut tab,
        6,
        "storage.removeItem",
        Json::object([("area", Json::from("local")), ("key", Json::from("theme"))]),
    );
    assert_eq!(
        ask(&mut tab, 7, "storage.local", Json::object([])),
        Json::Array(vec![])
    );
    ask(&mut tab, 8, "storage.clearSite", Json::object([]));
    for (id, method) in [
        (9, "storage.session"),
        (10, "storage.cookies"),
        (11, "storage.idb"),
    ] {
        assert_eq!(
            ask(&mut tab, id, method, Json::object([])),
            Json::Array(vec![]),
            "{method}"
        );
    }
    let usage = ask(&mut tab, 12, "storage.usage", Json::object([]));
    assert_eq!(usage.get("usage").and_then(Json::as_u64), Some(0));
    assert!(!fresh(), "its cache entries gone");
    // A page of an opaque origin keeps nothing.
    tab.load(net::Url::parse("data:text/html,x").expect("a URL"));
    let refused = tab
        .engine
        .storage_inspect(r#"{"id":13,"method":"storage.local"}"#);
    assert_eq!(refused, r#"{"id":13,"error":"the page keeps nothing"}"#);
}

#[test]
fn a_sites_caches_are_listed_deleted_and_cleared() {
    // M39: the Cache API's caches in the storage panel.
    let server = TestServer::plain(|request| {
        if request.target != "/" {
            return Reply::with_fields(404, &[], b"");
        }
        Reply::with_fields(
            200,
            &[("Content-Type", "text/html")],
            b"<script>(async () => {
              await (await caches.open('one')).put('/a', new Response('12345'));
              await (await caches.open('two')).put('/b', new Response('x'));
              console.log('kept');
            })();</script>",
        )
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/"));
    tab.run_until(|lines| lines.iter().any(|l| l == "kept"));
    let origin = server.url("/").origin().serialization();
    let listed = ask(&mut tab, 1, "storage.caches", Json::object([]));
    let text = listed.write().replace(&origin, "ORIGIN");
    assert_eq!(
        text,
        r#"[{"name":"one","entries":[{"url":"ORIGIN/a","type":"default","status":200,"size":5}]},{"name":"two","entries":[{"url":"ORIGIN/b","type":"default","status":200,"size":1}]}]"#
    );
    // Listed among the sites, with what they use.
    let sites = ask(&mut tab, 2, "storage.sites", Json::object([]));
    assert!(sites.write().contains(&origin), "{}", sites.write());
    ask(
        &mut tab,
        3,
        "storage.deleteCache",
        Json::object([("name", Json::from("one"))]),
    );
    let listed = ask(&mut tab, 4, "storage.caches", Json::object([]));
    assert_eq!(listed.as_array().map(<[Json]>::len), Some(1));
    ask(&mut tab, 5, "storage.clearSite", Json::object([]));
    assert_eq!(
        ask(&mut tab, 6, "storage.caches", Json::object([])),
        Json::Array(vec![])
    );
}

/// A host that runs nothing but says each version's script listened for
/// nothing and ran; and whether it was asked to stop one.
struct Ran(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl crate::service_worker::ServiceWorkerHost for Ran {
    fn evaluate(
        &self,
        _start: crate::service_worker::Start,
        done: crate::service_worker::Done<Option<crate::service_worker::Evaluated>>,
    ) {
        done(Some(crate::service_worker::Evaluated::default()));
    }

    fn install(&self, _version: u64, done: crate::service_worker::Done<bool>) {
        done(true);
    }

    fn activate(&self, _version: u64, done: crate::service_worker::Done<()>) {
        done(());
    }

    fn terminate(&self, _version: u64) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn a_sites_service_workers_are_listed_and_unregistered() {
    // M39: the registrations in the storage panel.
    let server = serve(|path| match path {
        "/" => Some(html("<p>page</p>")),
        "/sw.js" => Some(Reply::with_fields(
            200,
            &[("Content-Type", "text/javascript")],
            b"1",
        )),
        _ => None,
    });
    let context = crate::Context::new();
    let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    context
        .service_workers()
        .set_host(std::sync::Arc::new(Ran(stopped.clone())));
    let origin = server.url("/").origin().serialization();
    let (sender, receiver) = std::sync::mpsc::channel();
    context.service_workers().schedule(
        crate::service_worker::Job {
            kind: crate::service_worker::JobKind::Register,
            key: origin.clone(),
            scope: server.url("/"),
            script: server.url("/sw.js"),
            update_via_cache: crate::service_worker::UpdateViaCache::Imports,
            force_bypass: false,
            partition: None,
        },
        Box::new(move |settled: crate::service_worker::Settled| {
            let _ = sender.send(settled.outcome);
        }),
    );
    receiver
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("registered");
    let mut tab = Tab::new();
    tab.engine.join(&context);
    tab.engine.navigate(server.url("/"));
    tab.wait_until(|engine| engine.load_state() == &crate::LoadState::Loaded);
    let mut listed = String::new();
    wait_for(|| {
        listed = ask(&mut tab, 1, "storage.serviceWorkers", Json::object([]))
            .write()
            .replace(&origin, "ORIGIN");
        listed.contains("activated")
    });
    assert_eq!(
        listed,
        r#"[{"scope":"ORIGIN/","versions":[{"slot":"active","script":"ORIGIN/sw.js","state":"activated"}]}]"#
    );
    let sites = ask(&mut tab, 2, "storage.sites", Json::object([]));
    assert!(sites.write().contains(&origin), "{}", sites.write());
    ask(
        &mut tab,
        3,
        "storage.unregister",
        Json::object([("scope", Json::from(format!("{origin}/")))]),
    );
    wait_for(|| {
        ask(&mut tab, 4, "storage.serviceWorkers", Json::object([])) == Json::Array(vec![])
    });
    // Its version stopped.
    wait_for(|| stopped.load(std::sync::atomic::Ordering::SeqCst));
}
