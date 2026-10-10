//! A script's `fetch()` (M27.1, docs/design/fetch.md): the response it
//! may see, same-origin, by CORS and opaque; the bodies it sends; its
//! aborts; `Headers`, `Request` and `Response` themselves; its answers
//! as tasks; and the bound on fetches in flight.

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

/// A page whose script is `script`, served by `server` at `/page.html`
/// with the server's other paths, loaded with scripting on.
fn page_with(
    script: &str,
    route: impl Fn(&ServerRequest) -> Option<Reply> + Send + Sync + 'static,
) -> (Tab, TestServer) {
    let page = format!("<script>{script}</script>");
    let server = TestServer::plain(move |request| {
        if request.target == "/page.html" {
            return Reply::ok("text/html", page.as_bytes());
        }
        route(request).unwrap_or_else(|| {
            Reply::with_fields(404, &[("Content-Type", "text/plain")], b"not here")
        })
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    (tab, server)
}

#[test]
fn a_fetch_resolves_with_what_a_script_may_see_of_a_basic_response() {
    let (mut tab, _server) = page_with(
        "fetch('/r').then(r => r.json().then(j => console.log(r.status, r.ok, r.statusText, r.type, \
         r.url.endsWith('/data.json'), r.redirected, [...r.headers].map(h => h[0]).join(), \
         r.headers.get('set-cookie'), JSON.stringify(j), r.bodyUsed)));",
        |request| match request.target.as_str() {
            "/r" => Some(Reply::with_fields(302, &[("Location", "/data.json")], b"")),
            "/data.json" => Some(Reply::with_fields(
                200,
                &[
                    ("Content-Type", "application/json"),
                    ("X-Custom", "c"),
                    ("Set-Cookie", "a=1"),
                ],
                b"{\"a\":[1,2]}",
            )),
            _ => None,
        },
    );
    assert_eq!(
        logged(&mut tab, 1),
        [
            "200 true Status basic true true content-length,content-type,x-custom null {\"a\":[1,2]} true"
        ]
    );
}

#[test]
fn another_origins_response_is_opaque_refused_or_shown_by_cors() {
    let other = TestServer::plain(|request: &ServerRequest| match request.target.as_str() {
        "/open" => Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/plain"),
                ("X-Custom", "c"),
                ("Access-Control-Allow-Origin", "*"),
            ],
            b"open",
        ),
        _ => Reply::with_fields(200, &[("Content-Type", "text/plain")], b"secret"),
    });
    let at = |path: &str| other.url(path).to_string();
    let script = format!(
        "fetch('{closed}', {{mode: 'no-cors'}}).then(r => r.text().then(t => console.log(r.type, r.status, r.url === '', [...r.headers].length, JSON.stringify(t))));\
         fetch('{closed}').then(() => console.log('read'), e => console.log(e.name, e.message));\
         fetch('{open}').then(r => r.text().then(t => console.log(r.type, [...r.headers].map(h => h[0]).join(), t)));\
         fetch('{closed}', {{mode: 'same-origin'}}).catch(e => console.log('same-origin', e.message));",
        closed = at("/closed"),
        open = at("/open"),
    );
    let (mut tab, _server) = page_with(&script, |_| None);
    let mut lines = logged(&mut tab, 6);
    lines.sort();
    // The console says why; the page learns only that it failed.
    let closed = at("/closed");
    assert_eq!(
        lines,
        [
            format!("Failed to fetch {closed}: refused by CORS: no Access-Control-Allow-Origin"),
            format!("Failed to fetch {closed}: refused: a same-origin request to another origin"),
            "TypeError Failed to fetch".to_owned(),
            "cors content-length,content-type open".to_owned(),
            "opaque 0 true 0 \"\"".to_owned(),
            "same-origin Failed to fetch".to_owned(),
        ]
    );
}

