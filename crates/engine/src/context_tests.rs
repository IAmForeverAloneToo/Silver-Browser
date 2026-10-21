//! What the tabs of a profile share (M37): cookies and `localStorage`
//! seen by both, `sessionStorage` each tab's own, and a change of an
//! area heard as a `storage` event by the other tab's documents of its
//! origin, but not by the document that made it.

use net::Url;
use net::test_server::TestServer;

use crate::testing::*;
use crate::{Context, LoadState};

/// Loads `url` in `tab` until it is shown and loaded.
fn load(tab: &mut Tab, url: Url) {
    tab.engine.navigate(url.clone());
    tab.wait_until(|engine| {
        engine.url() == Some(&url) && engine.load_state() == &LoadState::Loaded
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

/// A tab with scripts on, of `context`.
fn tab_of(context: &Context) -> Tab {
    let mut tab = Tab::new();
    tab.engine.join(context);
    tab.engine.set_scripting(true);
    tab
}

const HEARING: &str = "<script>addEventListener('storage', e => console.log('heard ' + \
                       e.key + ' ' + e.newValue + ' ' + (e.storageArea === localStorage) + ' ' + \
                       e.url.split('/').pop()));</script>";

fn server() -> TestServer {
    serve(|path| match path {
        "/listen.html" => Some(html(HEARING)),
        "/write.html" => Some(html(&format!(
            "{HEARING}<script>document.cookie = 'c=1'; localStorage.setItem('k', 'v'); \
             sessionStorage.setItem('s', 'w'); console.log('wrote');</script>"
        ))),
        "/read.html" => Some(html(
            "<script>console.log(document.cookie + ' ' + localStorage.getItem('k') + ' ' + \
             sessionStorage.getItem('s'));</script>",
        )),
        _ => None,
    })
}

#[test]
fn the_tabs_of_a_context_share_cookies_and_local_storage_not_session_storage() {
    let server = server();
    let context = Context::new();
    let mut first = tab_of(&context);
    let mut second = tab_of(&context);
    assert_ne!(first.engine.tab(), second.engine.tab());
    load(&mut first, server.url("/write.html"));
    assert_eq!(console(&mut first), ["wrote"]);
    load(&mut second, server.url("/read.html"));
    assert_eq!(console(&mut second), ["c=1 v null"]);
    // The first tab's own sessionStorage is still its own.
    load(&mut first, server.url("/read.html"));
    assert_eq!(console(&mut first), ["c=1 v w"]);
}

#[test]
fn tabs_of_two_contexts_share_nothing() {
    let server = server();
    let mut first = tab_of(&Context::new());
    let mut second = tab_of(&Context::new());
    load(&mut first, server.url("/write.html"));
    load(&mut second, server.url("/read.html"));
    assert_eq!(console(&mut second), [" null null"]);
}

#[test]
fn a_change_in_one_tab_is_heard_in_the_other_not_by_its_maker() {
    let server = server();
    let context = Context::new();
    let mut first = tab_of(&context);
    let mut second = tab_of(&context);
    load(&mut second, server.url("/listen.html"));
    load(&mut first, server.url("/write.html"));
    // Woken by the change, as the shell is.
    let heard = second.run_until(|lines| !lines.is_empty());
    // The localStorage change alone (the sessionStorage one is the other
    // tab's), with the URL of the document that made it.
    assert_eq!(heard, ["heard k v true write.html"]);
    first.engine.run_tasks();
    assert_eq!(
        console(&mut first),
        ["wrote"],
        "the maker hears none of its own"
    );
}

#[test]
fn a_tab_closed_forgets_its_session_storage() {
    let server = server();
    let context = Context::new();
    let mut first = tab_of(&context);
    load(&mut first, server.url("/write.html"));
    let tab = first.engine.tab();
    let origin = server.url("/").origin().serialization();
    let session = |context: &Context| {
        context
            .shared_storage()
            .lock()
            .unwrap()
            .area(crate::storage::Scope::Session(tab), &origin)
            .len()
    };
    assert_eq!(session(&context), 1);
    drop(first);
    context.close_tab(tab);
    assert_eq!(session(&context), 0);
}
