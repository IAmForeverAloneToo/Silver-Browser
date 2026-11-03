//! The session history scripts see (gap batch H, HTML section 7.2.2):
//! `history` over the engine's own session history, `pushState` and
//! `replaceState` making entries of the document shown, the traversals
//! between them with `popstate` and `hashchange`, and fragment
//! navigations done when the script asks for them.

use net::test_server::TestServer;

use crate::testing::*;
use crate::{Engine, LoadState, differential};

/// A server of `/page`, whose head runs `head`, and of `/other`.
fn server_with(head: &'static str) -> TestServer {
    serve(move |path| {
        let path = path.split(['?', '#']).next().unwrap_or_default();
        match path {
            "/page" => Some(html(&format!(
                "<title>page</title><script>{head}</script><p id=a>a</p>"
            ))),
            "/other" => Some(html("<title>other</title><p>other</p>")),
            _ => None,
        }
    })
}

fn scripted() -> Tab {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab
}

/// Loads `url` in `tab`, its scripts run, until it is shown loaded.
fn open(tab: &mut Tab, url: net::Url) {
    load_view(&mut tab.engine, url);
}

/// What `script` logs, run in the document shown.
fn run(engine: &mut Engine, script: &str) -> Vec<String> {
    engine.take_console();
    differential::run_script(engine, script);
    console(engine)
}

