//! A renderer for each site of a tab (M34.1, D13, D28): navigations to
//! another site, by the address bar or by a page, shown by that site's
//! renderer, the response handed over and never fetched twice; the
//! history across renderers; the cap, eviction and restoring; the spare.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use net::test_server::{Reply, TestServer};

use super::{RemoteTab, ThreadLauncher};
use crate::view::View;
use crate::{LoadState, Url};

const WAIT: Duration = Duration::from_secs(30);

fn fonts() -> text::FontCollection {
    text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
}

fn tab(cap: usize, spare: bool) -> RemoteTab {
    let launcher = Arc::new(ThreadLauncher::with_fonts(fonts()));
    let mut tab = RemoteTab::with_cap(launcher, fonts(), cap, spare);
    tab.set_viewport(200, 150);
    tab.set_scripting(true);
    let began = Instant::now();
    tab.set_clock(Arc::new(move || began.elapsed()));
    tab
}

/// Takes what the renderers say until `done` holds of the tab.
fn settle(tab: &mut RemoteTab, what: &str, mut done: impl FnMut(&mut RemoteTab) -> bool) {
    let began = Instant::now();
    loop {
        tab.poll_network();
        tab.run_tasks();
        if done(tab) {
            return;
        }
        assert!(
            began.elapsed() < WAIT,
            "{what}: {:?} {:?} {:?}",
            tab.url(),
            tab.load_state(),
            tab.renderers()
        );
        thread::sleep(Duration::from_millis(2));
    }
}

/// Waits until the tab shows `url`, loaded.
fn at(tab: &mut RemoteTab, url: &Url) {
    settle(tab, url.as_str(), |tab| {
        tab.url() == Some(url) && *tab.load_state() == LoadState::Loaded
    });
}

/// A server of pages under `a.localhost`, `b.localhost` and
/// `c.localhost`, counting the POSTs it is sent.
fn server(posts: Arc<AtomicUsize>) -> TestServer {
    TestServer::plain(move |request| {
        let host = request.header("Host").unwrap_or_default().to_owned();
        let port = host.rsplit(':').next().unwrap_or_default().to_owned();
        let body = match request.target.as_str() {
            "/link" => format!(
                "<a id=l href='http://b.localhost:{port}/page'>b</a>\
                 <script>onload = () => document.getElementById('l').click()</script>"
            ),
            "/form" => format!(
                "<form id=f method=post action='http://b.localhost:{port}/posted'>\
                 <input name=q value=1></form>\
                 <script>onload = () => document.getElementById('f').submit()</script>"
            ),
            "/posted" => {
                posts.fetch_add(1, Ordering::Relaxed);
                format!("<title>posted {}</title>", request.method)
            }
            "/data" => "data".into(),
            _ => format!(
                "<title>{host}{}</title><script>fetch('/data').then(r => r.text())\
                 .then(t => console.log('{host} got', t))</script>",
                request.target
            ),
        };
        Reply::ok("text/html", body.as_bytes())
    })
}

fn url(server: &TestServer, host: &str, path: &str) -> Url {
    Url::parse(&format!("http://{host}:{}{path}", server.port())).expect("a URL")
}

