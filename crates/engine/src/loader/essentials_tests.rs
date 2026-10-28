//! Gap batch A's scripting essentials (`docs/design/scripting-essentials.md`),
//! run in a page and checked by what it logs.

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

use net::test_server::Reply;

use super::script_tests::scripted;
use super::tests::load_all;
use crate::testing::*;

/// The console's lines of `script` run in a page with scripting on.
fn logged(script: &str) -> Vec<String> {
    let page: &'static str = Box::leak(format!("<script>{script}</script>").into_boxed_str());
    scripted(page, true).1
}

/// The console's lines of `script` once there are `count`, the engine's
/// tasks run as results come; fails after ten seconds.
fn waited(script: &str, count: usize) -> Vec<String> {
    let page: &'static str = Box::leak(format!("<script>{script}</script>").into_boxed_str());
    let (mut tab, mut lines) = scripted(page, true);
    let deadline = Instant::now() + Duration::from_secs(10);
    while lines.len() < count {
        assert!(Instant::now() < deadline, "logged only {lines:?}");
        std::thread::sleep(Duration::from_millis(2));
        tab.engine.poll_network();
        tab.engine.run_tasks();
        lines.extend(
            tab.engine
                .take_console()
                .into_iter()
                .map(|entry| entry.text),
        );
    }
    lines
}

/// The console's single line after `script`, failing on an error.
fn one(script: &str) -> String {
    let lines = logged(script);
    assert_eq!(lines.len(), 1, "{lines:?}");
    lines.into_iter().next().unwrap()
}

#[test]
fn a_url_gives_its_parts_and_takes_new_ones() {
    assert_eq!(
        one(
            r#"const u = new URL('b?q=1#f', 'HTTPS://u:p@Example.com:8080/a/');
            console.log(JSON.stringify([u.href, u.origin, u.protocol, u.username, u.password, u.host,
              u.hostname, u.port, u.pathname, u.search, u.hash, String(u), JSON.stringify(u)]))"#
        ),
        r##"["https://u:p@example.com:8080/a/b?q=1#f","https://example.com:8080","https:","u","p","example.com:8080","example.com","8080","/a/b","?q=1","#f","https://u:p@example.com:8080/a/b?q=1#f","\"https://u:p@example.com:8080/a/b?q=1#f\""]"##
    );
    // A setter's value the part cannot take leaves the URL as it was.
    assert_eq!(
        one(r#"const u = new URL('http://a/p');
            u.port = '81'; u.pathname = 'x y'; u.hash = 'h'; u.protocol = 'mailto'; u.host = 'b:65536';
            const one = u.href; u.protocol = 'ws'; u.username = 'me'; u.search = '?s';
            console.log(one, u.href, u.origin)"#),
        "http://b:81/x%20y#h ws://me@b:81/x%20y?s#h ws://b:81"
    );
    assert_eq!(
        logged(
            r#"for (const f of [() => new URL('nope'), () => new URL('a', 'nope'), () => new URL(),
              () => { new URL('http://a/').href = 'nope' }, () => URL.canParse()]) {
              try { f() } catch (e) { console.log(e.name + ': ' + e.message) } }
            console.log(URL.canParse('nope'), URL.canParse('a', 'http://b/'), URL.parse('nope'),
              URL.parse('a', 'http://b/').href, Object.prototype.toString.call(new URL('http://a/')))"#
        ),
        [
            "TypeError: Failed to construct 'URL': Invalid URL",
            "TypeError: Failed to construct 'URL': Invalid base URL",
            "TypeError: Failed to construct 'URL': 1 argument required, but only 0 present.",
            "TypeError: Failed to set the 'href' property on 'URL': Invalid URL",
            "TypeError: Failed to execute 'canParse' on 'URL': 1 argument required, but only 0 present.",
            "false true null http://b/a [object URL]",
        ]
    );
}

