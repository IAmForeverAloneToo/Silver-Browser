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

#[test]
fn text_is_encoded_and_decoded_as_the_encoding_standard_says() {
    assert_eq!(
        logged(
            r#"const e = new TextEncoder();
            console.log(e.encoding, e.encode('h\u00e9\u{1F600}\ud800').join(), e.encode().length,
              JSON.stringify(e.encodeInto('h\u00e9', new Uint8Array(2))),
              JSON.stringify(e.encodeInto('\u{1F600}a', new Uint8Array(5))));
            const d = new TextDecoder();
            console.log(d.encoding, d.fatal, d.ignoreBOM, d.decode(new Uint8Array([0xef, 0xbb, 0xbf, 0x68, 0xc3, 0xa9])) === 'h\u00e9',
              new TextDecoder('utf-8', {ignoreBOM: true}).decode(new Uint8Array([0xef, 0xbb, 0xbf])) === '\ufeff',
              new TextDecoder('latin1').encoding, new TextDecoder('latin1').decode(new Uint8Array([0xe9])) === '\u00e9',
              new TextDecoder('utf-16le').decode(new Uint16Array([0xfeff, 0x68]).buffer), JSON.stringify(d.decode()));
            // A stream keeps a sequence cut between its pieces.
            const s = new TextDecoder();
            const parts = [s.decode(new Uint8Array([0xe2, 0x82]), {stream: true}), s.decode(new Uint8Array([0xac]), {stream: true}), s.decode()];
            console.log(JSON.stringify(parts) === JSON.stringify(['', '\u20ac', '']), s.decode(new Uint8Array([0xe2])) === '\ufffd');
            for (const f of [() => new TextDecoder('nope'), () => new TextDecoder('replacement'),
              () => new TextDecoder('utf-8', {fatal: true}).decode(new Uint8Array([0xff])),
              () => new TextDecoder().decode('x'), () => e.encodeInto('x', [])]) {
              try { f() } catch (x) { console.log(x.name + ': ' + x.message) } }"#
        ),
        [
            "utf-8 104,195,169,240,159,152,128,239,191,189 0 {\"read\":1,\"written\":1} {\"read\":3,\"written\":5}",
            "utf-8 false false true true windows-1252 true h \"\"",
            "true true",
            "RangeError: Failed to construct 'TextDecoder': The encoding label provided ('nope') is invalid.",
            "RangeError: Failed to construct 'TextDecoder': The encoding label provided ('replacement') is invalid.",
            "TypeError: Failed to execute 'decode' on 'TextDecoder': The encoded data was not valid.",
            "TypeError: Failed to execute 'decode' on 'TextDecoder': parameter 1 is not of type 'ArrayBuffer'.",
            "TypeError: Failed to execute 'encodeInto' on 'TextEncoder': parameter 2 is not of type 'Uint8Array'.",
        ]
    );
}

#[test]
fn a_file_reader_reads_a_blob_with_its_events_each_a_task() {
    let lines = waited(
        r#"const blob = new Blob(['h\u00e9'], {type: 'text/plain;charset=latin1'});
        const r = new FileReader();
        console.log(r.readyState, r.result, r.error, FileReader.LOADING, r.DONE);
        for (const type of ['loadstart', 'progress', 'load', 'loadend']) {
          r.addEventListener(type, e => console.log(type, r.readyState, e.loaded, e.total));
        }
        r.readAsText(blob);
        console.log('reading', r.readyState);
        try { r.readAsText(blob) } catch (e) { console.log(e.name + ': ' + e.message) }
        r.onloadend = () => {
          console.log(r.result === 'h\u00c3\u00a9');
          const all = [];
          const read = (how, then) => { const f = new FileReader(); f.onload = () => { all.push(f.result); then() }; f[how](blob) };
          read('readAsArrayBuffer', () => read('readAsDataURL', () => read('readAsBinaryString', () => {
            const t = new FileReader(); t.onload = () => { all.push(t.result);
              console.log(all[0].byteLength, all[1], all[2] === 'h\u00c3\u00a9', all[3]);
              const a = new FileReader(); a.onabort = () => console.log('abort', a.readyState, a.result);
              a.onload = () => console.log('loaded after abort'); a.readAsText(blob); a.abort();
              setTimeout(() => console.log('done'), 10);
            }; t.readAsText(new Blob([new Uint8Array([0xff, 0xfe, 0x68, 0])]), 'utf-8');
          })));
        };"#,
        11,
    );
    assert_eq!(
        lines,
        [
            "0 null null 1 2",
            "reading 1",
            "InvalidStateError: Failed to execute 'readAsText' on 'FileReader': The object is already busy reading Blobs.",
            "loadstart 1 0 3",
            "progress 1 3 3",
            "load 2 3 3",
            "loadend 2 3 3",
            "true",
            "3 data:text/plain;charset=latin1;base64,aMOp true h",
            "abort 2 null",
            "done",
        ]
    );
}

