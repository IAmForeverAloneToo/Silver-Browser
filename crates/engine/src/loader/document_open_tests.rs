//! `document.open()` (HTML section 8.4.2.1, gap batch D's document
//! gaps): the document loading again, a load still running aborted with
//! its parser, the listeners erased; `DOMContentLoaded` a task of its
//! own; and a form's controls by index and name.

use std::time::Duration;

use net::test_server::Reply;

use super::script_tests::scripted;
use super::tests::load_all;
use crate::testing::*;

#[test]
fn open_makes_the_document_load_again_and_erases_its_listeners() {
    let server = serve(|path| {
        (path == "/page.html").then(|| {
            html(
                "<p id=a onclick=\"console.log('handler')\">a</p><script>\
                 document.addEventListener('x', () => console.log('document listener'));\
                 window.addEventListener('x', () => console.log('window listener'));\
                 const p = document.getElementById('a');\
                 p.addEventListener('x', () => console.log('p listener'));\
                 onload = () => setTimeout(() => {\
                   console.log(document.readyState);\
                   document.open();\
                   console.log(document.readyState, document.childNodes.length);\
                   document.dispatchEvent(new Event('x'));\
                   window.dispatchEvent(new Event('x'));\
                   p.dispatchEvent(new Event('x'));\
                   p.click();\
                   document.write('<p>new</p>');\
                   document.close();\
                   console.log(document.readyState, document.body.textContent);\
                 });\
                 </script>",
            )
        })
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    let lines = tab.run_until(|lines| lines.len() >= 3);
    assert_eq!(lines, ["complete", "loading 0", "complete new"]);
}

#[test]
fn open_during_the_load_aborts_its_parser_and_fetch() {
    // The body the parser was filling is kept, to see whether the old
    // parser goes on filling it once the rest arrives.
    let first = format!(
        "<p>first</p><script>setTimeout(() => {{\
           console.log(document.readyState);\
           const old = document.body;\
           document.open(); document.write('<p>opened</p>');\
           console.log(document.readyState);\
           setTimeout(() => console.log('old body', old.textContent.includes('Z' + 'Z')), 600);\
         }})</script>{}",
        " ".repeat(2000)
    );
    let late = "<p>ZZ</p>";
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n",
        first.len() + late.len()
    );
    let server = serve(move |path| {
        (path == "/page.html").then(|| {
            Reply::Pieces(vec![
                (Duration::ZERO, format!("{head}{first}").into_bytes()),
                (Duration::from_millis(300), late.as_bytes().to_vec()),
            ])
        })
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let lines = tab.run_until(|lines| lines.len() >= 3);
    assert_eq!(lines, ["loading", "loading", "old body false"]);
    let shown = text(&tab.engine);
    assert!(shown.contains("opened"), "{shown}");
    assert!(!shown.contains("ZZ") && !shown.contains("first"), "{shown}");
}

#[test]
fn dom_content_loaded_runs_after_the_timers_the_deferred_scripts_set() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script>\
             document.addEventListener('DOMContentLoaded', () => console.log('DCL'));\
             </script><script defer src=/defer.js></script>",
        )),
        "/defer.js" => Some(Reply::ok(
            "text/javascript",
            b"console.log('defer');\
              setTimeout(() => { console.log('timer one'); \
                setTimeout(() => console.log('timer two')); });",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    let _clock = tab.hand_clock();
    tab.engine.navigate(server.url("/page.html"));
    let lines = tab.run_until(|lines| lines.len() >= 4);
    assert_eq!(lines, ["defer", "timer one", "DCL", "timer two"]);
}

#[test]
fn a_forms_controls_are_its_indexed_and_named_properties() {
    let (_, console) = scripted(
        "<form id=f><input name=user><input id=pass><input name=r><input name=r></form><script>\
         const f = document.getElementById('f');\
         console.log(f[0].name, f[1].id, f[4], f.user === f[0], f.pass === f[1]);\
         console.log(f.r.length, f.nothing, f.id, typeof f.submit, f instanceof HTMLElement);\
         </script>",
        true,
    );
    assert_eq!(
        console,
        [
            "user pass undefined true true",
            "2 undefined f function true"
        ]
    );
}

#[test]
fn a_documents_listeners_wait_for_load_when_scripts_set_no_timer() {
    // `DOMContentLoaded` before `load`, whether or not a timer waits.
    let server = serve(|path| {
        (path == "/page.html").then(|| {
            html(
                "<script>\
                 document.addEventListener('DOMContentLoaded', () => console.log('DCL'));\
                 addEventListener('load', () => console.log('load'));\
                 </script>",
            )
        })
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    let lines = tab.run_until(|lines| lines.len() >= 2);
    assert_eq!(lines, ["DCL", "load"]);
}
