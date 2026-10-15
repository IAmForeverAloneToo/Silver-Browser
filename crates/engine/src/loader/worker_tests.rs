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
         var next = null;\
         var later = function (f) { setTimeout(f, 1000); };\
         for (var i = 0; i < 20; i++) {\
           var w = new Worker('stay.js');\
           w.onmessage = function () { up++;\
             if (up === 16) { later(function () {\
               console.log('up', up);\
               next = function () { console.log('after terminate', up);\
                 next = function () { later(function () {\
                   console.log('after close', up); console.log('done'); }); };\
                 all[1].postMessage('close'); };\
               all[0].terminate(); }); }\
             else if (next) { var f = next; next = null; f(); } };\
           all.push(w);\
         }\
         </script>",
        &[(
            "/stay.js",
            "postMessage('up'); onmessage = function (e) { if (e.data === 'close') { close(); } };",
        )],
    );
    // Sixteen at once, none more for a second; another once one is
    // terminated, and once one closes, and none more for a second.
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

#[test]
fn a_workers_scope_has_its_location_imports_fetches_and_canvases() {
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<script>\
             var w = new Worker('scope.js');\
             w.onmessage = function (e) { e.data.forEach(function (line) { console.log(line); }); \
               console.log('done'); };\
             w.onerror = function (e) { console.log('failed', e.message); console.log('done'); };\
             </script>",
        )),
        "/scope.js" => Some(script(SCOPE)),
        "/a.js" => Some(script("self.order = ['a'];")),
        "/b.js" => Some(script("self.order.push('b');")),
        "/throws.js" => Some(script("throw new Error('from import');")),
        "/gone.js" => Some(Reply::with_fields(
            404,
            &[("Content-Type", "text/javascript")],
            b"self.order.push('gone');",
        )),
        "/data.txt" => Some(Reply::ok("text/plain", b"hello")),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let lines = until(&mut tab, "done");
    assert_eq!(
        lines,
        [
            "location scope.js /scope.js true scope.js [object WorkerLocation]",
            "navigator Mozilla/5.0 string en-US true [object WorkerNavigator]",
            "performance number true true",
            "imported a,b",
            "missing NetworkError 19 Failed to execute 'importScripts' on 'WorkerGlobalScope': \
             The script at '/missing.js' failed to load.",
            "html NetworkError",
            "gone NetworkError a,b",
            "throws from import",
            "bad url SyntaxError 12",
            "sync xhr 200 hello",
            "canvas 255,0,0,255 0,0,0,0",
            "bitmap 4x2 0",
            "created 2",
            "fetched hello",
            "async xhr hello",
            "aborted AbortError",
            "wasm object",
            "timer",
            "done",
        ]
    );
}