#[test]
fn the_bodies_a_script_sends_go_with_their_types() {
    let (mut tab, server) = page_with(
        "const f = new FormData(); f.append('a', '1'); f.append('file', new Blob(['xy'], {type: 'text/x'}), 'f.txt');\
         Promise.all([\
           fetch('/s', {method: 'POST', body: 'text'}),\
           fetch('/u', {method: 'POST', body: new URLSearchParams('a=1 2')}),\
           fetch('/f', {method: 'POST', body: f}),\
           fetch('/b', {method: 'PUT', body: new Blob(['\\u00e9'], {type: 'x/y'})}),\
           fetch('/a', {method: 'post', body: new Uint8Array([0, 255]).buffer, headers: {'X-A': '1', Cookie: 'no', 'Sec-X': 'no'}}),\
         ]).then(() => console.log('sent'));",
        |_| Some(Reply::ok("text/plain", b"ok")),
    );
    assert_eq!(logged(&mut tab, 1), ["sent"]);
    let request = |path: &str| {
        server
            .requests()
            .into_iter()
            .find(|request| request.target == path)
            .unwrap_or_else(|| panic!("{path}"))
    };
    let typed = |path: &str| request(path).header("Content-Type").map(str::to_owned);
    assert_eq!(typed("/s").as_deref(), Some("text/plain;charset=UTF-8"));
    assert_eq!(request("/s").body, b"text");
    assert_eq!(
        typed("/u").as_deref(),
        Some("application/x-www-form-urlencoded;charset=UTF-8")
    );
    assert_eq!(request("/u").body, b"a=1+2");
    let multipart = typed("/f").unwrap();
    let boundary = multipart
        .strip_prefix("multipart/form-data; boundary=")
        .expect("multipart");
    let body = String::from_utf8(request("/f").body).unwrap();
    assert_eq!(
        body,
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n1\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"f.txt\"\r\n\
             Content-Type: text/x\r\n\r\nxy\r\n--{boundary}--\r\n"
        )
    );
    assert_eq!(
        (request("/b").method.as_str(), typed("/b").as_deref()),
        ("PUT", Some("x/y"))
    );
    assert_eq!(request("/b").body, "\u{e9}".as_bytes());
    let raw = request("/a");
    assert_eq!(
        (
            raw.method.as_str(),
            raw.body.as_slice(),
            raw.header("Content-Type")
        ),
        ("POST", &[0, 255][..], None)
    );
    assert_eq!(
        (raw.header("X-A"), raw.header("Cookie"), raw.header("Sec-X")),
        (Some("1"), None, None)
    );
}

#[test]
fn a_fetch_aborted_rejects_with_its_signals_reason_and_stops() {
    let (mut tab, server) = page_with(
        "const before = new AbortController(); before.abort();\
         fetch('/x', {signal: before.signal}).catch(e => console.log('before', e.name, e.message));\
         fetch('/x', {signal: AbortSignal.abort('why')}).catch(e => console.log('reason', e));\
         const during = new AbortController();\
         fetch('/hang', {signal: during.signal}).then(() => console.log('came'), e => console.log('during', e.name));\
         setTimeout(() => during.abort(), 50);",
        |request| match request.target.as_str() {
            "/hang" => Some(Reply::Hang),
            _ => Some(Reply::ok("text/plain", b"x")),
        },
    );
    let mut lines = logged(&mut tab, 3);
    lines.sort();
    assert_eq!(
        lines,
        [
            "before AbortError signal is aborted without reason",
            "during AbortError",
            "reason why",
        ]
    );
    // The aborted fetches went nowhere, or were stopped.
    assert!(
        server
            .requests()
            .iter()
            .all(|request| request.target != "/x")
    );
    // Nothing is left in flight: the hanging fetch was stopped (settling
    // waits for the page's icon too, and would wait for it forever).
    tab.settle();
    assert_eq!(tab.engine.network.in_flight(), 0);
}

