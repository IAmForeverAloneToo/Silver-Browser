//! Events in the engine's documents (M26, `docs/design/events.md`):
//! event handlers from the markup and from scripts, a link a script
//! clicks, a disabled control's `click()`, the dialogs, and uncaught
//! exceptions reported as `error` events before the console.

use std::time::Duration;

use net::test_server::Reply;

use super::script_tests::scripted;
use super::tests::load_all;
use crate::testing::*;
use crate::{LoadState, ScriptState};

#[test]
fn markup_handlers_run_in_their_place_with_their_scope() {
    let (_, console) = scripted(
        "<body onload=\"console.log('load')\">\
         <div><button id=b onclick=\"console.log('attr', this === b, typeof event, \
           event.type)\">x</button></div>\
         <form id=f><input name=q id=q onclick=\"console.log('scope', formOnly, \
           typeof getElementById, name)\">\
           <div id=v onclick=\"console.log('div', typeof formOnly)\"></div></form>\
         <script>\
         var b = document.getElementById('b');\
         b.click();\
         document.getElementById('f').formOnly = 'form';\n         document.getElementById('q').click();\
         document.getElementById('v').click();\
         console.log(String(b.onclick));\
         var r = []; var x = document.createElement('button');\
         x.addEventListener('click', function () { r.push('ONE'); });\
         x.setAttribute('onclick', \"r.push('NOT CALLED')\");\
         x.addEventListener('click', function () { r.push('THREE'); });\
         x.onclick = function () { r.push('TWO'); };\
         x.addEventListener('click', function () { r.push('FOUR'); });\
         x.click(); console.log(r.join(' '));\
         var y = document.createElement('div'); y.setAttribute('onclick', 'console.log(1)');\
         y.removeAttribute('onclick'); y.click(); console.log(String(y.onclick));         var w = []; var u = document.createElement('div');         u.setAttribute('onclick', \"w.push('A')\");         u.addEventListener('click', function () { w.push('B'); });         u.removeAttribute('onclick'); u.setAttribute('onclick', \"w.push('C')\");         u.click(); console.log(w.join(' '));\
         var z = document.createElement('div'); z.setAttribute('ONCLICK', 'return false');\
         var e = new MouseEvent('click', {cancelable: true}); z.dispatchEvent(e);\
         console.log(e.defaultPrevented);\
         console.log(typeof window.onload, document.body.onload === window.onload);\
         var bo = document.createElement('body'); bo.setAttribute('onerror', 'x');\
         console.log(window.onerror.length, String(document.onload));\
         window.onerror = null;\
         </script>",
        true,
    );
    assert_eq!(
        console,
        [
            "attr true object click",
            "scope form function q",
            "div undefined",
            "function onclick(event) {\nconsole.log('attr', this === b, typeof event, \
             event.type)\n}",
            "ONE TWO THREE FOUR",
            "null",
            "B C",
            "true",
            "function true",
            "5 null",
            // The body's onload, the window's, when the load ends (M26.3).
            "load",
        ]
    );
}

#[test]
fn a_handler_that_does_not_compile_is_reported_and_null() {
    let (tab, console) = scripted(
        "<div id=x onclick=\"return }\"></div>\
         <script>\
         window.addEventListener('error', function (e) { console.log('event', e.message); });\
         console.log(String(document.getElementById('x').onclick));\
         </script>",
        true,
    );
    assert_eq!(console.len(), 3, "{console:?}");
    assert!(console[0].starts_with("event Uncaught SyntaxError"));
    assert!(console[1].starts_with("Uncaught SyntaxError"));
    assert_eq!(console[2], "null");
    assert_eq!(tab.engine.scripts(), ScriptState::Running { errors: 1 });
}

#[test]
fn a_link_a_script_clicks_is_followed_unless_canceled() {
    let server = serve(|path| match path {
        "/one.html" => Some(html(
            "<a id=a href=/two.html>two</a><a id=c href=/three.html>three</a>\
             <script>\
             var c = document.getElementById('c');\
             c.addEventListener('click', function (e) { e.preventDefault(); });\
             setTimeout(function () { c.click(); });\
             setTimeout(function () { document.getElementById('a').click(); }, 5);\
             </script>",
        )),
        "/two.html" => Some(html("<p>two</p>")),
        "/three.html" => Some(html("<p>three</p>")),
        _ => None,
    });
    let mut tab = Tab::new();
    let clock = tab.hand_clock();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/one.html"));
    tab.engine.run_tasks();
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert_eq!(tab.engine.url(), Some(&server.url("/one.html")));
    clock.advance(5);
    tab.engine.run_tasks();
    let two = server.url("/two.html");
    tab.wait_until(|engine| {
        engine.url() == Some(&two) && engine.load_state() == &LoadState::Loaded
    });
    // Followed as a click is: a new entry.
    assert!(tab.engine.can_go_back());
}

#[test]
fn a_link_out_of_the_document_or_without_href_goes_nowhere() {
    let (tab, console) = scripted(
        "<a id=a>no href</a>\
         <script>\
         document.getElementById('a').click();\
         var gone = document.createElement('a'); gone.href = '/elsewhere.html'; gone.click();\
         var ok = document.createElement('a').dispatchEvent(new MouseEvent('click'));\
         console.log('clicked', ok);\
         </script>",
        true,
    );
    assert_eq!(console, ["clicked true"]);
    assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
    assert!(
        tab.engine
            .url()
            .is_some_and(|url| url.as_str().ends_with("/page.html"))
    );
    assert!(!tab.engine.can_go_back());
}

