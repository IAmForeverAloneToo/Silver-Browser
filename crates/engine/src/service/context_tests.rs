//! Tabs of one context in many processes (M37): renderers numbered
//! apart, cookies and `localStorage` the context's, `sessionStorage`
//! each tab's, and a change of an area told by the browser to the other
//! tab's renderer as a `storage` event, never to a renderer of another
//! site.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use net::Url;
use net::test_server::{Reply, TestServer};

use super::{RemoteTab, ThreadLauncher};
use crate::view::View;
use crate::{Context, LoadState};

const WAIT: Duration = Duration::from_secs(30);

fn fonts() -> text::FontCollection {
    text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
}

fn tab_of(context: &Context) -> RemoteTab {
    let launcher = Arc::new(ThreadLauncher::with_fonts(fonts()));
    let mut tab = RemoteTab::in_context(launcher, fonts(), context);
    tab.set_viewport(320, 240);
    let began = Instant::now();
    tab.set_clock(Arc::new(move || began.elapsed()));
    tab.set_scripting(true);
    tab
}

/// Takes what the renderers say, in both tabs, until `done` holds.
fn settle(tabs: &mut [&mut RemoteTab], mut done: impl FnMut(&mut [&mut RemoteTab]) -> bool) {
    let began = Instant::now();
    loop {
        for tab in tabs.iter_mut() {
            tab.poll_network();
            tab.run_tasks();
        }
        if done(tabs) {
            return;
        }
        assert!(began.elapsed() < WAIT, "the tabs get there in time");
        thread::sleep(Duration::from_millis(2));
    }
}

fn load(tab: &mut RemoteTab, url: Url) {
    tab.navigate(url.clone());
    settle(&mut [tab], |tabs| {
        tabs[0].url() == Some(&url) && *tabs[0].load_state() == LoadState::Loaded
    });
}

fn console(tab: &mut RemoteTab) -> Vec<String> {
    tab.take_console().into_iter().map(|e| e.text).collect()
}

const HEARING: &str = "<script>addEventListener('storage', e => console.log('heard ' + \
                       e.key + ' ' + e.newValue + ' ' + (e.storageArea === localStorage) + ' ' + \
                       e.url.split('/').pop()));</script>";

fn server() -> TestServer {
    TestServer::plain(|request| match request.target.as_str() {
        "/listen.html" => Reply::ok("text/html", HEARING.as_bytes()),
        "/write.html" => Reply::ok(
            "text/html",
            format!(
                "{HEARING}<script>document.cookie = 'c=1'; localStorage.setItem('k', 'v'); \
                 sessionStorage.setItem('s', 'w'); console.log('wrote');</script>"
            )
            .as_bytes(),
        ),
        "/read.html" => Reply::ok(
            "text/html",
            b"<script>console.log(document.cookie + ' ' + localStorage.getItem('k') + ' ' + \
              sessionStorage.getItem('s'));</script>",
        ),
        _ => Reply::with_fields(404, &[], b""),
    })
}

#[test]
fn the_tabs_of_a_context_share_its_storage_and_number_renderers_apart() {
    let server = server();
    let context = Context::new();
    let mut first = tab_of(&context);
    let mut second = tab_of(&context);
    load(&mut first, server.url("/write.html"));
    settle(&mut [&mut first], |tabs| console(tabs[0]) == ["wrote"]);
    load(&mut second, server.url("/read.html"));
    settle(&mut [&mut second], |tabs| {
        let lines = console(tabs[0]);
        assert!(lines.is_empty() || lines == ["c=1 v null"], "{lines:?}");
        !lines.is_empty()
    });
    let mut numbers: Vec<u32> = first.renderer_numbers();
    numbers.extend(second.renderer_numbers());
    let count = numbers.len();
    numbers.sort_unstable();
    numbers.dedup();
    assert_eq!(
        numbers.len(),
        count,
        "no two tabs' renderers share a number"
    );
}

#[test]
fn a_change_is_told_to_the_other_tabs_renderer_not_to_its_maker() {
    let server = server();
    let context = Context::new();
    let mut first = tab_of(&context);
    let mut second = tab_of(&context);
    load(&mut second, server.url("/listen.html"));
    load(&mut first, server.url("/write.html"));
    let (mut heard, mut made) = (Vec::new(), Vec::new());
    settle(&mut [&mut first, &mut second], |tabs| {
        made.extend(console(tabs[0]));
        heard.extend(console(tabs[1]));
        !heard.is_empty() && !made.is_empty()
    });
    // Time for an echo to come, were one sent.
    let began = Instant::now();
    settle(&mut [&mut first, &mut second], |tabs| {
        made.extend(console(tabs[0]));
        heard.extend(console(tabs[1]));
        began.elapsed() > Duration::from_millis(500)
    });
    // The localStorage change alone: the sessionStorage one is the other
    // tab's.
    assert_eq!(heard, ["heard k v true write.html"]);
    assert_eq!(made, ["wrote"], "the maker hears none of its own");
}

#[test]
fn a_renderer_of_another_site_is_told_nothing() {
    let server = server();
    let other = TestServer::plain(|_| Reply::ok("text/html", HEARING.as_bytes()));
    let context = Context::new();
    let mut first = tab_of(&context);
    let mut second = tab_of(&context);
    // The same host by another name: another site, another renderer.
    let elsewhere = Url::parse(&format!(
        "http://localhost:{}/listen.html",
        other.url("/").port().expect("a port")
    ))
    .expect("parses");
    load(&mut second, elsewhere);
    load(&mut first, server.url("/write.html"));
    settle(&mut [&mut first], |tabs| console(tabs[0]) == ["wrote"]);
    thread::sleep(Duration::from_millis(300));
    settle(&mut [&mut first, &mut second], |_| true);
    assert!(console(&mut second).is_empty());
    assert_eq!(second.storage_changes_sent(), 0);
}