#[test]
fn the_css_namespace_answers_supports_and_escapes() {
    assert_eq!(
        logged(
            r#"console.log(CSS.supports('display', 'flex'), CSS.supports('display: grid'), CSS.supports('(color: red) and (nope)'),
              CSS.supports('selector(a > b)'), CSS.supports('DISPLAY', 'block'), CSS.supports('display', 'flex !important'),
              CSS.supports('--x', 'anything'), CSS.supports('nope', '1'), Object.prototype.toString.call(CSS));
            console.log(CSS.escape('1a b\0-') === '\\31 a\\ b\ufffd-', CSS.escape('-'), CSS.escape('-1'), CSS.escape('\x7f_a'));
            try { CSS.escape() } catch (e) { console.log(e.name + ': ' + e.message) }"#
        ),
        [
            "true true false true true false true false [object CSS]",
            "true \\- -\\31  \\7f _a",
            "TypeError: Failed to execute 'escape' on 'CSS': 1 argument required, but only 0 present.",
        ]
    );
}

#[test]
fn a_bindings_functions_say_they_are_native_code() {
    assert_eq!(
        logged(
            r#"console.log(document.createElement.toString());
            console.log(Object.getOwnPropertyDescriptor(Document.prototype, 'title').get.toString());
            console.log(String(URL), Function.prototype.toString.toString(), String(Math.max));
            console.log(String(function f(a) { return a }), String(() => 1), String(class A {}));"#
        ),
        [
            "function createElement() { [native code] }",
            "function get title() { [native code] }",
            "function URL() { [native code] } function toString() { [native code] } function max() { [native code] }",
            "function f(a) { return a } () => 1 class A {}",
        ]
    );
}

#[test]
fn a_worker_has_the_text_and_file_interfaces_and_no_css() {
    let lines = waited(
        r#"const source = `const r = new FileReader();
          r.onload = () => postMessage([new TextDecoder().decode(new TextEncoder().encode('ok')), r.result,
            typeof CSS, new URL('b', 'http://a/').href].join(' '));
          r.readAsText(new Blob(['read']));`;
        const w = new Worker(URL.createObjectURL(new Blob([source])));
        w.onmessage = e => console.log(e.data);"#,
        1,
    );
    assert_eq!(lines, ["ok read undefined http://a/b"]);
}

#[test]
fn a_media_query_list_answers_now_and_hears_each_change_in_a_frame() {
    let page: &'static str = r#"<script>
        const wide = matchMedia('(MIN-WIDTH:700px)');
        const kept = matchMedia('print');
        wide.addEventListener('change', e => console.log('change', e.media, e.matches, wide.matches, e instanceof MediaQueryListEvent));
        wide.onchange = () => console.log('onchange');
        wide.addListener(() => console.log('listener'));
        kept.addListener(() => console.log('print changed'));
        console.log(Object.prototype.toString.call(wide), wide.media, matchMedia('(nope').media, matchMedia('screen and').media);
        try { matchMedia() } catch (e) { console.log(e.name + ': ' + e.message) }
        const made = new MediaQueryListEvent('change', {media: 'x', matches: true});
        console.log(made.media, made.matches, made.isTrusted);
    </script>"#;
    let (mut tab, mut lines) = scripted(page, true);
    let frame = |tab: &mut Tab, width| {
        let now = tab.engine.now();
        tab.engine.frame_at(width, 300, crate::FrameTime::at(now));
        tab.engine
            .take_console()
            .into_iter()
            .map(|entry| entry.text)
            .collect::<Vec<_>>()
    };
    lines.extend(frame(&mut tab, 500));
    lines.extend(frame(&mut tab, 900));
    lines.extend(frame(&mut tab, 900));
    lines.extend(frame(&mut tab, 600));
    assert_eq!(
        lines,
        [
            "[object MediaQueryList] (min-width: 700px) (nope not all",
            "TypeError: Failed to execute 'matchMedia' on 'Window': 1 argument required, but only 0 present.",
            "x true false",
            "change (min-width: 700px) true true true",
            "onchange",
            "listener",
            "change (min-width: 700px) false false true",
            "onchange",
            "listener",
        ]
    );
}

#[test]
fn an_idle_callback_runs_when_the_page_is_idle_or_at_its_timeout() {
    let lines = waited(
        r#"const order = [];
        requestIdleCallback(d => order.push('idle ' + d.didTimeout + ' ' + (d.timeRemaining() > 0 && d.timeRemaining() <= 50)));
        const gone = requestIdleCallback(() => order.push('cancelled'));
        cancelIdleCallback(gone);
        setTimeout(() => order.push('timer'), 0);
        setTimeout(() => console.log(order.join(', ')), 100);
        console.log(typeof requestIdleCallback(() => {}), IdleDeadline.name);
        try { requestIdleCallback() } catch (e) { console.log(e.name + ': ' + e.message) }
        try { requestIdleCallback(1) } catch (e) { console.log(e.name + ': ' + e.message) }"#,
        4,
    );
    assert_eq!(
        lines,
        [
            "number IdleDeadline",
            "TypeError: Failed to execute 'requestIdleCallback' on 'Window': 1 argument required, but only 0 present.",
            "TypeError: Failed to execute 'requestIdleCallback' on 'Window': The callback provided as parameter 1 is not a function.",
            // After the timer due when it was asked for.
            "timer, idle false true",
        ]
    );
    // A page busy past the timeout: the timeout's task, due before the
    // idle period could start, runs it.
    let lines = waited(
        r#"requestIdleCallback(d => console.log('ran', d.didTimeout, d.timeRemaining()), {timeout: 1});
        const until = performance.now() + 20;
        while (performance.now() < until) {}"#,
        1,
    );
    assert_eq!(lines, ["ran true 0"]);
}
