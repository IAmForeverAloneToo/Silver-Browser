//! Web Storage kept by the browser (M34.1): a renderer's areas through
//! its link, keyed by the browser, partitioned by what it knows of the
//! frame, held to the quota, and refused for another site's origin.

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use net::{Origin, Partition, Url};

use super::network::{FrameGrant, Grants, Lock};
use super::storage::StorageService;
use crate::ipc::{self, Sender, StorageOp, ToBrowser, ToRenderer};
use crate::renderer::link::{Link, RemoteAreas};
use crate::storage::{AreaKey, Areas, STORAGE_QUOTA, Storage};

const WAIT: Duration = Duration::from_secs(20);

/// A renderer's link to a browser's storage service, renderer 1 locked to
/// `lock` with frames by their numbers; the browser's storage; and where
/// the browser says why it would kill the renderer.
struct Wired {
    link: Arc<Link>,
    storage: Arc<Mutex<Storage>>,
    bad: mpsc::Receiver<String>,
}

/// The browser's areas, as tab 1's renderers reach them.
fn browser(wired: &Wired) -> crate::storage::TabAreas {
    crate::storage::TabAreas {
        storage: wired.storage.clone(),
        tab: 1,
    }
}

fn wired(lock: Lock, frames: &[(u32, FrameGrant)]) -> Wired {
    let grants = Grants::new();
    assert!(grants.set_lock(1, lock));
    for (frame, grant) in frames {
        grants.grant_frame(1, *frame, grant.clone());
    }
    let storage = Arc::new(Mutex::new(Storage::new()));
    let service = StorageService::new(storage.clone(), grants, 1);
    let ((browser_in, browser_out), (renderer_in, renderer_out)) = ipc::duplex();
    let reply = Sender::<ToRenderer>::new(browser_out);
    let (said, bad) = mpsc::channel();
    ipc::spawn_reader::<ToBrowser, _>(
        "test-browser",
        browser_in,
        move |message| match service.serve(1, message, &reply) {
            Ok(None) => {}
            Ok(Some(other)) => panic!("not the storage's: {other:?}"),
            Err(why) => {
                let _ = said.send(why);
            }
        },
        |_| {},
    )
    .expect("a thread");
    let link = Link::new(Sender::new(renderer_out));
    let routing = link.clone();
    ipc::spawn_reader::<ToRenderer, _>(
        "test-renderer",
        renderer_in,
        move |message| {
            let left = routing.route(message).expect("routed");
            assert!(left.is_none(), "{left:?}");
        },
        |_| {},
    )
    .expect("a thread");
    Wired { link, storage, bad }
}

fn url(text: &str) -> Url {
    Url::parse(text).expect("a URL")
}

fn top_of(origin: &Origin) -> FrameGrant {
    FrameGrant {
        top: origin.clone(),
        cross_site: false,
    }
}

fn key(origin: &Origin) -> AreaKey {
    AreaKey {
        origin: origin.clone(),
        partition: None,
    }
}

#[test]
fn a_renderers_areas_are_the_browsers() {
    let site = url("https://a.test/").origin();
    let wired = wired(Lock::of(&url("https://a.test/")), &[(7, top_of(&site))]);
    let areas = RemoteAreas::new(wired.link.clone(), 7);
    let key = key(&site);
    assert_eq!(areas.set(true, &key, "a", "1"), Ok(Some(None)), "new");
    assert_eq!(areas.set(true, &key, "b", "2"), Ok(Some(None)));
    assert_eq!(areas.set(true, &key, "a", "1"), Ok(None), "unchanged");
    assert_eq!(
        areas.set(true, &key, "a", "3"),
        Ok(Some(Some("1".into()))),
        "changed"
    );
    assert_eq!(areas.length(true, &key), 2);
    assert_eq!(areas.keys(true, &key), ["a", "b"]);
    assert_eq!(areas.key(true, &key, 1).as_deref(), Some("b"));
    assert_eq!(areas.get(true, &key, "a").as_deref(), Some("3"));
    assert_eq!(areas.get(false, &key, "a"), None, "sessionStorage apart");
    assert_eq!(areas.remove(true, &key, "a").as_deref(), Some("3"));
    assert_eq!(areas.remove(true, &key, "a"), None);
    // The browser's storage holds them: the renderer holds nothing.
    assert_eq!(browser(&wired).get(true, &key, "b").as_deref(), Some("2"));
    assert!(areas.clear(true, &key));
    assert!(!areas.clear(true, &key));
    assert_eq!(areas.length(true, &key), 0);
    let big = "x".repeat(STORAGE_QUOTA);
    assert!(
        areas.set(false, &key, "big", &big).is_err(),
        "past the quota"
    );
    assert!(wired.bad.try_recv().is_err());
}

