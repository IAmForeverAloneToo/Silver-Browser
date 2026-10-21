//! The browser's own pages (M37): `about:downloads` shown with its
//! scripts on whatever the setting, which comes back with the next page;
//! its requests reach the browser through the bridge; and no page's
//! link reaches it.

use net::Url;

use crate::testing::*;
use crate::{Engine, LoadState};

fn downloads() -> Url {
    Url::parse(crate::internal::DOWNLOADS).expect("parses")
}

/// Runs the engine until `done` holds of it.
fn until(tab: &mut Tab, mut done: impl FnMut(&mut Engine) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !done(&mut tab.engine) {
        assert!(std::time::Instant::now() < deadline, "in time");
        tab.engine.poll_network();
        tab.engine.run_tasks();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn the_downloads_page_asks_the_browser_with_scripts_off_and_they_stay_off_after() {
    let server = serve(|_| {
        Some(html(
            "<title>page</title><script>document.title = 'scripted'</script>",
        ))
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(false);
    tab.engine.navigate(downloads());
    // Its script runs: it asks the browser for the list.
    let mut asked = Vec::new();
    until(&mut tab, |engine| {
        asked.extend(engine.take_devtools());
        !asked.is_empty()
    });
    assert!(asked[0].contains("\"downloads.list\""), "{asked:?}");
    assert_eq!(tab.engine.title(), "Downloads");
    assert!(!tab.engine.scripting(), "the user's setting stays");
    // The next page's scripts do not run.
    tab.engine.navigate(server.url("/"));
    until(&mut tab, |engine| {
        *engine.load_state() == LoadState::Loaded && engine.title() == "page"
    });
    assert!(!tab.engine.scripting());
    // Back to it from history, it asks again.
    tab.engine.go_back();
    let mut again = Vec::new();
    until(&mut tab, |engine| {
        again.extend(engine.take_devtools());
        !again.is_empty()
    });
    assert_eq!(tab.engine.url(), Some(&downloads()));
}

#[test]
fn the_browsers_answer_reaches_the_page() {
    let mut tab = Tab::new();
    tab.engine.navigate(downloads());
    let mut asked = Vec::new();
    until(&mut tab, |engine| {
        asked.extend(engine.take_devtools());
        !asked.is_empty()
    });
    let request = devtools::Request::parse(&asked[0]).expect("a request");
    let list = devtools::Json::parse(
        "[{\"id\":1,\"name\":\"a.txt\",\"url\":\"https://a.test/a.txt\",\"received\":3,\
         \"total\":3,\"state\":\"complete\",\"why\":null}]",
    )
    .expect("parses");
    tab.engine.devtools_send(devtools::result(request.id, list));
    until(&mut tab, |engine| {
        let document = engine.document();
        document.text_content(document.root()).contains("a.txt")
    });
}

#[test]
fn no_page_reaches_the_browsers_own_pages() {
    let server = serve(|_| {
        Some(html(
            "<body style='margin:0'><a href=about:downloads style='display:block;height:40px'>x</a>\
             <iframe src=about:downloads></iframe>",
        ))
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/"));
    until(&mut tab, |engine| *engine.load_state() == LoadState::Loaded);
    tab.engine.frame(400, 300);
    tab.engine.pointer_move(10.0, 20.0);
    tab.engine
        .pointer_down(10.0, 20.0, crate::Button::Primary, 1);
    tab.engine.pointer_up(10.0, 20.0, crate::Button::Primary, 1);
    assert!(tab.engine.take_notice().is_some(), "the link is refused");
    assert_eq!(tab.engine.url(), Some(&server.url("/")));
    assert!(tab.engine.take_devtools().is_empty());
}
