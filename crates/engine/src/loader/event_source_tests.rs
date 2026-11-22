//! A page's EventSource (G74), against a server of the test's own on
//! 127.0.0.1 that streams its answers a piece at a time: the events of a
//! stream in order with their fields, the reconnection with its last
//! event ID, the failures the reference browser fails for good with its
//! console's words, the constructor's checks, a cross-origin stream and
//! a worker's. Each expectation is what Chrome 157 did with the same
//! streams (probed 2026-11-22).

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use net::test_server::sockets::SocketListener;

use super::tests::load_all;
use crate::testing::*;

/// What the server answers a request with: the head's status line's
/// code, its type, more fields, the body's pieces (sent 20 ms apart),
/// and how long the connection is held open after them.
struct Answer {
    status: u16,
    content_type: &'static str,
    fields: Vec<(&'static str, String)>,
    pieces: Vec<&'static [u8]>,
    hold: Duration,
}

impl Answer {
    fn stream(pieces: Vec<&'static [u8]>) -> Answer {
        Answer {
            status: 200,
            content_type: "text/event-stream",
            fields: Vec::new(),
            pieces,
            hold: Duration::ZERO,
        }
    }

    fn held(mut self, hold: Duration) -> Answer {
        self.hold = hold;
        self
    }

    fn page(body: String) -> Answer {
        Answer {
            status: 200,
            content_type: "text/html",
            fields: vec![("Content-Length", body.len().to_string())],
            pieces: vec![Box::leak(body.into_boxed_str()).as_bytes()],
            hold: Duration::ZERO,
        }
    }

    fn status(status: u16) -> Answer {
        Answer {
            status,
            content_type: "text/plain",
            fields: vec![("Content-Length", "0".to_owned())],
            pieces: Vec::new(),
            hold: Duration::ZERO,
        }
    }
}

/// A server for the test's time (its thread is let go with the test):
/// each connection on a thread of its own, answered as `route` says for
/// its request's target and how many times that target was asked for
/// before; each request's head goes to the receiver returned.
fn serve(
    route: impl Fn(&str, usize) -> Answer + Send + Sync + 'static,
) -> (u16, mpsc::Receiver<String>) {
    let listener = SocketListener::bind();
    let port = listener.port();
    let (heads, seen) = mpsc::channel();
    let route = std::sync::Arc::new(route);
    let asked = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
        String,
        usize,
    >::new()));
    thread::spawn(move || {
        loop {
            let mut peer = listener.accept();
            let (route, asked, heads) = (route.clone(), asked.clone(), heads.clone());
            // A connection that sends no request ends its own thread.
            thread::spawn(move || {
                let head = peer.head();
                let target = head.split(' ').nth(1).unwrap_or_default().to_owned();
                let path = target.split('?').next().unwrap_or_default().to_owned();
                let n = {
                    let mut asked = asked.lock().unwrap();
                    let n = asked.entry(path.clone()).or_default();
                    *n += 1;
                    *n - 1
                };
                let answer = route(&path, n);
                let _ = heads.send(head);
                let mut out = format!(
                    "HTTP/1.1 {} X\r\nContent-Type: {}\r\nConnection: close\r\n",
                    answer.status, answer.content_type
                );
                for (name, value) in &answer.fields {
                    out.push_str(&format!("{name}: {value}\r\n"));
                }
                out.push_str("\r\n");
                peer.try_write(out.as_bytes());
                for piece in &answer.pieces {
                    peer.try_write(piece);
                    thread::sleep(Duration::from_millis(20));
                }
                thread::sleep(answer.hold);
                peer.shutdown();
            });
        }
    });
    (port, seen)
}

/// Runs the engine's tasks as results come until `count` lines are
/// logged, or fails after `within`.
fn logged(tab: &mut Tab, count: usize, within: Duration) -> Vec<String> {
    let mut lines = Vec::new();
    let deadline = Instant::now() + within;
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
        thread::sleep(Duration::from_millis(2));
    }
}

/// The script that logs each event of source `s` with its fields:
/// `<type> <readyState> <data> <origin> <lastEventId>`.
const WATCH: &str = "const rec = (e) => console.log([e.type, s.readyState, e.constructor.name,\
     e.data === undefined ? '-' : JSON.stringify(e.data), e.origin === undefined ? '-' : e.origin,\
     e.lastEventId === undefined ? '-' : JSON.stringify(e.lastEventId)].join(' '));\
     s.onopen = rec; s.onmessage = rec; s.onerror = rec; s.addEventListener('custom', rec);";

fn tab_at(port: u16) -> Tab {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    let url = net::Url::parse(&format!("http://127.0.0.1:{port}/page.html")).unwrap();
    load_all(&mut tab, url);
    tab
}