/// What a worker's scope is given to do, line by line.
const SCOPE: &str = "\
var out = [];\
var tag = function (o) { return Object.prototype.toString.call(o); };\
out.push(['location', location.href.split('/').pop(), location.pathname, \
  location.origin === self.origin, String(location).split('/').pop(), tag(location)].join(' '));\
out.push(['navigator', navigator.userAgent.slice(0, 11), typeof navigator.platform, \
  navigator.language, navigator.onLine, tag(navigator)].join(' '));\
out.push(['performance', typeof performance.now(), performance.now() >= 0, \
  performance.timeOrigin > 0].join(' '));\
importScripts('a.js', 'b.js');\
out.push('imported ' + self.order.join(','));\
try { importScripts('missing.js'); } catch (e) { \
  out.push(['missing', e.name, e.code, e.message.split(location.origin).join('')].join(' ')); }\
try { importScripts('page.html'); } catch (e) { out.push('html ' + e.name); }\
try { importScripts('gone.js'); } catch (e) { out.push('gone ' + e.name + ' ' + self.order); }\
try { importScripts('throws.js'); } catch (e) { out.push('throws ' + e.message); }\
try { importScripts('http://[x'); } catch (e) { out.push('bad url ' + e.name + ' ' + e.code); }\
importScripts();\
var sync = new XMLHttpRequest(); sync.open('GET', 'data.txt', false); sync.send();\
out.push('sync xhr ' + sync.status + ' ' + sync.responseText);\
var canvas = new OffscreenCanvas(4, 2); var ctx = canvas.getContext('2d');\
ctx.fillStyle = '#f00'; ctx.fillRect(0, 0, 2, 2);\
out.push('canvas ' + Array.from(ctx.getImageData(1, 1, 1, 1).data) + ' ' + \
  Array.from(ctx.getImageData(3, 1, 1, 1).data));\
var bitmap = canvas.transferToImageBitmap();\
out.push('bitmap ' + bitmap.width + 'x' + bitmap.height + ' ' + \
  Array.from(ctx.getImageData(0, 0, 1, 1).data)[0]);\
createImageBitmap(new ImageData(2, 2)).then(function (b) { out.push('created ' + b.width); \
  return fetch('data.txt'); }).then(function (r) { return r.text(); }).then(function (t) {\
  out.push('fetched ' + t);\
  var later = new XMLHttpRequest(); later.open('GET', 'data.txt');\
  later.onload = function () { out.push('async xhr ' + later.responseText);\
    var c = new AbortController(); var p = fetch('data.txt', { signal: c.signal }); c.abort();\
    p.catch(function (e) { out.push('aborted ' + e.name); \
      return WebAssembly.instantiate(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])); })\
    .then(function (r) { out.push('wasm ' + typeof r.instance);\
      setTimeout(function () { out.push('timer'); postMessage(out); }, 1); });\
  };\
  later.send();\
});";

#[test]
fn a_ports_message_wakes_the_engine_with_nothing_else_due() {
    // As a scheduler posts its work to itself through a channel.
    let (mut tab, _server) = page(
        "<script>\
         var c = new MessageChannel();\
         c.port1.onmessage = function (e) { console.log('got', e.data); };\
         c.port2.postMessage('work');\
         </script>",
        &[],
    );
    assert_eq!(until(&mut tab, "got work"), ["got work"]);
}