fn console(engine: &mut Engine) -> Vec<String> {
    engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// Runs the tasks due until the console has said `lines` lines.
fn lines(tab: &mut Tab, count: usize) -> Vec<String> {
    tab.run_until(|lines| lines.len() >= count)
}

#[test]
fn push_state_puts_an_entry_of_the_same_document_and_its_state() {
    let server = server_with("");
    let mut tab = scripted();
    open(&mut tab, server.url("/page"));
    let said = run(
        &mut tab.engine,
        "console.log(history.length, history.state, history.scrollRestoration);
         history.pushState({n: 1}, '', '?one');
         console.log(history.length, JSON.stringify(history.state), location.search,
           history.state === history.state, document.URL === location.href);
         history.replaceState([2], 'ignored');
         console.log(history.length, JSON.stringify(history.state), location.search);",
    );
    assert_eq!(
        said,
        ["1 null auto", "2 {\"n\":1} ?one true true", "2 [2] ?one"]
    );
    assert_eq!(tab.engine.history.len(), 2);
    assert_eq!(tab.engine.url(), Some(&server.url("/page?one")));
    // The same document, nothing fetched for the new entry.
    assert_eq!(paths(&server), ["/page"]);
    let entries = tab.engine.history.entries();
    assert_eq!(entries[0].state, entries[1].state);
    assert!(entries[0].classic.is_none() && entries[1].classic.is_some());
}

#[test]
fn a_url_the_document_cannot_be_rewritten_to_is_a_security_error() {
    let server = server_with("");
    let mut tab = scripted();
    open(&mut tab, server.url("/page"));
    let said = run(
        &mut tab.engine,
        "for (const url of ['http://example.com/', 'https://[::1', '//' + location.host + '/x']) {
           try { history.pushState(null, '', url); console.log('pushed', location.pathname); }
           catch (e) { console.log(e.name, e.message.startsWith(\"Failed to execute 'pushState' \
             on 'History': A history state object with URL '\")); }
         }
         try { history.pushState(() => 1, ''); } catch (e) { console.log(e.name); }
         try { history.replaceState(null); } catch (e) { console.log(e.name); }
         history.pushState(null, '', '');
         console.log(history.length, location.pathname);",
    );
    assert_eq!(
        said,
        [
            "SecurityError true",
            "SecurityError true",
            "pushed /x",
            "DataCloneError",
            "TypeError",
            "3 /x"
        ]
    );
}

#[test]
fn traversals_between_the_documents_entries_fire_popstate_then_hashchange() {
    let server = server_with("");
    let mut tab = scripted();
    open(&mut tab, server.url("/page"));
    let said = run(
        &mut tab.engine,
        "addEventListener('popstate', e => console.log('popstate', JSON.stringify(e.state),
           e.state === history.state, location.hash));
         addEventListener('hashchange', e => console.log('hashchange',
           e.oldURL.split('/').pop(), e.newURL.split('/').pop()));
         history.replaceState({n: 1}, '', '#one');
         history.pushState({n: 2}, '', '#two');
         history.pushState({n: 3}, '', '#three');
         history.back();
         console.log('queued', location.hash);",
    );
    // `back()` waits for a task of its own.
    assert_eq!(said, ["queued #three"]);
    let said = lines(&mut tab, 2);
    assert_eq!(
        said,
        [
            "popstate {\"n\":2} true #two",
            "hashchange page#three page#two"
        ]
    );
    // Two back, to the first entry, by `go`.
    run(&mut tab.engine, "history.go(-1);");
    let said = lines(&mut tab, 2);
    assert_eq!(
        said,
        [
            "popstate {\"n\":1} true #one",
            "hashchange page#two page#one"
        ]
    );
    // The browser's own traversal does the same.
    tab.engine.go_forward();
    let said = lines(&mut tab, 2);
    assert_eq!(said[0], "popstate {\"n\":2} true #two");
    assert_eq!(tab.engine.history.len(), 3);
    assert_eq!(paths(&server), ["/page"]);
}

#[test]
fn a_fragment_navigation_by_script_is_done_at_once() {
    let server = server_with("");
    let mut tab = scripted();
    open(&mut tab, server.url("/page"));
    let said = run(
        &mut tab.engine,
        "addEventListener('popstate', e => console.log('popstate', e.state, location.hash));
         addEventListener('hashchange', e => console.log('hashchange', e.newURL.split('#')[1]));
         location.hash = 'a';
         console.log('set', location.hash, history.length);
         location.hash = 'a';
         console.log('again', history.length);",
    );
    // `popstate` as the URL changes, `hashchange` in a task after.
    assert_eq!(said, ["popstate null #a", "set #a 2", "again 2"]);
    assert_eq!(lines(&mut tab, 1), ["hashchange a"]);
    assert_eq!(tab.engine.history.len(), 2);
    assert_eq!(tab.engine.url(), Some(&server.url("/page#a")));
}

#[test]
fn push_state_while_the_document_loads_comes_after_its_own_entry() {
    let server = server_with(
        "console.log(history.length); history.pushState('loading', '', '/page?pushed');
         console.log(history.length, location.search);",
    );
    let mut tab = scripted();
    open(&mut tab, server.url("/other"));
    tab.engine.navigate(server.url("/page"));
    assert_eq!(lines(&mut tab, 2), ["2", "3 ?pushed"]);
    tab.wait_until(|engine| engine.load_state() == &LoadState::Loaded);
    let urls: Vec<String> = tab
        .engine
        .history
        .entries()
        .iter()
        .map(|entry| entry.url.as_ref().unwrap().as_str().to_owned())
        .collect();
    assert_eq!(
        urls,
        [
            server.url("/other").to_string(),
            server.url("/page").to_string(),
            server.url("/page?pushed").to_string()
        ]
    );
    // The state is the entry's: a document fetched again for it reads it.
    tab.engine.reload();
    tab.settle();
    let said = run(
        &mut tab.engine,
        "console.log(history.state, history.length);",
    );
    assert_eq!(said, ["loading 3"]);
}

#[test]
fn scroll_restoration_is_each_entrys_and_a_new_one_takes_the_current_ones() {
    let server = server_with("");
    let mut tab = scripted();
    open(&mut tab, server.url("/page"));
    let said = run(
        &mut tab.engine,
        "history.scrollRestoration = 'bogus';
         console.log(history.scrollRestoration);
         history.scrollRestoration = 'manual';
         history.pushState(null, '', '?b');
         console.log(history.scrollRestoration);
         history.scrollRestoration = 'auto';",
    );
    assert_eq!(said, ["auto", "manual"]);
    let entries = tab.engine.history.entries();
    assert!(entries[0].manual && !entries[1].manual);
    tab.engine.go_back();
    let said = run(&mut tab.engine, "console.log(history.scrollRestoration);");
    assert_eq!(said, ["manual"]);
}

#[test]
fn the_history_interface_is_the_windows_alone() {
    let mut engine = Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    );
    engine.set_scripting(true);
    engine.load_document("<p>x</p>");
    let said = run(
        &mut engine,
        "console.log(history instanceof History, Object.prototype.toString.call(history));
         try { new History(); } catch (e) { console.log(e.message); }
         try { History.prototype.go.call({}); } catch (e) { console.log(e.message); }
         console.log(History.prototype.pushState.length, History.prototype.go.length);
         const p = new PopStateEvent('popstate', {state: {q: 1}});
         const h = document.createEvent('HashChangeEvent');
         console.log(p.state.q, p.hasUAVisualTransition, h.oldURL === '', h instanceof HashChangeEvent);",
    );
    assert_eq!(
        said,
        [
            "true [object History]",
            "Failed to construct 'History': Illegal constructor",
            "Illegal invocation",
            "2 0",
            "1 false true true"
        ]
    );
}

