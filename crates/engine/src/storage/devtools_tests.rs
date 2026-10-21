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
    let server = TestServer::plain(|_| {
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