#[test]
fn ports_deliver_once_started_across_realms_as_the_reference_browser_does() {
    let (mut tab, _server) = page(
        "<script>\
         var err = function (e) { return e.name + ': ' + e.message + ' [' + e.code + ']'; };\
         var log = function (k, v) { console.log(k + '=' + (typeof v === 'string' ? v : JSON.stringify(v))); };\
         var wait = function (ms) { return new Promise(function (ok) { setTimeout(ok, ms); }); };\
         (async function () {\
           var c = new MessageChannel();\
           log('channel', [c.constructor.name, c.port1.constructor.name, \
             Object.getOwnPropertyNames(MessageChannel.prototype).sort().join(' ')]);\
           log('port proto chain', [Object.getPrototypeOf(MessagePort.prototype) === EventTarget.prototype]);\
           var got = [];\
           c.port2.addEventListener('message', function (e) { got.push('listener ' + e.data); });\
           c.port1.postMessage('a');\
           await wait(50);\
           log('before start', got.slice());\
           c.port2.start();\
           await wait(50);\
           log('after start', got.slice());\
           var d = new MessageChannel();\
           d.port2.onmessage = function (e) { got.push(['onmessage', e.data, e.origin, \
             e.source === null, e.ports.length, e.target === d.port2, e.lastEventId]); };\
           d.port1.postMessage({ x: 1 });\
           await wait(50);\
           log('onmessage', got.slice(-1));\
           var e2 = new MessageChannel();\
           e2.port1.postMessage(1); e2.port1.postMessage(2); e2.port1.postMessage(3);\
           var seq = [];\
           e2.port2.onmessage = function (e) { seq.push(e.data); };\
           await wait(50);\
           log('order', seq);\
           var f = new MessageChannel();\
           var closed = [];\
           f.port2.onmessage = function (e) { closed.push(e.data); };\
           f.port1.close();\
           f.port1.postMessage('after close');\
           await wait(50);\
           log('closed sender', closed);\
           var g = new MessageChannel();\
           g.port2.onmessage = function (e) { closed.push('g ' + e.data); };\
           g.port2.close();\
           g.port1.postMessage('to closed');\
           await wait(50);\
           log('closed receiver', closed);\
           var h = new MessageChannel();\
           try { h.port1.postMessage(1, [h.port1]); log('self transfer', 'ok'); } catch (e) { log('self transfer', err(e)); }\
           try { h.port1.postMessage(h.port2); log('clone port', 'ok'); } catch (e) { log('clone port', err(e)); }\
           try { h.port1.postMessage(1, [h.port2, h.port2]); log('dup port', 'ok'); } catch (e) { log('dup port', err(e)); }\
           var w = new Worker('port.js');\
           var k = new MessageChannel();\
           var fromWorker = [];\
           w.onmessage = function (e) { fromWorker.push(e.data); };\
           k.port1.onmessage = function (e) { fromWorker.push(['port', e.data, e.target === k.port1]); };\
           w.postMessage('hello', [k.port2]);\
           try { k.port2.postMessage('x'); log('port after transfer', 'ok'); } catch (e) { log('port after transfer', err(e)); }\
           try { w.postMessage(1, [k.port2]); log('transfer again', 'ok'); } catch (e) { log('transfer again', err(e)); }\
           k.port1.postMessage('ping');\
           while (fromWorker.length < 2) { await wait(10); }\
           log('worker', fromWorker);\
           var q = new MessageChannel();\
           log('structuredClone port', structuredClone(q.port1, { transfer: [q.port1] }).constructor.name);\
           console.log('done');\
         })();\
         </script>",
        &[(
            "/port.js",
            "onmessage = function (e) {\
               var port = e.ports[0];\
               postMessage(['worker got', e.data, e.ports.length, port && port.constructor.name]);\
               port.onmessage = function (m) { port.postMessage('echo ' + m.data); };\
             };",
        )],
    );
    let lines = until(&mut tab, "done");
    let refused = |what: &str, why: &str| {
        format!("{what}=DataCloneError: Failed to execute 'postMessage' on {why} [25]")
    };
    assert_eq!(
        lines,
        [
            "channel=[\"MessageChannel\",\"MessagePort\",\"constructor port1 port2\"]".to_owned(),
            "port proto chain=[true]".to_owned(),
            "before start=[]".to_owned(),
            "after start=[\"listener a\"]".to_owned(),
            "onmessage=[[\"onmessage\",{\"x\":1},\"\",true,0,true,\"\"]]".to_owned(),
            "order=[1,2,3]".to_owned(),
            "closed sender=[]".to_owned(),
            "closed receiver=[]".to_owned(),
            refused(
                "self transfer",
                "'MessagePort': Port at index 0 contains the source port."
            ),
            refused(
                "clone port",
                "'MessagePort': A MessagePort could not be cloned because it was not transferred."
            ),
            refused(
                "dup port",
                "'MessagePort': Message port at index 1 is a duplicate of an earlier port."
            ),
            "port after transfer=ok".to_owned(),
            refused(
                "transfer again",
                "'Worker': Port at index 0 is already neutered."
            ),
            "worker=[[\"worker got\",\"hello\",1,\"MessagePort\"],[\"port\",\"echo ping\",true]]"
                .to_owned(),
            "structuredClone port=MessagePort".to_owned(),
            "done".to_owned(),
        ]
    );
}