#[test]
fn headers_requests_and_responses_hold_what_fetch_says() {
    let (mut tab, _server) = page_with(
        "const h = new Headers([['B', '1'], ['a', '2'], ['A', ' 3 '], ['set-cookie', 'x'], ['Set-Cookie', 'y']]);\
         console.log(JSON.stringify([...h]), h.get('a'), JSON.stringify(h.getSetCookie()));\
         try { h.append('a b', '1') } catch (e) { console.log(e.message) }\
         try { h.append('a', '1\\n2') } catch (e) { console.log(e.message) }\
         const q = new Request('/x?y', {method: 'post', headers: {Cookie: 'c', 'X-A': '1', Origin: 'o'}, body: 'b'});\
         console.log(q.method, q.url.endsWith('/x?y'), q.mode, q.credentials, q.redirect, q.cache, q.referrer, [...q.headers].map(e => e.join(':')).join());\
         try { new Request('/', {body: 'x'}) } catch (e) { console.log(e.message) }\
         try { new Request('/', {mode: 'no-cors', method: 'PUT'}) } catch (e) { console.log(e.message) }\
         try { new Request('/', {method: 'TRACE'}) } catch (e) { console.log(e.message) }\
         console.log([...new Request('/', {mode: 'no-cors', headers: {'X-A': '1', Accept: 'a'}}).headers].join());\
         const r = new Response('hi', {status: 201, statusText: 'Made', headers: {'Set-Cookie': 'z', 'X': '1'}});\
         console.log(r.type, r.status, r.statusText, r.ok, [...r.headers].join('|'));\
         try { new Response('x', {status: 204}) } catch (e) { console.log(e.message) }\
         try { new Response('x', {status: 99}) } catch (e) { console.log(e.name, e.message) }\
         console.log(Response.error().type, Response.error().status, Response.redirect('/x', 301).headers.get('location').endsWith('/x'));\
         const c = r.clone(); r.text().then(t => c.text().then(u => console.log(t, u, r.bodyUsed, c.bodyUsed)));\
         r.text().catch(e => console.log(e.message));\
         Response.json({k: 1}).text().then(t => console.log(t));\
         fetch('data:text/plain,hello').then(r => r.text().then(t => console.log(r.type, t)));\
         fetch('/x').then(r => { try { r.headers.set('a', 'b') } catch (e) { console.log(e.message) } });\
         fetch('about:blank').catch(e => console.log('about', e.message));\
         fetch('http://[').catch(e => console.log(e.message));",
        |_| Some(Reply::ok("text/plain", b"x")),
    );
    let lines = logged(&mut tab, 20);
    let first: Vec<&str> = lines[..12].iter().map(String::as_str).collect();
    assert_eq!(
        first,
        [
            "[[\"a\",\"2, 3\"],[\"b\",\"1\"],[\"set-cookie\",\"x\"],[\"set-cookie\",\"y\"]] 2, 3 [\"x\",\"y\"]",
            "Failed to execute 'append' on 'Headers': Invalid name",
            "Failed to execute 'append' on 'Headers': Invalid value",
            "POST true cors same-origin follow default about:client content-type:text/plain;charset=UTF-8,x-a:1",
            "Failed to construct 'Request': Request with GET/HEAD method cannot have body.",
            "Failed to construct 'Request': 'PUT' is unsupported in no-cors mode.",
            "Failed to construct 'Request': 'TRACE' HTTP method is unsupported.",
            "accept,a",
            "default 201 Made true content-type,text/plain;charset=UTF-8|x,1",
            "Failed to construct 'Response': Response with null body status cannot have body",
            "RangeError Failed to construct 'Response': The status provided (99) is outside the range [200, 599].",
            "error 0 true",
        ]
    );
    let mut rest: Vec<&str> = lines[12..].iter().map(String::as_str).collect();
    rest.sort();
    assert_eq!(
        rest,
        [
            "Failed to execute 'fetch' on 'Window': Failed to parse URL from http://[",
            "Failed to execute 'set' on 'Headers': Headers are immutable",
            "Failed to execute 'text' on 'Response': body stream already read",
            "Failed to fetch about:blank: about: URLs are not fetched",
            "about Failed to fetch",
            "basic hello",
            "hi hi true true",
            "{\"k\":1}",
        ]
    );
}