#[test]
fn a_page_of_another_site_is_its_sites_renderers_and_back_goes_to_the_first() {
    let posts = Arc::new(AtomicUsize::new(0));
    let server = server(posts);
    let mut tab = tab(16, false);
    let link = url(&server, "a.localhost", "/link");
    tab.navigate(link.clone());
    let page = url(&server, "b.localhost", "/page");
    at(&mut tab, &page);
    let renderers = tab.renderers();
    assert_eq!(renderers.len(), 2, "{renderers:?}");
    let port = server.port();
    assert_eq!(
        renderers
            .iter()
            .map(|(_, lock)| lock.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["http://a.localhost", "http://b.localhost"],
        "each locked to its site, the first never moved ({port})"
    );
    let b = tab.shown_renderer();
    // Its fetches are b's own.
    let mut lines = Vec::new();
    settle(&mut tab, "b's fetch", |tab| {
        lines.extend(tab.take_console().into_iter().map(|e| e.text));
        !lines.is_empty()
    });
    assert_eq!(lines, [format!("b.localhost:{port} got data")]);
    assert!(tab.can_go_back());
    tab.go_back();
    at(&mut tab, &link);
    assert_ne!(tab.shown_renderer(), b, "the first renderer's view again");
    assert_eq!(tab.renderers().len(), 2, "no renderer started for it");
    assert!(tab.can_go_forward());
    tab.go_forward();
    at(&mut tab, &page);
    assert_eq!(tab.shown_renderer(), b);
}

#[test]
fn a_post_to_another_site_is_sent_once_and_shown_by_its_renderer() {
    let posts = Arc::new(AtomicUsize::new(0));
    let server = server(posts.clone());
    let mut tab = tab(16, false);
    tab.navigate(url(&server, "a.localhost", "/form"));
    let posted = url(&server, "b.localhost", "/posted");
    at(&mut tab, &posted);
    assert_eq!(tab.title(), "posted POST");
    assert_eq!(
        posts.load(Ordering::Relaxed),
        1,
        "handed over, not fetched again"
    );
    assert_eq!(tab.renderers().len(), 2);
}

#[test]
fn the_address_bar_picks_the_renderer_of_the_sites_url() {
    let server = server(Arc::default());
    let mut tab = tab(16, false);
    let first = url(&server, "a.localhost", "/one");
    tab.navigate(first.clone());
    at(&mut tab, &first);
    let a = tab.shown_renderer();
    let same = url(&server, "a.localhost", "/two");
    tab.navigate(same.clone());
    at(&mut tab, &same);
    assert_eq!(tab.shown_renderer(), a, "the same site, the same renderer");
    let other = url(&server, "b.localhost", "/three");
    tab.navigate(other.clone());
    at(&mut tab, &other);
    assert_ne!(tab.shown_renderer(), a);
    // Back twice: through a's entries, in a's renderer.
    tab.go_back();
    at(&mut tab, &same);
    assert_eq!(tab.shown_renderer(), a);
    tab.go_back();
    at(&mut tab, &first);
    assert!(!tab.can_go_back());
}

#[test]
fn past_the_cap_the_idle_renderer_goes_and_its_pages_come_back_in_another() {
    let server = server(Arc::default());
    let mut tab = tab(2, false);
    let pages: Vec<Url> = ["a", "b", "c"]
        .iter()
        .map(|site| url(&server, &format!("{site}.localhost"), "/p"))
        .collect();
    for page in &pages {
        tab.navigate(page.clone());
        at(&mut tab, page);
        assert!(tab.renderers().len() <= 2, "{:?}", tab.renderers());
    }
    // a's renderer went for c's: back to b, then a, restored.
    let sites = |tab: &RemoteTab| {
        tab.renderers()
            .into_iter()
            .map(|(_, lock)| lock.unwrap_or_default())
            .collect::<Vec<_>>()
    };
    assert!(!sites(&tab).contains(&"http://a.localhost".to_owned()));
    tab.go_back();
    at(&mut tab, &pages[1]);
    tab.go_back();
    at(&mut tab, &pages[0]);
    let port = server.port();
    assert_eq!(tab.title(), format!("a.localhost:{port}/p"), "loaded again");
    assert!(sites(&tab).contains(&"http://a.localhost".to_owned()));
    assert!(tab.renderers().len() <= 2);
    tab.go_forward();
    at(&mut tab, &pages[1]);
}

#[test]
fn a_spare_waits_for_the_next_site() {
    let server = server(Arc::default());
    let mut tab = tab(16, true);
    let first = url(&server, "a.localhost", "/p");
    tab.navigate(first.clone());
    at(&mut tab, &first);
    let renderers = tab.renderers();
    assert_eq!(renderers.len(), 2, "one more, unlocked: {renderers:?}");
    assert!(renderers.iter().any(|(_, lock)| lock.is_none()));
    let spare = renderers
        .iter()
        .find(|(_, lock)| lock.is_none())
        .map(|(number, _)| *number);
    let other = url(&server, "b.localhost", "/p");
    tab.navigate(other.clone());
    at(&mut tab, &other);
    assert_eq!(tab.shown_renderer(), spare, "the spare took the site");
    assert_eq!(tab.renderers().len(), 3, "and another waits");
}

#[test]
fn a_page_whose_renderer_is_gone_comes_back_on_reload() {
    let server = server(Arc::default());
    let mut tab = tab(16, false);
    let page = url(&server, "a.localhost", "/p");
    tab.navigate(page.clone());
    at(&mut tab, &page);
    let first = tab.shown_renderer();
    tab.kill_renderer();
    settle(&mut tab, "the renderer gone", |tab| {
        matches!(tab.load_state(), LoadState::Failed(_))
    });
    tab.reload();
    at(&mut tab, &page);
    assert_ne!(tab.shown_renderer(), first, "a new renderer");
}

#[test]
fn a_new_site_after_going_back_drops_the_entries_ahead() {
    let server = server(Arc::default());
    let mut tab = tab(16, false);
    let one = url(&server, "a.localhost", "/one");
    let two = url(&server, "a.localhost", "/two");
    tab.navigate(one.clone());
    at(&mut tab, &one);
    tab.navigate(two.clone());
    at(&mut tab, &two);
    tab.go_back();
    at(&mut tab, &one);
    let b = url(&server, "b.localhost", "/b");
    tab.navigate(b.clone());
    at(&mut tab, &b);
    tab.go_back();
    at(&mut tab, &one);
    // Reloaded, the page reports its history again: the second page of a
    // went from it too.
    tab.reload();
    at(&mut tab, &one);
    assert!(tab.can_go_forward());
    tab.go_forward();
    at(&mut tab, &b);
    assert!(!tab.can_go_forward(), "the second page of a went");
}