#[test]
fn a_frame_of_another_site_has_its_areas_apart_by_the_top() {
    let site = url("https://b.test/").origin();
    let top = url("https://a.test/").origin();
    let elsewhere = url("https://c.test/").origin();
    let under = |top: &Origin| FrameGrant {
        top: top.clone(),
        cross_site: true,
    };
    let wired = wired(
        Lock::of(&url("https://b.test/")),
        &[(1, top_of(&site)), (2, under(&top)), (3, under(&elsewhere))],
    );
    let first = RemoteAreas::new(wired.link.clone(), 1);
    let framed = RemoteAreas::new(wired.link.clone(), 2);
    let other = RemoteAreas::new(wired.link.clone(), 3);
    // The renderer claims no partition: the browser's grant decides.
    let key = key(&site);
    assert!(first.set(true, &key, "k", "first").is_ok());
    assert!(framed.set(true, &key, "k", "under a").is_ok());
    assert_eq!(first.get(true, &key, "k").as_deref(), Some("first"));
    assert_eq!(framed.get(true, &key, "k").as_deref(), Some("under a"));
    assert_eq!(other.get(true, &key, "k"), None);
    let partitioned = AreaKey {
        origin: site.clone(),
        partition: Some(Partition {
            top: top.clone(),
            cross_site: true,
        }),
    };
    assert_eq!(
        browser(&wired).get(true, &partitioned, "k").as_deref(),
        Some("under a")
    );
    assert!(wired.bad.try_recv().is_err());
}

/// The reason the browser gave for killing the renderer when it asked
/// `op` of `origin`'s area in `partition` for frame `frame`.
fn refused(wired: &Wired, frame: u32, origin: Origin, partition: Option<Partition>) -> String {
    wired
        .link
        .send(&ToBrowser::Storage {
            call: 1,
            frame,
            local: true,
            origin,
            partition,
            op: StorageOp::Get { item: "k".into() },
        })
        .expect("sent");
    wired.bad.recv_timeout(WAIT).expect("a bad message")
}

#[test]
fn another_sites_areas_kill_the_renderer() {
    let site = url("https://a.test/").origin();
    let wired = wired(Lock::of(&url("https://a.test/")), &[(7, top_of(&site))]);
    assert_eq!(
        refused(&wired, 7, url("https://victim.test/").origin(), None),
        "the storage of https://victim.test"
    );
    assert_eq!(
        refused(&wired, 7, Origin::new_opaque(), None),
        "the storage of null"
    );
    assert_eq!(
        refused(&wired, 8, site.clone(), None),
        "frame 8 is not renderer 1's"
    );
    let victim = url("https://victim.test/").origin();
    assert_eq!(
        refused(
            &wired,
            7,
            site,
            Some(Partition {
                top: victim,
                cross_site: true
            })
        ),
        "a request in the partition of https://victim.test"
    );
}

#[test]
fn an_engines_scripts_reach_the_browsers_areas() {
    let server = net::test_server::TestServer::plain(|request| {
        let body: &[u8] = match request.target.as_str() {
            "/set.html" => {
                b"<script>localStorage.setItem('k', 'v'); \
                sessionStorage.s = 'one'; console.log(localStorage.length, \
                localStorage.getItem('k'), sessionStorage.s)</script>"
            }
            _ => {
                b"<script>console.log(localStorage.getItem('k'), \
                sessionStorage.getItem('s'))</script>"
            }
        };
        net::test_server::Reply::ok("text/html", body)
    });
    let page = server.url("/");
    let site = page.origin();
    let wired = wired(Lock::of(&page), &[(7, top_of(&site))]);
    let mut tab = crate::testing::Tab::new();
    tab.engine.set_scripting(true);
    tab.engine
        .set_remote_storage(Arc::new(RemoteAreas::new(wired.link.clone(), 7)));
    tab.engine.navigate(server.url("/set.html"));
    let lines = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(lines, ["1 v one"]);
    assert_eq!(
        browser(&wired).get(true, &key(&site), "k").as_deref(),
        Some("v")
    );
    // Another engine, as another renderer of the site, finds them.
    let mut other = crate::testing::Tab::new();
    other.engine.set_scripting(true);
    other
        .engine
        .set_remote_storage(Arc::new(RemoteAreas::new(wired.link.clone(), 7)));
    other.engine.navigate(server.url("/get.html"));
    assert_eq!(other.run_until(|lines| !lines.is_empty()), ["v one"]);
    assert!(wired.bad.try_recv().is_err());
}

#[test]
fn a_frames_scripts_reach_the_browsers_areas_too() {
    let server = net::test_server::TestServer::plain(|_| {
        net::test_server::Reply::ok(
            "text/html",
            b"<iframe srcdoc=\"<script>localStorage.setItem('f', 'framed'); \
              console.log('set')</script>\"></iframe>",
        )
    });
    let page = server.url("/page.html");
    let site = page.origin();
    let wired = wired(Lock::of(&page), &[(7, top_of(&site))]);
    let mut tab = crate::testing::Tab::new();
    tab.engine.set_scripting(true);
    tab.engine
        .set_remote_storage(Arc::new(RemoteAreas::new(wired.link.clone(), 7)));
    tab.engine.navigate(page);
    tab.run_until(|lines| lines.iter().any(|line| line.ends_with("] set")));
    assert_eq!(
        browser(&wired).get(true, &key(&site), "f").as_deref(),
        Some("framed")
    );
    assert!(wired.bad.try_recv().is_err());
}
