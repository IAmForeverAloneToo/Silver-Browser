//! `XMLHttpRequest` (M27.6, docs/design/fetch.md): its states and events
//! in the reference browser's order, its upload's, its errors, aborts
//! and timeouts, its synchronous form, its response's fields and types.

use std::time::{Duration, Instant};

use net::test_server::{Reply, ServerRequest, TestServer};

use super::tests::load_all;
use crate::testing::*;

/// Runs the engine's tasks as results come until `count` lines are
/// logged, or fails after ten seconds.
fn logged(tab: &mut Tab, count: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        tab.engine.poll_network();
        tab.engine.run_tasks();
        lines.extend(
            tab.engine
                .take_console()
                .into_iter()
                .map(|entry| entry.text),
        );
        if lines.len() >= count {
            return lines;
        }
        assert!(Instant::now() < deadline, "logged only {lines:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The script every page here begins with: `watch(x)` records `x`'s
/// events and its upload's, and logs them with its fields at `loadend`.
const WATCH: &str = "function watch(x, name) {\
    const seen = [];\
    const record = e => seen.push(e.type + (e.type === 'readystatechange' ? x.readyState : '') +\
      (e instanceof ProgressEvent ? '(' + e.loaded + '/' + e.total + '/' + e.lengthComputable + ')' : ''));\
    for (const t of ['readystatechange', 'loadstart', 'progress', 'load', 'error', 'abort', 'timeout', 'loadend']) x.addEventListener(t, record);\
    for (const t of ['loadstart', 'progress', 'load', 'error', 'abort', 'loadend']) x.upload.addEventListener(t, e => seen.push('up:' + e.type));\
    x.addEventListener('loadend', () => console.log(name, JSON.stringify(seen), x.status, JSON.stringify(x.statusText), x.responseURL.replace(location.origin, ''), x.readyState));\
    return seen;\
  }";

fn page_with(
    script: &str,
    route: impl Fn(&ServerRequest) -> Option<Reply> + Send + Sync + 'static,
) -> (Tab, TestServer) {
    let page = format!("<script>{WATCH}{script}</script>");
    let server = TestServer::plain(move |request| {
        if request.target == "/page.html" {
            return Reply::ok("text/html", page.as_bytes());
        }
        route(request)
            .unwrap_or_else(|| Reply::with_fields(404, &[("Content-Type", "text/plain")], b"no"))
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    (tab, server)
}

fn ok(body: &'static [u8]) -> Option<Reply> {
    Some(Reply::with_fields(
        200,
        &[("Content-Type", "text/plain")],
        body,
    ))
}

#[test]
fn an_xhr_goes_through_its_states_with_the_reference_browsers_events() {
    let (mut tab, server) = page_with(
        "const get = new XMLHttpRequest(); watch(get, 'get'); get.open('GET', '/data'); \
         get.send(); get.addEventListener('loadstart', () => {}); \
         const post = new XMLHttpRequest(); const seen = watch(post, 'post'); post.open('POST', '/data'); \
         post.send('abc'); seen.push('sent');          const empty = new XMLHttpRequest(); watch(empty, 'empty'); empty.open('GET', '/empty'); empty.send();",
        |request| match request.target.as_str() {
            "/data" => ok(b"hello"),
            "/empty" => Some(Reply::with_fields(204, &[], b"")),
            _ => None,
        },
    );
    let mut lines = logged(&mut tab, 3);
    lines.sort();
    assert_eq!(
        lines,
        [
            "empty [\"readystatechange1\",\"loadstart(0/0/false)\",\"readystatechange2\",\"progress(0/0/true)\",\"readystatechange4\",\"load(0/0/true)\",\"loadend(0/0/true)\"] 204 \"Status\" /empty 4",
            "get [\"readystatechange1\",\"loadstart(0/0/false)\",\"readystatechange2\",\"readystatechange3\",\"progress(5/5/true)\",\"readystatechange4\",\"load(5/5/true)\",\"loadend(5/5/true)\"] 200 \"Status\" /data 4",
            "post [\"readystatechange1\",\"loadstart(0/0/false)\",\"up:loadstart\",\"sent\",\"up:progress\",\"up:load\",\"up:loadend\",\"readystatechange2\",\"readystatechange3\",\"progress(5/5/true)\",\"readystatechange4\",\"load(5/5/true)\",\"loadend(5/5/true)\"] 200 \"Status\" /data 4",
        ]
    );
    let post = server
        .requests()
        .into_iter()
        .find(|r| r.method == "POST")
        .unwrap();
    assert_eq!(
        (post.body.as_slice(), post.header("Content-Type")),
        (&b"abc"[..], Some("text/plain;charset=UTF-8"))
    );
}

#[test]
fn errors_aborts_and_timeouts_end_an_xhr_with_their_events() {
    let other = TestServer::plain(|_: &ServerRequest| {
        Reply::with_fields(200, &[("Content-Type", "text/plain")], b"x")
    });
    let script = format!(
        "const cors = new XMLHttpRequest(); watch(cors, 'cors'); cors.open('GET', '{elsewhere}'); cors.send();\
         const gone = new XMLHttpRequest(); watch(gone, 'gone'); gone.open('GET', 'http://127.0.0.1:1/'); gone.send();\
         const stop = new XMLHttpRequest(); watch(stop, 'stop'); stop.open('GET', '/hang'); stop.send(); stop.abort();\
         console.log('after', stop.readyState);\
         const late = new XMLHttpRequest(); watch(late, 'late'); late.open('GET', '/hang'); late.timeout = 40; late.send();",
        elsewhere = other.url("/x")
    );
    let (mut tab, _server) = page_with(&script, |request| match request.target.as_str() {
        "/hang" => Some(Reply::Hang),
        _ => None,
    });
    let mut lines = logged(&mut tab, 7);
    lines.sort();
    // The console says why the two failed; the page learns only that.
    let said: Vec<String> = lines.drain(..2).collect();
    assert_eq!(
        said[0],
        "Failed to fetch http://127.0.0.1:1/: port 1 is not fetched"
    );
    assert_eq!(
        said[1],
        format!(
            "Failed to fetch {}: refused by CORS: no Access-Control-Allow-Origin",
            other.url("/x")
        )
    );
    let failed = "[\"readystatechange1\",\"loadstart(0/0/false)\",\"readystatechange4\",\"error(0/0/false)\",\"loadend(0/0/false)\"] 0 \"\"  4";
    assert_eq!(
        lines,
        [
            "after 0".to_owned(),
            format!("cors {failed}"),
            format!("gone {failed}"),
            "late [\"readystatechange1\",\"loadstart(0/0/false)\",\"readystatechange4\",\"timeout(0/0/false)\",\"loadend(0/0/false)\"] 0 \"\"  4".to_owned(),
            "stop [\"readystatechange1\",\"loadstart(0/0/false)\",\"readystatechange4\",\"abort(0/0/false)\",\"loadend(0/0/false)\"] 0 \"\"  4".to_owned(),
        ]
    );
}

#[test]
fn an_upload_listened_to_is_preflighted_across_origins() {
    let other = TestServer::plain(|_: &ServerRequest| {
        Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/plain"),
                ("Access-Control-Allow-Origin", "*"),
            ],
            b"x",
        )
    });
    let script = format!(
        "const x = new XMLHttpRequest(); x.upload.onprogress = () => {{}}; x.open('POST', '{elsewhere}');\
         x.onloadend = () => console.log('done', x.status); x.send('a');",
        elsewhere = other.url("/x")
    );
    let (mut tab, _server) = page_with(&script, |_| None);
    assert_eq!(logged(&mut tab, 1), ["done 200"]);
    let methods: Vec<String> = other.requests().into_iter().map(|r| r.method).collect();
    assert_eq!(methods, ["OPTIONS", "POST"]);
}

#[test]
fn a_synchronous_xhr_answers_before_send_returns() {
    let (mut tab, _server) = page_with(
        "const x = new XMLHttpRequest(); const ev = [];\
         x.onreadystatechange = () => ev.push(x.readyState); x.onload = () => ev.push('load');\
         x.open('GET', '/data', false); x.send(); console.log(JSON.stringify(ev), x.status, x.responseText);\
         try { x.open('GET', '/data', false); x.responseType = 'json' } catch (e) { console.log(e.name) }\
         try { x.timeout = 5 } catch (e) { console.log(e.name) }\
         const t = new XMLHttpRequest(); t.timeout = 5;         try { t.open('GET', '/data', false) } catch (e) { console.log(e.name, t.readyState) }         const e = new XMLHttpRequest(); e.open('GET', '/empty', false); e.onreadystatechange = () => ev.push('e' + e.readyState);         ev.length = 0; e.send(); console.log(JSON.stringify(ev), e.status, JSON.stringify(e.responseText));         const y = new XMLHttpRequest(); y.open('GET', 'http://127.0.0.1:1/', false);\
         try { y.send() } catch (e) { console.log(e.name, e.message.replace(location.origin, '')) }",
        |request| match request.target.as_str() {
            "/data" => ok(b"now"),
            "/empty" => Some(Reply::with_fields(204, &[], b"")),
            _ => None,
        },
    );
    let mut lines = logged(&mut tab, 7);
    // The console says why the last failed, before the page hears it.
    assert_eq!(
        lines.remove(5),
        "Failed to fetch http://127.0.0.1:1/: port 1 is not fetched"
    );
    assert_eq!(
        lines,
        [
            "[1,4,\"load\"] 200 now",
            "InvalidAccessError",
            "InvalidAccessError",
            "InvalidAccessError 0",
            "[\"e4\"] 204 \"\"",
            "NetworkError Failed to execute 'send' on 'XMLHttpRequest': Failed to load 'http://127.0.0.1:1/'.",
        ]
    );
}

#[test]
fn opening_sending_and_fields_are_checked_as_the_standard_says() {
    let (mut tab, server) = page_with(
        "const x = new XMLHttpRequest();\
         for (const f of [() => x.open('TRACE', '/'), () => x.open('G T', '/'), () => x.open('GET', 'http://['), () => x.send(), () => x.setRequestHeader('a', 'b')]) {\
           try { f() } catch (e) { console.log(e.name, e.message) } }\
         x.open('POST', '/echo'); x.setRequestHeader('X-A', '1'); x.setRequestHeader('x-a', '2');\
         x.setRequestHeader('Cookie', 'no'); x.setRequestHeader('Content-Type', 'text/x');\
         try { x.setRequestHeader('a b', '1') } catch (e) { console.log(e.name) }\
         x.onload = () => console.log(x.getResponseHeader('A-ONE'), x.getResponseHeader('set-cookie'), JSON.stringify(x.getAllResponseHeaders()));\
         x.send(new URLSearchParams('q=1'));",
        |request| match request.target.as_str() {
            "/echo" => Some(Reply::with_fields(
                200,
                &[
                    ("Content-Type", "text/plain"),
                    ("b-two", "2"),
                    ("A-One", "1"),
                    ("a-one", "3"),
                    ("Set-Cookie", "s=1"),
                ],
                b"ok",
            )),
            _ => None,
        },
    );
    assert_eq!(
        logged(&mut tab, 7),
        [
            "SecurityError Failed to execute 'open' on 'XMLHttpRequest': 'TRACE' HTTP method is unsupported.",
            "SyntaxError Failed to execute 'open' on 'XMLHttpRequest': 'G T' is not a valid HTTP method.",
            "SyntaxError Failed to execute 'open' on 'XMLHttpRequest': Invalid URL",
            "InvalidStateError Failed to execute 'send' on 'XMLHttpRequest': The object's state must be OPENED.",
            "InvalidStateError Failed to execute 'setRequestHeader' on 'XMLHttpRequest': The object's state must be OPENED.",
            "SyntaxError",
            "1, 3 null \"a-one: 1, 3\\r\\nb-two: 2\\r\\ncontent-length: 2\\r\\ncontent-type: text/plain\\r\\n\"",
        ]
    );
    let sent = server
        .requests()
        .into_iter()
        .find(|r| r.target == "/echo")
        .unwrap();
    assert_eq!(
        (
            sent.header("X-A"),
            sent.header("Cookie"),
            sent.header("Content-Type")
        ),
        (Some("1, 2"), None, Some("text/x"))
    );
}

#[test]
fn a_response_is_text_json_a_buffer_or_a_blob_as_asked() {
    let (mut tab, _server) = page_with(
        "function get(type, then) { const x = new XMLHttpRequest(); x.open('GET', '/json'); x.responseType = type;\
           x.onload = () => then(x); x.send(); return x; }\
         const j = get('json', x => console.log('json', JSON.stringify(x.response), x.response === x.response));\
         try { j.responseText } catch (e) { console.log(e.name) }\
         console.log('before', j.response);\
         get('arraybuffer', x => console.log('buffer', x.response.byteLength, new Uint8Array(x.response)[0]));\
         get('blob', x => x.response.text().then(t => console.log('blob', x.response.type, t)));\
         get('text', x => console.log('text', x.response));\
         const w = new XMLHttpRequest(); w.open('GET', '/latin'); w.overrideMimeType('text/plain; charset=windows-1252');\
         w.onload = () => console.log('latin', w.responseText); w.send();",
        |request| match request.target.as_str() {
            "/json" => Some(Reply::with_fields(
                200,
                &[("Content-Type", "application/json")],
                b"{\"a\":1}",
            )),
            "/latin" => Some(Reply::with_fields(
                200,
                &[("Content-Type", "text/plain")],
                b"caf\xe9",
            )),
            _ => None,
        },
    );
    let mut lines = logged(&mut tab, 7);
    let sync: Vec<String> = lines.drain(..2).collect();
    assert_eq!(sync, ["InvalidStateError", "before null"]);
    lines.sort();
    assert_eq!(
        lines,
        [
            "blob application/json {\"a\":1}",
            "buffer 7 123",
            "json {\"a\":1} true",
            "latin caf\u{e9}",
            "text {\"a\":1}",
        ]
    );
}

#[test]
fn a_base_a_script_inserts_is_what_open_resolves_against() {
    let (mut tab, _server) = page_with("", |request| match request.target.as_str() {
        "/r/f.txt" => ok(b"bottom"),
        "/f.txt" => ok(b"top"),
        _ => None,
    });
    // A base's href reads against the document's own URL, not itself.
    tab.engine.run_classic(
        "var b = document.createElement('base'); b.href = 'r/';\
         document.head.appendChild(b);\
         console.log(b.href.replace(location.origin, ''), b.getAttribute('href'));\
         var c = new XMLHttpRequest(); c.open('GET', 'f.txt', false); c.send();\
         console.log(c.responseURL.replace(location.origin, ''), c.responseText);",
    );
    assert_eq!(logged(&mut tab, 2), ["/r/ r/", "/r/f.txt bottom"]);
}
