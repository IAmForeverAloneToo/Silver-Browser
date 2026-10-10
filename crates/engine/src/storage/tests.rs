//! The profile in the engine (M28): the cookies with an expiry kept
//! across engines, and written when a change has waited its interval
//! and when the engine is dropped.

use std::time::{Duration, SystemTime};

use net::Url;
use net::cookies::{Api, Context, Jar};
use net::test_server::{Reply, ServerRequest, TestServer};
use profile::{Changes, Memory, Profile};

use super::FLUSH_INTERVAL;
use crate::testing::*;

/// A server whose pages set cookies, and one that reads them.
fn cookie_server() -> TestServer {
    serve(|path| match path {
        "/set.html" => Some(Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Set-Cookie", "kept=1; Max-Age=3600"),
                ("Set-Cookie", "session=1"),
            ],
            b"<p>set</p>",
        )),
        "/more.html" => Some(Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/html"),
                ("Set-Cookie", "more=1; Max-Age=3600"),
            ],
            b"<p>more</p>",
        )),
        "/session.html" => Some(Reply::with_fields(
            200,
            &[("Content-Type", "text/html"), ("Set-Cookie", "other=1")],
            b"<p>session</p>",
        )),
        "/read.html" => Some(html("<p>read</p>")),
        _ => None,
    })
}

/// The `Cookie` the server was last sent for `path`.
fn sent(server: &TestServer, path: &str) -> Option<String> {
    server
        .requests()
        .into_iter()
        .rev()
        .find(|request: &ServerRequest| request.target == path)
        .and_then(|request| request.header("Cookie").map(str::to_owned))
}

/// A tab whose engine keeps what it keeps in `memory`.
fn on(memory: &Memory) -> Tab {
    let mut tab = Tab::new();
    tab.engine
        .set_profile(Profile::in_memory(memory).unwrap())
        .unwrap();
    tab
}

/// How many cookies `memory` has written, as a browser stopped now
/// would find them.
fn written(memory: &Memory) -> usize {
    let profile = Profile::in_memory(&memory.snapshot()).unwrap();
    profile.cookies().unwrap().len()
}

#[test]
fn the_cookies_with_an_expiry_outlive_the_engine_in_its_profile() {
    let server = cookie_server();
    let memory = Memory::new();
    let mut tab = on(&memory);
    tab.load(server.url("/set.html"));
    tab.engine.flush_profile().unwrap();
    drop(tab);
    let mut tab = on(&memory);
    tab.load(server.url("/read.html"));
    assert_eq!(sent(&server, "/read.html").as_deref(), Some("kept=1"));
    // An engine with no profile starts with none.
    let mut tab = Tab::new();
    tab.load(server.url("/read.html"));
    assert_eq!(sent(&server, "/read.html"), None);
}

#[test]
fn an_engine_dropped_writes_what_changed() {
    let server = cookie_server();
    let memory = Memory::new();
    let mut tab = on(&memory);
    tab.load(server.url("/set.html"));
    drop(tab);
    assert_eq!(written(&memory), 1);
}

#[test]
fn a_change_is_written_when_its_interval_has_passed() {
    let server = cookie_server();
    let memory = Memory::new();
    let mut tab = on(&memory);
    let clock = tab.hand_clock();
    assert_eq!(tab.engine.next_task_at(), None);
    // Nothing written yet: written at once.
    tab.load(server.url("/set.html"));
    assert_eq!(tab.engine.next_task_at(), Some(Duration::ZERO));
    tab.engine.run_tasks();
    assert_eq!(tab.engine.next_task_at(), None);
    assert_eq!(written(&memory), 1);
    // Then an interval after the last write.
    tab.load(server.url("/more.html"));
    assert_eq!(tab.engine.next_task_at(), Some(FLUSH_INTERVAL));
    tab.engine.run_tasks();
    assert_eq!(written(&memory), 1);
    clock.advance(FLUSH_INTERVAL.as_millis() as u64);
    tab.engine.run_tasks();
    assert_eq!(written(&memory), 2);
    assert_eq!(tab.engine.next_task_at(), None);
    // A session cookie waits for nothing.
    tab.load(server.url("/session.html"));
    assert_eq!(tab.engine.next_task_at(), None);
}

#[test]
fn a_profiles_expired_cookies_are_dropped_and_it_is_written_again() {
    let memory = Memory::new();
    {
        let hours_ago = SystemTime::now() - Duration::from_secs(7200);
        let mut jar = Jar::new();
        let at = Url::parse("http://a.test/").unwrap();
        for line in ["gone=1; Max-Age=60", "kept=1; Max-Age=86400"] {
            jar.store(
                line.as_bytes(),
                &at,
                Api::Http,
                Context::SAME_SITE,
                hours_ago,
            );
        }
        let mut changes = Changes::new();
        changes.cookies(jar.export());
        Profile::in_memory(&memory)
            .unwrap()
            .commit(&changes)
            .unwrap();
    }
    assert_eq!(written(&memory), 2);
    let mut tab = on(&memory);
    tab.hand_clock();
    assert_eq!(tab.engine.next_task_at(), Some(Duration::ZERO));
    tab.engine.run_tasks();
    assert_eq!(written(&memory), 1);
    // And kept ones alone are not written again.
    drop(tab);
    let tab = on(&memory);
    assert_eq!(tab.engine.next_task_at(), None);
}

#[test]
fn an_engine_with_no_profile_waits_for_nothing() {
    let server = cookie_server();
    let mut tab = Tab::new();
    tab.load(server.url("/set.html"));
    assert_eq!(tab.engine.next_task_at(), None);
    tab.engine.flush_profile().unwrap();
}