#[test]
fn a_disabled_control_is_not_clicked() {
    let (_, console) = scripted(
        "<button id=a disabled>a</button>\
         <fieldset disabled><legend><button id=b>b</button></legend>\
         <button id=c>c</button></fieldset>\
         <script>\
         ['a', 'b', 'c'].forEach(function (id) {\
           var el = document.getElementById(id);\
           el.addEventListener('click', function () { console.log('clicked', id); });\
           el.click(); });\
         </script>",
        true,
    );
    assert_eq!(console, ["clicked b"]);
}

#[test]
fn dialogs_with_no_one_to_ask_say_their_message_in_the_console() {
    let (_, console) = scripted(
        "<script>alert('hi'); console.log(confirm('sure?'), prompt('name', 'x'));</script>",
        true,
    );
    assert_eq!(
        console,
        ["alert: hi", "confirm: sure?", "prompt: name", "false null"]
    );
}

#[test]
fn uncaught_exceptions_fire_the_error_event_before_the_console() {
    let (mut tab, console) = scripted(
        "<script>\
         window.addEventListener('error', function (e) { \
           console.log('event', e.message, e.lineno > 0, e.error instanceof Error); \
           if (e.message.indexOf('quiet') >= 0) e.preventDefault(); });\
         </script>\
         <script>throw new Error('loud')</script>\
         <script>throw new Error('quiet')</script>\
         <script>setTimeout(function () { throw new Error('timer'); });\
         setTimeout(\"throw new Error('string')\", 1);</script>\
         <script>var s = document.createElement('script');\
         s.textContent = \"throw new Error('nested')\"; document.documentElement.appendChild(s);</script>",
        true,
    );
    let lines: Vec<&str> = console.iter().map(String::as_str).collect();
    assert_eq!(lines.len(), 5, "{lines:?}");
    assert_eq!(lines[0], "event Uncaught Error: loud true true");
    assert!(lines[1].starts_with("Uncaught Error: loud at "));
    assert_eq!(lines[2], "event Uncaught Error: quiet true true");
    assert_eq!(lines[3], "event Uncaught Error: nested true true");
    assert!(lines[4].starts_with("Uncaught Error: nested at "));
    assert_eq!(tab.engine.scripts(), ScriptState::Running { errors: 2 });
    // The timers are due by now.
    std::thread::sleep(Duration::from_millis(2));
    tab.engine.run_tasks();
    let later: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(later.len(), 4, "{later:?}");
    assert_eq!(later[0], "event Uncaught Error: timer true true");
    assert!(later[1].starts_with("Uncaught Error: timer at "));
    // A string's errors name the document.
    assert_eq!(later[2], "event Uncaught Error: string true true");
    assert!(later[3].starts_with("Uncaught Error: string at http"));
    assert!(later[3].contains("/page.html:1:"));
}

#[test]
fn the_bodys_window_handlers_hear_the_windows_events() {
    // No script touches the body: the error event at the window finds
    // its handler all the same, with five arguments, and cancels it.
    let (tab, console) = scripted(
        "<body onerror=\"console.log('body', arguments.length, event); return true\">\
         <script>throw new Error('x')</script>",
        true,
    );
    assert_eq!(console, ["body 5 Uncaught Error: x"]);
    assert_eq!(tab.engine.scripts(), ScriptState::Running { errors: 0 });
}

#[test]
fn a_link_clicked_before_its_document_has_loaded_still_makes_an_entry() {
    // The image keeps the shown document loading while a timer clicks
    // the link: a new entry, where `location` would replace it, as in the
    // reference browser.
    let server = serve(|path| match path {
        "/start.html" => Some(html("<p>start</p>")),
        "/one.html" => Some(html(
            "<body><img src=/slow.gif><a id=a href=/two.html>two</a>             <script>setTimeout(function () { document.getElementById('a').click(); }, 100)             </script>",
        )),
        "/slow.gif" => Some(Reply::Pieces(vec![
            (
                Duration::ZERO,
                b"HTTP/1.1 200 OK
Content-Type: image/gif
Content-Length: 10

"
                .to_vec(),
            ),
            (Duration::from_millis(2000), b"GIF89a    ".to_vec()),
        ])),
        _ => Some(html("<p>two</p>")),
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/start.html"));
    let (one, two) = (server.url("/one.html"), server.url("/two.html"));
    tab.engine.navigate(one.clone());
    let deadline = std::time::Instant::now() + WAIT;
    let mut clicked_while_loading = false;
    while !(tab.engine.url() == Some(&two) && tab.engine.load_state() == &LoadState::Loaded) {
        assert!(
            std::time::Instant::now() < deadline,
            "the link is followed in time"
        );
        tab.engine.poll_network();
        if tab.engine.url() == Some(&one) && tab.engine.load_state() != &LoadState::Loaded {
            clicked_while_loading = true;
        }
        tab.engine.run_tasks();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(clicked_while_loading);
    let back = tab
        .engine
        .history
        .by_delta(-1)
        .map(|(_, entry)| entry.url.clone());
    assert_eq!(back, Some(Some(one)));
}

#[test]
fn an_event_goes_up_the_documents_tree_to_the_window() {
    let (_, console) = scripted(
        "<div><p><span id=s>x</span></p></div>\
         <script>\
         var s = document.getElementById('s');\
         s.addEventListener('x', function (e) { console.log(e.composedPath().map(function (n) { \
           return n === window ? 'window' : n.nodeName; }).join()); });\
         s.dispatchEvent(new Event('x'));\
         </script>",
        true,
    );
    assert_eq!(console, ["SPAN,P,DIV,BODY,HTML,#document,window"]);
}