#[test]
fn a_stream_fires_its_events_and_is_made_again_with_its_last_event_id() {
    // The stream's `retry: 300`, not the 3 seconds a stream starts with,
    // is the wait before the connection is made again.
    let page = format!(
        "<script>const s = new EventSource('/s'); {WATCH}\
         let erred; s.addEventListener('error', () => {{ erred ??= performance.now(); }});\
         s.addEventListener('open', () => {{ if (erred !== undefined && !s.timed) {{\
           s.timed = true; console.log('again within 2 s', performance.now() - erred < 2000); }} }});</script>"
    );
    let (port, heads) = serve(move |path, n| match (path, n) {
        ("/page.html", _) => Answer::page(page.clone()),
        ("/s", 0) => Answer::stream(vec![
            b"retry: 300\nid: 1\ndata: a\ndata: b\n\n",
            b"event: custom\ndata: c\n\n",
            b": comment\n\n",
            b"id: 7\ndata: d\n\n",
            b"data: tail-not-sent",
        ]),
        ("/s", 1) => Answer::stream(vec![b"data: again\n\n"]),
        _ => Answer::status(204),
    });
    let mut tab = tab_at(port);
    let lines = logged(&mut tab, 10, Duration::from_secs(10));
    let origin = format!("http://127.0.0.1:{port}");
    assert_eq!(
        lines,
        vec![
            "open 1 Event - - -".to_owned(),
            format!("message 1 MessageEvent \"a\\nb\" {origin} \"1\""),
            format!("custom 1 MessageEvent \"c\" {origin} \"1\""),
            format!("message 1 MessageEvent \"d\" {origin} \"7\""),
            "error 0 Event - - -".to_owned(),
            "open 1 Event - - -".to_owned(),
            "again within 2 s true".to_owned(),
            format!("message 1 MessageEvent \"again\" {origin} \"7\""),
            "error 0 Event - - -".to_owned(),
            // The third answer, 204, fails it for good.
            "error 2 Event - - -".to_owned(),
        ]
    );
    let heads: Vec<String> = heads.try_iter().collect();
    let streams: Vec<&String> = heads.iter().filter(|h| h.starts_with("GET /s ")).collect();
    assert_eq!(streams.len(), 3);
    for head in &streams {
        assert!(head.contains("\r\nAccept: text/event-stream\r\n"), "{head}");
        assert!(head.contains("\r\nCache-Control: no-cache\r\n"), "{head}");
        assert!(head.contains("\r\nPragma: no-cache\r\n"), "{head}");
    }
    assert!(!streams[0].contains("Last-Event-ID"));
    assert!(
        streams[1].contains("\r\nLast-Event-ID: 7\r\n"),
        "{}",
        streams[1]
    );
}

#[test]
fn a_wrong_status_type_or_charset_fails_for_good_with_the_references_words() {
    let page = "<script>\
        for (const path of ['/404', '/plain', '/latin']) {\
          const s = new EventSource(path);\
          s.onopen = () => console.log(path, 'open');\
          s.onerror = () => console.log(path, 'error', s.readyState);\
        }\
        const c = new EventSource('/held'); c.onopen = () => console.log('closed one opened');\
        c.close(); console.log('closed', c.readyState);</script>"
        .to_owned();
    let (port, heads) = serve(move |path, _| match path {
        "/page.html" => Answer::page(page.clone()),
        "/404" => Answer {
            status: 404,
            ..Answer::stream(vec![b"data: x\n\n"])
        },
        "/plain" => Answer {
            content_type: "text/plain",
            ..Answer::stream(vec![b"data: x\n\n"])
        },
        "/latin" => Answer {
            content_type: "text/event-stream;charset=iso-8859-1",
            ..Answer::stream(vec![b"data: x\n\n"])
        },
        _ => Answer::stream(vec![b"data: x\n\n"]).held(Duration::from_millis(300)),
    });
    let mut tab = tab_at(port);
    let mut lines = logged(&mut tab, 6, Duration::from_secs(10));
    // The closed one's connection, if it was made, fires nothing.
    let quiet = Instant::now() + Duration::from_millis(500);
    while Instant::now() < quiet {
        tab.engine.poll_network();
        tab.engine.run_tasks();
        lines.extend(tab.engine.take_console().into_iter().map(|e| e.text));
        thread::sleep(Duration::from_millis(5));
    }
    // Closed before its connection was to be made, it made none.
    assert!(!heads.try_iter().any(|head| head.starts_with("GET /held ")));
    lines.sort();
    assert_eq!(
        lines,
        vec![
            "/404 error 2",
            "/latin error 2",
            "/plain error 2",
            "EventSource's response has a MIME type (\"text/plain\") that is not \"text/event-stream\". Aborting the connection.",
            "EventSource's response has a charset (\"iso-8859-1\") that is not UTF-8. Aborting the connection.",
            "closed 2",
        ]
    );
}