#[test]
fn an_answer_is_a_task_and_a_document_has_a_bound_on_its_fetches() {
    let (mut tab, _server) = page_with(
        "fetch('/x').then(r => console.log('answered'));",
        |request| match request.target.as_str() {
            "/hang" => Some(Reply::Hang),
            _ => Some(Reply::ok("text/plain", b"x")),
        },
    );
    // The answer came: a task, run when tasks are, not when it came.
    tab.wait_until(|engine| engine.next_task_at().is_some());
    assert!(tab.engine.take_console().is_empty());
    tab.engine.run_tasks();
    assert_eq!(tab.engine.take_console().len(), 1);
    // A thousand in flight; the next is refused.
    let limit = crate::scripts::MAX_SCRIPT_FETCHES;
    tab.engine.run_classic(&format!(
        "for (let i = 0; i < {limit}; i++) fetch('/hang');\
         fetch('/hang').catch(e => console.log(e.message));"
    ));
    assert_eq!(
        logged(&mut tab, 1),
        ["Failed to execute 'fetch' on 'Window': Too many requests in flight."]
    );
}

#[test]
fn an_answer_and_a_timer_run_in_the_order_they_are_due() {
    let server = TestServer::plain(|request: &ServerRequest| match request.target.as_str() {
        "/page.html" => Reply::ok(
            "text/html",
            b"<script>setTimeout(() => console.log('timer'), 1000);\
              fetch('/x').then(() => console.log('fetch'));</script>",
        ),
        _ => Reply::ok("text/plain", b"x"),
    });
    let mut tab = Tab::new();
    let clock = tab.hand_clock();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    tab.wait_until(|engine| engine.next_task_at() == Some(Duration::ZERO));
    tab.engine.run_tasks();
    assert_eq!(
        tab.engine
            .take_console()
            .into_iter()
            .map(|e| e.text)
            .collect::<Vec<_>>(),
        ["fetch"]
    );
    clock.advance(1000);
    tab.engine.run_tasks();
    assert_eq!(
        tab.engine
            .take_console()
            .into_iter()
            .map(|e| e.text)
            .collect::<Vec<_>>(),
        ["timer"]
    );
}

#[test]
fn a_document_on_its_way_hears_its_fetches_answer_before_it_is_shown() {
    let first = format!(
        "<script>fetch('/x').then(r => r.text()).then(t => console.log('fetched', t));</script><!--{}-->",
        "x".repeat(1500)
    );
    let rest = "<body><script>console.log('rest');</script>";
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{first}",
        first.len() + rest.len()
    );
    // The rest waits long enough for the fetch to answer on a busy
    // machine, where a whole test run can take twice its time.
    let server = TestServer::plain(
        move |request: &ServerRequest| match request.target.as_str() {
            "/page.html" => Reply::Pieces(vec![
                (Duration::ZERO, head.clone().into_bytes()),
                (Duration::from_secs(2), rest.as_bytes().to_vec()),
            ]),
            _ => Reply::ok("text/plain", b"early"),
        },
    );
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    let url = server.url("/page.html");
    tab.engine.navigate(url.clone());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(tab.engine.url() == Some(&url) && tab.engine.load_state() == &crate::LoadState::Loaded)
    {
        assert!(Instant::now() < deadline, "the page loads in time");
        tab.engine.poll_network();
        tab.engine.run_tasks();
        std::thread::sleep(Duration::from_millis(5));
    }
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(lines, ["fetched early", "rest"]);
}

#[test]
fn answers_that_came_at_one_time_run_in_the_order_they_were_asked_for() {
    let (mut tab, _server) = page_with("", |request| match request.target.as_str() {
        "/slow" => Some(Reply::After(
            Duration::from_millis(300),
            Box::new(Reply::ok("text/plain", b"slow")),
        )),
        _ => Some(Reply::ok("text/plain", b"fast")),
    });
    // A hand's clock, which does not move while both come.
    tab.engine
        .set_clock(std::sync::Arc::new(|| Duration::from_secs(1)));
    tab.engine.run_classic(
        "for (const path of ['/slow', '/fast']) fetch(path).then(r => r.text()).then(t => console.log(t));",
    );
    crate::differential::settle_network(&mut tab.engine);
    assert!(tab.engine.take_console().is_empty());
    tab.engine.run_tasks();
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(lines, ["slow", "fast"]);
}
