//! Dedicated workers (M31, `docs/design/workers.md`): a page's workers
//! on the test server, each on a thread of its own, with messages both
//! ways, errors through every step, `close` and `terminate`, nested
//! workers, and the bound on live workers; the words as the reference
//! browser has them (scratchpad m31 `p3-errors.html`, Chrome 154).

use std::sync::atomic::Ordering;

use net::test_server::{Reply, TestServer};

use crate::LoadState;
use crate::testing::*;

fn script(body: &str) -> Reply {
    Reply::ok("text/javascript", body.as_bytes())
}

/// A page of `body` whose scripts' files `files` gives by path, loaded
/// with scripts on; the tab and the server, which must outlive it.
fn page(body: &'static str, files: &'static [(&'static str, &'static str)]) -> (Tab, TestServer) {
    let server = serve(move |path| match path {
        "/page.html" => Some(html(body)),
        _ => files
            .iter()
            .find(|(at, _)| *at == path)
            .map(|(_, text)| script(text)),
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    (tab, server)
}

/// The console's lines until one is `last`.
fn until(tab: &mut Tab, last: &str) -> Vec<String> {
    tab.run_until(|lines| lines.iter().any(|line| line == last))
}

#[test]
fn messages_go_both_ways_as_structured_clones_with_transfer() {
    let (mut tab, _server) = page(
        "<script>\
         var w = new Worker('echo.js', { name: 'probe' });\
         var n = 0;\
         w.onmessage = function (e) {\
           console.log('page got', JSON.stringify(e.data), e.constructor.name, \
             JSON.stringify(e.origin), e.source, e.isTrusted, e.ports.length, e.target === w);\
           if (++n === 4) { console.log('done'); }\
         };\
         var buffer = new Uint8Array([1, 2, 3]).buffer;\
         w.postMessage({ a: [1, 'x'], m: new Map([[1, 2]]) });\
         w.postMessage(buffer, [buffer]);\
         console.log('detached', buffer.byteLength);\
         var other = new Uint8Array([4]).buffer;\
         w.postMessage(other, { transfer: [other] });\
         console.log('detached by options', other.byteLength);\
         w.postMessage('name');\
         console.log('script', String(w), typeof w.terminate);\
         </script>",
        &[(
            "/echo.js",
            "onmessage = function (e) {\
               var d = e.data;\
               if (d instanceof ArrayBuffer) { postMessage(Array.from(new Uint8Array(d))); return; }\
               if (d === 'name') { postMessage([self.name, String(self), e.source, e.origin]); return; }\
               postMessage({ back: d.a, map: d.m.get(1), isMap: d.m instanceof Map });\
             };",
        )],
    );
    let lines = until(&mut tab, "done");
    assert_eq!(
        lines,
        [
            "detached 0",
            "detached by options 0",
            "script [object Worker] function",
            "page got {\"back\":[1,\"x\"],\"map\":2,\"isMap\":true} MessageEvent \"\" null true 0 true",
            "page got [1,2,3] MessageEvent \"\" null true 0 true",
            "page got [4] MessageEvent \"\" null true 0 true",
            "page got [\"probe\",\"[object DedicatedWorkerGlobalScope]\",null,\"\"] MessageEvent \
             \"\" null true 0 true",
            "done",
        ]
    );
}

#[test]
fn a_workers_url_is_its_creators_origin_or_data() {
    let (mut tab, server) = page(
        "<base href='/sub/'><script>\
         var err = function (e) { return e.name + ': ' + e.message + ' [' + e.code + ']'; };\
         try { new Worker('http://127.0.0.2:1/w.js'); } catch (e) { console.log('cross', err(e)); }\
         try { new Worker('http://[x/'); } catch (e) { console.log('bad', err(e)); }\
         try { new Worker(); } catch (e) { console.log('none', e.message); }\
         try { new Worker('w.js', { type: 'nope' }); } catch (e) { console.log('type', e.message); }\
         try { new Worker('w.js', 5); } catch (e) { console.log('options', e.message); }\
         var based = new Worker('w.js');\
         based.onmessage = function (e) { console.log('based', e.data); \
           var d = new Worker('data:text/javascript,postMessage(self.origin)');\
           d.onmessage = function (e) { console.log('data', e.data); console.log('done'); };\
           d.onerror = function () { console.log('data failed'); console.log('done'); };\
         };\
         </script>",
        &[(
            "/sub/w.js",
            "postMessage('from sub ' + self.origin.length);",
        )],
    );
    let origin = server.url("/").to_string();
    let origin = origin.trim_end_matches('/');
    let lines = until(&mut tab, "done");
    assert_eq!(
        lines[..5],
        [
            format!(
                "cross SecurityError: Failed to construct 'Worker': Script at \
                 'http://127.0.0.2:1/w.js' cannot be accessed from origin '{origin}'. [18]"
            ),
            "bad SyntaxError: Failed to construct 'Worker': The URL 'http://[x/' is invalid. [12]"
                .to_owned(),
            "none Failed to construct 'Worker': 1 argument required, but only 0 present."
                .to_owned(),
            "type Failed to construct 'Worker': Failed to read the 'type' property from \
             'WorkerOptions': The provided value 'nope' is not a valid enum value of type \
             WorkerType."
                .to_owned(),
            "options Failed to construct 'Worker': The provided value is not of type \
             'WorkerOptions'."
                .to_owned(),
        ]
    );
    assert!(
        lines.contains(&format!("based from sub {}", origin.len())),
        "{lines:#?}"
    );
    // A data: worker's origin is opaque.
    assert!(lines.contains(&"data null".to_owned()), "{lines:#?}");
    // Fetched as a worker's script, in same-origin mode.
    let request = server
        .requests()
        .into_iter()
        .find(|request| request.target == "/sub/w.js")
        .expect("the script was asked for");
    assert_eq!(request.header("Sec-Fetch-Dest"), Some("worker"));
    assert_eq!(request.header("Sec-Fetch-Mode"), Some("same-origin"));
}

#[test]
fn a_workers_uncaught_error_reaches_its_object_then_the_window() {
    let (mut tab, _server) = page(
        "<script>\
         var show = function (where, e) { console.log(where, e.constructor.name, e.type, \
           e.cancelable, e.bubbles, e.message, String(e.filename).split('/').pop(), \
           e.lineno > 0, e.colno >= 0, e.error); };\
         window.onerror = function (message, file, line, column, error) {\
           console.log('window onerror', message, String(file).split('/').pop(), error); };\
         var thrower = new Worker('throw.js');\
         thrower.onerror = function (e) { show('worker onerror', e); };\
         thrower.onmessage = function (e) { console.log('message', e.data); };\
         var handled = new Worker('handled.js');\
         handled.onmessage = function (e) { console.log('handled', e.data); \
           var canceled = new Worker('throw.js');\
           canceled.addEventListener('error', function (e) { e.preventDefault(); \
             console.log('canceled at the object'); \
             var missing = new Worker('missing.js');\
             missing.onerror = function (e) { show('missing', e); \
               var module = new Worker('throw.js', { type: 'module' });\
               module.onerror = function (e) { console.log('module', e.constructor.name); \
                 var inHandler = new Worker('handler.js');\
                 inHandler.onerror = function (e) { show('handler', e); console.log('done'); };\
                 inHandler.postMessage(1); };\
             }; });\
         };\
         </script>",
        &[
            (
                "/throw.js",
                "postMessage('before');\nthrow new Error('boom');",
            ),
            (
                "/handled.js",
                "self.onerror = function (m, f, l, c, e) { postMessage(['onerror', m, e.message].join(' ')); return true; };\nthrow new TypeError('handled one');",
            ),
            (
                "/handler.js",
                "onmessage = function () {\n  throw new Error('in handler');\n};",
            ),
        ],
    );
    let lines = until(&mut tab, "done");
    let want = [
        "message before",
        "worker onerror ErrorEvent error true false Uncaught Error: boom throw.js true true null",
        "window onerror Uncaught Error: boom throw.js null",
        "handled onerror Uncaught TypeError: handled one handled one",
        "canceled at the object",
        "missing Event error true false undefined undefined false false undefined",
        "module Event",
        "handler ErrorEvent error true false Uncaught Error: in handler handler.js true true null",
        "window onerror Uncaught Error: in handler handler.js null",
        "done",
    ];
    for line in want {
        assert!(
            lines.contains(&line.to_owned()),
            "no {line:?} in {lines:#?}"
        );
    }
    // The canceled one went no further than its object.
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("window onerror Uncaught Error: boom"))
            .count(),
        1,
        "{lines:#?}"
    );
}