#[test]
fn a_fragment_navigation_scrolls_before_its_hashchange() {
    // HTML's "scroll to the fragment" is part of the navigation, and the
    // traversal back restores the position within it, before the events.
    let server = serve(|path| (path == "/page").then(|| html("<p>top</p>")));
    let mut tab = scripted();
    open(&mut tab, server.url("/page"));
    tab.engine.frame(200, 100);
    let said = run(
        &mut tab.engine,
        "const pre = document.createElement('pre');
         pre.textContent = 'line\\n'.repeat(200);
         const a = document.createElement('a');
         a.name = 'foo';
         a.textContent = 'foo';
         pre.appendChild(a);
         document.body.appendChild(pre);
         onhashchange = () => {
           console.log('there', location.hash, scrollY > 0);
           onhashchange = () => console.log('back', location.hash, scrollY);
           history.back();
         };
         location.hash = 'foo';
         console.log('now', scrollY > 0, a.getAttribute('name'));",
    );
    assert_eq!(said, ["now true foo"]);
    assert_eq!(lines(&mut tab, 2), ["there #foo true", "back  0"]);
}

#[test]
fn an_entry_of_manual_scroll_restoration_keeps_the_position_a_traversal_finds() {
    let server = serve(|path| {
        let paragraphs: String = (0..40).map(|n| format!("<p>{n}</p>")).collect();
        (path.split('?').next() == Some("/tall")).then(|| html(&paragraphs))
    });
    let mut tab = scripted();
    open(&mut tab, server.url("/tall"));
    tab.engine.frame(200, 100);
    tab.engine.set_scroll(300.0);
    tab.engine.frame(200, 100);
    run(&mut tab.engine, "history.pushState(null, '', '?b');");
    tab.engine.set_scroll(100.0);
    tab.engine.frame(200, 100);
    // Auto: back restores the first entry's position.
    tab.engine.go_back();
    tab.engine.frame(200, 100);
    assert_eq!(tab.engine.scroll_y(), 300.0);
    // Manual: the position stays where it was.
    run(&mut tab.engine, "history.scrollRestoration = 'manual';");
    tab.engine.set_scroll(50.0);
    tab.engine.frame(200, 100);
    tab.engine.go_forward();
    tab.engine.frame(200, 100);
    assert_eq!(tab.engine.scroll_y(), 100.0, "the second entry is auto");
    tab.engine.go_back();
    tab.engine.frame(200, 100);
    assert_eq!(tab.engine.scroll_y(), 100.0, "the first is manual");
}