#[test]
fn a_canvas_shows_what_its_offscreen_draws_in_a_worker_once_each_task_ends() {
    let (mut tab, _server) = page(
        "<canvas id=c width=40 height=20></canvas><script>\
         var err = function (e) { return e.name + ': ' + e.message + ' [' + e.code + ']'; };\
         var log = function (k, v) { console.log(k + '=' + (typeof v === 'string' ? v : JSON.stringify(v))); };\
         var wait = function (ms) { return new Promise(function (ok) { setTimeout(ok, ms); }); };\
         var c = document.getElementById('c');\
         var reader = document.createElement('canvas'); reader.width = 40; reader.height = 20;\
         var rx = reader.getContext('2d');\
         var sample = function () {\
           rx.clearRect(0, 0, 40, 20);\
           try { rx.drawImage(c, 0, 0); } catch (e) { return err(e); }\
           var px = function (x, y) { return Array.from(rx.getImageData(x, y, 1, 1).data).join(','); };\
           return [px(2, 2), px(12, 2), px(2, 12), px(32, 2)].join(' | ');\
         };\
         var msgs = [];\
         var heard = async function () { while (msgs.length === 0) { await wait(5); } \
           await wait(50); return msgs.splice(0); };\
         (async function () {\
           var other = document.createElement('canvas'); other.getContext('2d');\
           try { other.transferControlToOffscreen(); log('after getContext', 'ok'); } catch (e) { log('after getContext', err(e)); }\
           var oc = c.transferControlToOffscreen();\
           log('offscreen', [oc.constructor.name, oc.width, oc.height]);\
           try { c.transferControlToOffscreen(); log('twice', 'ok'); } catch (e) { log('twice', err(e)); }\
           try { c.getContext('2d'); log('getContext after', 'ok'); } catch (e) { log('getContext after', err(e)); }\
           try { c.width = 50; log('width after', 'ok'); } catch (e) { log('width after', err(e)); }\
           log('placeholder before', sample());\
           var w = new Worker('draw.js');\
           w.onmessage = function (e) { msgs.push(e.data); };\
           var oc3 = document.createElement('canvas').transferControlToOffscreen();\
           try { w.postMessage({ canvas: oc3 }); log('post without transfer', 'ok'); } catch (e) { log('post without transfer', err(e)); }\
           var oc4 = document.createElement('canvas').transferControlToOffscreen(); oc4.getContext('2d');\
           try { w.postMessage(oc4, [oc4]); log('transfer with context', 'ok'); } catch (e) { log('transfer with context', err(e)); }\
           w.postMessage({ canvas: oc }, [oc]);\
           log('oc after transfer', [oc.width, oc.height]);\
           try { rx.drawImage(oc, 0, 0); log('draw transferred', 'ok'); } catch (e) { log('draw transferred', err(e)); }\
           log('worker', await heard());\
           log('placeholder drawn', sample());\
           w.postMessage('raf');\
           log('worker', await heard());\
           log('placeholder raf', sample());\
           w.postMessage('resize');\
           log('worker', await heard());\
           log('placeholder resized', [c.width, c.height, sample()]);\
           w.postMessage('busy');\
           await wait(50);\
           log('placeholder during busy', sample());\
           log('worker', await heard());\
           log('placeholder after busy', sample());\
           w.postMessage('bitmap');\
           var sent = (await heard())[0];\
           rx.clearRect(0, 0, 40, 20); rx.drawImage(sent[1], 0, 0);\
           log('bitmap from worker', [sent[0], sent[1] instanceof ImageBitmap, sent[1].width,\
           Array.from(rx.getImageData(1, 1, 1, 1).data).join(',')]);\
           var local = document.createElement('canvas'); local.width = 4; local.height = 4;\
           var lc = local.transferControlToOffscreen().getContext('2d');\
           lc.fillStyle = '#00f'; lc.fillRect(0, 0, 4, 4);\
           rx.clearRect(0, 0, 40, 20); rx.drawImage(local, 0, 0);\
           log('drawn here', Array.from(rx.getImageData(1, 1, 1, 1).data).join(','));\
           console.log('done');\
         })();\
         </script>",
        &[(
            "/draw.js",
            "var canvas, ctx;\
             onmessage = function (e) {\
               if (e.data.canvas) {\
                 canvas = e.data.canvas; ctx = canvas.getContext('2d');\
                 ctx.fillStyle = '#00ff00'; ctx.fillRect(0, 0, 10, 10);\
                 postMessage(['drawn', canvas.width, canvas.height, typeof requestAnimationFrame]);\
                 return;\
               }\
               if (e.data === 'resize') {\
                 canvas.width = 30; ctx.fillStyle = '#0000ff'; ctx.fillRect(0, 0, 5, 5);\
                 postMessage(['resized', canvas.width, canvas.height]);\
                 return;\
               }\
               if (e.data === 'raf') {\
                 var t0 = performance.now();\
                 requestAnimationFrame(function (t) {\
                   ctx.fillStyle = '#ff0000'; ctx.fillRect(10, 0, 10, 10);\
                   postMessage(['raf', typeof t, t >= t0 - 1000]);\
                 });\
                 return;\
               }\
               if (e.data === 'bitmap') {\
               var small = new OffscreenCanvas(2, 2); var sc = small.getContext('2d');\
               sc.fillStyle = '#f0f'; sc.fillRect(0, 0, 2, 2);\
               postMessage(['bitmap', small.transferToImageBitmap()]);\
               return;\
               }\
               if (e.data === 'busy') {\
                 ctx.fillStyle = '#ffff00'; ctx.fillRect(0, 10, 10, 10);\
                 var end = Date.now() + 600; while (Date.now() < end) {}\
                 postMessage(['busy done']);\
               }\
             };",
        )],
    );
    let lines = until(&mut tab, "done");
    let state = |what: &str, method: &str, why: &str| {
        format!("{what}=InvalidStateError: Failed to {method}: {why} [11]")
    };
    assert_eq!(
        lines,
        [
            state(
                "after getContext",
                "execute 'transferControlToOffscreen' on 'HTMLCanvasElement'",
                "Cannot transfer control from a canvas that has a rendering context."
            ),
            "offscreen=[\"OffscreenCanvas\",40,20]".to_owned(),
            state(
                "twice",
                "execute 'transferControlToOffscreen' on 'HTMLCanvasElement'",
                "Cannot transfer control from a canvas for more than one time."
            ),
            state(
                "getContext after",
                "execute 'getContext' on 'HTMLCanvasElement'",
                "Cannot get context from a canvas that has transferred its control to offscreen."
            ),
            state(
                "width after",
                "set the 'width' property on 'HTMLCanvasElement'",
                "Cannot resize canvas after call to transferControlToOffscreen()."
            ),
            "placeholder before=0,0,0,0 | 0,0,0,0 | 0,0,0,0 | 0,0,0,0".to_owned(),
            "post without transfer=DataCloneError: Failed to execute 'postMessage' on 'Worker': \
             An OffscreenCanvas could not be cloned because it was not transferred. [25]"
                .to_owned(),
            state(
                "transfer with context",
                "execute 'postMessage' on 'Worker'",
                "An OffscreenCanvas could not be transferred because it had a rendering context."
            ),
            "oc after transfer=[0,0]".to_owned(),
            state(
                "draw transferred",
                "execute 'drawImage' on 'CanvasRenderingContext2D'",
                "The image source is detached."
            ),
            "worker=[[\"drawn\",40,20,\"function\"]]".to_owned(),
            "placeholder drawn=0,255,0,255 | 0,0,0,0 | 0,0,0,0 | 0,0,0,0".to_owned(),
            "worker=[[\"raf\",\"number\",true]]".to_owned(),
            "placeholder raf=0,255,0,255 | 255,0,0,255 | 0,0,0,0 | 0,0,0,0".to_owned(),
            "worker=[[\"resized\",30,20]]".to_owned(),
            "placeholder resized=[30,20,\"0,0,255,255 | 0,0,0,0 | 0,0,0,0 | 0,0,0,0\"]".to_owned(),
            "placeholder during busy=0,0,255,255 | 0,0,0,0 | 0,0,0,0 | 0,0,0,0".to_owned(),
            "worker=[[\"busy done\"]]".to_owned(),
            "placeholder after busy=0,0,255,255 | 0,0,0,0 | 255,255,0,255 | 0,0,0,0".to_owned(),
            "bitmap from worker=[\"bitmap\",true,2,\"255,0,255,255\"]".to_owned(),
            "drawn here=0,0,255,255".to_owned(),
            "done".to_owned(),
        ]
    );
}