#[test]
fn close_ends_the_task_and_terminate_stops_at_once() {
    let (mut tab, _server) = page(
        "<script>\
         var closer = new Worker('close.js');\
         closer.onmessage = function (e) { console.log('closer', e.data); \
           if (e.data === 'after close') { setTimeout(function () { console.log('closed'); \
             var ticker = new Worker('tick.js');\
             var ticks = 0;\
             ticker.onmessage = function (e) { ticks++; if (ticks === 3) { ticker.terminate(); \
               ticker.postMessage('still'); \
               setTimeout(function () { console.log('ticks', ticks); console.log('done'); }, 100); } };\
           }, 100); } };\
         closer.postMessage(1);\
         </script>",
        &[
            (
                "/close.js",
                "onmessage = function (e) { postMessage('got ' + e.data); close(); \
                 setTimeout(function () { postMessage('timer after close'); }, 0); \
                 postMessage('after close'); };",
            ),
            (
                "/tick.js",
                "var n = 0; setInterval(function () { postMessage(n++); }, 1);",
            ),
        ],
    );
    let lines = until(&mut tab, "done");
    assert_eq!(
        lines,
        [
            "closer got 1",
            "closer after close",
            "closed",
            "ticks 3",
            "done"
        ]
    );
}

#[test]
fn a_workers_console_and_nested_workers_reach_the_page() {
    let (mut tab, _server) = page(
        "<script>\
         var outer = new Worker('outer.js');\
         outer.onmessage = function (e) { console.log('page got', e.data); console.log('done'); };\
         outer.postMessage(5);\
         </script>",
        &[
            (
                "/outer.js",
                "console.log('outer says', typeof Worker);\
                 var inner = new Worker('inner.js');\
                 inner.onmessage = function (e) { postMessage('nested ' + e.data); };\
                 onmessage = function (e) { inner.postMessage(e.data); };",
            ),
            (
                "/inner.js",
                "console.warn('inner says');\
                 onmessage = function (e) { postMessage('echo ' + e.data); };",
            ),
        ],
    );
    let lines = until(&mut tab, "done");
    for line in [
        "outer says function",
        "inner says",
        "page got nested echo 5",
    ] {
        assert!(
            lines.contains(&line.to_owned()),
            "no {line:?} in {lines:#?}"
        );
    }
}