#[test]
fn the_constructor_checks_and_members_are_the_references() {
    let page = "<script>\
        const r = [];\
        const t = (f) => { try { r.push('ok:' + String(f())); } catch (e) { r.push(e.name + ': ' + e.message); } };\
        t(() => new EventSource('http://[::1'));\
        t(() => new EventSource());\
        t(() => { const s = new EventSource('/n?x'); s.close(); return s.url + ' ' + s.withCredentials; });\
        t(() => { const s = new EventSource('/n', { withCredentials: true }); s.close(); return s.withCredentials; });\
        r.push([EventSource.CONNECTING, EventSource.OPEN, EventSource.CLOSED].join());\
        r.push(Object.getOwnPropertyNames(EventSource.prototype).join());\
        r.push(Object.getOwnPropertyNames(EventSource).join());\
        r.push(Object.prototype.toString.call(new EventSource('/n')));\
        console.log(r.join('|'));</script>"
        .to_owned();
    let (port, _heads) = serve(move |path, _| match path {
        "/page.html" => Answer::page(page.clone()),
        _ => Answer::status(204),
    });
    let mut tab = tab_at(port);
    let lines = logged(&mut tab, 1, Duration::from_secs(10));
    let expected = [
        "SyntaxError: Failed to construct 'EventSource': Cannot open an EventSource to 'http://[::1'. The URL is invalid.".to_owned(),
        "TypeError: Failed to construct 'EventSource': 1 argument required, but only 0 present.".to_owned(),
        format!("ok:http://127.0.0.1:{port}/n?x false"),
        "ok:true".to_owned(),
        "0,1,2".to_owned(),
        "url,withCredentials,readyState,onopen,onmessage,onerror,CONNECTING,OPEN,CLOSED,close,constructor".to_owned(),
        "length,name,prototype,CONNECTING,OPEN,CLOSED".to_owned(),
        "[object EventSource]".to_owned(),
    ];
    assert_eq!(lines[0], expected.join("|"));
}

#[test]
fn another_origins_stream_needs_cors_and_a_refused_connection_is_tried_again() {
    let (stream_port, _stream_heads) = serve(|path, _| match path {
        "/ok" => Answer {
            fields: vec![("Access-Control-Allow-Origin", "*".to_owned())],
            ..Answer::stream(vec![b"data: cors\n\n"])
        }
        .held(Duration::from_millis(300)),
        _ => Answer::stream(vec![b"data: nocors\n\n"]),
    });
    let refused = net::test_server::sockets::closed_port();
    let page = format!(
        "<script>\
         const ok = new EventSource('http://127.0.0.1:{stream_port}/ok');\
         ok.onmessage = (e) => {{ console.log('ok', e.data, e.origin); ok.close(); }};\
         const no = new EventSource('http://127.0.0.1:{stream_port}/no');\
         no.onmessage = () => console.log('no message');\
         no.onerror = () => console.log('no error', no.readyState);\
         const gone = new EventSource('http://127.0.0.1:{refused}/x');\
         gone.onerror = () => {{ console.log('gone error', gone.readyState); gone.close(); }};</script>"
    );
    let (port, _heads) = serve(move |_, _| Answer::page(page.clone()));
    let mut tab = tab_at(port);
    let mut lines = logged(&mut tab, 3, Duration::from_secs(10));
    lines.sort();
    assert_eq!(
        lines,
        vec![
            "gone error 0".to_owned(),
            "no error 2".to_owned(),
            format!("ok cors http://127.0.0.1:{stream_port}"),
        ]
    );
}

#[test]
fn a_dedicated_workers_event_source_works_as_a_windows_does() {
    let worker = "const s = new EventSource('/s');\
        s.onopen = () => postMessage('open ' + s.readyState);\
        s.onmessage = (e) => { postMessage('message ' + e.data + ' ' + e.lastEventId); s.close(); };";
    let page = "<script>const w = new Worker('/w.js');\
        w.onmessage = (e) => console.log(e.data);</script>"
        .to_owned();
    let (port, _heads) = serve(move |path, _| match path {
        "/page.html" => Answer::page(page.clone()),
        "/w.js" => Answer {
            content_type: "text/javascript",
            fields: vec![("Content-Length", worker.len().to_string())],
            ..Answer::stream(vec![worker.as_bytes()])
        },
        _ => Answer::stream(vec![b"id: 3\ndata: hi\n\n"]).held(Duration::from_millis(300)),
    });
    let mut tab = tab_at(port);
    let lines = logged(&mut tab, 2, Duration::from_secs(10));
    assert_eq!(lines, vec!["open 1", "message hi 3"]);
}