#[test]
fn a_placeholder_drawn_here_takes_its_offscreens_size() {
    let (mut tab, _server) = page(
        "<canvas id=c width=40 height=20></canvas><script>\
         var c = document.getElementById('c');\
         var oc = c.transferControlToOffscreen();\
         oc.width = 7;\
         oc.getContext('2d').fillRect(0, 0, 7, 20);\
         setTimeout(function () { console.log('size', c.width, c.height); }, 50);\
         </script>",
        &[],
    );
    assert_eq!(until(&mut tab, "size 7 20"), ["size 7 20"]);
}

#[test]
fn a_message_to_a_worker_that_ended_is_dropped() {
    // A module worker fails to load here (G41): its thread never runs.
    let (mut tab, _server) = page(
        "<script>\
         var w = new Worker('w.js', { type: 'module' });\
         w.onerror = function () {\
           try { w.postMessage(1); console.log('posted'); } catch (e) { console.log('threw', e.name); }\
         };\
         </script>",
        &[("/w.js", "postMessage(1);")],
    );
    assert_eq!(until(&mut tab, "posted"), ["posted"]);
}

#[test]
fn a_workers_xhr_scripts_and_url_are_a_workers() {
    // A page in windows-1252, whose worker's URL keeps its query so.
    let page = b"<meta charset=windows-1252><script>\
        var w = new Worker('xhr.js?\xDF');\
        w.onmessage = function (e) { console.log('xhr', JSON.stringify(e.data));\
          var bom = new Worker('utf16.js');\
          bom.onerror = function (e) { e.preventDefault(); console.log('utf-16 refused');\
            var eight = new Worker('utf8.js');\
            eight.onmessage = function (e) { console.log('utf-8', e.data); console.log('done'); }; };\
          bom.onmessage = function () { console.log('utf-16 ran'); console.log('done'); }; };\
        </script>"
        .to_vec();
    let server = serve(move |path| match path {
        "/page.html" => Some(Reply::ok("text/html", &page)),
        // XHR in a worker: no responseXML, no "document", and a timeout
        // and a response type for a synchronous request.
        "/xhr.js?%DF" => Some(script(
            "var x = new XMLHttpRequest();\
             var out = ['responseXML' in x];\
             x.responseType = 'document'; out.push(x.responseType);\
             x.open('GET', 'data.json', false); x.responseType = 'json'; x.timeout = 1000;\
             x.send(); out.push(x.response);\
             var y = new XMLHttpRequest(); y.responseType = 'text'; y.timeout = 1000;\
             y.open('GET', 'data.json', false); y.send(); out.push(y.response);\
             postMessage(out);",
        )),
        "/data.json" => Some(Reply::ok("application/json", b"[1]")),
        // A UTF-16 byte order mark is no UTF-8 one: the script's text is
        // garbage, its compilation an error.
        "/utf16.js" => Some(Reply::ok(
            "text/javascript",
            b"\xFE\xFF\0p\0o\0s\0t\0M\0e\0s\0s\0a\0g\0e\0(\x001\0)",
        )),
        "/utf8.js" => Some(Reply::ok(
            "text/javascript",
            b"\xEF\xBB\xBFpostMessage('bom');",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let lines = until(&mut tab, "done");
    assert_eq!(
        lines,
        [
            "xhr [false,\"\",[1],\"[1]\"]",
            "utf-16 refused",
            "utf-8 bom",
            "done"
        ]
    );
}