#[test]
fn a_urls_search_params_and_its_query_follow_each_other() {
    assert_eq!(
        one(
            r#"const u = new URL('http://a/?x=1&y=2#h'); const p = u.searchParams;
            p.append('z', 'a b'); const one = u.href; p.delete('x'); p.sort(); const two = u.href;
            u.search = '?k=v'; const three = [...p].join(); u.href = 'http://b/?m=n'; const four = p.get('m');
            p.delete('m'); console.log(one, two, three, four, u.href, p === u.searchParams)"#
        ),
        "http://a/?x=1&y=2&z=a+b#h http://a/?y=2&z=a+b#h k,v n http://b/ true"
    );
    // An opaque path's space before a query is kept, encoded, when the
    // query goes.
    assert_eq!(
        one(
            r#"const u = new URL('sc:a ?q#h'); u.hash = ''; const one = u.href; u.searchParams.delete('q');
            console.log(JSON.stringify([one, u.href]))"#
        ),
        r#"["sc:a%20?q","sc:a%20"]"#
    );
}

#[test]
fn a_blob_url_answers_with_its_blob_until_revoked() {
    // Each network error is said in the console too, as the reference
    // browser says it.
    let lines = waited(
        r#"const u = URL.createObjectURL(new Blob(['hello blob'], {type: 'Text/Plain'}));
            console.log(/^blob:http:\/\/127\.0\.0\.1:\d+\/[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(u));
            (async () => {
              const r = await fetch(u + '#frag');
              console.log(r.status, r.statusText, r.headers.get('content-type'), r.headers.get('content-length'), await r.text());
              const p = await fetch(u, {headers: {Range: 'bytes=6-'}});
              console.log(p.status, p.headers.get('content-range'), await p.text());
              const s = await fetch(u, {headers: {Range: 'bytes=-4'}});
              console.log(s.status, await s.text());
              for (const init of [{method: 'POST'}, {headers: {Range: 'bytes=20-'}}]) {
                await fetch(u, init).catch(e => console.log(e.name));
              }
              URL.revokeObjectURL(u);
              await fetch(u).catch(e => console.log('revoked', e.name));
              const w = new Worker(URL.createObjectURL(new Blob(['postMessage(self.origin + " " + typeof URL.createObjectURL)'])));
              console.log(await new Promise(r => w.onmessage = e => r(e.data.replace(/:\d+/, ':PORT'))));
            })()"#,
        11,
    );
    let (errors, lines): (Vec<_>, Vec<_>) = lines
        .into_iter()
        .partition(|line| line.starts_with("Failed to fetch blob:"));
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(
        errors[0].ends_with(": no blob for this blob: URL"),
        "{errors:?}"
    );
    assert_eq!(
        lines,
        [
            "true",
            "200 OK text/plain 10 hello blob",
            "206 bytes 6-9/10 blob",
            "206 blob",
            "TypeError",
            "TypeError",
            "revoked TypeError",
            "http://127.0.0.1:PORT function",
        ]
    );
    assert_eq!(
        logged(
            r#"for (const f of [() => URL.createObjectURL('x'), () => URL.createObjectURL(), () => URL.revokeObjectURL()]) {
              try { f() } catch (e) { console.log(e.name + ': ' + e.message) } }
            URL.revokeObjectURL('nope');"#
        ),
        [
            "TypeError: Failed to execute 'createObjectURL' on 'URL': Overload resolution failed.",
            "TypeError: Failed to execute 'createObjectURL' on 'URL': 1 argument required, but only 0 present.",
            "TypeError: Failed to execute 'revokeObjectURL' on 'URL': 1 argument required, but only 0 present.",
        ]
    );
}

#[test]
fn a_client_given_to_the_engine_answers_blob_urls_too() {
    let server = serve(|path| {
        match path {
        "/page.html" => Some(Reply::ok(
            "text/html",
            b"<script>const s = document.createElement('script');               s.src = URL.createObjectURL(new Blob(['console.log(\"from a blob\")']));               document.head.append(s)</script>",
        )),
        _ => None,
    }
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_network(net::Client::default());
    load_all(&mut tab, server.url("/page.html"));
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut lines = Vec::new();
    while lines.is_empty() {
        assert!(Instant::now() < deadline, "nothing logged");
        std::thread::sleep(Duration::from_millis(2));
        tab.engine.poll_network();
        tab.engine.run_tasks();
        lines.extend(
            tab.engine
                .take_console()
                .into_iter()
                .map(|entry| entry.text),
        );
    }
    assert_eq!(lines, ["from a blob"]);
}