#[test]
fn past_the_live_bound_a_worker_waits_for_one_to_end() {
    let (mut tab, _server) = page(
        "<script>\
         var all = [];\
         var up = 0;\
         var later = function (f) { setTimeout(f, 1000); };\
         for (var i = 0; i < 20; i++) {\
           var w = new Worker('stay.js');\
           w.onmessage = function () { up++; if (up === 16) { later(function () {\
             console.log('up', up); all[0].terminate();\
             later(function () { console.log('after terminate', up); all[1].postMessage('close');\
               later(function () { console.log('after close', up); console.log('done'); });\
             }); }); } };\
           all.push(w);\
         }\
         </script>",
        &[(
            "/stay.js",
            "postMessage('up'); onmessage = function (e) { if (e.data === 'close') { close(); } };",
        )],
    );
    // Sixteen at once; another as one is terminated, and as one closes.
    let lines = until(&mut tab, "done");
    assert_eq!(
        lines,
        ["up 16", "after terminate 17", "after close 18", "done"]
    );
}

#[test]
fn a_documents_workers_end_with_it() {
    let (mut tab, server) = page(
        "<script>\
         var w = new Worker('tick.js');\
         w.onmessage = function (e) { if (e.data === 2) { console.log('ticking'); } };\
         </script>",
        &[(
            "/tick.js",
            "var n = 0; setInterval(function () { postMessage(n++); }, 1);",
        )],
    );
    until(&mut tab, "ticking");
    let live = tab.engine.live_workers();
    assert_eq!(live.load(Ordering::SeqCst), 1);
    let other = server.url("/other.html");
    tab.engine.navigate(other.clone());
    tab.wait_until(|engine| {
        incoming(engine).is_none()
            && engine.url() == Some(&other)
            && engine.load_state() == &LoadState::Loaded
    });
    assert_eq!(live.load(Ordering::SeqCst), 0);
    assert!(tab.engine.take_console().is_empty());
}
